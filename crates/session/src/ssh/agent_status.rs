//! Whether an SSH agent answers and which keys it holds, for the Keys page.

use std::path::PathBuf;
use std::time::Duration;

use russh::keys::HashAlg;
use russh::keys::agent::AgentIdentity;

/// How long the agent may take to list its keys.
const TIMEOUT: Duration = Duration::from_secs(2);

/// A key the agent holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentKey {
    /// The key type (`ssh-ed25519`, …; certificates say so).
    pub algorithm: String,
    /// `SHA256:…`.
    pub fingerprint: String,
    /// The comment the key was added with.
    pub comment: String,
}

/// What the agent answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    /// No agent, or it did not answer (why).
    Unavailable(String),
    /// The agent at `endpoint` holds `keys`.
    Ready {
        /// Its socket or named pipe.
        endpoint: PathBuf,
        /// Its keys.
        keys: Vec<AgentKey>,
    },
}

/// Asks the agent Fogu would use (`SSH_AUTH_SOCK`, or the Windows OpenSSH service's pipe) for
/// its keys.
pub async fn agent_status() -> AgentStatus {
    let Some((endpoint, mut agent)) = super::agent::find(None).await else {
        return AgentStatus::Unavailable(if cfg!(windows) {
            "the OpenSSH Authentication Agent service is not running".to_owned()
        } else {
            "no agent: SSH_AUTH_SOCK is not set or nothing listens on it".to_owned()
        });
    };
    match tokio::time::timeout(TIMEOUT, agent.request_identities()).await {
        Ok(Ok(identities)) => AgentStatus::Ready {
            endpoint,
            keys: identities.iter().map(key).collect(),
        },
        Ok(Err(err)) => AgentStatus::Unavailable(format!("the agent failed: {err}")),
        Err(_) => AgentStatus::Unavailable("the agent did not answer".to_owned()),
    }
}

fn key(identity: &AgentIdentity) -> AgentKey {
    match identity {
        AgentIdentity::PublicKey { key, comment } => AgentKey {
            algorithm: key.algorithm().to_string(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            comment: comment.clone(),
        },
        AgentIdentity::Certificate {
            certificate,
            comment,
        } => AgentKey {
            algorithm: format!("{} certificate", certificate.algorithm()),
            fingerprint: certificate
                .public_key()
                .fingerprint(HashAlg::Sha256)
                .to_string(),
            comment: comment.clone(),
        },
    }
}
