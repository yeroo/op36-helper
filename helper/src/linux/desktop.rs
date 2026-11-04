//! What the desktop says, watched through `gsettings` (GNOME and every desktop that keeps the
//! GNOME schemas, which includes Ubuntu's): the keyboard input source in use and the light/dark
//! preference. Also the XDG autostart entry.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};

use super::xkb::{XkbLayout, hkl_for, set_sources};
use crate::model::Hkl;

/// 0 = unknown, 1 = light, 2 = dark.
static SCHEME: AtomicU8 = AtomicU8::new(0);

/// The desktop's "apps use light" flag in the shape `theme::is_dark` takes.
pub fn apps_use_light() -> Option<bool> {
    match SCHEME.load(Ordering::Relaxed) {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

fn gsettings_get(schema: &str, key: &str) -> Option<String> {
    let out = Command::new("gsettings")
        .args(["get", schema, key])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `'prefer-dark'` → dark, `'prefer-light'` / `'default'` → light.
pub fn parse_color_scheme(value: &str) -> u8 {
    let v = value.trim().trim_matches('\'');
    match v {
        "prefer-dark" => 2,
        "prefer-light" | "default" => 1,
        _ => 0,
    }
}

/// GVariant text of an `a(ss)` input-source list: `[('xkb', 'us'), ('xkb', 'ru')]`.
pub fn parse_sources(value: &str) -> Vec<XkbLayout> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')') else {
            break;
        };
        let tuple = &rest[open + 1..open + close];
        let parts: Vec<&str> = tuple
            .split(',')
            .map(|p| p.trim().trim_matches('\'').trim_matches('"'))
            .collect();
        if parts.len() == 2 && parts[0] == "xkb" {
            out.push(XkbLayout::parse_source_id(parts[1]));
        }
        rest = &rest[open + close + 1..];
    }
    out
}

/// Run `gsettings monitor` on `schema key…` and call `on_line` for every change; restarts if the
/// monitor exits. Nothing happens on systems without gsettings.
fn monitor(schema: &'static str, keys: &'static [&'static str], on_line: impl Fn(&str, &str) + Send + 'static) {
    std::thread::Builder::new()
        .name(format!("gsettings-{schema}"))
        .spawn(move || {
            for _ in 0..5 {
                let mut cmd = Command::new("gsettings");
                cmd.arg("monitor").arg(schema);
                if keys.len() == 1 {
                    cmd.arg(keys[0]);
                }
                let Ok(mut child) = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
                    log::info!("gsettings not available: no live {schema} updates");
                    return;
                };
                if let Some(out) = child.stdout.take() {
                    for line in BufReader::new(out).lines().map_while(Result::ok) {
                        // `key: value`
                        if let Some((k, v)) = line.split_once(':')
                            && keys.contains(&k.trim())
                        {
                            on_line(k.trim(), v.trim());
                        }
                    }
                }
                let _ = child.wait();
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        })
        .expect("spawn gsettings monitor");
}

/// Read the colour scheme now and keep it current; `changed` runs after each change.
pub fn watch_theme(changed: impl Fn() + Send + 'static) {
    if let Some(v) = gsettings_get("org.gnome.desktop.interface", "color-scheme") {
        SCHEME.store(parse_color_scheme(&v), Ordering::Relaxed);
    }
    monitor(
        "org.gnome.desktop.interface",
        &["color-scheme"],
        move |_, v| {
            SCHEME.store(parse_color_scheme(v), Ordering::Relaxed);
            changed();
        },
    );
}

/// The current input source: GNOME moves the active source to the front of `mru-sources`
/// (the `current` key is no longer maintained); `sources` is the configured list.
fn current_from(mru: &[XkbLayout], sources: &[XkbLayout]) -> Option<XkbLayout> {
    mru.first().or_else(|| sources.first()).cloned()
}

/// Report the current layout now and on every switch.
pub fn watch_language(on_change: impl Fn(Hkl) + Send + Sync + 'static) {
    let on_change = std::sync::Arc::new(on_change);
    let read_all = || {
        let sources = gsettings_get("org.gnome.desktop.input-sources", "sources")
            .map(|v| parse_sources(&v))
            .unwrap_or_default();
        let mru = gsettings_get("org.gnome.desktop.input-sources", "mru-sources")
            .map(|v| parse_sources(&v))
            .unwrap_or_default();
        (sources, mru)
    };
    let (sources, mru) = read_all();
    if sources.is_empty() && mru.is_empty() {
        log::info!("no GNOME input sources: assuming the US layout");
        set_sources(vec![XkbLayout::new("us", "")]);
        on_change(hkl_for(&XkbLayout::new("us", "")));
        return;
    }
    set_sources(if sources.is_empty() { mru.clone() } else { sources.clone() });
    if let Some(cur) = current_from(&mru, &sources) {
        on_change(hkl_for(&cur));
    }
    let cb = on_change.clone();
    monitor(
        "org.gnome.desktop.input-sources",
        &["sources", "mru-sources"],
        move |key, value| {
            let list = parse_sources(value);
            if key == "sources" {
                if !list.is_empty() {
                    set_sources(list);
                }
                return;
            }
            if let Some(cur) = list.first() {
                cb(hkl_for(cur));
            }
        },
    );
}

/// The XDG autostart entry for this binary.
pub fn autostart_entry_text(exe: &std::path::Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=OP36 Helper\nComment=Ergohaven OmegaPoint36 layout overlay\nExec=\"{}\"\nIcon=op36-helper\nTerminal=false\nX-GNOME-Autostart-enabled=true\nX-GNOME-Autostart-Delay=3\n",
        exe.display()
    )
}

pub fn autostart_enabled() -> bool {
    crate::paths::autostart_entry().is_file()
}

pub fn set_autostart(enabled: bool, exe: &std::path::Path) -> std::io::Result<()> {
    let path = crate::paths::autostart_entry();
    if enabled {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, autostart_entry_text(exe))
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_parse_from_gvariant_text() {
        assert_eq!(
            parse_sources("[('xkb', 'us'), ('xkb', 'ru'), ('ibus', 'mozc-jp'), ('xkb', 'de+nodeadkeys')]"),
            vec![
                XkbLayout::new("us", ""),
                XkbLayout::new("ru", ""),
                XkbLayout::new("de", "nodeadkeys")
            ]
        );
        assert!(parse_sources("@a(ss) []").is_empty());
    }

    #[test]
    fn color_scheme_values() {
        assert_eq!(parse_color_scheme("'prefer-dark'"), 2);
        assert_eq!(parse_color_scheme("'default'"), 1);
        assert_eq!(parse_color_scheme("'odd'"), 0);
    }
}
