//! Host-key verification against OpenSSH-format `known_hosts` files: our own
//! `$XDG_CONFIG_HOME/fogu/known_hosts` (read + write, `0600`) first, then `~/.ssh/known_hosts`
//! (read-only).
//!
//! Own line parser over `ssh_key`'s `Entry` instead of `russh::keys::known_hosts`: russh's helper
//! fails the whole file on one bad line, splits on single spaces, miscounts line numbers after
//! comments and ignores `@revoked`; we need exact line numbers for the "changed" screen.
//! Matching follows OpenSSH: `host` for port 22, `[host]:port` otherwise; comma-separated
//! patterns with `*`/`?` globs and `!` negation, case-insensitive; hashed `|1|salt|hash` entries
//! via HMAC-SHA1. Only `known_hosts` lines are read here: never the key material of the user.

use std::fs::OpenOptions;
use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use fogu_core::fsutil;
use russh::keys::ssh_key::known_hosts::{Entry, HostPatterns, Marker};
use russh::keys::{HashAlg, PublicKey};

/// `known_hosts` files bigger than this are ignored (with a warning) instead of read.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Result of looking a server key up.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Lookup {
    /// A recorded key for this host equals the offered one.
    Known,
    /// Nothing recorded for this host.
    Unknown,
    /// Keys are recorded for this host, none equal to the offered one.
    Changed {
        /// File of the first conflicting record.
        path: PathBuf,
        /// Its 1-based line.
        line: usize,
        /// The recorded key.
        old: Box<PublicKey>,
    },
    /// The offered key is marked `@revoked`.
    Revoked {
        /// File of the `@revoked` line.
        path: PathBuf,
        /// Its 1-based line.
        line: usize,
    },
}

/// One parsed `known_hosts` record that matches the host being looked up.
struct Found {
    path: PathBuf,
    line: usize,
    marker: Option<Marker>,
    key: PublicKey,
}

/// `SHA256:<base64, no padding>`, as OpenSSH prints it.
pub(crate) fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// The name OpenSSH looks up: `host` for port 22, `[host]:port` otherwise.
fn lookup_name(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_ascii_lowercase()
    } else {
        format!("[{}]:{port}", host.to_ascii_lowercase())
    }
}

/// Looks `key` up for `host:port` in `files` (in order). Any equal record wins (`Known`),
/// unless the key is `@revoked`; otherwise the first different record is reported (`Changed`).
pub(crate) fn lookup(files: &[PathBuf], host: &str, port: u16, key: &PublicKey) -> Lookup {
    let found = matching_entries(files, host, port);
    let same = |f: &&Found| f.key.key_data() == key.key_data();
    if let Some(f) = found
        .iter()
        .filter(|f| f.marker == Some(Marker::Revoked))
        .find(same)
    {
        return Lookup::Revoked {
            path: f.path.clone(),
            line: f.line,
        };
    }
    let plain: Vec<&Found> = found.iter().filter(|f| f.marker.is_none()).collect();
    if plain.iter().any(same) {
        return Lookup::Known;
    }
    plain.first().map_or(Lookup::Unknown, |f| Lookup::Changed {
        path: f.path.clone(),
        line: f.line,
        old: Box::new(f.key.clone()),
    })
}

/// Algorithms of the keys recorded for `host:port` (to prefer them in negotiation, like
/// OpenSSH, so a server with several host keys presents the one we know).
pub(crate) fn known_algorithms(
    files: &[PathBuf],
    host: &str,
    port: u16,
) -> Vec<russh::keys::Algorithm> {
    matching_entries(files, host, port)
        .into_iter()
        .filter(|f| f.marker.is_none())
        .map(|f| f.key.algorithm())
        .collect()
}

fn matching_entries(files: &[PathBuf], host: &str, port: u16) -> Vec<Found> {
    let name = lookup_name(host, port);
    let mut found = Vec::new();
    for path in files {
        let Some(text) = read_limited(path) else {
            continue;
        };
        for (index, raw) in text.lines().enumerate() {
            let Some(entry) = parse_line(raw) else {
                continue;
            };
            if matches_host(entry.host_patterns(), &name) {
                found.push(Found {
                    path: path.clone(),
                    line: index + 1,
                    marker: entry.marker().copied(),
                    key: entry.public_key().clone(),
                });
            }
        }
    }
    found
}

/// Reads a `known_hosts` file; `None` if it is missing, unreadable or oversized.
fn read_limited(path: &Path) -> Option<String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "known_hosts unreadable");
            return None;
        }
    };
    let mut text = String::new();
    let read = file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text);
    match read {
        Ok(n) if n as u64 > MAX_FILE_BYTES => {
            tracing::warn!(path = %path.display(), "known_hosts too large; ignored");
            None
        }
        Ok(_) => Some(text),
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "known_hosts unreadable");
            None
        }
    }
}

/// One line → entry; comments, blank and unparsable lines (unknown key types, junk) → `None`.
fn parse_line(raw: &str) -> Option<Entry> {
    let line = raw.split_once('#').map_or(raw, |(before, _)| before).trim();
    if line.is_empty() {
        return None;
    }
    // OpenSSH separates fields by any run of blanks; `Entry` wants single spaces.
    let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
    normalized.parse::<Entry>().ok()
}

fn matches_host(patterns: &HostPatterns, name: &str) -> bool {
    match patterns {
        HostPatterns::HashedName { salt, hash } => {
            let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, salt);
            ring::hmac::verify(&key, name.as_bytes(), hash).is_ok()
        }
        HostPatterns::Patterns(list) => {
            let mut positive = false;
            for pattern in list {
                let pattern = pattern.to_ascii_lowercase();
                if let Some(negated) = pattern.strip_prefix('!') {
                    if glob(negated.as_bytes(), name.as_bytes()) {
                        return false;
                    }
                } else if glob(pattern.as_bytes(), name.as_bytes()) {
                    positive = true;
                }
            }
            positive
        }
    }
}

/// OpenSSH `match_pattern`: `*` = any run, `?` = one char. Iterative (no recursion blow-up).
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match pattern.get(p) {
            Some(b'*') => {
                star = Some((p, t));
                p += 1;
            }
            Some(&c) if c == b'?' || Some(&c) == text.get(t) => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    p = sp + 1;
                    t = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    pattern
        .get(p..)
        .is_some_and(|rest| rest.iter().all(|&c| c == b'*'))
}

/// The host field we write: the plain lookup name when it is made only of hostname/address
/// characters, else a hashed `|1|salt|hash` entry, so a host string containing `,` `*` `!` or
/// whitespace can never widen what the record trusts.
fn host_field(host: &str, port: u16) -> io::Result<String> {
    let name = lookup_name(host, port);
    let plain = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':'));
    if plain {
        return Ok(name);
    }
    let mut salt = [0_u8; 20];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut salt)
        .map_err(|_| io::Error::other("no randomness for a hashed known_hosts entry"))?;
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &salt);
    let mut hash = [0_u8; 20];
    hash.copy_from_slice(ring::hmac::sign(&key, name.as_bytes()).as_ref());
    Ok(HostPatterns::HashedName {
        salt: salt.to_vec(),
        hash,
    }
    .to_string())
}

/// One OpenSSH `known_hosts` line (with trailing newline) for `key` at `host:port`.
fn record_line(host: &str, port: u16, key: &PublicKey) -> io::Result<String> {
    let bare = PublicKey::new(key.key_data().clone(), "");
    let key_text = bare.to_openssh().map_err(io::Error::other)?;
    Ok(format!("{} {key_text}\n", host_field(host, port)?))
}

/// Appends a record for `key` to our `known_hosts` at `path` (dir `0700`, file `0600`).
pub(crate) fn learn(path: &Path, host: &str, port: u16, key: &PublicKey) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fsutil::create_private_dir(dir)?;
    }
    let mut line = record_line(host, port, key)?;
    if !ends_with_newline(path)? {
        line.insert(0, '\n');
    }
    fsutil::append_private(path, line.as_bytes())
}

/// `true` when `path` is missing, empty or ends in a newline (reads one byte, never the file).
fn ends_with_newline(path: &Path) -> io::Result<bool> {
    use std::io::{Seek as _, SeekFrom};
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(err) => return Err(err),
    };
    if file.metadata()?.len() == 0 {
        return Ok(true);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0_u8; 1];
    file.read_exact(&mut last)?;
    Ok(last == *b"\n")
}

/// The explicit "replace and connect" flow: drops every plain record for `host:port` from our
/// `known_hosts` and appends `key`, atomically (temp file `0600` + rename). Records in
/// `~/.ssh/known_hosts` are never touched; ours is consulted first and an equal key wins.
pub(crate) fn replace(path: &Path, host: &str, port: u16, key: &PublicKey) -> io::Result<()> {
    let name = lookup_name(host, port);
    let old = match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_FILE_BYTES => {
            // Never rewrite a file we refused to read: `lookup` ignored it, so nothing in it
            // produced the "changed" verdict either.
            return Err(io::Error::other("known_hosts is too large to rewrite"));
        }
        Ok(_) => std::fs::read_to_string(path)?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    let mut text: String = old
        .lines()
        .filter(|raw| {
            parse_line(raw)
                .is_none_or(|e| e.marker().is_some() || !matches_host(e.host_patterns(), &name))
        })
        .flat_map(|raw| [raw, "\n"])
        .collect();
    text.push_str(&record_line(host, port, key)?);
    if let Some(dir) = path.parent() {
        fsutil::create_private_dir(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(fsutil::FILE_MODE)
        .open(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
#[path = "hostkeys_tests.rs"]
mod tests;
