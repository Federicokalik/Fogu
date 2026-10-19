//! Stored passwords and key passphrases on Windows: the user's Credential Manager through the
//! safe `keyring` API, never a file. The public surface is identical to the Linux Secret Service
//! backend; only the storage differs.
//!
//! Failure is visible and never silent: a locked or unavailable Credential Manager is an
//! `Err(SecretError)` whose text the caller shows, and the caller asks the user instead. There
//! is no fallback storage. Secrets exist only as [`SecretString`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use fogu_core::SecretKey;
use keyring::Entry;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Handle;
use tokio::sync::OnceCell;

use crate::prompt::{PROMPT_TEXT_MAX_CHARS, sanitize_remote};

/// How long one Credential Manager call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(15);
/// Why a stored secret could not be read, written or removed (sanitized, never a secret).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SecretError(String);

impl SecretError {
    fn new(text: impl std::fmt::Display) -> Self {
        Self(sanitize_remote(
            &text.to_string(),
            PROMPT_TEXT_MAX_CHARS,
            false,
        ))
    }
}

/// Where secrets are kept.
#[derive(Debug)]
pub enum SecretStore {
    /// The current user's Windows Credential Manager.
    Windows,
    /// No store; the text says why (shown to the user).
    Disabled(String),
    /// Tests only.
    #[cfg(any(test, feature = "test-store"))]
    InMemory(std::sync::Mutex<BTreeMap<String, SecretString>>),
}

impl SecretStore {
    /// The Credential Manager is always present on Windows; failures surface on first use.
    pub async fn detect() -> Self {
        Self::Windows
    }

    /// The test store.
    #[cfg(any(test, feature = "test-store"))]
    #[must_use]
    pub fn in_memory() -> Self {
        Self::InMemory(std::sync::Mutex::default())
    }

    /// The stored secret, if any.
    pub async fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        match self {
            Self::Windows => {
                let id = id(key);
                match call(id, |entry| entry.get_password()).await {
                    Ok(text) => Ok(Some(SecretString::from(text))),
                    Err(err) if err.to_string().contains("No matching entry") => Ok(None),
                    Err(err) => Err(SecretError::new(err)),
                }
            }
            Self::Disabled(reason) => Err(SecretError::new(reason)),
            #[cfg(any(test, feature = "test-store"))]
            Self::InMemory(map) => Ok(map.lock().ok().and_then(|m| m.get(&id(key)).cloned())),
        }
    }

    /// Whether a secret is stored (nothing is read out of the Credential Manager).
    pub async fn exists(&self, key: &SecretKey) -> Result<bool, SecretError> {
        match self {
            Self::Windows => match self.get(key).await {
                Ok(Some(_)) => Ok(true),
                Ok(None) => Ok(false),
                Err(err) => Err(err),
            },
            Self::Disabled(reason) => Err(SecretError::new(reason)),
            #[cfg(any(test, feature = "test-store"))]
            Self::InMemory(map) => Ok(map.lock().is_ok_and(|m| m.contains_key(&id(key)))),
        }
    }

    /// Stores `value`, replacing what `key` held.
    pub async fn set(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
        match self {
            Self::Windows => {
                let id = id(key);
                let secret = value.expose_secret().to_owned();
                call(id, move |entry| entry.set_password(&secret))
                    .await
                    .map_err(SecretError::new)
            }
            Self::Disabled(reason) => Err(SecretError::new(reason)),
            #[cfg(any(test, feature = "test-store"))]
            Self::InMemory(map) => {
                if let Ok(mut map) = map.lock() {
                    map.insert(id(key), value.clone());
                }
                Ok(())
            }
        }
    }

    /// Removes the secret (nothing stored is not an error).
    pub async fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        match self {
            Self::Windows => {
                let id = id(key);
                match call(id, |entry| entry.delete_credential()).await {
                    Ok(()) => Ok(()),
                    Err(err) if err.to_string().contains("No matching entry") => Ok(()),
                    Err(err) => Err(SecretError::new(err)),
                }
            }
            Self::Disabled(reason) => Err(SecretError::new(reason)),
            #[cfg(any(test, feature = "test-store"))]
            Self::InMemory(map) => {
                if let Ok(mut map) = map.lock() {
                    map.remove(&id(key));
                }
                Ok(())
            }
        }
    }
}

/// A stable Credential Manager target name from the key's attributes. No secret is included.
fn id(key: &SecretKey) -> String {
    let attributes = key.attributes();
    let mut out = String::from("fogu:");
    for (name, value) in attributes {
        out.push_str(name);
        out.push('=');
        out.push_str(&value);
        out.push(';');
    }
    out
}

/// Runs one blocking Credential Manager call on the blocking pool.
async fn call(
    target: String,
    operation: impl FnOnce(&Entry) -> keyring::Result<()> + Send + 'static,
) -> Result<(), String> {
    let task = tokio::task::spawn_blocking(move || {
        let entry = Entry::new("fogu", &target).map_err(|err| err.to_string())?;
        operation(&entry).map_err(|err| err.to_string())
    });
    match tokio::time::timeout(CALL_TIMEOUT, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => Err(format!("Credential Manager task failed: {err}")),
        Err(_elapsed) => Err("Credential Manager did not answer in time".to_owned()),
    }
}

/// Runs a blocking call that returns a secret string.
async fn get_secret(target: String) -> Result<String, String> {
    let task = tokio::task::spawn_blocking(move || {
        let entry = Entry::new("fogu", &target).map_err(|err| err.to_string())?;
        entry.get_password().map_err(|err| err.to_string())
    });
    match tokio::time::timeout(CALL_TIMEOUT, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => Err(format!("Credential Manager task failed: {err}")),
        Err(_elapsed) => Err("Credential Manager did not answer in time".to_owned()),
    }
}

/// The session runtime's secret store: connected on first use, called on the session runtime.
#[derive(Clone, Debug)]
pub struct Secrets {
    rt: Handle,
    store: Arc<OnceCell<SecretStore>>,
}

impl Secrets {
    /// The Windows Credential Manager, used when first needed.
    #[must_use]
    pub fn new(rt: Handle) -> Self {
        Self {
            rt,
            store: Arc::new(OnceCell::new()),
        }
    }

    /// A fixed `store` (tests, `Disabled`).
    #[must_use]
    pub fn with_store(rt: Handle, store: SecretStore) -> Self {
        Self {
            rt,
            store: Arc::new(OnceCell::new_with(Some(store))),
        }
    }

    /// Runs `fut` on the session runtime, whichever executor awaits the result.
    async fn spawn<T: Send + 'static>(
        &self,
        fut: impl Future<Output = Result<T, SecretError>> + Send + 'static,
    ) -> Result<T, SecretError> {
        self.rt
            .spawn(fut)
            .await
            .unwrap_or_else(|_| Err(SecretError::new("the secret store task failed")))
    }

    /// See [`SecretStore::get`].
    pub async fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        let (store, key) = (Arc::clone(&self.store), key.clone());
        let fut = async move {
            let store = store.get_or_init(SecretStore::detect).await;
            match store {
                SecretStore::Windows => match get_secret(id(&key)).await {
                    Ok(text) => Ok(Some(SecretString::from(text))),
                    Err(err) if err.contains("No matching entry") => Ok(None),
                    Err(err) => Err(SecretError::new(err)),
                },
                other => other.get(&key).await,
            }
        };
        self.spawn(fut).await
    }

    /// See [`SecretStore::exists`].
    pub async fn exists(&self, key: &SecretKey) -> Result<bool, SecretError> {
        let (store, key) = (Arc::clone(&self.store), key.clone());
        let fut = async move {
            store
                .get_or_init(SecretStore::detect)
                .await
                .exists(&key)
                .await
        };
        self.spawn(fut).await
    }

    /// See [`SecretStore::set`].
    pub async fn set(&self, key: &SecretKey, value: SecretString) -> Result<(), SecretError> {
        let (store, key) = (Arc::clone(&self.store), key.clone());
        let fut = async move {
            let store = store.get_or_init(SecretStore::detect).await;
            store.set(&key, &value).await
        };
        self.spawn(fut).await
    }

    /// See [`SecretStore::delete`].
    pub async fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        let (store, key) = (Arc::clone(&self.store), key.clone());
        let fut = async move {
            store
                .get_or_init(SecretStore::detect)
                .await
                .delete(&key)
                .await
        };
        self.spawn(fut).await
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn key(user: &str) -> SecretKey {
        SecretKey::SshPassword {
            user: user.into(),
            host: "h".into(),
            port: 22,
        }
    }

    #[tokio::test]
    async fn in_memory_round_trip_and_disabled_errors() {
        let store = SecretStore::in_memory();
        assert!(
            store
                .get(&key("a"))
                .await
                .unwrap()
                .map(|s| s.expose_secret().to_owned())
                .is_none()
        );
        store
            .set(&key("a"), &SecretString::from("one"))
            .await
            .unwrap();
        store
            .set(&key("a"), &SecretString::from("two"))
            .await
            .unwrap();
        assert!(store.exists(&key("a")).await.unwrap());
        assert!(!store.exists(&key("b")).await.unwrap());
        assert_eq!(
            store
                .get(&key("a"))
                .await
                .unwrap()
                .unwrap()
                .expose_secret(),
            "two"
        );
        store.delete(&key("a")).await.unwrap();
        assert!(!store.exists(&key("a")).await.unwrap());

        let off = SecretStore::Disabled("no store".into());
        assert_eq!(
            off.get(&key("a")).await.unwrap_err().to_string(),
            "no store"
        );
    }

    #[test]
    fn ids_are_stable_and_never_contain_a_secret() {
        let a = id(&key("alice"));
        let b = id(&key("alice"));
        assert_eq!(a, b);
        assert!(a.starts_with("fogu:"));
        assert!(!a.contains("hunter2"));
        assert_ne!(a, id(&key("bob")));
    }
}
