//! The terminal bell (BEL): a pane flash, a dot on a background tab, the taskbar flashing while
//! the window is not focused and a short sound, as `[terminal] bell` asks. Bells closer than
//! [`COALESCE`] count once, so `yes $'\a'` cannot flood the UI or the speakers.

use std::time::{Duration, Instant};

use fogu_core::config::BellMode;
use iced::{Task, window};

use crate::app::App;
use crate::message::Message;
use crate::tab::PaneId;

/// Bells closer together than this ring once.
const COALESCE: Duration = Duration::from_millis(100);
/// How long a pane flashes.
pub(crate) const FLASH: Duration = Duration::from_millis(120);

impl App {
    /// Pane `pane` rang the bell.
    pub(crate) fn on_bell(&mut self, pane: PaneId) -> Task<Message> {
        let now = Instant::now();
        if self
            .last_bell
            .is_some_and(|last| now.saturating_duration_since(last) < COALESCE)
        {
            return Task::none();
        }
        self.last_bell = Some(now);
        let modes = self.config.terminal.bell.clone();
        let mut tasks = Vec::new();
        if modes.contains(&BellMode::Visual)
            && let Some(p) = self.pane_mut(pane)
        {
            p.flash = true;
            tasks.push(self.after(FLASH, Message::BellFlashEnd(pane)));
        }
        if modes.contains(&BellMode::Badge) {
            let active = self.active;
            if let Some(tab) = self
                .tabs
                .iter_mut()
                .find(|t| t.id != active && t.panes.contains_key(&pane))
            {
                tab.bell = true;
            }
        }
        if modes.contains(&BellMode::Attention) && !self.focused {
            tasks.push(window::latest().and_then(|id| {
                window::request_user_attention(id, Some(window::UserAttention::Informational))
            }));
        }
        if modes.contains(&BellMode::Sound) {
            ring(self.config.terminal.bell_volume);
        }
        Task::batch(tasks)
    }

    /// Pane `pane`'s flash is over.
    pub(crate) fn bell_flash_end(&mut self, pane: PaneId) {
        if let Some(p) = self.pane_mut(pane) {
            p.flash = false;
        }
    }
}

/// Plays the bell sound at `volume` (0..=1) on the sound thread.
#[cfg(feature = "bell-sound")]
fn ring(volume: f32) {
    sound::ring(volume);
}

/// This build has no sound output.
#[cfg(not(feature = "bell-sound"))]
fn ring(_volume: f32) {
    tracing::debug!("bell sound requested but this build has no audio output");
}

/// The bell sound: one thread owns the audio output (opened on the first bell, kept for the
/// next ones) and plays the embedded clip at the volume each request carries.
#[cfg(feature = "bell-sound")]
mod sound {
    use std::num::NonZero;
    use std::sync::OnceLock;
    use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

    use rodio::Source as _;
    use rodio::buffer::SamplesBuffer;

    /// The clip: a short two-partial chime, mono 16-bit PCM (`assets/sounds/bell.wav`).
    const BELL_WAV: &[u8] = include_bytes!("../../../assets/sounds/bell.wav");

    /// Queues one bell; a bell while one is still queued is dropped.
    pub(super) fn ring(volume: f32) {
        static SENDER: OnceLock<Option<SyncSender<f32>>> = OnceLock::new();
        let sender = SENDER.get_or_init(|| {
            let (tx, rx) = sync_channel(1);
            std::thread::Builder::new()
                .name("fogu-bell".to_owned())
                .spawn(move || play(&rx))
                .map_err(|err| tracing::warn!(error = %err, "no bell sound thread"))
                .ok()
                .map(|_| tx)
        });
        if let Some(sender) = sender {
            // Full: a bell is already waiting, this one is not needed.
            let _queued = sender.try_send(volume.clamp(0.0, 1.0));
        }
    }

    /// The sound thread: opens the output on the first request and plays each one.
    fn play(requests: &Receiver<f32>) {
        let Some(clip) = Clip::parse(BELL_WAV) else {
            tracing::warn!("the embedded bell sound is not 16-bit PCM WAV");
            return;
        };
        let mut output = None;
        let mut failed = false;
        while let Ok(volume) = requests.recv() {
            if output.is_none() && !failed {
                match rodio::DeviceSinkBuilder::open_default_sink() {
                    Ok(mut sink) => {
                        sink.log_on_drop(false);
                        output = Some(sink);
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "no audio output for the bell");
                        failed = true;
                    }
                }
            }
            if let Some(sink) = &output {
                let source = SamplesBuffer::new(clip.channels, clip.rate, clip.samples.clone());
                sink.mixer().add(source.amplify(volume));
            }
        }
    }

    /// Decoded PCM samples.
    struct Clip {
        channels: NonZero<u16>,
        rate: NonZero<u32>,
        samples: Vec<f32>,
    }

    impl Clip {
        /// A RIFF/WAVE file's `fmt ` and `data` chunks, 16-bit PCM only.
        fn parse(wav: &[u8]) -> Option<Self> {
            let le16 = |b: &[u8], at: usize| {
                b.get(at..at + 2)
                    .and_then(|s| s.try_into().ok())
                    .map(u16::from_le_bytes)
            };
            let le32 = |b: &[u8], at: usize| {
                b.get(at..at + 4)
                    .and_then(|s| s.try_into().ok())
                    .map(u32::from_le_bytes)
            };
            if wav.get(0..4)? != b"RIFF" || wav.get(8..12)? != b"WAVE" {
                return None;
            }
            let (mut at, mut format, mut data) = (12, None, None);
            while let (Some(id), Some(len)) = (wav.get(at..at + 4), le32(wav, at + 4)) {
                let body = at + 8;
                let len = usize::try_from(len).ok()?;
                let chunk = wav.get(body..body.checked_add(len)?)?;
                match id {
                    b"fmt " => format = Some(chunk),
                    b"data" => data = Some(chunk),
                    _ => {}
                }
                // Chunks are padded to an even length.
                at = body + len + (len & 1);
            }
            let format = format?;
            let (pcm, channels, rate, bits) = (
                le16(format, 0)?,
                le16(format, 2)?,
                le32(format, 4)?,
                le16(format, 14)?,
            );
            if pcm != 1 || bits != 16 {
                return None;
            }
            let samples = data?
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.0)
                .collect();
            Some(Self {
                channels: NonZero::new(channels)?,
                rate: NonZero::new(rate)?,
                samples,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_embedded_bell_decodes() {
            let clip = Clip::parse(BELL_WAV).map(|c| (c.channels.get(), c.samples.len()));
            assert!(matches!(clip, Some((1, n)) if n > 1000), "{clip:?}");
        }
    }
}
