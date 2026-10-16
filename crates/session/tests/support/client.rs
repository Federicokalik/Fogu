//! A session under test plus the plumbing to drive it synchronously: prompts answered by a
//! closure, "wait until the screen shows X" with a timeout instead of sleeps.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use fogu_core::{EffectiveSsh, ProfileId, Size};
use fogu_session::{
    PromptKind, PromptReply, SessionHandle, SessionInput, SessionOutput, SessionRuntime, SshPaths,
    open_ssh,
};
use fogu_term::{TermHandle, WORD_SEPARATORS};

use super::prompter::Prompter;

pub const SIZE: Size = Size {
    cols: 80,
    rows: 24,
    px_w: 0,
    px_h: 0,
};
/// Generous: the waits return as soon as the condition holds.
pub const WAIT: Duration = Duration::from_secs(15);

pub struct Client<F> {
    pub rt: Arc<SessionRuntime>,
    pub session: SessionHandle,
    /// The UI's view of the terminal the session feeds.
    pub view: TermHandle,
    pub prompter: Prompter<F>,
}

impl<F: FnMut(&PromptKind) -> PromptReply> Client<F> {
    /// Opens `ssh` with `paths`; `answer` replies to every prompt.
    pub fn connect(ssh: &EffectiveSsh, paths: SshPaths, answer: F) -> Self {
        let rt = Arc::new(SessionRuntime::new().expect("runtime"));
        let term = TermHandle::new(SIZE.cols, SIZE.rows, 100, WORD_SEPARATORS);
        let view = term.share();
        let session = open_ssh(rt.handle(), ssh, paths, SIZE, term);
        Self {
            rt,
            session,
            view,
            prompter: Prompter::new(answer),
        }
    }

    /// Opens `ssh` as profile `profile` on `rt`, reusing `rt`'s live connection of that
    /// profile (`reuse_session`), like the UI does.
    pub fn connect_shared(
        rt: &Arc<SessionRuntime>,
        profile: &str,
        ssh: &EffectiveSsh,
        paths: SshPaths,
        answer: F,
    ) -> Self {
        let rt = Arc::clone(rt);
        let term = TermHandle::new(SIZE.cols, SIZE.rows, 100, WORD_SEPARATORS);
        let view = term.share();
        let profile = ProfileId(profile.to_owned());
        let session = rt.open_ssh_shared(&profile, ssh, paths, SIZE, term);
        Self {
            rt,
            session,
            view,
            prompter: Prompter::new(answer),
        }
    }

    /// Pumps events until `done` accepts one (returned) or `WAIT` passes.
    pub fn until(&mut self, done: impl FnMut(&SessionOutput) -> bool) -> Option<SessionOutput> {
        self.until_for(WAIT, done)
    }

    /// [`Self::until`] with an explicit limit.
    pub fn until_for(
        &mut self,
        limit: Duration,
        done: impl FnMut(&SessionOutput) -> bool,
    ) -> Option<SessionOutput> {
        let Self {
            rt,
            session,
            view,
            prompter,
        } = self;
        rt.handle()
            .block_on(prompter.run_until(session, view, limit, done))
    }

    /// Waits for `Exited` / `Error` and returns it.
    pub fn end(&mut self) -> SessionOutput {
        let end = self.until(|e| matches!(e, SessionOutput::Error(_) | SessionOutput::Exited(_)));
        end.unwrap_or_else(|| panic!("session did not end; screen:\n{}", self.view.dump_grid()))
    }

    /// Waits until the visible screen contains `needle`; returns whether the text appeared.
    pub fn wait_text_for(&mut self, limit: Duration, needle: &str) -> bool {
        let view = self.view.share();
        if view.dump_grid().contains(needle) {
            return true;
        }
        self.until_for(limit, |event| {
            (matches!(event, SessionOutput::Wakeup) && view.dump_grid().contains(needle))
                || matches!(event, SessionOutput::Error(_) | SessionOutput::Exited(_))
        });
        self.view.dump_grid().contains(needle)
    }

    /// Like [`Self::wait_text_for`] with [`WAIT`], but panics with the screen when it never shows.
    pub fn expect_text(&mut self, needle: &str) {
        let seen = self.wait_text_for(WAIT, needle);
        assert!(
            seen,
            "never saw {needle:?}; screen:\n{}",
            self.view.dump_grid()
        );
    }

    /// Types `text` into the session.
    /// A session that already ended drops the input silently: the caller's next wait reports why.
    pub fn send(&self, text: &str) {
        let input = SessionInput::Data(Bytes::copy_from_slice(text.as_bytes()));
        let _ended = self.rt.handle().block_on(self.session.input.send(input));
    }

    /// Resizes the terminal and tells the session, like the UI does.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.view.resize(cols, rows);
        let size = Size {
            cols,
            rows,
            px_w: 0,
            px_h: 0,
        };
        self.rt
            .handle()
            .block_on(self.session.input.send(SessionInput::Resize(size)))
            .expect("session input");
    }

    /// Asks the session to close.
    pub fn close(&self) {
        self.rt
            .handle()
            .block_on(self.session.input.send(SessionInput::Close))
            .expect("session input");
    }

    /// Waits until the remote shell is up and answering: input typed before the shell starts is
    /// dropped by the session, so the probe is re-sent until its output shows.
    pub fn wait_shell(&mut self) {
        for _ in 0..30 {
            self.send("echo ready-$((40+2))\r");
            if self.wait_text_for(Duration::from_millis(500), "ready-42") {
                return;
            }
        }
        panic!("no shell; screen:\n{}", self.view.dump_grid());
    }
}
