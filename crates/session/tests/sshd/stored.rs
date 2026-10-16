//! Stored passwords and key passphrases with the in-memory store: a stored secret is used
//! without a prompt, one the server refuses is asked again exactly once (never looped) and can be
//! replaced or forgotten, a prompt's secret is stored only after it proved right, a missing store
//! is shown and the prompt still works.

use fogu_core::{EffectiveSsh, SecretKey, SshAuth};
use fogu_session::{
    PromptKind, PromptReply, Remember, SecretStore, SessionError, SessionOutput, SessionWarning,
    SshPaths, key_fingerprint,
};
use secrecy::{ExposeSecret as _, SecretString};

use super::auth::{fake_paths, fake_profile};
use super::{error_of, no_prompts, secret};
use crate::support::client::Client;
use crate::support::fake_server::{self, FakeServer};
use crate::support::sshd::TestSshd;
use crate::support::tmp::TempDir;

fn password_key(ssh: &EffectiveSsh) -> SecretKey {
    SecretKey::SshPassword {
        user: ssh.user.clone(),
        host: ssh.host.clone(),
        port: ssh.port,
    }
}

/// An in-memory store already holding `entries`.
fn store_with(entries: &[(&SecretKey, &str)]) -> SecretStore {
    let store = SecretStore::in_memory();
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    for (key, value) in entries {
        rt.block_on(store.set(key, &secret(value))).unwrap();
    }
    store
}

fn stored<F: FnMut(&PromptKind) -> PromptReply>(
    client: &Client<F>,
    key: &SecretKey,
) -> Option<String> {
    let secrets = client.rt.secrets().clone();
    client
        .rt
        .handle()
        .block_on(secrets.get(key))
        .unwrap()
        .map(|s| s.expose_secret().to_owned())
}

fn typed(text: &str, remember: bool) -> PromptReply {
    PromptReply::Secret {
        value: secret(text),
        remember,
    }
}

fn passwords(asked: &[PromptKind]) -> Vec<Remember> {
    asked
        .iter()
        .filter_map(|k| match k {
            PromptKind::Password { remember, .. } => Some(remember.clone()),
            _ => None,
        })
        .collect()
}

fn connect<F: FnMut(&PromptKind) -> PromptReply>(
    server: &FakeServer,
    dir: &TempDir,
    store: SecretStore,
    answer: F,
) -> Client<F> {
    let mut ssh = fake_profile(server, SshAuth::Password);
    ssh.clear_service_messages_on_connect = false;
    Client::connect_with(&ssh, fake_paths(dir), store, answer)
}

#[test]
fn a_stored_password_logs_in_without_a_prompt() {
    let server = FakeServer::start();
    let dir = TempDir::new("stored-pw");
    let key = password_key(&fake_profile(&server, SshAuth::Password));
    let store = store_with(&[(&key, fake_server::PASSWORD)]);
    let mut client = connect(&server, &dir, store, |kind| match kind {
        PromptKind::HostKeyUnknown { .. } => PromptReply::Accept,
        other => panic!("unexpected prompt: {other:?}"),
    });
    client.expect_text(fake_server::READY);
    client.expect_text("Using the stored password");
    assert!(!client.view.dump_grid().contains(fake_server::PASSWORD));
}

#[test]
fn a_rejected_stored_password_is_asked_once_and_can_be_replaced() {
    let server = FakeServer::start();
    let dir = TempDir::new("stored-bad");
    let key = password_key(&fake_profile(&server, SshAuth::Password));
    let store = store_with(&[(&key, "old password")]);
    let mut client = connect(&server, &dir, store, |kind| match kind {
        PromptKind::HostKeyUnknown { .. } => PromptReply::Accept,
        PromptKind::Password { .. } => typed(fake_server::PASSWORD, true),
        other => panic!("unexpected prompt: {other:?}"),
    });
    client.expect_text(fake_server::READY);
    // One prompt (the stored password is not retried), flagged as a rejected stored secret.
    assert_eq!(passwords(&client.prompter.asked), [Remember::Rejected]);
    client.expect_text("Stored password rejected");
    client.expect_text("Saved the password");
    assert_eq!(
        stored(&client, &key).as_deref(),
        Some(fake_server::PASSWORD)
    );
}

#[test]
fn forgetting_a_rejected_stored_password_deletes_it_and_asks_again() {
    let server = FakeServer::start();
    let dir = TempDir::new("stored-forget");
    let key = password_key(&fake_profile(&server, SshAuth::Password));
    let store = store_with(&[(&key, "old password")]);
    let mut first = true;
    let mut client = connect(&server, &dir, store, |kind| match kind {
        PromptKind::HostKeyUnknown { .. } => PromptReply::Accept,
        PromptKind::Password { .. } if std::mem::take(&mut first) => PromptReply::Forget,
        PromptKind::Password { .. } => typed(fake_server::PASSWORD, false),
        other => panic!("unexpected prompt: {other:?}"),
    });
    client.expect_text(fake_server::READY);
    assert_eq!(
        passwords(&client.prompter.asked),
        [Remember::Rejected, Remember::Available]
    );
    assert_eq!(stored(&client, &key), None);
}

#[test]
fn a_typed_password_is_stored_only_after_the_server_accepted_it() {
    let server = FakeServer::start();
    let dir = TempDir::new("stored-remember");
    let key = password_key(&fake_profile(&server, SshAuth::Password));
    let mut tries = 0;
    let mut client = connect(&server, &dir, SecretStore::in_memory(), |kind| match kind {
        PromptKind::HostKeyUnknown { .. } => PromptReply::Accept,
        PromptKind::Password { .. } => {
            tries += 1;
            typed(
                if tries == 1 {
                    "wrong"
                } else {
                    fake_server::PASSWORD
                },
                true,
            )
        }
        other => panic!("unexpected prompt: {other:?}"),
    });
    client.expect_text(fake_server::READY);
    assert_eq!(
        stored(&client, &key).as_deref(),
        Some(fake_server::PASSWORD)
    );
    // Without "Remember" nothing is stored.
    let dir = TempDir::new("stored-no-remember");
    let mut client = connect(&server, &dir, SecretStore::in_memory(), |kind| match kind {
        PromptKind::HostKeyUnknown { .. } => PromptReply::Accept,
        _ => typed(fake_server::PASSWORD, false),
    });
    client.expect_text(fake_server::READY);
    assert_eq!(stored(&client, &key), None);
}

#[test]
fn a_missing_store_is_shown_and_the_prompt_still_works() {
    let server = FakeServer::start();
    let dir = TempDir::new("stored-off");
    let mut client = connect(
        &server,
        &dir,
        SecretStore::Disabled("no keyring here".into()),
        |kind| match kind {
            PromptKind::HostKeyUnknown { .. } => PromptReply::Accept,
            PromptKind::Password { .. } => typed(fake_server::PASSWORD, true),
            other => panic!("unexpected prompt: {other:?}"),
        },
    );
    let warning =
        client.until(|e| matches!(e, SessionOutput::Warning(SessionWarning::SecretStore(_))));
    let Some(SessionOutput::Warning(SessionWarning::SecretStore(text))) = warning else {
        panic!("no warning; screen:\n{}", client.view.dump_grid());
    };
    assert_eq!(text, "Stored secrets unavailable: no keyring here");
    client.expect_text(fake_server::READY);
    client.expect_text("Stored secrets unavailable: no keyring here");
    assert_eq!(
        passwords(&client.prompter.asked),
        [Remember::Unavailable("no keyring here".into())]
    );
}

#[test]
fn stored_secrets_off_never_touches_the_store_and_offers_no_remember() {
    let server = FakeServer::start();
    let dir = TempDir::new("stored-opt-out");
    let mut ssh = fake_profile(&server, SshAuth::Password);
    ssh.stored_secrets = false;
    ssh.clear_service_messages_on_connect = false;
    let key = password_key(&ssh);
    let store = store_with(&[(&key, fake_server::PASSWORD)]);
    let mut client = Client::connect_with(&ssh, fake_paths(&dir), store, |kind| match kind {
        PromptKind::HostKeyUnknown { .. } => PromptReply::Accept,
        PromptKind::Password { .. } => typed(fake_server::PASSWORD, true),
        other => panic!("unexpected prompt: {other:?}"),
    });
    client.expect_text(fake_server::READY);
    // The stored password was ignored (a prompt came) and nothing was warned about.
    let asked = passwords(&client.prompter.asked);
    let [Remember::Unavailable(why)] = &asked[..] else {
        panic!("{:?}", client.prompter.asked);
    };
    assert!(why.contains("stored_secrets"), "{why}");
}

#[test]
fn three_wrong_typed_passwords_still_fail_after_a_rejected_stored_one() {
    let server = FakeServer::start();
    let dir = TempDir::new("stored-fail");
    let key = password_key(&fake_profile(&server, SshAuth::Password));
    let store = store_with(&[(&key, "old password")]);
    let mut client = connect(&server, &dir, store, |kind| match kind {
        PromptKind::HostKeyUnknown { .. } => PromptReply::Accept,
        _ => typed("wrong", true),
    });
    assert!(matches!(error_of(client.end()), SessionError::AuthFailed));
    assert_eq!(passwords(&client.prompter.asked).len(), 3);
    assert_eq!(stored(&client, &key).as_deref(), Some("old password"));
}

fn locked_key(sshd: &TestSshd, pass: &str) -> (std::path::PathBuf, SecretKey, EffectiveSsh) {
    let path = sshd.new_key("stored-locked", "ed25519", pass, true);
    let key = SecretKey::KeyPassphrase {
        key_fingerprint: key_fingerprint(&path).unwrap(),
    };
    let mut ssh = sshd.profile();
    ssh.private_keys = vec![path.clone()];
    ssh.clear_service_messages_on_connect = false;
    (path, key, ssh)
}

fn paths(sshd: &TestSshd) -> SshPaths {
    sshd.paths()
}

#[test]
fn a_stored_key_passphrase_unlocks_the_key_without_a_prompt() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    sshd.trust();
    let (path, key, ssh) = locked_key(&sshd, "s3cret pass");
    let store = store_with(&[(&key, "s3cret pass")]);
    let mut client = Client::connect_with(&ssh, paths(&sshd), store, no_prompts);
    client.wait_shell();
    client.expect_text("Using the stored passphrase");
    assert!(client.prompter.asked.is_empty(), "{path:?}");
}

#[test]
fn a_wrong_stored_passphrase_is_asked_once_and_a_remembered_one_is_stored() {
    let Some(sshd) = TestSshd::start() else {
        return;
    };
    sshd.trust();
    let (_path, key, ssh) = locked_key(&sshd, "s3cret pass");
    let store = store_with(&[(&key, "stale")]);
    let mut client = Client::connect_with(&ssh, paths(&sshd), store, |kind| match kind {
        PromptKind::KeyPassphrase { remember, .. } => {
            assert_eq!(*remember, Remember::Rejected);
            typed("s3cret pass", true)
        }
        other => panic!("unexpected prompt: {other:?}"),
    });
    client.wait_shell();
    assert_eq!(client.prompter.asked.len(), 1);
    assert_eq!(stored(&client, &key).as_deref(), Some("s3cret pass"));
}

#[test]
fn secrets_never_reach_the_screen_or_a_debug_dump() {
    let reply = typed("hunter2-secret", true);
    assert!(!format!("{reply:?}").contains("hunter2"));
    let text: SecretString = secret("hunter2-secret");
    assert!(!format!("{text:?}").contains("hunter2"));
}
