//! The SFTP page: the panel and its transfer engine (`[sftp]`).

use std::path::PathBuf;

use fogu_core::Config;

use super::Section;
use super::spec::{Applies, Kind, Spec, Unit, Value, from_u64, from_usize, round, to_f32};

/// One `[sftp]` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SftpKey {
    DownloadDir,
    ShowHidden,
    FollowCwd,
    PanelWidth,
    TransferLinger,
    MaxParallel,
    KeepPartial,
    KeepExecutable,
    AllowGrowing,
    MaxDownload,
    FreeSpaceReserve,
    RequestTimeout,
    MaxListing,
    MaxWalkEntries,
    MaxWalkDepth,
}

impl SftpKey {
    /// Every key, in page order.
    pub const ALL: [Self; 15] = [
        Self::DownloadDir,
        Self::ShowHidden,
        Self::FollowCwd,
        Self::PanelWidth,
        Self::TransferLinger,
        Self::MaxParallel,
        Self::KeepPartial,
        Self::KeepExecutable,
        Self::AllowGrowing,
        Self::MaxDownload,
        Self::FreeSpaceReserve,
        Self::RequestTimeout,
        Self::MaxListing,
        Self::MaxWalkEntries,
        Self::MaxWalkDepth,
    ];

    /// Its description.
    #[expect(
        clippy::too_many_lines,
        reason = "the page's settings table: one short entry per setting"
    )]
    pub fn spec(self) -> Spec {
        let row = |group, key, label, help, keywords, kind, applies| Spec {
            section: Section::Sftp,
            group,
            path: &["sftp"],
            key,
            label,
            help,
            keywords,
            kind,
            applies,
        };
        let number = |min, max| Kind::Number { min, max };
        let panels = Applies::NewPanels;
        match self {
            Self::DownloadDir => row(
                "Panel",
                "download_dir",
                "Download folder",
                "Where a double-clicked file goes; empty is your Downloads folder.",
                "downloads folder directory save",
                Kind::Folder {
                    placeholder: "Downloads",
                },
                Applies::Live,
            ),
            Self::ShowHidden => row(
                "Panel",
                "show_hidden",
                "Show hidden files",
                "Dot files are listed when the panel opens.",
                "dotfiles hidden",
                Kind::Toggle,
                panels,
            ),
            Self::FollowCwd => row(
                "Panel",
                "follow_cwd",
                "Follow the terminal",
                "The panel goes where the remote shell is (needs OSC 7 from the shell).",
                "cwd follow directory osc7",
                Kind::Toggle,
                panels,
            ),
            Self::PanelWidth => row(
                "Panel",
                "panel_width",
                "Panel width",
                "Its share of the tab; drag its edge to resize, double-click to come back here.",
                "width size",
                Kind::Slider {
                    min: 0.15,
                    max: 0.85,
                    step: 0.05,
                    unit: Unit::Percent,
                    auto: None,
                },
                Applies::Live,
            ),
            Self::TransferLinger => row(
                "Transfers",
                "transfer_linger_secs",
                "Keep finished transfers listed",
                "Seconds a finished transfer stays in the list.",
                "transfers list linger seconds",
                Kind::Slider {
                    min: 0.0,
                    max: 120.0,
                    step: 1.0,
                    unit: Unit::Secs,
                    auto: None,
                },
                Applies::Live,
            ),
            Self::MaxParallel => row(
                "Transfers",
                "max_parallel_files",
                "Files at the same time",
                "Per connection.",
                "parallel concurrent files",
                Kind::Slider {
                    min: 1.0,
                    max: 16.0,
                    step: 1.0,
                    unit: Unit::Plain(0),
                    auto: None,
                },
                panels,
            ),
            Self::KeepPartial => row(
                "Transfers",
                "keep_partial",
                "Keep partial downloads",
                "A cancelled or failed download leaves <name>.part behind.",
                "partial resume cancel",
                Kind::Toggle,
                panels,
            ),
            Self::KeepExecutable => row(
                "Transfers",
                "keep_executable_bit",
                "Keep the executable bit",
                "Off: downloads are never executable, so a script cannot run from a \
                 double-click in your file manager.",
                "permissions chmod executable 0755",
                Kind::Toggle,
                panels,
            ),
            Self::AllowGrowing => row(
                "Limits",
                "allow_growing_files",
                "Allow growing files",
                "Download past the size the server reported (logs being written).",
                "growing size log",
                Kind::Toggle,
                panels,
            ),
            Self::MaxDownload => row(
                "Limits",
                "max_download_bytes",
                "Largest download",
                "Per file, like 2 GiB or 500 MB; empty is no limit beyond the reported size.",
                "size limit bytes",
                Kind::Bytes { optional: true },
                panels,
            ),
            Self::FreeSpaceReserve => row(
                "Limits",
                "free_space_reserve_bytes",
                "Free space to keep",
                "A download stops before the local disk has less than this left.",
                "disk space reserve bytes",
                Kind::Bytes { optional: false },
                panels,
            ),
            Self::RequestTimeout => row(
                "Limits",
                "request_timeout_secs",
                "Request timeout (s)",
                "How long one SFTP request may wait for its reply.",
                "timeout seconds",
                number(1, 3600),
                panels,
            ),
            Self::MaxListing => row(
                "Limits",
                "max_listing_entries",
                "Entries per folder",
                "More are left out of the listing, with a notice.",
                "listing entries limit",
                number(1, 10_000_000),
                panels,
            ),
            Self::MaxWalkEntries => row(
                "Limits",
                "max_walk_entries",
                "Entries per folder transfer",
                "A folder transfer walking more than this stops.",
                "recursive walk entries limit",
                number(1, 100_000_000),
                panels,
            ),
            Self::MaxWalkDepth => row(
                "Limits",
                "max_walk_depth",
                "Folder depth per transfer",
                "A folder transfer going deeper than this stops.",
                "recursive depth limit",
                number(1, 1024),
                panels,
            ),
        }
    }

    /// Its current value.
    pub fn get(self, config: &Config) -> Value {
        let s = &config.sftp;
        match self {
            Self::DownloadDir => Value::Text(
                s.download_dir
                    .as_ref()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default(),
            ),
            Self::ShowHidden => Value::Bool(s.show_hidden),
            Self::FollowCwd => Value::Bool(s.follow_cwd),
            Self::PanelWidth => Value::Num(f64::from(s.panel_width)),
            Self::TransferLinger => Value::Num(from_u64(s.transfer_linger_secs)),
            Self::MaxParallel => Value::Num(from_usize(s.max_parallel_files)),
            Self::KeepPartial => Value::Bool(s.keep_partial),
            Self::KeepExecutable => Value::Bool(s.keep_executable_bit),
            Self::AllowGrowing => Value::Bool(s.allow_growing_files),
            Self::MaxDownload => s
                .max_download_bytes
                .map_or(Value::Auto, |b| Value::Num(from_u64(b))),
            Self::FreeSpaceReserve => Value::Num(from_u64(s.free_space_reserve_bytes)),
            Self::RequestTimeout => Value::Num(from_u64(s.request_timeout_secs)),
            Self::MaxListing => Value::Num(from_usize(s.max_listing_entries)),
            Self::MaxWalkEntries => Value::Num(from_usize(s.max_walk_entries)),
            Self::MaxWalkDepth => Value::Num(from_usize(s.max_walk_depth)),
        }
    }

    /// Sets it in the running config.
    pub fn set(self, config: &mut Config, value: &Value) {
        let s = &mut config.sftp;
        let count = |v: &Value| usize::try_from(round(v.num())).unwrap_or(1).max(1);
        let secs = |v: &Value| u64::try_from(round(v.num())).unwrap_or(0);
        match self {
            Self::DownloadDir => {
                let text = value.text().trim();
                s.download_dir = (!text.is_empty()).then(|| PathBuf::from(text));
            }
            Self::ShowHidden => s.show_hidden = value.bool(),
            Self::FollowCwd => s.follow_cwd = value.bool(),
            Self::PanelWidth => s.panel_width = to_f32(value.num()),
            Self::TransferLinger => s.transfer_linger_secs = secs(value),
            Self::MaxParallel => s.max_parallel_files = count(value),
            Self::KeepPartial => s.keep_partial = value.bool(),
            Self::KeepExecutable => s.keep_executable_bit = value.bool(),
            Self::AllowGrowing => s.allow_growing_files = value.bool(),
            Self::MaxDownload => {
                s.max_download_bytes = match value {
                    Value::Auto => None,
                    v => Some(secs(v)),
                };
            }
            Self::FreeSpaceReserve => s.free_space_reserve_bytes = secs(value),
            Self::RequestTimeout => s.request_timeout_secs = secs(value).max(1),
            Self::MaxListing => s.max_listing_entries = count(value),
            Self::MaxWalkEntries => s.max_walk_entries = count(value),
            Self::MaxWalkDepth => s.max_walk_depth = count(value),
        }
    }
}
