//! Start with Windows via a shortcut in the user's Startup folder (no registry).

use std::path::Path;

use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{Interface, PCWSTR};

use crate::paths::startup_shortcut;
use crate::win::to_wide;

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
