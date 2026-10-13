//! Panic isolation for session tasks.

use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::{SessionError, SessionOutput};

/// Awaits a session task; a panic becomes `SessionOutput::Error(SessionError::Crashed)`.
pub async fn supervise(task: JoinHandle<()>, out_tx: Sender<SessionOutput>) {
    match task.await {
        Err(err) if err.is_panic() => {
            tracing::error!("session task panicked");
            // The UI may already be gone; nothing else to do then.
            if out_tx
                .send(SessionOutput::Error(SessionError::Crashed))
                .await
                .is_err()
            {
                tracing::debug!("crash not reported: UI receiver dropped");
            }
        }
        Err(_) | Ok(()) => {}
    }
}
