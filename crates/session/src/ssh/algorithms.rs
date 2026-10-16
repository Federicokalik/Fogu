//! The algorithm names russh supports, per category (the profile editor's Algorithms section),
//! and a profile's allow-lists mapped onto russh's `Preferred` ([`preferred`]). `none`/`clear`
//! entries (no encryption, no MAC, no key exchange) are left out: they are never offered.
//!
//! A list keeps the user's order, drops unknown names (reported) and falls back to russh's
//! default when nothing usable is left. Names outside russh's default set (SHA-1 MACs, CBC
//! ciphers, `ssh-dss`, group1/group14-sha1 …) are allowed but reported as weaker, so the UI
//! warns. The key-exchange extension markers (`ext-info-c`, strict KEX) are always appended:
//! a kex list can't switch off the Terrapin countermeasure; the PQ hybrid stays first by default.

use std::borrow::Cow;

use fogu_core::Algorithms;
use russh::keys::Algorithm;
use russh::keys::key::ALL_KEY_TYPES;
use russh::{Preferred, cipher, compression, kex, mac};

/// Supported names per category, in russh's order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Supported {
    /// Key exchange.
    pub kex: Vec<String>,
    /// Ciphers.
    pub cipher: Vec<String>,
    /// MACs.
    pub hmac: Vec<String>,
    /// Host key algorithms.
    pub server_host_key: Vec<String>,
    /// Compression (`none` = off, kept: it is not a security downgrade).
    pub compression: Vec<String>,
}

/// What this build of russh can negotiate.
#[must_use]
pub fn supported() -> Supported {
    fn names<N: AsRef<str>>(list: &[&N], skip: &[&str]) -> Vec<String> {
        list.iter()
            .map(|n| n.as_ref().to_owned())
            .filter(|n| !skip.contains(&n.as_str()))
            .collect()
    }
    Supported {
        kex: names(kex::ALL_KEX_ALGORITHMS, &["none"]),
        cipher: names(cipher::ALL_CIPHERS, &["none", "clear"]),
        hmac: names(mac::ALL_MAC_ALGORITHMS, &["none"]),
        server_host_key: host_key_types().map(|a| a.to_string()).collect(),
        compression: names(compression::ALL_COMPRESSION_ALGORITHMS, &[]),
    }
}

impl Supported {
    /// The lists in [`Algorithms::lists`] order.
    #[must_use]
    pub fn lists(self) -> [Vec<String>; 5] {
        [
            self.kex,
            self.cipher,
            self.hmac,
            self.server_host_key,
            self.compression,
        ]
    }
}

/// Host key types: russh's list plus its defaults (RSA-SHA2 is verified without the `rsa`
/// feature), once each.
fn host_key_types() -> impl Iterator<Item = Algorithm> {
    let mut out: Vec<Algorithm> = Vec::new();
    for alg in ALL_KEY_TYPES.iter().chain(Preferred::DEFAULT.key.iter()) {
        if !out.contains(alg) {
            out.push(alg.clone());
        }
    }
    out.into_iter()
}

/// A profile's allow-lists as russh preferences, plus what to tell the user.
pub(crate) struct Mapped {
    pub(crate) preferred: Preferred,
    /// Names russh doesn't know (ignored).
    pub(crate) unknown: Vec<String>,
    /// Names used that russh doesn't offer by default (weaker or legacy).
    pub(crate) weak: Vec<String>,
}

/// Maps `alg` onto russh's [`Preferred`] (see the module docs).
pub(crate) fn preferred(alg: &Algorithms) -> Mapped {
    let mut out = Mapped {
        preferred: Preferred::default(),
        unknown: Vec::new(),
        weak: Vec::new(),
    };
    let d = Preferred::DEFAULT;
    if let Some(mut kex) = pick(&alg.kex, &d.kex, &mut out, |n| kex::Name::try_from(n).ok()) {
        kex.extend(d.kex.iter().filter(|n| is_kex_marker(n.as_ref())).copied());
        out.preferred.kex = Cow::Owned(kex);
    }
    if let Some(cipher) = pick(&alg.cipher, &d.cipher, &mut out, |n| {
        cipher::Name::try_from(n).ok()
    }) {
        out.preferred.cipher = Cow::Owned(cipher);
    }
    if let Some(mac) = pick(&alg.hmac, &d.mac, &mut out, |n| mac::Name::try_from(n).ok()) {
        out.preferred.mac = Cow::Owned(mac);
    }
    if let Some(key) = pick(&alg.server_host_key, &d.key, &mut out, |n| {
        host_key_types().find(|a| a.to_string() == n)
    }) {
        out.preferred.key = Cow::Owned(key);
    }
    if let Some(comp) = pick(&alg.compression, &d.compression, &mut out, |n| {
        compression::Name::try_from(n).ok()
    }) {
        out.preferred.compression = Cow::Owned(comp);
    }
    out
}

/// Names russh reports as key exchange extensions rather than algorithms.
fn is_kex_marker(name: &str) -> bool {
    name.starts_with("ext-info-") || name.starts_with("kex-strict-")
}

/// The usable names of `names` in order (`None` = keep russh's default), recording unknown
/// and non-default ones in `out`. `none`/`clear` never pass.
fn pick<T: AsRef<str> + PartialEq>(
    names: &[String],
    defaults: &[T],
    out: &mut Mapped,
    parse: impl Fn(&str) -> Option<T>,
) -> Option<Vec<T>> {
    let mut picked: Vec<T> = Vec::new();
    for name in names {
        let usable = !matches!(name.as_str(), "none" | "clear")
            || defaults.iter().any(|d| d.as_ref() == name);
        match parse(name).filter(|_| usable && !is_kex_marker(name)) {
            Some(value) if !picked.contains(&value) => {
                if !defaults.contains(&value) {
                    out.weak.push(name.clone());
                }
                picked.push(value);
            }
            Some(_) => {}
            None => out.unknown.push(crate::sanitize_remote(name, 64, false)),
        }
    }
    (!picked.is_empty()).then_some(picked)
}

/// Names in `alg` that russh doesn't offer by default (the editor's "weaker" note).
#[must_use]
pub fn non_default(alg: &Algorithms) -> Vec<String> {
    preferred(alg).weak
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names<T: AsRef<str>>(list: &[T]) -> Vec<&str> {
        list.iter().map(AsRef::as_ref).collect()
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn empty_lists_keep_russh_defaults() {
        let m = preferred(&Algorithms::default());
        let d = Preferred::DEFAULT;
        assert_eq!(names(&m.preferred.kex), names(&d.kex));
        assert_eq!(names(&m.preferred.cipher), names(&d.cipher));
        assert_eq!(m.preferred.key, d.key);
        assert!(m.unknown.is_empty() && m.weak.is_empty());
    }

    #[test]
    fn lists_keep_the_user_order_and_report_unknown_and_weak_names() {
        let alg = Algorithms {
            kex: strings(&["curve25519-sha256", "diffie-hellman-group1-sha1", "bogus"]),
            cipher: strings(&["aes256-gcm@openssh.com", "none", "aes256-gcm@openssh.com"]),
            hmac: strings(&["hmac-sha1", "hmac-sha2-256"]),
            server_host_key: strings(&["ssh-ed25519", "ssh-dss"]),
            compression: strings(&["zlib@openssh.com", "none"]),
        };
        let m = preferred(&alg);
        let kex = names(&m.preferred.kex);
        assert_eq!(
            kex.get(..2).unwrap_or_default(),
            ["curve25519-sha256", "diffie-hellman-group1-sha1"]
        );
        // The strict-KEX / ext-info markers are kept after the user's choice.
        assert!(kex.contains(&"kex-strict-c-v00@openssh.com"), "{kex:?}");
        assert!(kex.contains(&"ext-info-c"), "{kex:?}");
        assert_eq!(names(&m.preferred.cipher), ["aes256-gcm@openssh.com"]);
        assert_eq!(names(&m.preferred.mac), ["hmac-sha1", "hmac-sha2-256"]);
        assert_eq!(
            m.preferred
                .key
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["ssh-ed25519", "ssh-dss"]
        );
        assert_eq!(
            names(&m.preferred.compression),
            ["zlib@openssh.com", "none"]
        );
        assert_eq!(m.unknown, ["bogus", "none"]);
        assert_eq!(
            m.weak,
            ["diffie-hellman-group1-sha1", "hmac-sha1", "ssh-dss"]
        );
    }

    #[test]
    fn only_unknown_names_fall_back_to_the_defaults() {
        let alg = Algorithms {
            cipher: strings(&["bogus", "\x1b[31mevil"]),
            ..Algorithms::default()
        };
        let m = preferred(&alg);
        assert_eq!(
            names(&m.preferred.cipher),
            names(&Preferred::DEFAULT.cipher)
        );
        assert_eq!(m.unknown.len(), 2);
        assert!(
            m.unknown.iter().all(|n| !n.contains('\u{1b}')),
            "{:?}",
            m.unknown
        );
    }

    #[test]
    fn lists_are_filled_and_never_offer_no_encryption() {
        let s = supported();
        assert!(
            s.kex.iter().any(|n| n == "curve25519-sha256"),
            "{:?}",
            s.kex
        );
        assert!(
            s.cipher
                .iter()
                .any(|n| n == "chacha20-poly1305@openssh.com")
        );
        assert!(s.hmac.iter().any(|n| n == "hmac-sha2-256"));
        assert!(s.server_host_key.iter().any(|n| n == "ssh-ed25519"));
        assert!(s.compression.iter().any(|n| n == "zlib@openssh.com"));
        for list in [&s.kex, &s.cipher, &s.hmac] {
            assert!(
                !list.iter().any(|n| n == "none" || n == "clear"),
                "{list:?}"
            );
        }
    }
}
