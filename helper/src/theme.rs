//! Light / dark palette selection following the Windows app theme.

use crate::settings::Theme;

/// `AppsUseLightTheme` from `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`
/// (read only; nothing is ever written). `None` when the value is missing.
pub fn windows_apps_use_light() -> Option<bool> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;
    let mut value: u32 = 0;
    let mut size: u32 = std::mem::size_of::<u32>() as u32;
    // SAFETY: documented registry read into a sized local.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(value != 0)
}

/// Whether to draw dark, given the user's choice and Windows' light-theme flag.
pub fn is_dark(theme: Theme, apps_use_light: Option<bool>) -> bool {
    match theme {
        Theme::Light => false,
        Theme::Dark => true,
        Theme::Auto => apps_use_light == Some(false),
    }
}

pub fn dark_mode(theme: Theme) -> bool {
    is_dark(theme, windows_apps_use_light())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_follows_windows_light_flag() {
        assert!(!is_dark(Theme::Auto, Some(true)));
        assert!(is_dark(Theme::Auto, Some(false)));
        assert!(!is_dark(Theme::Auto, None), "unknown registry value means light");
        assert!(is_dark(Theme::Dark, Some(true)));
        assert!(!is_dark(Theme::Light, Some(false)));
    }
}
