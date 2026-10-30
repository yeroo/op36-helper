//! Start with Windows via a shortcut in the user's Startup folder (no registry).

use std::path::Path;

use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{Interface, PCWSTR};

use crate::paths::startup_shortcut;
use crate::win::to_wide;

/// Whether the helper starts with Windows: the Startup shortcut exists. The installer's task and
/// the Options checkbox manage the same file, so the folder is the truth, not the saved setting.
pub fn is_enabled() -> bool {
    is_enabled_at(&startup_shortcut())
}

pub fn is_enabled_at(link: &Path) -> bool {
    link.is_file()
}

/// Create or remove `OP36 Helper.lnk` pointing at `exe`.
pub fn set(enabled: bool, exe: &Path) -> Result<(), String> {
    let link_path = startup_shortcut();
    if !enabled {
        return match std::fs::remove_file(&link_path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        };
    }
    if let Some(dir) = link_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let exe_w = to_wide(&exe.to_string_lossy());
    let dir_w = to_wide(
        &exe.parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let desc_w = to_wide("OP36 layout helper");
    let link_w = to_wide(&link_path.to_string_lossy());
    // SAFETY: COM calls on an apartment-threaded main thread (CoInitializeEx runs in main);
    // all strings outlive the calls.
    unsafe {
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        link.SetPath(PCWSTR(exe_w.as_ptr()))
            .map_err(|e| e.to_string())?;
        link.SetWorkingDirectory(PCWSTR(dir_w.as_ptr()))
            .map_err(|e| e.to_string())?;
        link.SetDescription(PCWSTR(desc_w.as_ptr()))
            .map_err(|e| e.to_string())?;
        let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        file.Save(PCWSTR(link_w.as_ptr()), true)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_means_the_startup_shortcut_exists() {
        // The installer's "Start with Windows" task creates the same shortcut, so Options must read
        // the folder rather than trust the saved setting (default false on a fresh install).
        let dir = std::env::temp_dir().join(format!("op36-autostart-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("OP36 Helper.lnk");
        assert!(!is_enabled_at(&link));
        std::fs::write(&link, b"lnk").unwrap();
        assert!(is_enabled_at(&link));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
