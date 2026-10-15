//! The interactive shell channel: `pty-req` → `env COLORTERM` → `shell`, then one `select!`
//! loop shaped like `run_local`'s: input first, pending writes in ≤ 4 KiB chunks through the
//! channel's `AsyncWrite` (which waits for window space without blocking reads), one channel
//! message per iteration into the shared [`Pump`](crate::pipeline::Pump), debounced
//! `window-change`, and the synchronized-update timer.

use std::time::Duration;

use fogu_core::{EffectiveSsh, Size};
use russh::{ChannelMsg, Disconnect, Sig};
use tokio::io::AsyncWriteExt as _;
use tokio::time::{Instant, sleep_until};

use super::connect::{Connection, ssh_error};
use super::{Stop, Ui};
use crate::pipeline::WRITE_CHUNK;
use crate::{ExitInfo, SessionInput, SessionOutput};

/// Quiet time before a resize is sent (resize storms during split drags).
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(50);

/// Opens the shell and runs it until the channel closes or the user closes the tab.
pub(crate) async fn run(
    mut conn: Connection,
    ssh: &EffectiveSsh,
    ui: &mut Ui,
    ready: Duration,
) -> Result<ExitInfo, Stop> {
    let setup = async {
        let channel = conn.handle.channel_open_session().await?;
        let size = ui.size;
        channel
            .request_pty(
                false,
                &ssh.term,
                u32::from(size.cols.max(2)),
                u32::from(size.rows.max(1)),
                u32::from(size.px_w),
                u32::from(size.px_h),
                &[],
            )
            .await?;
        // Servers without `AcceptEnv COLORTERM` ignore this; that's fine.
        if let Err(err) = channel.set_env(false, "COLORTERM", "truecolor").await {
            tracing::debug!(error = %err, "COLORTERM not sent");
        }
        channel.request_shell(false).await?;
        Ok::<_, russh::Error>(channel)
    };
    let channel = match tokio::time::timeout(ready, setup).await {
        Ok(Ok(channel)) => channel,
        Ok(Err(err)) => return Err(ssh_error(&err).into()),
        Err(_elapsed) => return Err(crate::SessionError::Timeout.into()),
    };
    conn.drain_events(ssh, ui).await?;
    let (mut reader, write_half) = channel.split();
    let mut writer = write_half.make_writer();
    let mut status = ExitInfo {
        code: None,
        signal: None,
    };
    let mut first_data = true;
    let mut resize: Option<(Size, Instant)> = None;
    let end = loop {
        let pump = &mut ui.pump;
        let sync_deadline = pump.term.sync_timeout();
        let chunk = pump.pending.len().min(WRITE_CHUNK);
        tokio::select! {
            biased;
            input = ui.in_rx.recv() => match input {
                Some(SessionInput::Data(data)) => {
                    if pump.queue_input(&data) {
                        tracing::warn!(len = data.len(), "input backlog full; dropping input");
                        pump.emit(SessionOutput::InputDropped).await?;
                    }
                }
                Some(SessionInput::Resize(size)) => resize = Some((size, Instant::now() + RESIZE_DEBOUNCE)),
                Some(SessionInput::PromptReply(id, _)) => tracing::debug!(?id, "stale prompt reply"),
                // The remote cwd comes from OSC 7 only; nothing to query locally.
                Some(SessionInput::QueryCwd) => {}
                Some(SessionInput::Close) | None => break Err(Stop::Closed),
            },
            written = writer.write(pump.pending.get(..chunk).unwrap_or_default()), if chunk > 0 => {
                match written {
                    Ok(n) => pump.written(n),
                    Err(err) => {
                        tracing::debug!(error = %err, "channel write failed; dropping pending input");
                        pump.write_failed();
                    }
                }
            }
            msg = reader.wait() => match msg {
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, ext: 1 }) => {
                    if std::mem::take(&mut first_data) && ssh.clear_service_messages_on_connect {
                        pump.term.clear();
                    }
                    pump.feed(&data).await?;
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    status.code = Some(i32::try_from(exit_status).unwrap_or(i32::MAX));
                }
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => status.signal = signal_number(&signal_name),
                Some(ChannelMsg::Close) | None => break Ok(status),
                Some(_) => {}
            },
            () = sleep_until(resize.map_or_else(Instant::now, |(_, at)| at)), if resize.is_some() => {
                if let Some((size, _)) = resize.take() {
                    window_change(&write_half, size).await;
                }
            }
            () = sleep_until(sync_deadline.map_or_else(Instant::now, Instant::from_std)),
                if sync_deadline.is_some() => pump.feed(&[]).await?,
        }
    };
    if matches!(end, Err(Stop::Closed)) {
        // Best effort: the peer may already be gone.
        let _eof = write_half.eof().await;
        let _close = write_half.close().await;
    }
    let _bye = conn
        .handle
        .disconnect(Disconnect::ByApplication, "", "en")
        .await;
    end
}

async fn window_change<S>(channel: &russh::ChannelWriteHalf<S>, size: Size)
where
    S: From<(russh::ChannelId, ChannelMsg)> + Send + Sync + 'static,
{
    let result = channel
        .window_change(
            u32::from(size.cols.max(2)),
            u32::from(size.rows.max(1)),
            u32::from(size.px_w),
            u32::from(size.px_h),
        )
        .await;
    if let Err(err) = result {
        tracing::warn!(error = %err, "SSH window-change failed");
    }
}

/// Linux signal number for an SSH `exit-signal` name (RFC 4254 § 6.10).
fn signal_number(sig: &Sig) -> Option<i32> {
    Some(match sig {
        Sig::HUP => 1,
        Sig::INT => 2,
        Sig::QUIT => 3,
        Sig::ILL => 4,
        Sig::ABRT => 6,
        Sig::FPE => 8,
        Sig::KILL => 9,
        Sig::USR1 => 10,
        Sig::SEGV => 11,
        Sig::PIPE => 13,
        Sig::ALRM => 14,
        Sig::TERM => 15,
        Sig::Custom(_) => return None,
    })
}
