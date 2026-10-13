//! `fogu`: entry point. Initialises logging (stderr + daily file) and runs the GUI.
//!
//! Takes no CLI flags yet.

use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// Own crates at `info`, dependencies at `warn`; `RUST_LOG` overrides.
const DEFAULT_FILTER: &str = "warn,fogu=info,fogu_core=info,fogu_term=info,\
                              fogu_session=info,fogu_ui=info";
/// Daily log files kept.
const MAX_LOG_FILES: usize = 7;

fn main() -> fogu_ui::Result {
    let _log_guard = init_logging();
    fogu_ui::run()
}

/// stderr + non-blocking daily file in `$XDG_STATE_HOME/fogu/`; `log` records are bridged
/// by tracing-subscriber's `tracing-log` feature. The guard flushes the file on drop.
fn init_logging() -> Option<WorkerGuard> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let file = log_dir().and_then(|dir| {
        RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix("fogu.log")
            .max_log_files(MAX_LOG_FILES)
            .build(&dir)
            .map_err(|err| format!("{}: {err}", dir.display()))
    });
    let (file_layer, guard, file_err) = match file {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer);
            (Some(layer), Some(guard), None)
        }
        Err(err) => (None, None, Some(err)),
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(file_layer)
        .init();
    if let Some(err) = file_err {
        tracing::warn!("file logging disabled: {err}");
    }
    guard
}

/// `$XDG_STATE_HOME/fogu` (default `~/.local/state/fogu`), created with mode 0700.
fn log_dir() -> Result<PathBuf, String> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or("neither XDG_STATE_HOME nor HOME is set")?;
    let dir = state.join("fogu");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|err| format!("{}: {err}", dir.display()))?;
    Ok(dir)
}
