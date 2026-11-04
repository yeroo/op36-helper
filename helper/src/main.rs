//! OP36 layout helper: tray application drawing a click-through overlay of the keymap.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_common;
mod auto_hide;
mod cli;
mod find_key;
mod firmware;
mod firmware_thread;
mod hid_thread;
mod input;
mod keymap;
mod legend;
#[cfg(target_os = "linux")]
mod linux;
mod model;
mod offline_grace;
mod options;
mod overlay;
mod paths;
mod settings;
mod sink;
mod theme;
#[cfg(windows)]
mod tray;
mod update;
mod update_thread;
mod version;
mod vial;
#[cfg(windows)]
mod win;
#[cfg(windows)]
mod win_app;

fn main() {
    #[cfg(windows)]
    win_app::main();
    #[cfg(target_os = "linux")]
    linux::main();
}
