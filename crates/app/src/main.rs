//! `fogu`: entry point. Initialises logging (stderr only, for now) and runs the GUI.
//!
//! Takes no CLI flags yet.

fn main() -> fogu_ui::Result {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    fogu_ui::run()
}
