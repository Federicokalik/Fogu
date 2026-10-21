//! The private keys in `~/.ssh` as the Keys page lists them (type, fingerprint, comment,
//! whether a passphrase protects them), and a new ed25519 key written next to them. A key's
//! secret half is parsed only to find those facts and never leaves this module.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use fogu_core::fsutil;
use ring::rand::{SecureRandom as _, SystemRandom};
use russh::keys::HashAlg;
use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
use russh::keys::ssh_key::rand_core::{TryCryptoRng, TryRng};
use russh::keys::ssh_key::{LineEnding, PrivateKey, PublicKey};
use secrecy::{ExposeSecret as _, SecretString};

/// Files bigger than this are not keys.
const MAX_KEY_BYTES: u64 = 64 * 1024;
/// Files in `~/.ssh` that are never keys.
const NOT_KEYS: [&str; 6] = [
    "config",
    "known_hosts",
    "known_hosts.old",
    "authorized_keys",
    "authorized_keys2",
    "environment",
];

/// A private key file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyFile {
    /// Where it is.
    pub path: PathBuf,
    /// `ssh-ed25519`, `ecdsa-sha2-nistp256`, `ssh-rsa`, … (`RSA (PEM)` and the like for
    /// formats whose public half is not inside the file).
    pub algorithm: String,
    /// `SHA256:…` when the public half is known.
    pub fingerprint: Option<String>,
    /// The key's comment.
    pub comment: String,
    /// Whether a passphrase protects it.
    pub encrypted: bool,
    /// The public key as an `authorized_keys` line, when known.
    pub public: Option<String>,
}

/// The private keys in `dir` (not recursive; `.pub` files only complete their key).
#[must_use]
pub fn scan_key_dir(dir: &Path) -> Vec<KeyFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut keys: Vec<KeyFile> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            let ext = |e: &str| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(e));
            !NOT_KEYS.contains(&name)
                && !ext("pub")
                && !ext("bak")
                && !name.starts_with("known_hosts")
        })
        .filter_map(|p| inspect(&p))
        .collect();
    keys.sort_by(|a, b| a.path.cmp(&b.path));
    keys
}

/// What `path` holds, when it is a private key.
fn inspect(path: &Path) -> Option<KeyFile> {
    let text = read_small(path)?;
    let body = text.expose_secret();
    let public_file = std::fs::read_to_string(pub_path(path))
        .ok()
        .and_then(|t| PublicKey::from_openssh(t.trim()).ok());
    if body.contains("-----BEGIN OPENSSH PRIVATE KEY-----") {
        let key = PrivateKey::from_openssh(body).ok()?;
        let public = key.public_key();
        return Some(KeyFile {
            path: path.to_path_buf(),
            algorithm: public.algorithm().to_string(),
            fingerprint: Some(public.fingerprint(HashAlg::Sha256).to_string()),
            // An encrypted key keeps its comment in the encrypted part: the `.pub` has it.
            comment: Some(public.comment().to_string())
                .filter(|c| !c.is_empty())
                .or_else(|| public_file.as_ref().map(|k| k.comment().to_string()))
                .unwrap_or_default(),
            encrypted: key.is_encrypted(),
            public: public.to_openssh().ok(),
        });
    }
    let (algorithm, encrypted) = if body.starts_with("PuTTY-User-Key-File-") {
        let kind = body
            .lines()
            .next()
            .and_then(|l| l.split_once(':'))
            .map_or("PuTTY", |(_, k)| k.trim());
        let encrypted = body
            .lines()
            .any(|l| l.starts_with("Encryption:") && !l.contains("none"));
        (format!("{kind} (PuTTY)"), encrypted)
    } else {
        let kind = pem_kind(body)?;
        let encrypted =
            body.contains("Proc-Type: 4,ENCRYPTED") || body.contains("ENCRYPTED PRIVATE KEY");
        (format!("{kind} (PEM)"), encrypted)
    };
    Some(KeyFile {
        path: path.to_path_buf(),
        algorithm,
        fingerprint: public_file
            .as_ref()
            .map(|k| k.fingerprint(HashAlg::Sha256).to_string()),
        comment: public_file
            .as_ref()
            .map(|k| k.comment().to_string())
            .unwrap_or_default(),
        encrypted,
        public: public_file.and_then(|k| k.to_openssh().ok()),
    })
}

/// The key type of a PEM private key header.
fn pem_kind(body: &str) -> Option<&'static str> {
    [
        ("-----BEGIN RSA PRIVATE KEY-----", "RSA"),
        ("-----BEGIN EC PRIVATE KEY-----", "ECDSA"),
        ("-----BEGIN DSA PRIVATE KEY-----", "DSA"),
        ("-----BEGIN PRIVATE KEY-----", "PKCS#8"),
        ("-----BEGIN ENCRYPTED PRIVATE KEY-----", "PKCS#8"),
    ]
    .into_iter()
    .find(|(header, _)| body.contains(header))
    .map(|(_, kind)| kind)
}

fn pub_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".pub");
    PathBuf::from(name)
}

fn read_small(path: &Path) -> Option<SecretString> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_KEY_BYTES {
        return None;
    }
    let mut text = String::new();
    file.take(MAX_KEY_BYTES).read_to_string(&mut text).ok()?;
    Some(SecretString::from(text))
}

/// A new key to make.
#[derive(Debug)]
pub struct KeyGen {
    /// The folder (`~/.ssh`).
    pub dir: PathBuf,
    /// The file name (`id_ed25519`); letters, digits, `.`, `_` and `-` only.
    pub name: String,
    /// The key's comment (`user@host`).
    pub comment: String,
    /// Protects the private file when set.
    pub passphrase: Option<SecretString>,
}

/// Why no key was made.
#[derive(Debug, thiserror::Error)]
pub enum KeyGenError {
    /// The name is empty or has other characters.
    #[error("a file name of letters, digits, '.', '_' and '-'")]
    BadName,
    /// A file of that name is already there.
    #[error("{0} already exists")]
    Exists(PathBuf),
    /// Making or writing the key failed.
    #[error("{0}")]
    Failed(String),
}

/// Makes an ed25519 key: `<dir>/<name>` (private, `0600` on Unix, encrypted with the
/// passphrase when one is given) and `<name>.pub`. Never replaces a file.
pub fn generate_ed25519(req: &KeyGen) -> Result<KeyFile, KeyGenError> {
    let valid = !req.name.is_empty()
        && !req.name.starts_with('.')
        && req
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if !valid {
        return Err(KeyGenError::BadName);
    }
    let private_path = req.dir.join(&req.name);
    let public_path = pub_path(&private_path);
    for path in [&private_path, &public_path] {
        if path.exists() {
            return Err(KeyGenError::Exists(path.clone()));
        }
    }
    let failed = |e: &dyn std::fmt::Display| KeyGenError::Failed(e.to_string());
    let mut seed = [0_u8; 32];
    SystemRandom::new()
        .fill(&mut seed)
        .map_err(|_| KeyGenError::Failed("no randomness for a key".to_owned()))?;
    let pair = Ed25519Keypair::from_seed(&seed);
    seed.fill(0);
    let mut key =
        PrivateKey::new(KeypairData::Ed25519(pair), req.comment.clone()).map_err(|e| failed(&e))?;
    // The public line comes from the plain key: encryption moves the comment into the secret
    // part.
    let public = key.public_key().to_openssh().map_err(|e| failed(&e))?;
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let algorithm = key.public_key().algorithm().to_string();
    if let Some(passphrase) = req
        .passphrase
        .as_ref()
        .filter(|p| !p.expose_secret().is_empty())
    {
        key = key
            .encrypt(
                &mut SystemRng(SystemRandom::new()),
                passphrase.expose_secret(),
            )
            .map_err(|e| failed(&e))?;
    }
    let private = key.to_openssh(LineEnding::LF).map_err(|e| failed(&e))?;
    fsutil::create_private_dir(&req.dir).map_err(|e| failed(&e))?;
    fsutil::write_private_atomic(&private_path, private.as_bytes()).map_err(|e| failed(&e))?;
    std::fs::write(&public_path, format!("{public}\n")).map_err(|e| failed(&e))?;
    Ok(KeyFile {
        path: private_path,
        algorithm,
        fingerprint: Some(fingerprint),
        comment: req.comment.clone(),
        encrypted: key.is_encrypted(),
        public: Some(public),
    })
}

/// The system's secure random source for the key encryption's salt and IV.
struct SystemRng(SystemRandom);

/// The system random source failed.
#[derive(Debug, thiserror::Error)]
#[error("the system random source failed")]
struct NoRandomness;

impl TryRng for SystemRng {
    type Error = NoRandomness;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        let mut bytes = [0_u8; 4];
        self.try_fill_bytes(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        let mut bytes = [0_u8; 8];
        self.try_fill_bytes(&mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        self.0.fill(dst).map_err(|_| NoRandomness)
    }
}

impl TryCryptoRng for SystemRng {}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]

    use super::*;

    #[test]
    fn a_generated_key_is_listed_and_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("fogu-keys-{}", std::process::id()));
        let req = KeyGen {
            dir: dir.clone(),
            name: "id_test".to_owned(),
            comment: "me@here".to_owned(),
            passphrase: Some(SecretString::from("correct horse")),
        };
        let made = generate_ed25519(&req).unwrap();
        assert!(made.encrypted);
        let listed = scan_key_dir(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].fingerprint, made.fingerprint);
        assert_eq!(listed[0].comment, "me@here");
        assert!(listed[0].encrypted);
        assert!(matches!(
            generate_ed25519(&req),
            Err(KeyGenError::Exists(_))
        ));
        let bad = KeyGen {
            name: "../x".to_owned(),
            ..req
        };
        assert!(matches!(generate_ed25519(&bad), Err(KeyGenError::BadName)));
        let _ = std::fs::remove_dir_all(dir);
    }
}
