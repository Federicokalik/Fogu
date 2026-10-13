//! Side-channel OSC scanner: `alacritty_terminal`/`vte` parse and drop OSC 7, 133, 9, 9;4, 777 and
//! 1337, so nothing downstream ever sees them. [`OscScanner`] watches the same bytes the caller
//! also feeds to [`crate::TermHandle::feed`] *without* consuming or altering them, and reports the
//! handful of codes below as [`OscEvent`]s.
//!
//! Stateful across calls (a sequence split between two reads is still recognised) and bounded: a
//! sequence longer than [`MAX_SEQUENCE`] is discarded and the scanner resynchronises on the next
//! `ESC` (an unbounded OSC scan is a memory `DoS`).

use std::path::PathBuf;

use crate::event_proxy::sanitize_title;

/// Longest OSC payload (the bytes between `ESC ]` and its terminator) kept; a longer run is
/// discarded without producing an event.
const MAX_SEQUENCE: usize = 4096;

/// Non-consuming OSC side-channel scanner. See the module docs.
#[derive(Debug, Default)]
pub struct OscScanner {
    state: State,
    buf: Vec<u8>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    /// Saw `ESC`; `]` starts a payload, anything else drops back to ground.
    Escape,
    /// Buffering payload bytes until `BEL` or `ESC \` (ST).
    Payload,
    /// Saw `ESC` while buffering; `\` confirms ST, anything else abandons this sequence.
    PayloadEsc,
    /// Payload exceeded [`MAX_SEQUENCE`]; discarding bytes until a terminator, no event.
    Overflow,
    /// Saw `ESC` while discarding; `\` confirms ST, anything else keeps discarding.
    OverflowEsc,
}

/// A decoded side-channel event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OscEvent {
    /// OSC 7 (`file://host/path`, host kept verbatim, path percent-decoded) or OSC 1337
    /// (`CurrentDir=<path>`, no host field).
    Cwd {
        /// Empty for OSC 1337 and for an OSC 7 URI with no host.
        host: String,
        /// Percent-decoded path, exactly as reported.
        path: PathBuf,
    },
    /// OSC 133 FinalTerm-style semantic prompt mark.
    Prompt(PromptMark),
    /// OSC 9 (plain text, no title) or OSC 777 (`notify;title;body`) desktop notification.
    Notify {
        /// Present only for OSC 777.
        title: Option<String>,
        /// Sanitized (printable-only, capped) body text.
        body: String,
    },
    /// OSC 9;4 progress. `None` = no/indeterminate progress; `Some(pct)` is 0..=100.
    Progress(Option<u8>),
}

/// FinalTerm/VSCode-style semantic prompt marks (OSC 133), used by shell integration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptMark {
    /// `A`: about to print the prompt.
    PromptStart,
    /// `B`: prompt printed; the user is typing a command.
    CommandStart,
    /// `C`: command submitted; its output follows.
    OutputStart,
    /// `D[;exit]`: command finished, with the shell-reported exit code when given.
    CommandFinished {
        /// Exit code of the command that just finished, when the shell reported one.
        exit_code: Option<i32>,
    },
}

impl OscScanner {
    /// A fresh scanner in ground state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Scans `bytes` (the same bytes the caller also feeds to `Term::feed`) and returns every
    /// side-channel event found. Stateful across calls.
    pub fn scan(&mut self, bytes: &[u8]) -> Vec<OscEvent> {
        let mut events = Vec::new();
        for &byte in bytes {
            self.step(byte, &mut events);
        }
        events
    }

    fn step(&mut self, byte: u8, events: &mut Vec<OscEvent>) {
        match self.state {
            State::Ground | State::Escape => self.on_escape_candidate(byte),
            State::Payload => self.on_payload(byte, events),
            State::PayloadEsc => self.on_payload_esc(byte, events),
            State::Overflow => self.on_overflow(byte),
            State::OverflowEsc => self.on_overflow_esc(byte),
        }
    }

    /// Shared transition for "not currently inside a payload": from ground, or after abandoning
    /// one. `ESC` arms the next byte, `ESC ]` starts buffering, anything else is ignored.
    fn on_escape_candidate(&mut self, byte: u8) {
        self.state = match (self.state, byte) {
            (State::Escape, b']') => {
                self.buf.clear();
                State::Payload
            }
            (_, 0x1B) => State::Escape,
            _ => State::Ground,
        };
    }

    fn on_payload(&mut self, byte: u8, events: &mut Vec<OscEvent>) {
        match byte {
            0x07 => self.finish(events),
            0x1B => self.state = State::PayloadEsc,
            _ if self.buf.len() >= MAX_SEQUENCE => self.state = State::Overflow,
            _ => self.buf.push(byte),
        }
    }

    fn on_payload_esc(&mut self, byte: u8, events: &mut Vec<OscEvent>) {
        if byte == b'\\' {
            self.finish(events);
        } else {
            self.buf.clear();
            self.state = State::Escape;
            self.on_escape_candidate(byte);
        }
    }

    fn on_overflow(&mut self, byte: u8) {
        match byte {
            0x07 => self.state = State::Ground,
            0x1B => self.state = State::OverflowEsc,
            _ => {}
        }
    }

    fn on_overflow_esc(&mut self, byte: u8) {
        self.state = match byte {
            b'\\' => State::Ground,
            0x1B => State::OverflowEsc,
            _ => State::Overflow,
        };
    }

    fn finish(&mut self, events: &mut Vec<OscEvent>) {
        if let Some(event) = parse(&self.buf) {
            events.push(event);
        }
        self.buf.clear();
        self.state = State::Ground;
    }
}

/// Parses one complete OSC payload (the bytes between `ESC ]` and its terminator, terminator not
/// included). `None` for anything not recognised or malformed; never panics on arbitrary bytes.
fn parse(buf: &[u8]) -> Option<OscEvent> {
    let text = std::str::from_utf8(buf).ok()?;
    let (code, rest) = text.split_once(';').unwrap_or((text, ""));
    match code.parse::<u32>().ok()? {
        7 => parse_cwd_uri(rest),
        133 => parse_prompt(rest),
        9 => parse_nine(rest),
        777 => parse_notify(rest),
        1337 => parse_current_dir(rest),
        _ => None,
    }
}

/// `file://host/path` (host may be empty); percent-decodes the path, keeps the host verbatim.
fn parse_cwd_uri(rest: &str) -> Option<OscEvent> {
    let uri = rest.strip_prefix("file://")?;
    let (host, path) = uri.split_once('/').unwrap_or((uri, ""));
    Some(OscEvent::Cwd {
        host: host.to_owned(),
        path: PathBuf::from(format!("/{}", percent_decode(path))),
    })
}

/// `CurrentDir=<path>` (OSC 1337); no host field, so the host is empty (local).
fn parse_current_dir(rest: &str) -> Option<OscEvent> {
    let path = rest.strip_prefix("CurrentDir=")?;
    Some(OscEvent::Cwd {
        host: String::new(),
        path: PathBuf::from(percent_decode(path)),
    })
}

/// `A` | `B` | `C` | `D[;exit]` (OSC 133).
fn parse_prompt(rest: &str) -> Option<OscEvent> {
    let (mark, exit) = rest
        .split_once(';')
        .map_or((rest, None), |(mark, exit)| (mark, exit.parse().ok()));
    let mark = match mark {
        "A" => PromptMark::PromptStart,
        "B" => PromptMark::CommandStart,
        "C" => PromptMark::OutputStart,
        "D" => PromptMark::CommandFinished { exit_code: exit },
        _ => return None,
    };
    Some(OscEvent::Prompt(mark))
}

/// `<text>` (plain notify) or `4;<state>[;<pct>]` (progress) (OSC 9).
fn parse_nine(rest: &str) -> Option<OscEvent> {
    if rest == "4" {
        return Some(OscEvent::Progress(None));
    }
    if let Some(progress) = rest.strip_prefix("4;") {
        return parse_progress(progress);
    }
    Some(OscEvent::Notify {
        title: None,
        body: sanitize_title(rest),
    })
}

/// `<state>[;<pct>]`: state 0/3 (remove/indeterminate) carry no number, 1/2/4 (set/error/paused)
/// show `pct` (clamped 0..=100) when the sender gave one.
fn parse_progress(rest: &str) -> Option<OscEvent> {
    let (state, pct) = rest
        .split_once(';')
        .map_or((rest, None), |(state, pct)| (state, pct.parse::<u8>().ok()));
    let value = match state {
        "0" | "3" => None,
        "1" | "2" | "4" => pct.map(|pct| pct.min(100)),
        _ => return None,
    };
    Some(OscEvent::Progress(value))
}

/// `notify;<title>;<body>` (OSC 777, rxvt-unicode/urxvt extension).
fn parse_notify(rest: &str) -> Option<OscEvent> {
    let rest = rest.strip_prefix("notify;")?;
    let (title, body) = rest.split_once(';').unwrap_or((rest, ""));
    Some(OscEvent::Notify {
        title: Some(sanitize_title(title)),
        body: sanitize_title(body),
    })
}

/// Decodes `%XX` escapes; bytes that don't form a valid hex pair pass through unchanged. Invalid
/// UTF-8 in the result is replaced (paths are attacker input; never panics).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        let pair = (byte == b'%').then(|| {
            bytes
                .get(i + 1)
                .copied()
                .zip(bytes.get(i + 2).copied())
                .and_then(|(high, low)| hex_digit(high).zip(hex_digit(low)))
        });
        if let Some((high, low)) = pair.flatten() {
            out.push(high * 16 + low);
            i += 3;
        } else {
            out.push(byte);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[path = "osc_tests.rs"]
mod tests;
