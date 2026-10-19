<p align="center">
  <img src="assets/fogu.png" alt="Fogu" width="160">
</p>

<h1 align="center">Fogu Terminal</h1>

<p align="center">
  <strong>A native Linux terminal with a built-in SSH manager and SFTP browser.</strong><br>
  Written in Rust. No browser engine, no compromises on your daily workflow.
</p>

<p align="center">
  <img alt="Linux" src="https://img.shields.io/badge/platform-Linux-orange">
  <img alt="Rust" src="https://img.shields.io/badge/rust-1.99-orange">
  <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-red">
  <img alt="unsafe: forbidden" src="https://img.shields.io/badge/unsafe-forbidden-red">
</p>

---

## Why Fogu

Most terminals that ship a decent SSH manager are web apps in disguise: a few hundred megabytes of
browser engine to draw a grid of characters. The native alternatives are fast, but leave you
juggling `~/.ssh/config`, a separate SFTP client and a password manager.

Fogu is the terminal built for people who live on remote servers. Open the sidebar, click a host,
and you are in a shell. Open the SFTP panel and you are browsing the same machine, on the same
connection, already in the directory your shell is in. Everything starts in about a tenth of a
second and stays out of your way.

*Fogu* means *fire* in Sardinian.

## Features

### Terminal
- GPU rendering with [iced](https://iced.rs) (wgpu), automatic CPU fallback for broken drivers and VMs
- Terminal emulation by [`alacritty_terminal`](https://github.com/alacritty/alacritty): fast, correct, battle-tested
- Tabs with rename, colour and backgrounds (colour or image); split panes, resizable with the mouse
- Drag and drop for tabs, panes and profiles; maximize, equalize and resize from the keyboard
- Scrollback, mouse selection, copy to PRIMARY and CLIPBOARD, safe multi-line paste
- Mouse reporting for TUI apps, with Shift to override
- Clickable links: OSC 8 hyperlinks plus detected URLs and paths, behind a scheme allow-list
- Context menus everywhere, configurable right-click behaviour
- Tab recovery: layout and recent scrollback come back after a restart (opt-out)
- Colour schemes, fonts, cursor and every key binding configurable

### SSH manager
- Sidebar with groups, search and quick connect (`user@host:port`)
- Reads your `~/.ssh/config` live (Host, HostName, User, Port, IdentityFile, ProxyJump,
  ProxyCommand, HostKeyAlias…) — no import step, nothing duplicated
- Full profile editor that edits `config.toml` in place, keeping your comments
- Password, public key, ssh-agent and keyboard-interactive authentication
- Strict `known_hosts` checking: a changed host key is never accepted silently
- Jump hosts, proxy commands, SOCKS5 and HTTP proxies — a proxied profile never falls back to a
  direct connection
- Port forwarding: local, remote and dynamic (SOCKS5), managed live from a dialog
- Agent and X11 forwarding (off by default, clearly flagged when on)
- Connection reuse: tabs, splits and SFTP for the same host share one SSH connection
- Passwords and key passphrases stored in the desktop keyring (Secret Service), never on disk
- Login scripts, starting directory, remembered directory, automatic reconnect with backoff

### SFTP browser
- Side panel on the same SSH connection: no second login
- Follows the shell's working directory as you `cd`
- Upload, download, drag and drop, rename, delete, new folder, permissions
- Transfers with progress, cancel and resume
- Hardened against hostile servers: sanitised names, disk-fill protection, downloaded files never
  executable, nothing is ever launched from a remote listing

## Install

Fogu runs on Linux (X11 and Wayland). Build from source with a Rust toolchain
([rustup](https://rustup.rs) picks the pinned version automatically):

```bash
git clone https://github.com/Revdep/Fogu.git
cd Fogu
./scripts/install.sh              # release build, installs to ~/.local for the current user
fogu
```

`./scripts/install.sh --uninstall` removes exactly what was installed. No root needed.

Build dependencies on Debian/Ubuntu: `sudo apt install build-essential pkg-config libudev-dev libxkbcommon-dev libwayland-dev`.

## Configuration

Fogu works with no configuration. To customise it, edit `~/.config/fogu/config.toml`:

```toml
[appearance]
color_scheme = "Dracula"   # built in: Fogu Dark, Fogu Light, Solarized Dark, Dracula

[[profiles]]
id = "home-server"
name = "Home server"
type = "ssh"
host = "server.lan"
user = "me"
```

Hosts from `~/.ssh/config` appear in the sidebar automatically.

### Default keys

| Action | Keys |
|---|---|
| New tab / close tab | `Ctrl+Shift+T` / `Ctrl+Shift+W` |
| Profile picker | `Ctrl+Shift+E` |
| Profiles sidebar / SFTP panel | `Ctrl+Shift+M` / `Ctrl+Shift+O` |
| Split right / split down | `Ctrl+Shift+S` / `Ctrl+Shift+D` |
| Move between panes | `Ctrl+Alt+Arrows` |
| Copy / paste | `Ctrl+Shift+C` / `Ctrl+Shift+V` |
| Zoom | `Ctrl+=` / `Ctrl+-` / `Ctrl+0` |
| Full screen | `F11` |

Every action can be rebound under `[hotkeys]`.

### Command line

```
fogu [--config-dir PATH] [--software-render] [--no-recovery]
```

## Under the hood

Fogu is a Cargo workspace of five crates with one owner per concern:

| Crate | Role |
|---|---|
| `fogu-term` | Terminal state over `alacritty_terminal`, input encoding, OSC parsing, links |
| `fogu-session` | Local PTY and SSH sessions on Tokio: `russh`, SFTP, forwarding, proxies, keyring |
| `fogu-core` | Config, profiles, `~/.ssh/config` parsing, key map, colour schemes, split tree |
| `fogu-ui` | The iced application: tabs, panes, sidebar, SFTP panel, dialogs |
| `fogu` | The binary: command line, logging, crash log |

Design choices:

- **Pure Rust, no C crypto.** SSH via [`russh`](https://github.com/Eugeny/russh) on `ring`;
  `unsafe_code = "forbid"` for the whole workspace.
- **Remote text is attacker input.** Everything a server sends — terminal output, file names,
  banners, error messages — is sanitised before it reaches the UI or the logs.
- **Secrets stay secret.** Passwords live in `secrecy` types, never in messages, logs or errors,
  and are stored only in the desktop keyring.
- **Supply chain gate.** `cargo deny` checks advisories, licences and sources on every build; no
  vulnerability advisory is ever ignored.
- **Tested against the real thing.** Integration tests start a throwaway OpenSSH `sshd` and an
  in-process hostile SFTP server; parsers are fuzzed.

## Development

```bash
./scripts/check.sh    # fmt, clippy -D warnings, fuzz build, nextest, doc tests, cargo deny
cargo run             # debug build
```

The SSH integration tests need `sshd`, `ssh-keygen`, `ssh-agent` and `ssh-add`; without them they
are skipped.

## Acknowledgements

Fogu was inspired by [Tabby](https://github.com/Eugeny/tabby) and by many other terminals that
showed what a modern terminal can be. It stands on the shoulders of
[Alacritty](https://github.com/alacritty/alacritty), [iced](https://iced.rs),
[russh](https://github.com/Eugeny/russh) and [Tokio](https://tokio.rs). Third-party notices are in
[`THIRD_PARTY_LICENSES`](THIRD_PARTY_LICENSES).

## License

[MIT](LICENSE)
