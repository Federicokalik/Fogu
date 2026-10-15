use super::*;

#[test]
fn auth_order_filter_table() {
    use Method::{Agent, KeyboardInteractive, None, Password, PublicKey};
    let rows: [(SshAuth, &[Method]); 5] = [
        (
            SshAuth::Auto,
            &[None, Agent, PublicKey, Password, KeyboardInteractive],
        ),
        (SshAuth::Password, &[None, Password]),
        (SshAuth::PublicKey, &[None, PublicKey]),
        (SshAuth::Agent, &[None, Agent]),
        (SshAuth::KeyboardInteractive, &[None, KeyboardInteractive]),
    ];
    for (auth, want) in rows {
        assert_eq!(plan(auth), want, "{auth:?}");
    }
}

#[test]
fn server_method_list_filters_steps() {
    let only_keys = MethodSet::from(&[MethodKind::PublicKey][..]);
    // (allowed, method, tried)
    let rows = [
        (None, Method::Password, true),
        (Some(&only_keys), Method::None, true),
        (Some(&only_keys), Method::Agent, true),
        (Some(&only_keys), Method::PublicKey, true),
        (Some(&only_keys), Method::Password, false),
        (Some(&only_keys), Method::KeyboardInteractive, false),
    ];
    for (allowed, method, want) in rows {
        assert_eq!(server_allows(allowed, method), want, "{method:?}");
    }
    let empty = MethodSet::empty();
    assert!(server_allows(Some(&empty), Method::Password));
}

#[test]
fn kbd_prompt_texts_are_sanitized_and_capped() {
    let prompts = [russh::client::Prompt {
        prompt: format!("Pass\x1b]52;c;AAAA\x07word{}", "x".repeat(1000)),
        echo: false,
    }];
    let PromptKind::KeyboardInteractive {
        name,
        instructions,
        prompts,
    } = kbd_prompt("n\x1b[2Ja", "line1\nline2\u{202e}", &prompts)
    else {
        unreachable!()
    };
    assert_eq!(name, "n[2Ja");
    assert_eq!(instructions, "line1\nline2");
    let first = prompts.first().map(|p| (p.text.clone(), p.echo));
    let (text, echo) = first.unwrap_or_default();
    assert!(text.starts_with("Pass]52;c;AAAAword") && !echo);
    assert_eq!(text.chars().count(), PROMPT_TEXT_MAX_CHARS);
}

#[test]
fn home_expansion_and_pub_paths() {
    assert_eq!(
        expand_home(Path::new("/abs/key")),
        PathBuf::from("/abs/key")
    );
    if let Some(home) = std::env::var_os("HOME") {
        assert_eq!(
            expand_home(Path::new("~/.ssh/id")),
            PathBuf::from(home).join(".ssh/id")
        );
    }
}

#[test]
fn encrypted_key_detection() {
    let not = russh::keys::Error::CouldNotReadKey;
    assert!(needs_passphrase("", &russh::keys::Error::KeyIsEncrypted));
    assert!(needs_passphrase(
        "-----BEGIN ENCRYPTED PRIVATE KEY-----\n",
        &not
    ));
    assert!(needs_passphrase(
        "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: aes256-cbc\n",
        &not
    ));
    assert!(!needs_passphrase(
        "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\n",
        &not
    ));
    assert!(!needs_passphrase(
        "-----BEGIN OPENSSH PRIVATE KEY-----\n",
        &not
    ));
}
