//! The recovery phrase that guards the Keys page's "Reveal": twelve BIP-39 words made once,
//! shown once, and kept only as an Argon2id hash in `<state dir>/reveal-gate.json`.
//!
//! This guards Fogu's own button, nothing more: the secrets stay in the system keychain, where
//! any program running as the user can read them, so that connections keep working without the
//! phrase. A secret can be revealed only if it was saved under the current phrase (its
//! "epoch"); a reset makes a new phrase and leaves older secrets unrevealable (they can still be
//! replaced or deleted).

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use fogu_core::{SecretKey, fsutil};
use ring::rand::{SecureRandom as _, SystemRandom};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

/// The English BIP-39 word list (2048 words, `trezor/python-mnemonic`, MIT).
const WORDS: &str = include_str!("../../../assets/wordlists/bip39-english.txt");
/// Words in a phrase (128 bits of entropy plus a 4-bit checksum).
pub const PHRASE_WORDS: usize = 12;
/// The gate file's name in the state directory.
pub const GATE_FILE: &str = "reveal-gate.json";
/// Argon2id cost: 64 MiB, three passes, one lane.
const MEMORY_KIB: u32 = 64 * 1024;
const PASSES: u32 = 3;
const LANES: u32 = 1;

fn words() -> Vec<&'static str> {
    WORDS
        .lines()
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .collect()
}

/// A new random phrase: 16 random bytes and the first 4 bits of their SHA-256, as 12 words.
pub fn new_phrase() -> io::Result<SecretString> {
    let mut entropy = [0_u8; 16];
    SystemRandom::new()
        .fill(&mut entropy)
        .map_err(|_| io::Error::other("no randomness for a recovery phrase"))?;
    let list = words();
    let checksum = ring::digest::digest(&ring::digest::SHA256, &entropy);
    let check = checksum.as_ref().first().copied().unwrap_or(0) >> 4;
    // 132 bits: the entropy then the 4 checksum bits, read 11 bits per word.
    let bits = |i: usize| -> u16 {
        let byte = if i < 128 {
            entropy.get(i / 8).copied().unwrap_or(0) >> (7 - i % 8) & 1
        } else {
            check >> (131 - i) & 1
        };
        u16::from(byte)
    };
    let mut phrase = Vec::with_capacity(PHRASE_WORDS);
    for w in 0..PHRASE_WORDS {
        let index = (0..11).fold(0_u16, |acc, b| acc << 1 | bits(w * 11 + b));
        phrase.push(list.get(usize::from(index)).copied().unwrap_or("abandon"));
    }
    entropy.fill(0);
    Ok(SecretString::from(phrase.join(" ")))
}

/// `phrase` in its one comparable form: lower case, single spaces.
fn normalize(phrase: &str) -> String {
    phrase
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `phrase` is 12 list words with a matching checksum (tells a typo from a wrong
/// phrase).
#[must_use]
pub fn phrase_valid(phrase: &str) -> bool {
    let list = words();
    let normalized = normalize(phrase);
    let indices: Option<Vec<u16>> = normalized
        .split(' ')
        .map(|w| {
            list.iter()
                .position(|l| *l == w)
                .and_then(|i| u16::try_from(i).ok())
        })
        .collect();
    let Some(indices) = indices.filter(|i| i.len() == PHRASE_WORDS) else {
        return false;
    };
    let bit = |i: usize| -> u8 {
        let word = indices.get(i / 11).copied().unwrap_or(0);
        u8::from(word >> (10 - i % 11) & 1 == 1)
    };
    let mut entropy = [0_u8; 16];
    for (i, byte) in entropy.iter_mut().enumerate() {
        *byte = (0..8).fold(0, |acc, b| acc << 1 | bit(i * 8 + b));
    }
    let check = (128..132).fold(0_u8, |acc, i| acc << 1 | bit(i));
    let digest = ring::digest::digest(&ring::digest::SHA256, &entropy);
    digest.as_ref().first().copied().unwrap_or(0) >> 4 == check
}

/// What the gate file holds: the hash of the phrase and, per stored secret, the epoch it was
/// saved under. Never a secret, never the phrase.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct GateState {
    version: u32,
    /// Bumped by every reset.
    epoch: u32,
    /// Hex salt of the hash.
    salt: String,
    /// Hex Argon2id hash of the normalized phrase.
    hash: String,
    /// SHA-256 (hex) of each secret's lookup attributes → the epoch it was saved under.
    secrets: BTreeMap<String, u32>,
}

/// The recovery phrase's hash and the secrets it can reveal.
#[derive(Debug)]
pub struct RevealGate {
    path: PathBuf,
    state: GateState,
}

impl RevealGate {
    /// The gate at `path`; `None` when no phrase was made yet.
    pub fn load(path: &Path) -> io::Result<Option<Self>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        let state: GateState = serde_json::from_str(&text).map_err(io::Error::other)?;
        Ok(Some(Self {
            path: path.to_path_buf(),
            state,
        }))
    }

    /// Makes the gate for `phrase` at `path`; the secrets in `adopt` (already stored) become
    /// revealable with it.
    pub fn setup(path: &Path, phrase: &SecretString, adopt: &[SecretKey]) -> io::Result<Self> {
        let mut gate = Self {
            path: path.to_path_buf(),
            state: GateState {
                version: 1,
                ..GateState::default()
            },
        };
        gate.set_phrase(phrase)?;
        for key in adopt {
            gate.state.secrets.insert(id(key), gate.state.epoch);
        }
        gate.save()?;
        Ok(gate)
    }

    /// A new phrase: the secrets in `stored` (every one saved so far) can no longer be
    /// revealed; secrets saved from now on can.
    pub fn reset(&mut self, phrase: &SecretString, stored: &[SecretKey]) -> io::Result<()> {
        for key in stored {
            self.state.secrets.insert(id(key), self.state.epoch);
        }
        self.set_phrase(phrase)?;
        self.save()
    }

    fn set_phrase(&mut self, phrase: &SecretString) -> io::Result<()> {
        let mut salt = [0_u8; 16];
        SystemRandom::new()
            .fill(&mut salt)
            .map_err(|_| io::Error::other("no randomness for the phrase's salt"))?;
        let hash = hash(phrase, &salt)?;
        self.state.epoch += 1;
        self.state.salt = hex(&salt);
        self.state.hash = hex(&hash);
        Ok(())
    }

    /// Whether `phrase` is the gate's (slow on purpose: run it off the UI thread).
    #[must_use]
    pub fn verify(&self, phrase: &SecretString) -> bool {
        let (Some(salt), Some(want)) = (unhex(&self.state.salt), unhex(&self.state.hash)) else {
            return false;
        };
        let Ok(got) = hash(phrase, &salt) else {
            return false;
        };
        // Constant time: every byte is compared whatever the first difference.
        got.len() == want.len()
            && got
                .iter()
                .zip(&want)
                .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }

    /// `key` was just stored: it is revealable under the current phrase.
    pub fn record(&mut self, key: &SecretKey) -> io::Result<()> {
        self.state.secrets.insert(id(key), self.state.epoch);
        self.save()
    }

    /// `key` was deleted.
    pub fn forget(&mut self, key: &SecretKey) -> io::Result<()> {
        if self.state.secrets.remove(&id(key)).is_some() {
            self.save()?;
        }
        Ok(())
    }

    /// Whether `key` was stored under the current phrase: recorded with it, or saved somewhere
    /// else (the profile editor, a login prompt) since the last reset.
    #[must_use]
    pub fn revealable(&self, key: &SecretKey) -> bool {
        self.state
            .secrets
            .get(&id(key))
            .is_none_or(|epoch| *epoch == self.state.epoch)
    }

    /// How many phrases there have been (1 = the first).
    #[must_use]
    pub fn epoch(&self) -> u32 {
        self.state.epoch
    }

    fn save(&self) -> io::Result<()> {
        let text = serde_json::to_vec_pretty(&self.state).map_err(io::Error::other)?;
        if let Some(dir) = self.path.parent() {
            fsutil::create_private_dir(dir)?;
        }
        fsutil::write_private_atomic(&self.path, &text)
    }
}

/// Argon2id of the normalized `phrase` with `salt`.
fn hash(phrase: &SecretString, salt: &[u8]) -> io::Result<[u8; 32]> {
    let params = Params::new(MEMORY_KIB, PASSES, LANES, Some(32)).map_err(io::Error::other)?;
    let mut out = [0_u8; 32];
    let mut normalized = normalize(phrase.expose_secret());
    let result = Argon2::new(Algorithm::Argon2id, Version::V0x13, params).hash_password_into(
        normalized.as_bytes(),
        salt,
        &mut out,
    );
    secrecy::zeroize::Zeroize::zeroize(&mut normalized);
    result.map_err(io::Error::other)?;
    Ok(out)
}

/// The name a secret is filed under in the gate: SHA-256 of its lookup attributes.
fn id(key: &SecretKey) -> String {
    use std::fmt::Write as _;
    let text = key
        .attributes()
        .iter()
        .fold(String::new(), |mut text, (k, v)| {
            let _ = writeln!(text, "{k}={v}");
            text
        });
    hex(ring::digest::digest(&ring::digest::SHA256, text.as_bytes()).as_ref())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn the_bip39_vector_and_new_phrases_check_out() {
        // All-zero entropy (BIP-39 test vector).
        assert!(phrase_valid(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
             abandon about"
        ));
        assert!(!phrase_valid(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
             abandon abandon"
        ));
        let phrase = new_phrase().unwrap();
        assert_eq!(phrase.expose_secret().split(' ').count(), PHRASE_WORDS);
        assert!(phrase_valid(&phrase.expose_secret().to_uppercase()));
    }

    #[test]
    fn a_reset_leaves_older_secrets_unrevealable() {
        let dir = std::env::temp_dir().join(format!("fogu-gate-{}", std::process::id()));
        let path = dir.join(GATE_FILE);
        let key = SecretKey::KeyPassphrase {
            key_fingerprint: "SHA256:x".to_owned(),
        };
        let first = new_phrase().unwrap();
        let mut gate = RevealGate::setup(&path, &first, std::slice::from_ref(&key)).unwrap();
        assert!(gate.verify(&first));
        assert!(gate.revealable(&key));
        let second = new_phrase().unwrap();
        gate.reset(&second, std::slice::from_ref(&key)).unwrap();
        let gate = RevealGate::load(&path).unwrap().unwrap();
        assert!(!gate.verify(&first));
        assert!(gate.verify(&second));
        assert!(!gate.revealable(&key));
        assert_eq!(gate.epoch(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
