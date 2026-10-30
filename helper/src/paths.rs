//! File locations: everything lives under `%APPDATA%\op36-helper`, no registry.

use std::path::PathBuf;

const APP_DIR_NAME: &str = "op36-helper";

/// `%APPDATA%\op36-helper`, created on first use.
pub fn app_dir() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_DIR_NAME);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn settings_file() -> PathBuf {
    app_dir().join("settings.json")
}

pub fn keymap_cache_file() -> PathBuf {
    app_dir().join("keymap.bin")
}

pub fn log_file() -> PathBuf {
    app_dir().join("log.txt")
}

/// The per-user Startup folder shortcut that implements "Start with Windows".
pub fn startup_shortcut() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(r"Microsoft\Windows\Start Menu\Programs\Startup\OP36 Helper.lnk")
}
