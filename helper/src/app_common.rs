//! Small pieces both tray applications share: logging, the model clock, and the platform hooks
//! the CLI needs (single instance, char source, offline renderer).

use crate::paths;

const LOG_ROTATE_BYTES: u64 = 1_000_000;

/// Milliseconds since the first call (the model's clock for the switching timeout).
pub fn uptime_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

pub fn init_logging() {
    use simplelog::{CombinedLogger, Config, LevelFilter, WriteLogger};
    // Keep one rotated backup so the log never grows past ~1 MB.
    let path = paths::log_file();
    if std::fs::metadata(&path)
        .map(|m| m.len() > LOG_ROTATE_BYTES)
        .unwrap_or(false)
    {
        let _ = std::fs::rename(&path, path.with_extension("1.txt"));
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::log_file());
    let mut loggers: Vec<Box<dyn simplelog::SharedLogger>> = Vec::new();
    if let Ok(file) = file {
        loggers.push(WriteLogger::new(LevelFilter::Info, Config::default(), file));
    }
    #[cfg(debug_assertions)]
    loggers.push(simplelog::TermLogger::new(
        LevelFilter::Debug,
        Config::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    ));
    let _ = CombinedLogger::init(loggers);
}

/// `true` when this process is the first instance (held for the life of the process).
pub fn claim_single_instance() -> bool {
    #[cfg(windows)]
    {
        crate::win_app::claim_single_instance()
    }
    #[cfg(target_os = "linux")]
    {
        crate::linux::instance::claim()
    }
}

/// The platform's characters-per-layout source.
#[cfg(windows)]
pub fn char_source() -> crate::legend::chars::WinCharSource {
    crate::legend::chars::WinCharSource::new()
}

#[cfg(target_os = "linux")]
pub fn char_source() -> crate::linux::xkb::XkbCharSource {
    crate::linux::xkb::XkbCharSource::new()
}

/// Render one frame offline (the `--snapshot` command).
pub fn render_snapshot(
    frame: &crate::overlay::frame::Frame,
    scale: f32,
) -> Result<crate::overlay::BgraBitmap, String> {
    #[cfg(windows)]
    {
        let mut renderer =
            crate::overlay::render_d2d::Renderer::new().map_err(|e| e.to_string())?;
        renderer.switch_phase = true;
        renderer.render(frame, scale, 1.0).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "linux")]
    {
        let mut renderer = crate::overlay::render_skia::Renderer::new();
        renderer.switch_phase = true;
        Ok(renderer.render(frame, scale, 1.0))
    }
}
