//! Shared helpers for the session integration tests. A test binary that needs only some of them
//! declares `mod support { pub mod prompter; }` instead of this file (no dead code).

pub mod client;
pub mod fake_server;
pub mod prompter;
pub mod sshd;
pub mod tmp;
