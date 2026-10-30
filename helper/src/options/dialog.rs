//! The Options window: native Win32 controls, DPI scaled, live preview while adjusting.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_BTNFACE, HBRUSH, HFONT};

/// `TBM_GETPOS` = `WM_USER`; the crate omits it.
const TBM_GETPOS: u32 = 0x0400;
use windows::Win32::UI::Controls::{
    BST_CHECKED, HKM_GETHOTKEY, HKM_SETHOTKEY, ICC_BAR_CLASSES, ICC_HOTKEY_CLASS,
    ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, TBM_SETPOS, TBM_SETRANGE,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BN_CLICKED, BS_AUTOCHECKBOX, BS_AUTORADIOBUTTON, BS_GROUPBOX,
    BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE, CBS_DROPDOWNLIST,
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow,
    EN_CHANGE, ES_AUTOHSCROLL, GWLP_USERDATA, GetWindowLongPtrW, GetWindowTextW, HMENU,
    RegisterClassW, SW_SHOW, SendMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowTextW,
    ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_HSCROLL, WM_NCCREATE,
    WM_NCDESTROY, WM_SETFONT, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_DISABLED, WS_GROUP, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::PCWSTR;

use super::{
    SCALE_MAX_PCT, SCALE_MIN_PCT, absent_monitor, combo_to_monitor, control_word_to_hotkey,
    hotkey_to_control_word, monitor_to_combo, scale_to_slider, slider_to_scale,
};
use crate::overlay::placement::MonitorInfo;
use crate::settings::{Anchor, DisplayMode, Settings, Theme};
use crate::win::{hinstance, to_wide};

const CLASS_NAME: &str = "Op36HelperOptions";

// Control ids.
const ID_MONITOR: i32 = 100;
const ID_ANCHOR_BASE: i32 = 110; // 110..118, row-major 3×3
const ID_OFFSET_X: i32 = 120;
const ID_OFFSET_Y: i32 = 121;
const ID_SCALE: i32 = 130;
const ID_OPACITY: i32 = 131;
const ID_MODE: i32 = 140;
const ID_HOTKEY: i32 = 150;
const ID_AUTOSTART: i32 = 160;
const ID_FIND: i32 = 161;
const ID_FIND_SECS: i32 = 162;
const ID_EN_HINT: i32 = 163;
const ID_RELOAD: i32 = 170;
const ID_OK: i32 = 180;
const ID_CANCEL: i32 = 181;
/// What `IsDialogMessageW` sends for Enter / Esc.
const IDOK_STD: i32 = 1;
const IDCANCEL_STD: i32 = 2;
const ID_APPLY: i32 = 182;
const ID_STATUS: i32 = 190;
const ID_THEME: i32 = 194;
const ID_RESERVE: i32 = 195;
const ID_SCALE_LABEL: i32 = 191;
const ID_FW_LEFT: i32 = 200;
const ID_FW_RIGHT: i32 = 201;
const ID_FW_BUNDLED: i32 = 202;
const ID_FW_STATUS: i32 = 203;
const ID_FW_FLASH_LEFT: i32 = 204;
const ID_FW_FLASH_RIGHT: i32 = 205;
const ID_FW_CANCEL: i32 = 206;
const ID_UPDATE_CHECK: i32 = 210;
const ID_UPDATE_LINE: i32 = 211;
const ID_UPDATE_NOW: i32 = 212;
const ID_OPACITY_LABEL: i32 = 192;
const ID_FIND_SECS_LABEL: i32 = 193;

const ANCHORS: [Anchor; 9] = [
    Anchor::TopLeft,
    Anchor::Top,
    Anchor::TopRight,
    Anchor::Left,
    Anchor::Center,
    Anchor::Right,
    Anchor::BottomLeft,
    Anchor::Bottom,
    Anchor::BottomRight,
];

/// What the dialog asks the application to do.
pub enum DialogAction {
    /// Preview or apply these settings (`save` says whether to persist them).
    Settings {
        settings: Settings,
        save: bool,
    },
    ReloadKeymap,
    /// Flash the bundled firmware onto this half (the user double-taps its reset button).
    FlashHalf(crate::vial::protocol::Side),
    CancelFlash,
    /// Download, verify and install the latest release, then restart.
    UpdateNow,
    Closed,
}

/// What the Firmware group shows when the dialog opens.
#[derive(Debug, Clone, Default)]
pub struct FirmwareView {
    /// `left v… (up to date)` and `right …`, already worded.
    pub left: String,
    pub right: String,
    /// `bundled v…` or `no bundled firmware next to the exe`.
    pub bundled: String,
    /// Extra line: `halves differ` or empty.
    pub note: String,
    /// The Flash buttons are enabled (a bundle exists and no flash is running).
    pub can_flash: bool,
    /// `Helper 0.2.0 — up to date` / `0.2.1 available` / `checking…`.
    pub update_line: String,
}

pub type ActionHandler = Box<dyn FnMut(DialogAction) -> Result<(), String>>;

struct State {
    original: Settings,
    monitors: Vec<MonitorInfo>,
    status_text: String,
    firmware: FirmwareView,
    on_action: ActionHandler,
    font: HFONT,
    suppress: bool,
}

pub struct OptionsDialog {
    hwnd: HWND,
}

impl OptionsDialog {
    /// Open (or raise) the dialog. `status_text` is the keyboard status to show.
    pub fn open(
        settings: Settings,
        monitors: Vec<MonitorInfo>,
        status_text: String,
        firmware: FirmwareView,
        on_action: ActionHandler,
    ) -> Option<OptionsDialog> {
        // SAFETY: standard common-controls init and class registration.
        unsafe {
            let icc = INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_BAR_CLASSES | ICC_HOTKEY_CLASS | ICC_STANDARD_CLASSES,
            };
            let _ = InitCommonControlsEx(&icc);
            let class = to_wide(CLASS_NAME);
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance(),
                lpszClassName: PCWSTR(class.as_ptr()),
                hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as isize as *mut c_void),
                ..Default::default()
            };
            RegisterClassW(&wc);
            let state = Box::new(State {
                original: settings,
                monitors,
                status_text,
                firmware,
                on_action,
                font: HFONT::default(),
                suppress: true,
            });
            let title = to_wide("OP36 Helper — Options");
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_CAPTION | WS_SYSMENU,
                200,
                200,
                10,
                10,
                None,
                None::<HMENU>,
                Some(hinstance()),
                Some(Box::into_raw(state) as *const c_void),
            ) {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("options window: {e}");
                    return None;
                }
            };
            let icon = crate::tray::load_icon("icon_on.ico", crate::tray::ICON_ON);
            // WM_SETICON for both the big (Alt-Tab) and small (title bar) icon.
            SendMessageW(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::WM_SETICON,
                Some(WPARAM(1)),
                Some(LPARAM(icon.0 as isize)),
            );
            SendMessageW(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::WM_SETICON,
                Some(WPARAM(0)),
                Some(LPARAM(icon.0 as isize)),
            );
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            Some(OptionsDialog { hwnd })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// The update line (`0.2.1 available`, `downloading…`, a failure).
    pub fn set_update_line(&self, text: &str) {
        set_text(self.hwnd, ID_UPDATE_LINE, text);
    }

    /// Progress of a running flash (and the end of it: buttons come back).
    pub fn set_firmware_status(&self, text: &str, flashing: bool) {
        set_text(self.hwnd, ID_FW_STATUS, text);
        // SAFETY: own child controls.
        unsafe {
            for id in [ID_FW_FLASH_LEFT, ID_FW_FLASH_RIGHT] {
                let _ = EnableWindow(child(self.hwnd, id), !flashing);
            }
            let _ = EnableWindow(child(self.hwnd, ID_FW_CANCEL), flashing);
        }
    }

    /// Show an offset changed outside the dialog (Ctrl+Alt+Arrows) without triggering a preview.
    pub fn set_offset(&self, offset: (i32, i32)) {
        let Some(st) = state(self.hwnd) else { return };
        let was = st.suppress;
        st.suppress = true;
        set_text(self.hwnd, ID_OFFSET_X, &offset.0.to_string());
        set_text(self.hwnd, ID_OFFSET_Y, &offset.1.to_string());
        st.original.offset = offset; // Cancel keeps the nudged position too
        st.suppress = was;
    }
}

fn child(parent: HWND, id: i32) -> HWND {
    // SAFETY: GetDlgItem on our own window.
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(parent), id).unwrap_or_default()
    }
}

fn state(hwnd: HWND) -> Option<&'static mut State> {
    // SAFETY: GWLP_USERDATA holds the Box<State> pointer set in WM_NCCREATE until WM_NCDESTROY.
    unsafe {
        let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if p.is_null() { None } else { Some(&mut *p) }
    }
}

struct Layout {
    scale: f32,
}

impl Layout {
    fn px(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }
}

/// Create one child control.
fn make(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    id: i32,
    l: &Layout,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    font: HFONT,
) -> HWND {
    let class_w = to_wide(class);
    let text_w = to_wide(text);
    // SAFETY: standard child creation on our own window.
    unsafe {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_w.as_ptr()),
            PCWSTR(text_w.as_ptr()),
            WINDOW_STYLE(style) | WS_CHILD | WS_VISIBLE,
            l.px(x),
            l.px(y),
            l.px(w),
            l.px(h),
            Some(parent),
            Some(HMENU(id as isize as *mut c_void)),
            Some(hinstance()),
            None,
        )
        .unwrap_or_default();
        SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        hwnd
    }
}

fn build(hwnd: HWND, st: &mut State) {
    use windows::Win32::Graphics::Gdi::CreateFontIndirectW;
    use windows::Win32::UI::WindowsAndMessaging::{
        NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
        SystemParametersInfoW,
    };
    // SAFETY: DPI query and font creation on our own window.
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let l = Layout {
        scale: dpi as f32 / 96.0,
    };
    // SAFETY: documented call with a sized struct; the font lives as long as the dialog.
    st.font = unsafe {
        let mut ncm = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        let _ = SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            ncm.cbSize,
            Some(&mut ncm as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        let mut lf = ncm.lfMessageFont;
        if lf.lfFaceName[0] == 0 {
            let face: Vec<u16> = "Segoe UI".encode_utf16().collect();
            lf.lfFaceName[..face.len()].copy_from_slice(&face);
        }
        lf.lfHeight = -l.px(12);
        CreateFontIndirectW(&lf)
    };
    let f = st.font;
    let s = &st.original;
    let static_ = |text: &str, x, y, w, h, id| make(hwnd, "STATIC", text, 0, id, &l, x, y, w, h, f);

    // ---- Display group ----
    make(
        hwnd,
        "BUTTON",
        "Display",
        BS_GROUPBOX as u32,
        0,
        &l,
        10,
        8,
        330,
        356,
        f,
    );
    static_("Monitor", 20, 32, 80, 20, 0);
    let combo = make(
        hwnd,
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0 | WS_TABSTOP.0,
        ID_MONITOR,
        &l,
        110,
        28,
        220,
        200,
        f,
    );
    add_combo_item(combo, "Primary monitor");
    for m in &st.monitors {
        let (l_, t, r, b) = m.work_area;
        add_combo_item(
            combo,
            &format!(
                "{} — {}×{}",
                m.name.trim_start_matches(r"\\.\"),
                r - l_,
                b - t
            ),
        );
    }
    if let Some(name) = absent_monitor(&s.monitor, &st.monitors) {
        add_combo_item(
            combo,
            &format!("{} (not connected)", name.trim_start_matches(r"\\.\")),
        );
    }
    set_combo(combo, monitor_to_combo(&s.monitor, &st.monitors));

    static_("Position", 20, 62, 80, 20, 0);
    for (i, _) in ANCHORS.iter().enumerate() {
        let (col, row) = (i % 3, i / 3);
        let style = BS_AUTORADIOBUTTON as u32 | WS_TABSTOP.0 | if i == 0 { WS_GROUP.0 } else { 0 };
        let b = make(
            hwnd,
            "BUTTON",
            "",
            style,
            ID_ANCHOR_BASE + i as i32,
            &l,
            110 + col as i32 * 26,
            58 + row as i32 * 22,
            20,
            20,
            f,
        );
        if ANCHORS[i] == s.anchor {
            // SAFETY: own control.
            unsafe { SendMessageW(b, BM_SETCHECK, Some(WPARAM(BST_CHECKED.0 as usize)), None) };
        }
    }
    static_("Offset X / Y", 200, 62, 90, 20, 0);
    make(
        hwnd,
        "EDIT",
        &s.offset.0.to_string(),
        ES_AUTOHSCROLL as u32 | WS_TABSTOP.0 | 0x0080_0000, /*WS_BORDER*/
        ID_OFFSET_X,
        &l,
        200,
        84,
        60,
        22,
        f,
    );
    make(
        hwnd,
        "EDIT",
        &s.offset.1.to_string(),
        ES_AUTOHSCROLL as u32 | WS_TABSTOP.0 | 0x0080_0000,
        ID_OFFSET_Y,
        &l,
        268,
        84,
        60,
        22,
        f,
    );
    static_("Ctrl+Alt+Arrows nudge 10 px", 200, 112, 140, 20, 0);

    static_("Size", 20, 150, 80, 20, 0);
    let scale = make(
        hwnd,
        "msctls_trackbar32",
        "",
        WS_TABSTOP.0 | 0x0001, /*TBS_AUTOTICKS*/
        ID_SCALE,
        &l,
        110,
        146,
        180,
        28,
        f,
    );
    set_range(
        scale,
        SCALE_MIN_PCT,
        SCALE_MAX_PCT,
        scale_to_slider(s.scale),
    );
    static_(
        &format!("{} %", scale_to_slider(s.scale)),
        295,
        150,
        40,
        20,
        ID_SCALE_LABEL,
    );

    static_("Opacity", 20, 182, 80, 20, 0);
    let op = make(
        hwnd,
        "msctls_trackbar32",
        "",
        WS_TABSTOP.0 | 0x0001,
        ID_OPACITY,
        &l,
        110,
        178,
        180,
        28,
        f,
    );
    set_range(op, 10, 100, (s.opacity * 100.0).round() as i32);
    static_(
        &format!("{} %", (s.opacity * 100.0).round() as i32),
        295,
        182,
        40,
        20,
        ID_OPACITY_LABEL,
    );

    static_("Show", 20, 216, 80, 20, 0);
    let mode = make(
        hwnd,
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0 | WS_TABSTOP.0,
        ID_MODE,
        &l,
        110,
        212,
        220,
        120,
        f,
    );
    add_combo_item(mode, "Always (while enabled)");
    add_combo_item(mode, "Only while a layer or modifier is held");
    set_combo(
        mode,
        if s.display_mode == DisplayMode::Always {
            0
        } else {
            1
        },
    );

    static_("Toggle hotkey", 20, 248, 85, 20, 0);
    let hk = make(
        hwnd,
        "msctls_hotkey32",
        "",
        WS_TABSTOP.0 | 0x0080_0000,
        ID_HOTKEY,
        &l,
        110,
        244,
        220,
        24,
        f,
    );
    // SAFETY: own control.
    unsafe {
        SendMessageW(
            hk,
            HKM_SETHOTKEY,
            Some(WPARAM(hotkey_to_control_word(&s.hotkey) as usize)),
            None,
        )
    };
    let en = make(
        hwnd,
        "BUTTON",
        "Show Latin hint on non-Latin layouts",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0,
        ID_EN_HINT,
        &l,
        20,
        276,
        310,
        22,
        f,
    );
    set_check(en, s.show_en_hint);
    static_("Theme", 20, 306, 80, 20, 0);
    let theme = make(
        hwnd,
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0 | WS_TABSTOP.0,
        ID_THEME,
        &l,
        110,
        302,
        220,
        120,
        f,
    );
    add_combo_item(theme, "Follow Windows");
    add_combo_item(theme, "Light");
    add_combo_item(theme, "Dark");
    set_combo(
        theme,
        match s.theme {
            Theme::Auto => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
        },
    );
    let reserve = make(
        hwnd,
        "BUTTON",
        "Reserve screen space for maximized windows",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0,
        ID_RESERVE,
        &l,
        20,
        332,
        320,
        22,
        f,
    );
    set_check(reserve, s.reserve_space);

    // ---- Behaviour group ----
    make(
        hwnd,
        "BUTTON",
        "Behaviour",
        BS_GROUPBOX as u32,
        0,
        &l,
        350,
        8,
        300,
        140,
        f,
    );
    let auto = make(
        hwnd,
        "BUTTON",
        "Start with Windows",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0,
        ID_AUTOSTART,
        &l,
        360,
        30,
        280,
        22,
        f,
    );
    set_check(auto, s.autostart);
    let find = make(
        hwnd,
        "BUTTON",
        "Find a key when I press one",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0,
        ID_FIND,
        &l,
        360,
        56,
        280,
        22,
        f,
    );
    set_check(find, s.find_key);
    static_("Show for", 380, 86, 60, 20, 0);
    let secs = make(
        hwnd,
        "msctls_trackbar32",
        "",
        WS_TABSTOP.0 | 0x0001,
        ID_FIND_SECS,
        &l,
        440,
        82,
        150,
        28,
        f,
    );
    set_range(secs, 5, 50, (s.find_key_secs * 10.0).round() as i32);
    static_(
        &format!("{:.1} s", s.find_key_secs),
        595,
        86,
        50,
        20,
        ID_FIND_SECS_LABEL,
    );
    let upd = make(
        hwnd,
        "BUTTON",
        "Check for updates automatically",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0,
        ID_UPDATE_CHECK,
        &l,
        360,
        114,
        280,
        22,
        f,
    );
    set_check(upd, s.update_check);

    // ---- Keyboard group ----
    make(
        hwnd,
        "BUTTON",
        "Keyboard",
        BS_GROUPBOX as u32,
        0,
        &l,
        350,
        156,
        300,
        152,
        f,
    );
    static_(&st.status_text, 360, 178, 280, 90, ID_STATUS);
    make(
        hwnd,
        "BUTTON",
        "Reload keymap",
        BS_PUSHBUTTON as u32 | WS_TABSTOP.0,
        ID_RELOAD,
        &l,
        360,
        272,
        130,
        26,
        f,
    );

    // ---- Firmware group (full width under the others) ----
    make(
        hwnd,
        "BUTTON",
        "Firmware and updates",
        BS_GROUPBOX as u32,
        0,
        &l,
        10,
        372,
        640,
        144,
        f,
    );
    static_(&st.firmware.left, 20, 394, 300, 20, ID_FW_LEFT);
    static_(&st.firmware.right, 20, 416, 300, 20, ID_FW_RIGHT);
    static_(&st.firmware.bundled, 20, 438, 300, 20, ID_FW_BUNDLED);
    let flash_style = BS_PUSHBUTTON as u32
        | WS_TABSTOP.0
        | if st.firmware.can_flash {
            0
        } else {
            WS_DISABLED.0
        };
    make(
        hwnd,
        "BUTTON",
        "Flash left half…",
        flash_style,
        ID_FW_FLASH_LEFT,
        &l,
        340,
        392,
        140,
        26,
        f,
    );
    make(
        hwnd,
        "BUTTON",
        "Flash right half…",
        flash_style,
        ID_FW_FLASH_RIGHT,
        &l,
        490,
        392,
        150,
        26,
        f,
    );
    make(
        hwnd,
        "BUTTON",
        "Cancel flash",
        BS_PUSHBUTTON as u32 | WS_TABSTOP.0 | WS_DISABLED.0,
        ID_FW_CANCEL,
        &l,
        340,
        424,
        140,
        26,
        f,
    );
    static_(&st.firmware.note, 20, 462, 620, 22, ID_FW_STATUS);
    static_(&st.firmware.update_line, 20, 488, 460, 20, ID_UPDATE_LINE);
    make(
        hwnd,
        "BUTTON",
        "Update now",
        BS_PUSHBUTTON as u32 | WS_TABSTOP.0,
        ID_UPDATE_NOW,
        &l,
        490,
        484,
        150,
        26,
        f,
    );

    // ---- Buttons ----
    make(
        hwnd,
        "BUTTON",
        "OK",
        BS_PUSHBUTTON as u32 | WS_TABSTOP.0,
        ID_OK,
        &l,
        400,
        526,
        80,
        28,
        f,
    );
    make(
        hwnd,
        "BUTTON",
        "Cancel",
        BS_PUSHBUTTON as u32 | WS_TABSTOP.0,
        ID_CANCEL,
        &l,
        486,
        526,
        80,
        28,
        f,
    );
    make(
        hwnd,
        "BUTTON",
        "Apply",
        BS_PUSHBUTTON as u32 | WS_TABSTOP.0,
        ID_APPLY,
        &l,
        572,
        526,
        78,
        28,
        f,
    );

    // SAFETY: own window.
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
            hwnd,
            None,
            0,
            0,
            l.px(660) + l.px(16),
            l.px(564) + l.px(40),
            windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
        );
    }
    st.suppress = false;
}

fn add_combo_item(combo: HWND, text: &str) {
    let w = to_wide(text);
    // SAFETY: own control; string outlives the call.
    unsafe { SendMessageW(combo, CB_ADDSTRING, None, Some(LPARAM(w.as_ptr() as isize))) };
}

fn set_combo(combo: HWND, index: usize) {
    // SAFETY: own control.
    unsafe { SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(index)), None) };
}

fn get_combo(combo: HWND) -> usize {
    // SAFETY: own control.
    unsafe { SendMessageW(combo, CB_GETCURSEL, None, None).0.max(0) as usize }
}

fn set_range(bar: HWND, min: i32, max: i32, pos: i32) {
    // SAFETY: own control.
    unsafe {
        SendMessageW(
            bar,
            TBM_SETRANGE,
            Some(WPARAM(1)),
            Some(LPARAM(((max as isize) << 16) | (min as isize & 0xFFFF))),
        );
        SendMessageW(bar, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(pos as isize)));
    }
}

fn get_pos(bar: HWND) -> i32 {
    // SAFETY: own control.
    unsafe { SendMessageW(bar, TBM_GETPOS, None, None).0 as i32 }
}

fn set_check(button: HWND, on: bool) {
    // SAFETY: own control.
    unsafe {
        SendMessageW(
            button,
            BM_SETCHECK,
            Some(WPARAM(if on { BST_CHECKED.0 as usize } else { 0 })),
            None,
        )
    };
}

fn get_check(button: HWND) -> bool {
    // SAFETY: own control.
    unsafe { SendMessageW(button, BM_GETCHECK, None, None).0 as u32 == BST_CHECKED.0 }
}

/// The edit's integer, or `None` while it holds something unparsable (`-`, empty) mid-typing.
fn get_int(edit: HWND) -> Option<i32> {
    let mut buf = [0u16; 16];
    // SAFETY: own control, sized buffer.
    let n = unsafe { GetWindowTextW(edit, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n]).trim().parse().ok()
}

fn set_text(hwnd: HWND, id: i32, text: &str) {
    let w = to_wide(text);
    // SAFETY: own control.
    unsafe {
        let _ = SetWindowTextW(child(hwnd, id), PCWSTR(w.as_ptr()));
    }
}

/// Read every control into a `Settings` (visible flag and move-hotkey modifiers are kept as they were).
fn settings_from_controls(hwnd: HWND, st: &State) -> Settings {
    let mut s = st.original.clone();
    s.monitor = combo_to_monitor(
        get_combo(child(hwnd, ID_MONITOR)),
        &st.monitors,
        &st.original.monitor,
    );
    for (i, anchor) in ANCHORS.iter().enumerate() {
        if get_check(child(hwnd, ID_ANCHOR_BASE + i as i32)) {
            s.anchor = *anchor;
        }
    }
    s.offset = (
        get_int(child(hwnd, ID_OFFSET_X)).unwrap_or(s.offset.0),
        get_int(child(hwnd, ID_OFFSET_Y)).unwrap_or(s.offset.1),
    );
    s.scale = slider_to_scale(get_pos(child(hwnd, ID_SCALE)));
    s.opacity = get_pos(child(hwnd, ID_OPACITY)) as f32 / 100.0;
    s.display_mode = if get_combo(child(hwnd, ID_MODE)) == 0 {
        DisplayMode::Always
    } else {
        DisplayMode::WhileHeld
    };
    // SAFETY: own control.
    let word = unsafe { SendMessageW(child(hwnd, ID_HOTKEY), HKM_GETHOTKEY, None, None).0 as u32 };
    if word & 0xFF != 0 {
        s.hotkey = control_word_to_hotkey(word);
    }
    s.show_en_hint = get_check(child(hwnd, ID_EN_HINT));
    s.theme = match get_combo(child(hwnd, ID_THEME)) {
        1 => Theme::Light,
        2 => Theme::Dark,
        _ => Theme::Auto,
    };
    s.reserve_space = get_check(child(hwnd, ID_RESERVE));
    s.update_check = get_check(child(hwnd, ID_UPDATE_CHECK));
    s.autostart = get_check(child(hwnd, ID_AUTOSTART));
    s.find_key = get_check(child(hwnd, ID_FIND));
    s.find_key_secs = get_pos(child(hwnd, ID_FIND_SECS)) as f32 / 10.0;
    s
}

fn refresh_labels(hwnd: HWND) {
    set_text(
        hwnd,
        ID_SCALE_LABEL,
        &format!("{} %", get_pos(child(hwnd, ID_SCALE))),
    );
    set_text(
        hwnd,
        ID_OPACITY_LABEL,
        &format!("{} %", get_pos(child(hwnd, ID_OPACITY))),
    );
    set_text(
        hwnd,
        ID_FIND_SECS_LABEL,
        &format!("{:.1} s", get_pos(child(hwnd, ID_FIND_SECS)) as f32 / 10.0),
    );
}

fn emit(hwnd: HWND, st: &mut State, action: DialogAction) -> bool {
    match (st.on_action)(action) {
        Ok(()) => true,
        Err(e) => {
            set_text(hwnd, ID_STATUS, &format!("{}\n\n{e}", st.status_text));
            false
        }
    }
}

fn preview(hwnd: HWND) {
    let Some(st) = state(hwnd) else { return };
    if st.suppress {
        return;
    }
    refresh_labels(hwnd);
    let settings = settings_from_controls(hwnd, st);
    emit(
        hwnd,
        st,
        DialogAction::Settings {
            settings,
            save: false,
        },
    );
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: GWLP_USERDATA protocol as in `MsgWindow`; controls are our own.
    unsafe {
        match msg {
            WM_NCCREATE => {
                let cs = lparam.0 as *const CREATESTRUCTW;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
                let st = &mut *((*cs).lpCreateParams as *mut State);
                build(hwnd, st);
                LRESULT(1)
            }
            WM_HSCROLL => {
                preview(hwnd);
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                match id {
                    ID_OK | IDOK_STD | ID_APPLY => {
                        let mut ok = true;
                        if let Some(st) = state(hwnd) {
                            let settings = settings_from_controls(hwnd, st);
                            ok = emit(
                                hwnd,
                                st,
                                DialogAction::Settings {
                                    settings: settings.clone(),
                                    save: true,
                                },
                            );
                            if ok {
                                st.original = settings;
                            }
                        }
                        // On error the window stays open so the message in the status box is seen.
                        if id != ID_APPLY && ok {
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                    ID_CANCEL | IDCANCEL_STD => {
                        if let Some(st) = state(hwnd) {
                            let original = st.original.clone();
                            let _ = emit(
                                hwnd,
                                st,
                                DialogAction::Settings {
                                    settings: original,
                                    save: false,
                                },
                            );
                        }
                        let _ = DestroyWindow(hwnd);
                    }
                    ID_RELOAD => {
                        if let Some(st) = state(hwnd) {
                            let _ = emit(hwnd, st, DialogAction::ReloadKeymap);
                        }
                    }
                    ID_FW_FLASH_LEFT | ID_FW_FLASH_RIGHT => {
                        if let Some(st) = state(hwnd) {
                            let side = if id == ID_FW_FLASH_LEFT {
                                crate::vial::protocol::Side::Left
                            } else {
                                crate::vial::protocol::Side::Right
                            };
                            let _ = emit(hwnd, st, DialogAction::FlashHalf(side));
                        }
                    }
                    ID_FW_CANCEL => {
                        if let Some(st) = state(hwnd) {
                            let _ = emit(hwnd, st, DialogAction::CancelFlash);
                        }
                    }
                    ID_UPDATE_NOW => {
                        if let Some(st) = state(hwnd) {
                            let _ = emit(hwnd, st, DialogAction::UpdateNow);
                        }
                    }
                    _ if code == BN_CLICKED
                        || code == CBN_SELCHANGE
                        || code == EN_CHANGE as u32 =>
                    {
                        preview(hwnd)
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                if let Some(st) = state(hwnd) {
                    let original = st.original.clone();
                    let _ = emit(
                        hwnd,
                        st,
                        DialogAction::Settings {
                            settings: original,
                            save: false,
                        },
                    );
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
                if !p.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let mut st = Box::from_raw(p);
                    let _ = windows::Win32::Graphics::Gdi::DeleteObject(st.font.into());
                    let _ = (st.on_action)(DialogAction::Closed);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
