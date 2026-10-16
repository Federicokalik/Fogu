//! `ssh_config(5)` line lexing and OpenSSH host-pattern matching (`match_pattern`).

/// One directive: `Keyword args…` or `Keyword=args…`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Directive {
    /// The keyword as written (shown in warnings).
    pub name: String,
    /// The keyword lowercased (keywords are case-insensitive).
    pub key: String,
    /// Arguments split like OpenSSH's `argv_split`: blanks separate, `"…"`/`'…'` group,
    /// `\` escapes a quote, a backslash or a blank, a word starting with `#` ends the line.
    pub args: Vec<String>,
    /// Everything after the keyword, trimmed (`ProxyCommand` takes the rest of the line).
    pub raw: String,
}

/// Lexes one line; `Ok(None)` for blank and comment lines.
pub(super) fn lex(line: &str) -> Result<Option<Directive>, &'static str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let (name, rest) = line.split_at(end);
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
    if name.is_empty() {
        return Err("missing keyword");
    }
    Ok(Some(Directive {
        name: name.to_owned(),
        key: name.to_ascii_lowercase(),
        args: split_args(rest)?,
        raw: rest.to_owned(),
    }))
}

fn split_args(text: &str) -> Result<Vec<String>, &'static str> {
    let mut args = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (_, '\\')
                if chars
                    .peek()
                    .is_some_and(|n| matches!(n, '\\' | '"' | '\'' | ' ')) =>
            {
                word.extend(chars.next());
                in_word = true;
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, '#') if !in_word => break,
            (None, c) if c.is_whitespace() => {
                if in_word {
                    args.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err("unterminated quote");
    }
    if in_word {
        args.push(word);
    }
    Ok(args)
}

/// OpenSSH `match_pattern`: `*` = any run, `?` = one char. Iterative (no recursion blow-up).
#[must_use]
pub fn glob(pattern: &[u8], text: &[u8]) -> bool {
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

/// An OpenSSH host pattern list (`Host` line, `known_hosts` field): some positive pattern
/// matches and no `!pattern` does. Case-insensitive.
pub fn host_matches<'a>(patterns: impl IntoIterator<Item = &'a str>, name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let mut positive = false;
    for pattern in patterns {
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

/// A `Host` pattern that names one host (no `*`, `?`, `!`, no control characters): it becomes
/// a profile.
pub(super) fn is_plain_alias(pattern: &str) -> bool {
    !pattern.is_empty()
        && !pattern
            .chars()
            .any(|c| matches!(c, '*' | '?' | '!') || c.is_control() || c.is_whitespace())
}
