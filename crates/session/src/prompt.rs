//! Prompts a session needs answered: host-key decisions, passwords, key passphrases,
//! keyboard-interactive questions.
//!
//! [`PromptKind`] is `Clone` data that rides in `SessionOutput`/`Message`s. Its text fields that
//! come from the server (keyboard-interactive name, instructions, prompt texts) are attacker
//! input: they are sanitized and capped when built ([`sanitize_remote`]) and never printed by
//! `Debug` (lengths only). Secrets travel only UI → session, inside [`PromptReply`] as
//! [`SecretString`]s, whose `Debug` is redacted.

use std::fmt;
use std::path::PathBuf;

use secrecy::SecretString;

/// Cap for one keyboard-interactive prompt text or name (chars).
pub const PROMPT_TEXT_MAX_CHARS: usize = 256;
/// Cap for keyboard-interactive instructions (chars).
pub const PROMPT_INSTRUCTIONS_MAX_CHARS: usize = 2048;
/// More prompts than this in one keyboard-interactive request aborts that method.
pub const PROMPT_MAX_COUNT: usize = 16;

/// Identifies one prompt within a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PromptId(pub u64);

/// One keyboard-interactive question.
#[derive(Clone, PartialEq, Eq)]
pub struct KbdPrompt {
    /// Sanitized prompt text (server-chosen).
    pub text: String,
    /// `true` = the answer may be shown while typed; `false` = it is a secret.
    pub echo: bool,
}

impl fmt::Debug for KbdPrompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KbdPrompt")
            .field("text_len", &self.text.len())
            .field("echo", &self.echo)
            .finish()
    }
}

/// What the session is asking. Host and port come from the user's profile; fingerprints are
/// computed locally; everything else that comes from the server is sanitized.
#[derive(Clone, PartialEq, Eq)]
pub enum PromptKind {
    /// The server's host key is in no `known_hosts` file. Replies: `Accept` (this connection
    /// only), `AcceptAndSave` (append to our `known_hosts`), `Reject`.
    HostKeyUnknown {
        /// Host as configured.
        host: String,
        /// Port.
        port: u16,
        /// Key algorithm, e.g. `ssh-ed25519`.
        key_type: String,
        /// `SHA256:<base64, no padding>`.
        fingerprint: String,
    },
    /// The server's key differs from the recorded one: possible MITM. Only `AcceptAndSave`
    /// (the UI's explicit "replace and connect" flow) continues and replaces our record;
    /// `Accept` is treated as `Reject`.
    HostKeyChanged {
        /// Host as configured.
        host: String,
        /// Port.
        port: u16,
        /// Algorithm of the offered key.
        key_type: String,
        /// Fingerprint of the offered key.
        fingerprint: String,
        /// Fingerprint of the recorded key.
        old_fingerprint: String,
        /// File holding the recorded key.
        known_hosts_path: PathBuf,
        /// 1-based line of the recorded key in that file.
        line: usize,
    },
    /// Password for `user@host`. Reply: `Secret`.
    Password {
        /// Remote user.
        user: String,
        /// Host as configured.
        host: String,
    },
    /// Passphrase for an encrypted private key file. Reply: `Secret`.
    KeyPassphrase {
        /// The key file.
        path: PathBuf,
    },
    /// Keyboard-interactive round. Reply: `Secrets` with one answer per prompt, in order.
    KeyboardInteractive {
        /// Sanitized request name (may be empty).
        name: String,
        /// Sanitized instructions (may be empty; newlines kept).
        instructions: String,
        /// The questions.
        prompts: Vec<KbdPrompt>,
    },
}

impl fmt::Debug for PromptKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HostKeyUnknown {
                host,
                port,
                key_type,
                fingerprint,
            } => f
                .debug_struct("HostKeyUnknown")
                .field("host", host)
                .field("port", port)
                .field("key_type", key_type)
                .field("fingerprint", fingerprint)
                .finish(),
            Self::HostKeyChanged {
                host, port, line, ..
            } => f
                .debug_struct("HostKeyChanged")
                .field("host", host)
                .field("port", port)
                .field("line", line)
                .finish_non_exhaustive(),
            Self::Password { user, host } => f
                .debug_struct("Password")
                .field("user", user)
                .field("host", host)
                .finish(),
            Self::KeyPassphrase { path } => {
                f.debug_struct("KeyPassphrase").field("path", path).finish()
            }
            Self::KeyboardInteractive {
                name,
                instructions,
                prompts,
            } => f
                .debug_struct("KeyboardInteractive")
                .field("name_len", &name.len())
                .field("instructions_len", &instructions.len())
                .field("prompts", prompts)
                .finish(),
        }
    }
}

/// The user's answer to a [`PromptKind`].
pub enum PromptReply {
    /// Host key: trust for this connection only.
    Accept,
    /// Host key: trust and record in our `known_hosts` (for `HostKeyChanged`: replace).
    AcceptAndSave,
    /// Decline; the connection (or that auth method) is abandoned.
    Reject,
    /// A password or passphrase.
    Secret(SecretString),
    /// Keyboard-interactive answers, one per prompt.
    Secrets(Vec<SecretString>),
}

impl fmt::Debug for PromptReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accept => f.write_str("Accept"),
            Self::AcceptAndSave => f.write_str("AcceptAndSave"),
            Self::Reject => f.write_str("Reject"),
            Self::Secret(_) => f.write_str("Secret(<redacted>)"),
            Self::Secrets(answers) => write!(f, "Secrets(<{} redacted>)", answers.len()),
        }
    }
}

/// Makes server-supplied text safe to show: drops control characters (C0 except `\n` when
/// `keep_newlines`, DEL, C1) and bidi overrides/isolates (U+202A–202E, U+2066–2069, which can
/// make a prompt read differently than it is), then caps it at `max_chars`.
#[must_use]
pub fn sanitize_remote(text: &str, max_chars: usize, keep_newlines: bool) -> String {
    text.chars()
        .filter(|&c| (keep_newlines && c == '\n') || !c.is_control())
        .filter(|c| !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .take(max_chars)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_remote_text_or_secrets() {
        let kind = PromptKind::KeyboardInteractive {
            name: "evil-name".into(),
            instructions: "evil-instructions".into(),
            prompts: vec![KbdPrompt {
                text: "evil-prompt".into(),
                echo: false,
            }],
        };
        let debug = format!("{kind:?}");
        assert!(!debug.contains("evil"), "{debug}");
        let reply = PromptReply::Secrets(vec![SecretString::from("hunter2")]);
        let secret = PromptReply::Secret(SecretString::from("hunter2"));
        for debug in [format!("{reply:?}"), format!("{secret:?}")] {
            assert!(!debug.contains("hunter2"), "{debug}");
        }
    }

    #[test]
    fn sanitize_strips_controls_and_bidi_and_caps() {
        let text = "a\x1b[31mb\u{9b}c\u{202e}d\r\ne\u{7f}";
        assert_eq!(sanitize_remote(text, 100, true), "a[31mbcd\ne");
        assert_eq!(sanitize_remote(text, 100, false), "a[31mbcde");
        assert_eq!(sanitize_remote("abcdef", 3, false), "abc");
    }
}
