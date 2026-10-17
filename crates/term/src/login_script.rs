//! Login scripts: `expect` → `send` rules run in order on a session's output. Output is matched
//! against a rolling window of the last [`WINDOW`] bytes with escape sequences stripped (prompts
//! are often coloured). A match consumes the window up to its end, so the same prompt does not fire
//! the next rule too. A non-matching `optional` rule is skipped when a later rule matches; an empty
//! `expect` sends as soon as the rule is first in line. Escapes in `send` (and in a literal
//! `expect`) are unescaped (`\xNN`, `\n`, ...).

use fogu_core::LoginScript;
use regex::bytes::{Regex, RegexBuilder};

/// Bytes of stripped output kept for matching.
pub const WINDOW: usize = 4 * 1024;
/// Compiled size limit per `expect` regex.
const REGEX_SIZE_LIMIT: usize = 256 * 1024;

/// One compiled rule.
#[derive(Debug)]
struct Rule {
    expect: Option<Regex>,
    send: Vec<u8>,
    optional: bool,
}

/// Escape-sequence stripper state, kept across calls (a sequence may span chunks).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Strip {
    #[default]
    Text,
    /// Saw `ESC`.
    Escape,
    /// In `ESC [ …` until a final byte.
    Csi,
    /// In `ESC ] …` (or DCS/APC/PM/SOS) until `BEL` or `ESC \`.
    String,
    /// Saw `ESC` inside a string: `\` ends it.
    StringEscape,
}

/// Runs a profile's login scripts on its session output.
#[derive(Debug)]
pub struct LoginScriptMatcher {
    rules: Vec<Rule>,
    window: Vec<u8>,
    strip: Strip,
}

impl LoginScriptMatcher {
    /// Compiles `scripts`; `newline` is appended to every `send`. A rule whose regex does not
    /// compile is dropped and reported in the returned list (one line each).
    #[must_use]
    pub fn new(scripts: &[LoginScript], newline: &[u8]) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut rules = Vec::new();
        for (i, script) in scripts.iter().enumerate() {
            let pattern = if script.is_regex {
                script.expect.clone()
            } else {
                regex::escape(&String::from_utf8_lossy(&unescape(&script.expect)))
            };
            let expect = if script.expect.is_empty() {
                None
            } else {
                match RegexBuilder::new(&pattern)
                    .size_limit(REGEX_SIZE_LIMIT)
                    .build()
                {
                    Ok(re) => Some(re),
                    Err(err) => {
                        problems.push(format!("login script {}: invalid regex ({err})", i + 1));
                        continue;
                    }
                }
            };
            let mut send = unescape(&script.send);
            send.extend_from_slice(newline);
            rules.push(Rule {
                expect,
                send,
                optional: script.optional,
            });
        }
        let matcher = Self {
            rules,
            window: Vec::new(),
            strip: Strip::Text,
        };
        (matcher, problems)
    }

    /// Feeds session output; returns the bytes to send, in order.
    pub fn process(&mut self, output: &[u8]) -> Vec<Vec<u8>> {
        self.push_stripped(output);
        let mut replies = Vec::new();
        loop {
            self.send_unconditional(&mut replies);
            let Some((index, end)) = self.first_match() else {
                break;
            };
            // Optional rules before the one that matched are skipped.
            let rule = self.rules.drain(..=index).next_back();
            if let Some(rule) = rule {
                replies.push(rule.send);
            }
            self.window.drain(..end);
        }
        replies
    }

    /// `true` once every non-optional rule has fired.
    #[must_use]
    pub fn done(&self) -> bool {
        self.rules.iter().all(|r| r.optional)
    }

    /// Rules with an empty `expect` at the front of the line are sent right away.
    fn send_unconditional(&mut self, replies: &mut Vec<Vec<u8>>) {
        while self.rules.first().is_some_and(|r| r.expect.is_none()) {
            replies.push(self.rules.remove(0).send);
        }
    }

    /// The first rule that matches, looking past optional rules only: `(index, match end)`.
    fn first_match(&self) -> Option<(usize, usize)> {
        for (i, rule) in self.rules.iter().enumerate() {
            // An unconditional rule behind an optional one fires when the line reaches it.
            let re = rule.expect.as_ref()?;
            if let Some(m) = re.find(&self.window) {
                return Some((i, m.end()));
            }
            if !rule.optional {
                return None;
            }
        }
        None
    }

    fn push_stripped(&mut self, output: &[u8]) {
        for &b in output {
            self.strip = match (self.strip, b) {
                (Strip::Text, 0x1b) => Strip::Escape,
                (Strip::Text, b) => {
                    if b >= 0x20 || matches!(b, b'\n' | b'\r' | b'\t') {
                        self.window.push(b);
                    }
                    Strip::Text
                }
                (Strip::Escape, b'[') => Strip::Csi,
                (Strip::Escape, b']' | b'P' | b'_' | b'^' | b'X') => Strip::String,
                (Strip::Escape, _)
                | (Strip::Csi, 0x40..=0x7e)
                | (Strip::String, 0x07)
                | (Strip::StringEscape, b'\\') => Strip::Text,
                (Strip::Csi, _) => Strip::Csi,
                (Strip::String, 0x1b) => Strip::StringEscape,
                (Strip::String | Strip::StringEscape, _) => Strip::String,
            };
        }
        if self.window.len() > WINDOW {
            let excess = self.window.len() - WINDOW;
            self.window.drain(..excess);
        }
    }
}

/// Escape processing: `\xNN` and `\uNNNN` (hex), then `\a \b \e \f \n \r \t \v`; any other
/// escaped character stands for itself.
fn unescape(text: &str) -> Vec<u8> {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(next) = chars.next() else {
            out.push('\\');
            break;
        };
        let digits = match next {
            'x' => 2,
            'u' => 4,
            _ => 0,
        };
        if digits > 0 {
            let hex: String = chars.clone().take(digits).collect();
            if let (true, Ok(code)) = (hex.len() == digits, u32::from_str_radix(&hex, 16))
                && let Some(ch) = char::from_u32(code)
            {
                out.push(ch);
                for _ in 0..digits {
                    chars.next();
                }
                continue;
            }
        }
        out.push(match next {
            'a' => '\x07',
            'b' => '\x08',
            'e' => '\x1b',
            'f' => '\x0c',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'v' => '\x0b',
            other => other,
        });
    }
    out.into_bytes()
}

#[cfg(test)]
#[path = "login_script_tests.rs"]
mod tests;
