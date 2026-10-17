//! Quoting for text typed into a shell (the SFTP panel's "cd terminal here", a profile's `cwd`).

/// The bytes that type a `cd` into a shell: `cd -- '<path>'` + Enter. Inside the single quotes a
/// `'` becomes `'\''` and a `\` becomes `'\\'` (outside the quotes), so the line means the
/// same in POSIX shells and fish (where `\\` and `\'` are escapes even inside quotes). Paths
/// with control characters (a newline would run a second command) are refused.
pub fn cd_command(path: &str) -> Result<Vec<u8>, &'static str> {
    if path.is_empty() {
        return Err("no directory");
    }
    if path.chars().any(char::is_control) {
        return Err("the path contains control characters");
    }
    let mut line = String::from("cd -- '");
    for c in path.chars() {
        match c {
            '\'' => line.push_str("'\\''"),
            '\\' => line.push_str("'\\\\'"),
            c => line.push(c),
        }
    }
    line.push_str("'\r");
    Ok(line.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::cd_command;

    #[test]
    fn cd_here_quoting_table() {
        let ok = [
            ("/tmp", "cd -- '/tmp'\r"),
            ("/home/me/my files", "cd -- '/home/me/my files'\r"),
            ("/a'b", "cd -- '/a'\\''b'\r"),
            ("/x\\y", "cd -- '/x'\\\\'y'\r"),
            ("/$(rm -rf ~)`id`", "cd -- '/$(rm -rf ~)`id`'\r"),
            ("/-n", "cd -- '/-n'\r"),
            ("/日本", "cd -- '/日本'\r"),
        ];
        for (path, want) in ok {
            assert_eq!(cd_command(path).as_deref(), Ok(want.as_bytes()), "{path}");
        }
        for bad in [
            "/a\nrm -rf ~",
            "/a\rb",
            "/esc\x1b[2J",
            "/del\x7f",
            "/c1\u{9b}",
            "",
        ] {
            assert!(cd_command(bad).is_err(), "{bad:?}");
        }
    }
}
