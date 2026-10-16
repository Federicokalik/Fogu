//! The algorithm names russh supports, per category, for the profile editor's Algorithms
//! section. `none`/`clear`
//! entries (no encryption, no MAC, no key exchange) are left out: they are never offered.

use russh::keys::key::ALL_KEY_TYPES;
use russh::{cipher, compression, kex, mac};

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
        server_host_key: ALL_KEY_TYPES.iter().map(ToString::to_string).collect(),
        compression: names(compression::ALL_COMPRESSION_ALGORITHMS, &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
