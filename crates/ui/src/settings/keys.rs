//! The Keys page: the passwords and key passphrases the profiles use (set, replace, delete, and
//! reveal behind a recovery phrase), Fogu's `known_hosts`, the private keys in `~/.ssh` (and a
//! new ed25519 one), and the SSH agent.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use fogu_core::{EffectiveProfile, SecretKey, resolve};
use fogu_session::reveal::{GATE_FILE, RevealGate, new_phrase, phrase_valid};
use fogu_session::{
    AgentStatus, KeyFile, KeyGen, KnownHost, SshPaths, agent_status, generate_ed25519,
    key_fingerprint, list_known_hosts, remove_known_host, scan_key_dir,
};
use iced::widget::{Id, button, column, container, row, space, text};
use iced::{Alignment, Border, Element, Length, Task};
use secrecy::{ExposeSecret as _, SecretString};

use super::render::{badge_chip, field, small};
use super::{Confirm, SettingsMsg};
use crate::app::App;
use crate::controls::{self, ButtonKind};
use crate::message::Message;
use crate::notifications::{Level, Notification};
use crate::palette::Look;
use crate::prompt::PromptText;

/// How long a revealed secret stays on screen.
const SHOWN_FOR: Duration = Duration::from_secs(30);
/// Wrong phrases in a row before a pause.
const MAX_FAILURES: u32 = 5;
/// The pause.
const LOCKOUT: Duration = Duration::from_secs(30);

/// The ids of the page's blocks, for the search.
use super::{AGENT_ANCHOR, FILES_ANCHOR, HOSTS_ANCHOR, SECRETS_ANCHOR};

/// A secret a profile uses.
#[derive(Clone, Debug)]
pub struct SecretRow {
    /// Its keychain name.
    pub key: SecretKey,
    /// What it is ("Password for me@host", "Passphrase of `id_ed25519`").
    pub what: String,
    /// The profiles that use it.
    pub used_by: Vec<String>,
    /// Stored (`Some(true)`), not stored, or the keychain's error.
    pub stored: Option<Result<bool, String>>,
}

/// The page's state.
#[derive(Debug, Default)]
pub struct KeysPage {
    /// The secrets the profiles use.
    pub rows: Vec<SecretRow>,
    /// The recovery phrase gate (`None`: none made yet, or no state folder).
    pub gate: Option<RevealGate>,
    /// A new phrase, shown once until the user says it is written down.
    pub new_phrase: Option<SecretString>,
    /// "Replace" (`true`) or first setup.
    pub replacing_phrase: bool,
    /// The row whose value is being typed, and the text.
    pub setting: Option<(SecretKey, PromptText)>,
    /// The row whose reveal waits for the phrase, the phrase typed, and why the last try failed.
    pub unlock: Option<(SecretKey, PromptText, Option<String>)>,
    /// A revealed value, its row and the generation of its hide timer.
    pub revealed: Option<(SecretKey, SecretString, u64)>,
    reveal_gen: u64,
    /// Wrong phrases in a row, and until when the next try waits.
    pub failures: u32,
    pub locked_until: Option<Instant>,
    /// Hashing or a keychain call is running.
    pub busy: bool,
    /// Fogu's `known_hosts` and its records, and the system one.
    pub own_hosts: (PathBuf, Vec<KnownHost>),
    pub system_hosts: Option<(PathBuf, Vec<KnownHost>)>,
    /// The private keys in `~/.ssh`.
    pub files: Vec<KeyFile>,
    /// The new key form.
    pub generate: Option<GenForm>,
    /// What the agent answered (`None`: still asking).
    pub agent: Option<AgentStatus>,
}

/// The "New ed25519 key" form.
#[derive(Clone, Debug, Default)]
pub struct GenForm {
    pub name: String,
    pub comment: String,
    pub passphrase: Option<PromptText>,
    pub repeat: Option<PromptText>,
    pub error: Option<String>,
}

/// A choice on the Keys page.
#[derive(Clone, Debug)]
pub enum KeysMsg {
    /// Read everything again.
    Refresh,
    /// Which secrets are stored.
    Probed(Vec<(SecretKey, Result<bool, String>)>),
    /// Type a value for a secret.
    Set(SecretKey),
    /// The value typed.
    Value(PromptText),
    /// Store the typed value.
    Save,
    /// Stop typing.
    Cancel,
    /// A keychain write finished.
    Stored(SecretKey, bool, Result<(), String>),
    /// Delete a secret.
    Delete(SecretKey),
    /// Show a secret (asks for the phrase).
    Reveal(SecretKey),
    /// The phrase typed.
    Phrase(PromptText),
    /// Check the phrase.
    Unlock,
    /// The phrase was right (or not).
    Unlocked(SecretKey, bool),
    /// The secret's value.
    Revealed(SecretKey, Result<Option<PromptText>, String>),
    /// Hide revealed value of generation `.0`.
    Hide(u64),
    /// Copy the revealed value.
    CopyRevealed,
    /// Make a recovery phrase (shows it).
    NewPhrase,
    /// "I wrote it down": save the gate.
    PhraseSaved,
    /// The gate was written.
    GateWritten(Result<(), String>),
    /// Replace the phrase (asks first).
    ResetPhrase,
    /// Forget host-key record `line` of Fogu's `known_hosts` (holding key `.1`).
    RemoveHost(usize, String),
    /// Copy a public key.
    CopyPublic(String),
    /// Open / close the new key form.
    GenToggle,
    /// New key form fields.
    GenName(String),
    GenComment(String),
    GenPass(PromptText),
    GenRepeat(PromptText),
    /// Make the key.
    GenCreate,
    /// The key was made (its file name) or why not.
    Generated(Result<String, String>),
    /// The agent answered.
    Agent(AgentStatus),
}

fn msg(m: KeysMsg) -> Message {
    Message::Settings(SettingsMsg::Keys(m))
}

/// The page body.
pub fn body(page: &KeysPage, reveal_possible: bool, look: Look) -> Element<'_, Message> {
    column![
        block(SECRETS_ANCHOR, secrets(page, reveal_possible, look)),
        block(HOSTS_ANCHOR, hosts(page, look)),
        block(FILES_ANCHOR, files(page, look)),
        block(AGENT_ANCHOR, agent(page, look)),
    ]
    .spacing(22)
    .into()
}

fn block<'a>(id: &'static str, content: Element<'a, Message>) -> Element<'a, Message> {
    container(content)
        .id(Id::new(id))
        .padding([6, 8])
        .width(Length::Fill)
        .into()
}

fn flat<'a>(look: Look, label: &str, on: Option<Message>) -> Element<'a, Message> {
    button(
        text(controls::button_label(look, label))
            .size(12)
            .font(look.text.chrome),
    )
    .padding([3, 8])
    .style(move |_theme: &iced::Theme, status| {
        let style = controls::flat_button(look, status, false);
        if status == button::Status::Disabled {
            button::Style {
                text_color: look.dim,
                ..style
            }
        } else {
            style
        }
    })
    .on_press_maybe(on)
    .into()
}

fn mono<'a>(look: Look, value: &str) -> Element<'a, Message> {
    text(value.to_owned())
        .size(12)
        .font(look.text.mono)
        .color(look.fg)
        .into()
}

fn bordered(look: Look, content: Element<'_, Message>) -> Element<'_, Message> {
    container(content)
        .padding([6, 10])
        .width(Length::Fill)
        .style(move |_theme: &iced::Theme| container::Style {
            border: Border {
                color: look.border,
                width: 1.0,
                radius: look.shape.control.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Stored secrets and the recovery phrase.
fn secrets(page: &KeysPage, reveal_possible: bool, look: Look) -> Element<'_, Message> {
    let mut body = column![
        row![
            controls::section_label(look, "Stored secrets"),
            space().width(Length::Fill),
            flat(look, "Refresh", Some(msg(KeysMsg::Refresh))),
        ]
        .align_y(Alignment::Center),
        small(
            look,
            "Passwords and key passphrases the profiles use, kept in the system keychain. Only \
             secrets a profile uses are listed."
        ),
    ]
    .spacing(8);
    body = body.push(phrase_block(page, reveal_possible, look));
    if page.rows.is_empty() {
        body = body.push(small(
            look,
            "No SSH profile uses a password or a key file yet.",
        ));
    }
    for secret in &page.rows {
        body = body.push(secret_row(page, secret, reveal_possible, look));
    }
    body.into()
}

/// The recovery phrase: make it, show it once, replace it.
fn phrase_block(page: &KeysPage, reveal_possible: bool, look: Look) -> Element<'_, Message> {
    let honest = "The phrase only guards Fogu's Reveal button: anyone signed in to your \
                  account can read these secrets with the system's keychain tools, and Fogu \
                  reads them to connect without it.";
    if !reveal_possible {
        return small(
            look,
            "Reveal needs Fogu's state folder, which is not available.",
        );
    }
    if let Some(words) = &page.new_phrase {
        let list: Vec<&str> = words.expose_secret().split(' ').collect();
        let mut grid = column![].spacing(4);
        for (r, chunk) in list.chunks(4).enumerate() {
            let mut line = row![].spacing(18);
            for (c, word) in chunk.iter().enumerate() {
                line = line.push(
                    text(format!("{:>2}. {word}", r * 4 + c + 1))
                        .size(13)
                        .font(look.text.mono)
                        .color(look.fg)
                        .width(140),
                );
            }
            grid = grid.push(line);
        }
        return bordered(
            look,
            column![
                text(
                    "Write these 12 words down. They are shown once; without them a stored \
                      secret cannot be revealed (it can still be replaced or deleted)."
                )
                .size(12)
                .font(look.text.chrome)
                .color(look.fg),
                grid,
                row![
                    controls::action(
                        "I wrote them down",
                        ButtonKind::Primary,
                        look,
                        (!page.busy).then(|| msg(KeysMsg::PhraseSaved))
                    ),
                    small(look, honest),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            ]
            .spacing(10)
            .into(),
        );
    }
    let line = match &page.gate {
        Some(_) => row![
            badge_chip(look, "Recovery phrase set"),
            // Fills the gap and wraps, so the button keeps its room.
            container(small(look, honest)).width(Length::Fill),
            flat(look, "Replace phrase…", Some(msg(KeysMsg::ResetPhrase))),
        ],
        None => row![
            small(
                look,
                "Revealing a stored secret needs a recovery phrase: 12 words made once."
            ),
            space().width(Length::Fill),
            controls::action(
                "Make a recovery phrase",
                ButtonKind::Secondary,
                look,
                Some(msg(KeysMsg::NewPhrase))
            ),
        ],
    };
    line.spacing(10).align_y(Alignment::Center).into()
}

/// One secret: what it is, who uses it, whether it is stored, its buttons and inline editors.
#[expect(
    clippy::too_many_lines,
    reason = "one row with its three inline editors; splitting them apart hides the layout"
)]
fn secret_row<'a>(
    page: &'a KeysPage,
    secret: &SecretRow,
    reveal_possible: bool,
    look: Look,
) -> Element<'a, Message> {
    let key = secret.key.clone();
    let stored = matches!(secret.stored, Some(Ok(true)));
    let status = match &secret.stored {
        None => "checking…".to_owned(),
        Some(Ok(true)) => "stored".to_owned(),
        Some(Ok(false)) => "not stored".to_owned(),
        Some(Err(why)) => format!("unavailable: {why}"),
    };
    let revealable = page.gate.as_ref().is_some_and(|g| g.revealable(&key));
    let reveal = (stored && reveal_possible && page.gate.is_some() && revealable && !page.busy)
        .then(|| msg(KeysMsg::Reveal(key.clone())));
    let mut head = row![
        column![
            text(secret.what.clone())
                .size(13)
                .font(look.text.chrome)
                .color(look.fg),
            small(look, &format!("Used by {}", secret.used_by.join(", "))),
        ]
        .spacing(2)
        .width(Length::Fill),
        badge_chip(look, &status),
        flat(
            look,
            if stored { "Replace…" } else { "Set…" },
            Some(msg(KeysMsg::Set(key.clone())))
        ),
        flat(
            look,
            "Delete",
            stored.then(|| msg(KeysMsg::Delete(key.clone())))
        ),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    head = head.push(if stored && page.gate.is_some() && !revealable {
        controls::tip(
            flat(look, "Reveal", None),
            "Saved before the last phrase reset: it can be replaced, not revealed",
            iced::widget::tooltip::Position::Top,
            look,
        )
    } else {
        flat(look, "Reveal", reveal)
    });
    let mut body = column![head].spacing(6);
    if let Some((editing, value)) = &page.setting
        && editing == &key
    {
        body = body.push(
            row![
                field(look, "Value", value.as_str())
                    .secure(true)
                    .on_input(|t| msg(KeysMsg::Value(PromptText::new(t))))
                    .on_submit(msg(KeysMsg::Save))
                    .width(Length::Fill),
                controls::action(
                    "Save",
                    ButtonKind::Primary,
                    look,
                    (!value.is_empty() && !page.busy).then(|| msg(KeysMsg::Save))
                ),
                controls::action(
                    "Cancel",
                    ButtonKind::Secondary,
                    look,
                    Some(msg(KeysMsg::Cancel))
                ),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
    }
    if let Some((unlocking, phrase, error)) = &page.unlock
        && unlocking == &key
    {
        let locked = page.locked_until.is_some_and(|t| t > Instant::now());
        body = body.push(
            row![
                field(look, "Recovery phrase (12 words)", phrase.as_str())
                    .secure(true)
                    .on_input(|t| msg(KeysMsg::Phrase(PromptText::new(t))))
                    .on_submit(msg(KeysMsg::Unlock))
                    .width(Length::Fill),
                controls::action(
                    if page.busy { "Checking…" } else { "Reveal" },
                    ButtonKind::Primary,
                    look,
                    (!phrase.is_empty() && !page.busy && !locked).then(|| msg(KeysMsg::Unlock))
                ),
                controls::action(
                    "Cancel",
                    ButtonKind::Secondary,
                    look,
                    Some(msg(KeysMsg::Cancel))
                ),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
        let note = if locked {
            Some("Too many wrong phrases: wait 30 seconds.".to_owned())
        } else {
            error.clone()
        };
        if let Some(note) = note {
            body = body.push(
                text(note)
                    .size(11)
                    .font(look.text.chrome)
                    .color(super::render::ERROR),
            );
        }
    }
    if let Some((shown, value, _)) = &page.revealed
        && shown == &key
    {
        body = body.push(
            row![
                container(mono(look, value.expose_secret())).width(Length::Fill),
                flat(look, "Copy", Some(msg(KeysMsg::CopyRevealed))),
                flat(look, "Hide", Some(msg(KeysMsg::Hide(u64::MAX)))),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
    }
    bordered(look, body.into())
}

/// Fogu's `known_hosts` (removable records) and the system's (read-only).
fn hosts(page: &KeysPage, look: Look) -> Element<'_, Message> {
    let (own_path, own) = &page.own_hosts;
    let mut body = column![
        controls::section_label(look, "Known hosts"),
        small(
            look,
            &format!(
                "Host keys Fogu trusts, in {}. Removing one makes the next connection to that \
                 host ask about its key again.",
                own_path.display()
            )
        ),
    ]
    .spacing(6);
    if own.is_empty() {
        body = body.push(small(look, "None yet."));
    }
    for host in own {
        body = body.push(host_row(host, true, look));
    }
    if let Some((path, system)) = &page.system_hosts {
        body = body.push(space().height(6));
        body = body.push(small(
            look,
            &format!(
                "Also trusted, read-only: {} ({} records).",
                path.display(),
                system.len()
            ),
        ));
        for host in system.iter().take(50) {
            body = body.push(host_row(host, false, look));
        }
    }
    body.into()
}

fn host_row<'a>(host: &KnownHost, removable: bool, look: Look) -> Element<'a, Message> {
    let mut line = row![
        column![
            text(host.hosts.clone())
                .size(12)
                .font(look.text.chrome)
                .color(look.fg),
            small(look, &format!("{} {}", host.algorithm, host.fingerprint)),
        ]
        .spacing(2)
        .width(Length::Fill),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    if let Some(marker) = &host.marker {
        line = line.push(badge_chip(look, marker));
    }
    if removable {
        line = line.push(flat(
            look,
            "Remove",
            Some(msg(KeysMsg::RemoveHost(
                host.line,
                host.fingerprint.clone(),
            ))),
        ));
    }
    line.into()
}

/// The private keys in `~/.ssh` and the new key form.
fn files(page: &KeysPage, look: Look) -> Element<'_, Message> {
    let mut body = column![
        row![
            controls::section_label(look, "Key files"),
            space().width(Length::Fill),
            controls::action(
                if page.generate.is_some() {
                    "Close"
                } else {
                    "New ed25519 key"
                },
                ButtonKind::Secondary,
                look,
                Some(msg(KeysMsg::GenToggle))
            ),
        ]
        .align_y(Alignment::Center),
        small(
            look,
            "The private keys in ~/.ssh. Their secret parts never leave the files."
        ),
    ]
    .spacing(6);
    if let Some(form) = &page.generate {
        body = body.push(gen_form(form, page.busy, look));
    }
    if page.files.is_empty() {
        body = body.push(small(look, "No private keys in ~/.ssh."));
    }
    for file in &page.files {
        let name = file
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut title = row![text(name).size(13).font(look.text.chrome).color(look.fg)]
            .spacing(8)
            .align_y(Alignment::Center);
        title = title.push(badge_chip(look, &file.algorithm));
        if file.encrypted {
            title = title.push(badge_chip(look, "passphrase"));
        }
        let detail = match (&file.fingerprint, file.comment.is_empty()) {
            (Some(fp), true) => fp.clone(),
            (Some(fp), false) => format!("{fp} · {}", file.comment),
            (None, _) => "no public key next to it".to_owned(),
        };
        body = body.push(bordered(
            look,
            row![
                column![title, small(look, &detail)]
                    .spacing(2)
                    .width(Length::Fill),
                flat(
                    look,
                    "Copy public key",
                    file.public.clone().map(|p| msg(KeysMsg::CopyPublic(p)))
                ),
            ]
            .align_y(Alignment::Center)
            .into(),
        ));
    }
    body.into()
}

fn gen_form(form: &GenForm, busy: bool, look: Look) -> Element<'_, Message> {
    let empty = PromptText::new(String::new());
    let pass = form
        .passphrase
        .as_ref()
        .unwrap_or(&empty)
        .as_str()
        .to_owned();
    let repeat = form.repeat.as_ref().unwrap_or(&empty).as_str().to_owned();
    let mut body = column![
        row![
            field(look, "File name", &form.name)
                .on_input(|t| msg(KeysMsg::GenName(t)))
                .width(Length::FillPortion(1)),
            field(look, "Comment (user@host)", &form.comment)
                .on_input(|t| msg(KeysMsg::GenComment(t)))
                .width(Length::FillPortion(1)),
        ]
        .spacing(6),
        row![
            field(look, "Passphrase (empty: none)", &pass)
                .secure(true)
                .on_input(|t| msg(KeysMsg::GenPass(PromptText::new(t))))
                .width(Length::FillPortion(1)),
            field(look, "Passphrase again", &repeat)
                .secure(true)
                .on_input(|t| msg(KeysMsg::GenRepeat(PromptText::new(t))))
                .on_submit(msg(KeysMsg::GenCreate))
                .width(Length::FillPortion(1)),
        ]
        .spacing(6),
        row![
            controls::action(
                "Create key",
                ButtonKind::Primary,
                look,
                (!busy).then(|| msg(KeysMsg::GenCreate))
            ),
            small(look, "Written to ~/.ssh, never over an existing file."),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    ]
    .spacing(6);
    if let Some(error) = &form.error {
        body = body.push(
            text(error.clone())
                .size(11)
                .font(look.text.chrome)
                .color(super::render::ERROR),
        );
    }
    bordered(look, body.into())
}

/// What the agent answered.
fn agent(page: &KeysPage, look: Look) -> Element<'_, Message> {
    let mut body = column![
        row![
            controls::section_label(look, "SSH agent"),
            space().width(Length::Fill),
            flat(look, "Refresh", Some(msg(KeysMsg::Refresh))),
        ]
        .align_y(Alignment::Center),
    ]
    .spacing(6);
    match &page.agent {
        None => body = body.push(small(look, "Asking the agent…")),
        Some(AgentStatus::Unavailable(why)) => {
            body = body.push(small(look, &format!("Not available: {why}.")));
        }
        Some(AgentStatus::Ready { endpoint, keys }) => {
            body = body.push(small(
                look,
                &format!("{} — {} keys", endpoint.display(), keys.len()),
            ));
            for key in keys {
                body = body.push(small(
                    look,
                    &format!("{} {} {}", key.algorithm, key.fingerprint, key.comment),
                ));
            }
        }
    }
    body.into()
}

impl App {
    /// The Keys page is shown or refreshed: secrets, hosts, key files and the agent.
    pub(crate) fn keys_page_shown(&mut self) -> Task<Message> {
        let rows = secret_rows(self);
        let gate = self
            .state_dir
            .as_ref()
            .and_then(|dir| RevealGate::load(&dir.join(GATE_FILE)).ok().flatten());
        let paths =
            SshPaths::in_config_dir(&fogu_core::config::config_dir(self.config_dir.as_deref()));
        let own = list_known_hosts(&paths.known_hosts);
        let system = paths
            .system_known_hosts
            .clone()
            .map(|p| (p.clone(), list_known_hosts(&p)));
        let files = fogu_core::platform::home_dir()
            .map(|h| scan_key_dir(&h.join(".ssh")))
            .unwrap_or_default();
        let Some(page) = self.settings.as_mut() else {
            return Task::none();
        };
        let keys = &mut page.keys;
        let keep_new = keys.new_phrase.take();
        let replacing = keys.replacing_phrase;
        *keys = KeysPage {
            rows: rows.clone(),
            gate,
            new_phrase: keep_new,
            replacing_phrase: replacing,
            own_hosts: (paths.known_hosts, own),
            system_hosts: system,
            files,
            ..KeysPage::default()
        };
        let wanted: Vec<SecretKey> = rows.into_iter().map(|r| r.key).collect();
        let secrets = self.runtime.secrets().clone();
        let probe = Task::perform(
            async move {
                let mut out = Vec::new();
                for key in wanted {
                    let state = secrets.exists(&key).await.map_err(|e| e.to_string());
                    out.push((key, state));
                }
                out
            },
            |found| msg(KeysMsg::Probed(found)),
        );
        // The agent's pipe or socket needs the session runtime's reactor.
        let agent = self.runtime.handle().spawn(agent_status());
        let agent = Task::perform(agent, |s| {
            msg(KeysMsg::Agent(s.unwrap_or_else(|e| {
                AgentStatus::Unavailable(e.to_string())
            })))
        });
        Task::batch([probe, agent])
    }

    /// A choice on the Keys page.
    #[expect(
        clippy::too_many_lines,
        reason = "one dispatch over the page's messages; each arm is short"
    )]
    pub(crate) fn on_keys(&mut self, message: KeysMsg) -> Task<Message> {
        let gate_path = self.state_dir.as_ref().map(|d| d.join(GATE_FILE));
        let secrets = self.runtime.secrets().clone();
        let Some(page) = self.settings.as_mut().map(|p| &mut p.keys) else {
            return Task::none();
        };
        match message {
            KeysMsg::Refresh => return self.keys_page_shown(),
            KeysMsg::Probed(found) => {
                let found: BTreeMap<String, Result<bool, String>> =
                    found.into_iter().map(|(k, s)| (k.label(), s)).collect();
                for row in &mut page.rows {
                    row.stored = found.get(&row.key.label()).cloned();
                }
            }
            KeysMsg::Set(key) => {
                page.unlock = None;
                page.setting = Some((key, PromptText::new(String::new())));
            }
            KeysMsg::Value(text) => {
                if let Some((_, value)) = &mut page.setting {
                    *value = text;
                }
            }
            KeysMsg::Save => {
                let Some((key, value)) = page.setting.take().filter(|(_, v)| !v.is_empty()) else {
                    return Task::none();
                };
                page.busy = true;
                let value = value.into_secret();
                return Task::perform(
                    async move {
                        let done = secrets.set(&key, value).await.map_err(|e| e.to_string());
                        (key, done)
                    },
                    |(key, done)| msg(KeysMsg::Stored(key, true, done)),
                );
            }
            KeysMsg::Cancel => {
                page.setting = None;
                page.unlock = None;
            }
            KeysMsg::Stored(key, set, done) => {
                page.busy = false;
                if let Err(why) = done {
                    return self.notify(
                        Notification::new(Level::Error, "Keychain not changed").detail(why),
                    );
                }
                if let Some(gate) = &mut page.gate {
                    let recorded = if set {
                        gate.record(&key)
                    } else {
                        gate.forget(&key)
                    };
                    if let Err(err) = recorded {
                        tracing::warn!(error = %err, "reveal gate not updated");
                    }
                }
                return self.keys_page_shown();
            }
            KeysMsg::Delete(key) => {
                page.busy = true;
                return Task::perform(
                    async move {
                        let done = secrets.delete(&key).await.map_err(|e| e.to_string());
                        (key, done)
                    },
                    |(key, done)| msg(KeysMsg::Stored(key, false, done)),
                );
            }
            KeysMsg::Reveal(key) => {
                page.setting = None;
                page.unlock = Some((key, PromptText::new(String::new()), None));
            }
            KeysMsg::Phrase(text) => {
                if let Some((_, phrase, error)) = &mut page.unlock {
                    *phrase = text;
                    *error = None;
                }
            }
            KeysMsg::Unlock => {
                if page.locked_until.is_some_and(|t| t > Instant::now()) {
                    return Task::none();
                }
                let (Some(path), Some((key, phrase, error))) = (gate_path, page.unlock.as_mut())
                else {
                    return Task::none();
                };
                if !phrase_valid(phrase.as_str()) {
                    *error = Some("Not 12 words of the list: check the spelling.".to_owned());
                    return Task::none();
                }
                let key = key.clone();
                let phrase = SecretString::from(phrase.as_str());
                page.busy = true;
                let check = self.runtime.handle().spawn_blocking(move || {
                    RevealGate::load(&path)
                        .ok()
                        .flatten()
                        .is_some_and(|g| g.verify(&phrase))
                });
                return Task::perform(check, move |ok| {
                    msg(KeysMsg::Unlocked(key.clone(), matches!(ok, Ok(true))))
                });
            }
            KeysMsg::Unlocked(key, ok) => {
                page.busy = false;
                if !ok {
                    page.failures += 1;
                    if page.failures >= MAX_FAILURES {
                        page.failures = 0;
                        page.locked_until = Some(Instant::now() + LOCKOUT);
                    }
                    if let Some((_, _, error)) = &mut page.unlock {
                        *error = Some("That is not the recovery phrase.".to_owned());
                    }
                    return Task::none();
                }
                page.failures = 0;
                page.unlock = None;
                return Task::perform(
                    async move {
                        let value = secrets
                            .get(&key)
                            .await
                            .map(|v| v.map(|s| PromptText::new(s.expose_secret().to_owned())))
                            .map_err(|e| e.to_string());
                        (key, value)
                    },
                    |(key, value)| msg(KeysMsg::Revealed(key, value)),
                );
            }
            KeysMsg::Revealed(key, value) => match value {
                Ok(Some(value)) => {
                    page.reveal_gen += 1;
                    let generation = page.reveal_gen;
                    page.revealed = Some((key, value.into_secret(), generation));
                    return self.after(SHOWN_FOR, msg(KeysMsg::Hide(generation)));
                }
                Ok(None) => {
                    return self.notify(Notification::new(Level::Warn, "Nothing stored"));
                }
                Err(why) => {
                    return self.notify(
                        Notification::new(Level::Error, "Keychain not readable").detail(why),
                    );
                }
            },
            KeysMsg::Hide(generation) => {
                if page
                    .revealed
                    .as_ref()
                    .is_some_and(|(_, _, g)| generation == u64::MAX || *g == generation)
                {
                    page.revealed = None;
                }
            }
            KeysMsg::CopyRevealed => {
                if let Some((_, value, _)) = page.revealed.take() {
                    return iced::clipboard::write(value.expose_secret().to_owned());
                }
            }
            KeysMsg::NewPhrase => match new_phrase() {
                Ok(phrase) => {
                    page.replacing_phrase = page.gate.is_some();
                    page.new_phrase = Some(phrase);
                }
                Err(err) => {
                    return self.notify(
                        Notification::new(Level::Error, "No recovery phrase")
                            .detail(err.to_string()),
                    );
                }
            },
            KeysMsg::ResetPhrase => {
                if let Some(settings) = &mut self.settings {
                    settings.confirm = Some(Confirm::ResetPhrase);
                }
            }
            KeysMsg::PhraseSaved => {
                let (Some(path), Some(phrase)) = (gate_path, page.new_phrase.take()) else {
                    return Task::none();
                };
                let stored: Vec<SecretKey> = page
                    .rows
                    .iter()
                    .filter(|r| matches!(r.stored, Some(Ok(true))))
                    .map(|r| r.key.clone())
                    .collect();
                let replacing = page.replacing_phrase;
                page.busy = true;
                let write = self.runtime.handle().spawn_blocking(move || {
                    let done = if replacing {
                        RevealGate::load(&path).and_then(|gate| match gate {
                            Some(mut gate) => gate.reset(&phrase, &stored),
                            None => RevealGate::setup(&path, &phrase, &stored).map(|_| ()),
                        })
                    } else {
                        RevealGate::setup(&path, &phrase, &stored).map(|_| ())
                    };
                    done.map_err(|e| e.to_string())
                });
                return Task::perform(write, |done| {
                    msg(KeysMsg::GateWritten(
                        done.unwrap_or_else(|e| Err(e.to_string())),
                    ))
                });
            }
            KeysMsg::GateWritten(done) => {
                page.busy = false;
                if let Err(why) = done {
                    return self.notify(
                        Notification::new(Level::Error, "Recovery phrase not saved").detail(why),
                    );
                }
                return self.keys_page_shown();
            }
            KeysMsg::RemoveHost(line, fingerprint) => {
                let path = page.own_hosts.0.clone();
                if let Err(err) = remove_known_host(&path, line, &fingerprint) {
                    return self.notify(
                        Notification::new(Level::Error, "Host key not removed")
                            .detail(err.to_string()),
                    );
                }
                return self.keys_page_shown();
            }
            KeysMsg::CopyPublic(line) => {
                let note = Notification::new(Level::Info, "Public key copied");
                return Task::batch([iced::clipboard::write(line), self.notify(note)]);
            }
            KeysMsg::GenToggle => {
                page.generate = match page.generate.take() {
                    Some(_) => None,
                    None => Some(GenForm {
                        name: unused_key_name(&page.files),
                        comment: default_comment(),
                        ..GenForm::default()
                    }),
                };
            }
            KeysMsg::GenName(name) => {
                if let Some(form) = &mut page.generate {
                    form.name = name;
                }
            }
            KeysMsg::GenComment(comment) => {
                if let Some(form) = &mut page.generate {
                    form.comment = comment;
                }
            }
            KeysMsg::GenPass(pass) => {
                if let Some(form) = &mut page.generate {
                    form.passphrase = Some(pass);
                }
            }
            KeysMsg::GenRepeat(pass) => {
                if let Some(form) = &mut page.generate {
                    form.repeat = Some(pass);
                }
            }
            KeysMsg::GenCreate => {
                let Some(form) = &mut page.generate else {
                    return Task::none();
                };
                let pass = form.passphrase.as_ref().map_or("", PromptText::as_str);
                let repeat = form.repeat.as_ref().map_or("", PromptText::as_str);
                if pass != repeat {
                    form.error = Some("The two passphrases differ.".to_owned());
                    return Task::none();
                }
                let Some(dir) = fogu_core::platform::home_dir().map(|h| h.join(".ssh")) else {
                    form.error = Some("No home folder.".to_owned());
                    return Task::none();
                };
                let req = KeyGen {
                    dir,
                    name: form.name.trim().to_owned(),
                    comment: form.comment.trim().to_owned(),
                    passphrase: (!pass.is_empty()).then(|| SecretString::from(pass)),
                };
                form.error = None;
                page.busy = true;
                let make = self.runtime.handle().spawn_blocking(move || {
                    generate_ed25519(&req)
                        .map(|k| k.path.display().to_string())
                        .map_err(|e| e.to_string())
                });
                return Task::perform(make, |done| {
                    msg(KeysMsg::Generated(
                        done.unwrap_or_else(|e| Err(e.to_string())),
                    ))
                });
            }
            KeysMsg::Generated(done) => {
                page.busy = false;
                match done {
                    Ok(path) => {
                        page.generate = None;
                        let note = Notification::new(Level::Info, "Key created").detail(path);
                        return Task::batch([self.notify(note), self.keys_page_shown()]);
                    }
                    Err(why) => {
                        if let Some(form) = &mut page.generate {
                            form.error = Some(why);
                        }
                    }
                }
            }
            KeysMsg::Agent(status) => page.agent = Some(status),
        }
        Task::none()
    }
}

/// Every secret the SSH profiles (stored and from `~/.ssh/config`) use, once each, with the
/// profiles that use it.
fn secret_rows(app: &App) -> Vec<SecretRow> {
    let mut rows: Vec<SecretRow> = Vec::new();
    let mut add = |key: SecretKey, what: String, profile: &str| match rows
        .iter_mut()
        .find(|r| r.key == key)
    {
        Some(row) if !row.used_by.iter().any(|u| u == profile) => {
            row.used_by.push(profile.to_owned());
        }
        Some(_) => {}
        None => rows.push(SecretRow {
            key,
            what,
            used_by: vec![profile.to_owned()],
            stored: None,
        }),
    };
    let profiles = app
        .config
        .profiles
        .iter()
        .chain(&app.profiles.ssh_hosts.profiles);
    for profile in profiles {
        let EffectiveProfile::Ssh(ssh) = resolve(profile, &app.config.profile_defaults) else {
            continue;
        };
        let name = fogu_term::sanitize_title(&profile.name);
        if !ssh.host.is_empty() {
            let what = if ssh.port == 22 {
                format!("Password for {}@{}", ssh.user, ssh.host)
            } else {
                format!("Password for {}@{}:{}", ssh.user, ssh.host, ssh.port)
            };
            add(SecretKey::ssh_password(&ssh), what, &name);
        }
        for path in &ssh.private_keys {
            let Some(key_fingerprint) = key_fingerprint(path) else {
                continue;
            };
            let file = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            add(
                SecretKey::KeyPassphrase { key_fingerprint },
                format!("Passphrase of {file}"),
                &name,
            );
        }
    }
    rows
}

/// `id_ed25519`, or `id_ed25519_2`, … — a name no key in `~/.ssh` has.
fn unused_key_name(files: &[KeyFile]) -> String {
    let taken = |name: &str| {
        files
            .iter()
            .any(|f| f.path.file_name().is_some_and(|n| n == name))
            || fogu_core::platform::home_dir().is_some_and(|h| h.join(".ssh").join(name).exists())
    };
    if !taken("id_ed25519") {
        return "id_ed25519".to_owned();
    }
    (2..=999)
        .map(|n| format!("id_ed25519_{n}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| "id_ed25519_new".to_owned())
}

/// `user@computer`, the usual key comment.
fn default_comment() -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    match (user.is_empty(), host.is_empty()) {
        (false, false) => format!("{user}@{}", host.to_lowercase()),
        (false, true) => user,
        _ => String::new(),
    }
}
