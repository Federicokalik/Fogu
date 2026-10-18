//! Links in the terminal: OSC 8 hyperlinks from the cells, and URLs, IPv4 addresses and file paths
//! found in a row's text. Only finding them: what may be opened is decided by the UI's `open_link`.

use std::sync::LazyLock;

use regex::Regex;

use crate::snapshot::{Cell, RenderSnapshot};

/// What kind of text a link came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// An OSC 8 hyperlink: `uri` is the escape's target, whatever the text shows.
    Hyperlink,
    /// A URL (`www.` gets `https://`, a bare IPv4 address `http://`).
    Url,
    /// A file path as written (absolute, `~/`, `./`, or relative with a `/`).
    Path,
}

/// A link on one row: columns `start..end` and its target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub uri: String,
    pub kind: LinkKind,
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

/// URLs with a scheme or `www.`, then IPv4(:port), then paths; leftmost match wins, ties go to
/// the earlier alternative.
static LINKS: LazyLock<Option<Regex>> = LazyLock::new(|| {
    let url = r#"(?P<url>(?:(?:https?|ftp|file)://|mailto:|www\.)[^\s<>"'`{}|\\^]+)"#;
    let ip = r"(?P<ip>\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)(?::\d{1,5})?\b)";
    let path = r"(?P<path>(?:~|\.{1,2})?/[\w.~+@%-]+(?:/[\w.~+@%-]*)*|[\w.-]+(?:/[\w.~+@%-]+)+)";
    regex::RegexBuilder::new(&format!("{url}|{ip}|{path}"))
        // `HTTPS://…` too; the allow-list compares schemes case-insensitively.
        .case_insensitive(true)
        .size_limit(1 << 20)
        .build()
        .ok()
});

/// Every link found in `text` as `(char range, uri, kind)`, without OSC 8 ones.
pub fn detect_links(text: &str) -> Vec<(std::ops::Range<usize>, String, LinkKind)> {
    let Some(re) = LINKS.as_ref() else {
        return Vec::new();
    };
    let char_at = |byte: usize| text.get(..byte).map_or(0, |s| s.chars().count());
    re.captures_iter(text)
        .filter_map(|caps| {
            let (m, kind) = if let Some(m) = caps.name("url") {
                (m, LinkKind::Url)
            } else if let Some(m) = caps.name("ip") {
                (m, LinkKind::Url)
            } else {
                (caps.name("path")?, LinkKind::Path)
            };
            let found = trim_trailing(m.as_str());
            // "a.b/c" only counts as a path if it is not just a number or a fraction like 1/2.
            if kind == LinkKind::Path && !found.chars().any(char::is_alphabetic) {
                return None;
            }
            let uri = if caps.name("ip").is_some() {
                format!("http://{found}")
            } else if let Some(rest) = found.strip_prefix("www.") {
                format!("https://www.{rest}")
            } else {
                found.to_owned()
            };
            let start = char_at(m.start());
            Some((start..start + found.chars().count(), uri, kind))
        })
        .collect()
}

/// `s` without the punctuation a sentence puts after a link; a closing bracket stays when the
/// link opened one ("…/Rust_(language)").
fn trim_trailing(mut s: &str) -> &str {
    loop {
        let Some(last) = s.chars().last() else {
            return s;
        };
        let pair = match last {
            ')' => Some('('),
            ']' => Some('['),
            _ => None,
        };
        let drop = match pair {
            Some(open) => s.matches(open).count() < s.matches(last).count(),
            None => ".,;:!?'\"".contains(last),
        };
        if !drop {
            return s;
        }
        s = &s[..s.len() - last.len_utf8()];
    }
}

/// The link under cell `(col, row)` of `snapshot`: an OSC 8 hyperlink (all contiguous cells
/// with the same one), else a link detected in the row's text.
/// shortcut: a link wrapped onto the next row is cut at the row end, fine for typical widths.
pub fn link_at(snapshot: &RenderSnapshot, col: usize, row: usize) -> Option<Link> {
    let cells = snapshot.rows().nth(row)?;
    let cell = cells.get(col)?;
    if let Some(link) = &cell.hyperlink {
        let same = |c: &Cell| c.hyperlink.as_ref() == Some(link);
        let start = cells
            .get(..col)?
            .iter()
            .rposition(|c| !same(c))
            .map_or(0, |i| i + 1);
        let end = col + cells.get(col..)?.iter().take_while(|c| same(c)).count();
        return Some(Link {
            uri: link.uri().to_owned(),
            kind: LinkKind::Hyperlink,
            row,
            start,
            end,
        });
    }
    // One char per cell, so a char index is a column.
    let text: String = cells.iter().map(|c| c.ch).collect();
    detect_links(&text)
        .into_iter()
        .find(|(range, _, _)| range.contains(&col))
        .map(|(range, uri, kind)| Link {
            uri,
            kind,
            row,
            start: range.start,
            end: range.end,
        })
}

#[cfg(test)]
#[path = "links_tests.rs"]
mod tests;
