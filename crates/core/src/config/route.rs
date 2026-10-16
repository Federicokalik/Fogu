//! How an SSH profile reaches its host: directly, through a jump host chain, a proxy
//! command, or a SOCKS5 / HTTP CONNECT proxy. The session never looks profiles up, so the UI
//! resolves the `jump_host` chain with [`resolve_jumps`] before opening the session; a route the
//! session can't follow exactly ([`EffectiveSsh::route_problems`]) refuses to connect instead of
//! connecting directly (that could reach a different machine).

use std::collections::HashSet;

use super::ssh_opts::EffectiveSsh;
use crate::profile::ProfileId;

/// Most jump hosts in one chain (OpenSSH has no limit; this bounds the recursion).
pub const MAX_JUMP_HOPS: usize = 8;

/// The resolved `jump_host` of an [`EffectiveSsh`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Jump {
    /// Not resolved (or no jump host). With `jump_host` set, the session refuses to connect.
    #[default]
    None,
    /// Connect through this profile (its own `jump` is the next hop out).
    Via(Box<EffectiveSsh>),
    /// The chain can't be used (unknown or non-SSH profile, a loop, too long); the reason.
    Invalid(String),
}

impl EffectiveSsh {
    /// Why this profile's route can't be followed (empty = fine): more than one route option
    /// set, an unresolved or invalid jump chain, or a `ProxyJump`/`ProxyCommand` from
    /// `~/.ssh/config` that could not be mapped. The session refuses to connect when any is
    /// listed.
    #[must_use]
    pub fn route_problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let set: Vec<&str> = [
            (self.proxy_command.is_some(), "proxy_command"),
            (self.jump_host.is_some(), "jump_host"),
            (self.socks_proxy.is_some(), "socks_proxy"),
            (self.http_proxy.is_some(), "http_proxy"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
        if set.len() > 1 {
            out.push(format!("only one of {} may be set", set.join(", ")));
        }
        if let Some(id) = &self.jump_host {
            match &self.jump {
                Jump::Via(_) => {}
                Jump::None => out.push(format!("jump_host {} was not resolved", id.0)),
                Jump::Invalid(why) => out.push(format!("jump_host {}: {why}", id.0)),
            }
        }
        let from_ssh_config = self.ignored_ssh_config.iter().filter(|o| {
            let key = o.split_whitespace().next().unwrap_or_default();
            key.eq_ignore_ascii_case("ProxyJump") || key.eq_ignore_ascii_case("ProxyCommand")
        });
        out.extend(from_ssh_config.cloned());
        out
    }
}

/// Fills `ssh.jump` by following `jump_host` from profile `id` through `lookup` (a stored or
/// `~/.ssh/config` profile, resolved; `Err` = why it can't be used). A loop, more than
/// [`MAX_JUMP_HOPS`] hops or a failed lookup leaves [`Jump::Invalid`].
pub fn resolve_jumps(
    id: &ProfileId,
    ssh: &mut EffectiveSsh,
    mut lookup: impl FnMut(&ProfileId) -> Result<EffectiveSsh, String>,
) {
    let mut seen: HashSet<ProfileId> = HashSet::from([id.clone()]);
    let mut path = vec![id.0.clone()];
    let mut hops: Vec<EffectiveSsh> = Vec::new();
    let mut next = ssh.jump_host.clone();
    while let Some(hop_id) = next.take() {
        path.push(hop_id.0.clone());
        if !seen.insert(hop_id.clone()) {
            ssh.jump = Jump::Invalid(format!("jump host loop: {}", path.join(" → ")));
            return;
        }
        if hops.len() == MAX_JUMP_HOPS {
            ssh.jump = Jump::Invalid(format!("more than {MAX_JUMP_HOPS} jump hosts"));
            return;
        }
        match lookup(&hop_id) {
            Ok(hop) => {
                next.clone_from(&hop.jump_host);
                hops.push(hop);
            }
            Err(why) => {
                ssh.jump = Jump::Invalid(why);
                return;
            }
        }
    }
    // hops[0] is the one `ssh` connects through; the last one is reached directly.
    ssh.jump = hops.into_iter().rev().fold(Jump::None, |outer, mut hop| {
        hop.jump = outer;
        Jump::Via(Box::new(hop))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hop(host: &str, jump: Option<&str>) -> EffectiveSsh {
        let mut ssh = EffectiveSsh::new(host);
        ssh.jump_host = jump.map(|j| ProfileId(j.to_owned()));
        ssh
    }

    fn chain(ssh: &EffectiveSsh) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = &ssh.jump;
        while let Jump::Via(hop) = cur {
            out.push(hop.host.clone());
            cur = &hop.jump;
        }
        out
    }

    fn lookup(id: &ProfileId) -> Result<EffectiveSsh, String> {
        match id.0.as_str() {
            "a" => Ok(hop("a.lan", Some("b"))),
            "b" => Ok(hop("b.lan", None)),
            "self" => Ok(hop("self.lan", Some("self"))),
            "loop1" => Ok(hop("l1", Some("loop2"))),
            "loop2" => Ok(hop("l2", Some("loop1"))),
            "back" => Ok(hop("back", Some("t"))),
            n if n.starts_with("deep") => {
                let i: usize = n.trim_start_matches("deep").parse().unwrap_or(0);
                Ok(hop(n, Some(&format!("deep{}", i + 1))))
            }
            other => Err(format!("profile {other} not found")),
        }
    }

    #[test]
    fn a_chain_resolves_outermost_last() {
        let mut ssh = hop("target", Some("a"));
        resolve_jumps(&ProfileId("t".into()), &mut ssh, lookup);
        assert_eq!(chain(&ssh), ["a.lan", "b.lan"]);
        assert_eq!(ssh.route_problems(), Vec::<String>::new());
        let mut direct = hop("target", None);
        resolve_jumps(&ProfileId("t".into()), &mut direct, lookup);
        assert_eq!(direct.jump, Jump::None);
        assert_eq!(direct.route_problems(), Vec::<String>::new());
    }

    #[test]
    fn loops_unknown_hops_and_long_chains_are_invalid() {
        for (jump, want) in [
            ("self", "jump host loop: t → self → self"),
            ("loop1", "jump host loop: t → loop1 → loop2 → loop1"),
            ("back", "jump host loop: t → back → t"),
            ("nope", "profile nope not found"),
            ("deep0", "more than 8 jump hosts"),
        ] {
            let mut ssh = hop("target", Some(jump));
            resolve_jumps(&ProfileId("t".into()), &mut ssh, lookup);
            assert_eq!(ssh.jump, Jump::Invalid(want.into()), "{jump}");
            assert_eq!(ssh.route_problems(), [format!("jump_host {jump}: {want}")]);
        }
    }

    #[test]
    fn route_problems_lists_unresolved_combined_and_unmapped_routes() {
        let mut ssh = EffectiveSsh::new("h");
        assert_eq!(ssh.route_problems(), Vec::<String>::new());
        ssh.socks_proxy = Some(("p".into(), 1080));
        assert_eq!(
            ssh.route_problems(),
            Vec::<String>::new(),
            "one proxy is fine"
        );
        ssh.jump_host = Some(ProfileId("j".into()));
        ssh.ignored_ssh_config = vec!["ProxyJump ssh://gw".into(), "Ciphers aes".into()];
        assert_eq!(
            ssh.route_problems(),
            [
                "only one of jump_host, socks_proxy may be set",
                "jump_host j was not resolved",
                "ProxyJump ssh://gw",
            ]
        );
    }
}
