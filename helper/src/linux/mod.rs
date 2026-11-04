//! The Linux tray application: an X11 overlay window (XWayland on Wayland desktops), a
//! StatusNotifierItem tray icon, evdev keyboard input, and the Options window in a child process.

pub mod desktop;
pub mod instance;
pub mod xkb;

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(result) = crate::cli::run_cli(&args) {
        if let Err(e) = result {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    eprintln!("tray app not yet implemented");
}
