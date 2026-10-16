//! Remote path helpers: breadcrumb segments, parent, join. Remote paths are `/`-separated SFTP
//! strings (raw server data); labels shown to the user go through `display_name`.

use fogu_session::display_name;

/// One clickable breadcrumb segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// What the button shows (sanitized).
    pub label: String,
    /// The directory it navigates to (raw).
    pub path: String,
}

/// Segments of an absolute or relative `path`: `/home/u` → `/`, `home`, `u`.
pub fn segments(path: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut acc = String::new();
    if path.starts_with('/') {
        acc.push('/');
        out.push(Segment {
            label: "/".to_owned(),
            path: "/".to_owned(),
        });
    }
    for part in path.split('/').filter(|p| !p.is_empty()) {
        if !acc.is_empty() && !acc.ends_with('/') {
            acc.push('/');
        }
        acc.push_str(part);
        out.push(Segment {
            label: display_name(part),
            path: acc.clone(),
        });
    }
    out
}

/// The parent directory of `path`; `None` at the root (or for a bare relative name).
pub fn parent(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.rfind('/') {
        Some(0) => Some("/".to_owned()),
        Some(i) => trimmed.get(..i).map(str::to_owned),
        None => None,
    }
}

/// `dir/name` without doubling the separator.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_table() {
        let paths = |p: &str| -> Vec<(String, String)> {
            segments(p).into_iter().map(|s| (s.label, s.path)).collect()
        };
        let own = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect()
        };
        assert_eq!(paths("/"), own(&[("/", "/")]));
        assert_eq!(
            paths("/home/u"),
            own(&[("/", "/"), ("home", "/home"), ("u", "/home/u")])
        );
        assert_eq!(
            paths("/a//b/"),
            own(&[("/", "/"), ("a", "/a"), ("b", "/a/b")])
        );
        assert_eq!(paths("rel/x"), own(&[("rel", "rel"), ("x", "rel/x")]));
        // A hostile component is shown escaped but navigates by its raw path.
        assert_eq!(
            paths("/e\x1b[2Jvil"),
            own(&[("/", "/"), ("e\\x1b[2Jvil", "/e\x1b[2Jvil")])
        );
    }

    #[test]
    fn parent_and_join() {
        assert_eq!(parent("/home/u"), Some("/home".into()));
        assert_eq!(parent("/home/u/"), Some("/home".into()));
        assert_eq!(parent("/home"), Some("/".into()));
        assert_eq!(parent("/"), None);
        assert_eq!(parent("x"), None);
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(join("/home", "u"), "/home/u");
    }
}
