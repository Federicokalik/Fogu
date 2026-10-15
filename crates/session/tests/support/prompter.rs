//! Scripted prompt replier for SSH session tests (also reused by the sshd integration tests):
//! drives a [`SessionHandle`], answering every `SessionOutput::Prompt` from a closure and handing
//! every other event to the caller until it says stop.

use std::sync::atomic::Ordering;
use std::time::Duration;

use fogu_session::{PromptKind, PromptReply, SessionHandle, SessionInput, SessionOutput};
use fogu_term::TermHandle;

/// Answers prompts with `answer` and records which prompts were asked.
pub struct Prompter<F> {
    answer: F,
    /// Every prompt seen, in order.
    pub asked: Vec<PromptKind>,
}

impl<F: FnMut(&PromptKind) -> PromptReply> Prompter<F> {
    /// A prompter replying with `answer`.
    pub fn new(answer: F) -> Self {
        Self {
            answer,
            asked: Vec::new(),
        }
    }

    /// Pumps `session` until `done` returns true for a non-prompt event (returned), the event
    /// stream ends (`None`) or `limit` passes (`None`). `Wakeup`s clear `view`'s dirty flag like
    /// the UI does.
    pub async fn run_until(
        &mut self,
        session: &mut SessionHandle,
        view: &TermHandle,
        limit: Duration,
        mut done: impl FnMut(&SessionOutput) -> bool,
    ) -> Option<SessionOutput> {
        let pump = async {
            while let Some(event) = session.events.recv().await {
                match event {
                    SessionOutput::Prompt(id, kind) => {
                        let reply = (self.answer)(&kind);
                        self.asked.push(kind);
                        if session
                            .input
                            .send(SessionInput::PromptReply(id, reply))
                            .await
                            .is_err()
                        {
                            return None;
                        }
                    }
                    SessionOutput::Wakeup => view.dirty().store(false, Ordering::Release),
                    other if done(&other) => return Some(other),
                    _ => {}
                }
            }
            None
        };
        tokio::time::timeout(limit, pump).await.ok().flatten()
    }
}
