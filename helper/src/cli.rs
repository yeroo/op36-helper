//! Developer commands that run without the tray (both platforms): keymap dump and write, layer
//! probe, offline snapshot.

use crate::legend;
use crate::model::{self, Model, ModelEvent};
use crate::settings::Settings;
use crate::{keymap, paths, version, vial};

/// Push the ZMK tap-hold rules into the keyboard's global behaviour settings (persisted in flash):
/// permissive hold on, unilateral tap on, 280 ms hold timeout, 150 ms prior-idle (flow tap).
fn write_taphold_settings(t: &mut vial::transport::HidTransport) -> Result<(), String> {
    use vial::protocol::*;
    use vial::transport::VialTransport;
    let settings = [
        (SETTING_PERMISSIVE_HOLD, 1u16, "permissive hold"),
        (SETTING_UNILATERAL_TAP, 1, "unilateral tap"),
        (SETTING_MORSE_TIMEOUT, 280, "hold timeout ms"),
        (
            SETTING_PRIOR_IDLE_TIME,
            keymap::zmk_layout::ZMK_PRIOR_IDLE_MS,
            "prior idle ms",
        ),
    ];
    for (key, value, name) in settings {
        let req = req_set_behavior_setting(key, value);
        t.write(&req)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        loop {
            match t.read(50)? {
                Some(pkt) if pkt[0] == 0xE9 => continue,
                // The firmware echoes the request for the tap-hold keys and answers 0 for device settings.
                Some(pkt) if pkt[0] == 0 || pkt[..4] == req[..4] => break,
                Some(pkt) => return Err(format!("{name}: keyboard replied {:02x?}", &pkt[..4])),
                None if std::time::Instant::now() > deadline => {
                    return Err(format!("{name}: no reply"));
                }
                None => continue,
            }
        }
        let reply = vial::reader::request_vial_setting(t, key)?;
        let now = if value <= 1 {
            reply[1] as u16
        } else {
            u16::from_le_bytes([reply[1], reply[2]])
        };
        println!("{name}: wrote {value}, keyboard now reports {now}");
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    Ok(())
}

/// Developer commands that run without the tray: `--dump-keymap <file>`, `--probe-layers [secs]`.
pub fn run_cli(args: &[String]) -> Option<Result<(), String>> {
    use vial::protocol::{parse_layer_state, req_layer_state, req_layer_subscribe};
    use vial::reader::{read_keymap, request};
    use vial::transport::{HidTransport, VialTransport};

    let open = || -> Result<HidTransport, String> {
        // Two readers on one Vial channel steal each other's replies (the helper polls battery).
        if !crate::app_common::claim_single_instance() {
            return Err("op36-helper is running: exit it from the tray first, the CLI needs the keyboard to itself".into());
        }
        let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
        HidTransport::open(&api)
            .ok_or_else(|| "OP36 Vial interface not found (keyboard on USB?)".to_string())
    };
    match args.first().map(String::as_str) {
        Some("--dump-keymap") => Some((|| {
            let path = args.get(1).ok_or("usage: --dump-keymap <file>")?;
            let mut t = open()?;
            let km = read_keymap(&mut t, 6, 6)?;
            std::fs::write(path, km.to_bytes()).map_err(|e| e.to_string())?;
            for layer in 0..km.layers {
                let non_empty = keymap::OP36_KEYS
                    .iter()
                    .any(|k| km.action(layer, k) != keymap::KeyAction::No);
                if !non_empty {
                    continue;
                }
                println!("layer {layer} ({})", km.layer_name(layer));
                for k in keymap::OP36_KEYS.iter() {
                    println!(
                        "  {:>2} (r{} c{}): {:?}",
                        k.index,
                        k.row,
                        k.col,
                        km.action(layer, k)
                    );
                }
            }
            println!(
                "wrote {} ({} layers) to {path}",
                km.actions.len(),
                km.layers
            );
            Ok(())
        })()),
        Some("--probe-layers") => Some((|| {
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
            let mut t = open()?;
            let reply = request(&mut t, &req_layer_subscribe(60))?;
            match parse_layer_state(&reply) {
                Some(s) => println!("reply: {s:?}"),
                None => println!(
                    "layer notify: not supported by this firmware (echo {:02x?})",
                    &reply[..4]
                ),
            }
            let _ = request(&mut t, &req_layer_state())?;
            let start = std::time::Instant::now();
            let mut last_renew = start;
            while start.elapsed().as_secs() < secs {
                if last_renew.elapsed().as_secs() >= 20 {
                    t.write(&req_layer_subscribe(60))?;
                    last_renew = std::time::Instant::now();
                }
                if let Some(pkt) = t.read(200)? {
                    if let Some(s) = parse_layer_state(&pkt) {
                        println!("notify: {s:?}");
                    }
                }
            }
            Ok(())
        })()),
        Some("--write-taphold") => Some((|| {
            let mut t = open()?;
            write_taphold_settings(&mut t)
        })()),
        Some("--write-keymap-zmk") => Some((|| {
            // Write Boris's ZMK layout (layers 0–3) and its two bracket combos into the keyboard.
            use keymap::zmk_layout::{zmk_combos, zmk_layout};
            use vial::protocol::{action_to_via_keycode, req_combo_set, req_native_action_set};
            let mut t = open()?;
            let mut written = 0;
            for (layer, index, action) in zmk_layout() {
                let key = &keymap::OP36_KEYS[index as usize];
                let req = req_native_action_set(layer, key.row, key.col, &action)
                    .ok_or_else(|| format!("action does not fit a packet: {action:?}"))?;
                let reply = request(&mut t, &req)?;
                if reply[4] != 0 {
                    return Err(format!(
                        "keyboard rejected layer {layer} key {index} ({action:?}): status {}",
                        reply[4]
                    ));
                }
                written += 1;
                std::thread::sleep(std::time::Duration::from_millis(25)); // let the flash queue drain
            }
            for (i, combo) in zmk_combos().iter().enumerate() {
                let base = |idx: u8| {
                    let key = &keymap::OP36_KEYS[idx as usize];
                    let action = zmk_layout()
                        .into_iter()
                        .find(|(l, k, _)| *l == 0 && *k == idx)
                        .map(|e| e.2)
                        .unwrap_or(keymap::KeyAction::No);
                    let _ = key;
                    action_to_via_keycode(&action).unwrap_or(0)
                };
                let keys: Vec<u16> = combo.keys.iter().map(|k| base(*k)).collect();
                let out =
                    action_to_via_keycode(&combo.output).ok_or("combo output not expressible")?;
                t.write(&req_combo_set(i as u8, &keys, out))?;
                // Vial dynamic-entry replies start with the return code (0 = ok), not the command byte.
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
                loop {
                    match t.read(50)? {
                        Some(pkt) if pkt[0] == 0 => break,
                        Some(pkt) if pkt[0] == 0xE9 => continue,
                        Some(pkt) => return Err(format!("combo {i} rejected: {:02x?}", &pkt[..4])),
                        None if std::time::Instant::now() > deadline => {
                            return Err(format!("combo {i}: no reply"));
                        }
                        None => continue,
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            write_taphold_settings(&mut t)?;
            println!(
                "wrote {written} keys and {} combos; verifying…",
                zmk_combos().len()
            );
            let km = read_keymap(&mut t, 6, 6)?;
            let mut mismatches = 0;
            for (layer, index, action) in zmk_layout() {
                let got = km.action(layer, &keymap::OP36_KEYS[index as usize]);
                if got != action {
                    mismatches += 1;
                    println!(
                        "  layer {layer} key {index}: wanted {action:?}, keyboard has {got:?}"
                    );
                }
            }
            println!("verification: {} mismatches", mismatches);
            if mismatches == 0 {
                Ok(())
            } else {
                Err("keymap differs after write".into())
            }
        })()),
        Some("--version") => {
            println!("{}", version::version_line());
            Some(Ok(()))
        }
        Some("--snapshot") => Some((|| {
            // --snapshot <file.png> [EN|RU] [layer] [scale] [find-hid-hex]: render the keymap offline.
            let path = args
                .get(1)
                .ok_or("usage: --snapshot <file.png> [EN|RU] [layer] [scale]")?;
            let hkl = match args.get(2).map(String::as_str) {
                Some("RU") => legend::chars::RU,
                _ => legend::chars::EN,
            };
            let layer: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            let scale: f32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            let mut model = Model::new(
                Settings::default(),
                std::fs::read(paths::keymap_cache_file()).ok(),
            );
            model.apply(ModelEvent::Language(hkl));
            model.apply(ModelEvent::Hid(model::HidEvent::Connected(
                vial::transport::Transport::Usb,
            )));
            model.apply(ModelEvent::Hid(model::HidEvent::Layer(model::LayerState {
                active: layer,
                mask: if layer == 0 { 0 } else { 1 << layer },
                ..Default::default()
            })));
            if let Some(hid) = args
                .get(5)
                .and_then(|h| u8::from_str_radix(h.trim_start_matches("0x"), 16).ok())
            {
                model.apply(ModelEvent::Key(model::KeyPress {
                    hid: Some(hid),
                    scancode: 0,
                    extended: false,
                    mods_at_press: Default::default(),
                    device: model::DeviceKind::Other,
                }));
            }
            // Optional tokens anywhere after the file: charging=left|right|both, master=right, switching.
            let token = |name: &str| {
                args.iter()
                    .find_map(|a| a.strip_prefix(name).map(str::to_string))
            };
            let charging = token("charging=").unwrap_or_default();
            model.apply(ModelEvent::Hid(model::HidEvent::Battery(
                vial::protocol::BatteryHalves {
                    left: Some(85),
                    right: Some(15),
                    left_charging: matches!(charging.as_str(), "left" | "both"),
                    right_charging: matches!(charging.as_str(), "right" | "both"),
                },
            )));
            if args.iter().any(|a| a == "half-offline") {
                model.apply(ModelEvent::Hid(model::HidEvent::Layer(model::LayerState {
                    active: layer,
                    mask: if layer == 0 { 0 } else { 1 << layer },
                    peer_linked: false,
                    peer_link_known: true,
                    ..Default::default()
                })));
            }
            if token("master=").as_deref() == Some("right") {
                model.apply(ModelEvent::Hid(model::HidEvent::Layer(model::LayerState {
                    active: layer,
                    mask: if layer == 0 { 0 } else { 1 << layer },
                    central_is_right: true,
                    peer_linked: !args.iter().any(|a| a == "half-offline"),
                    peer_link_known: true,
                    ..Default::default()
                })));
            }
            if args.iter().any(|a| a == "switching") {
                model.apply(ModelEvent::Hid(model::HidEvent::Switching(
                    vial::protocol::Side::Right,
                )));
                model.apply(ModelEvent::Hid(model::HidEvent::Disconnected));
            }
            if args.iter().any(|a| a == "keyboard-off") {
                model.apply(ModelEvent::Hid(model::HidEvent::Disconnected));
            }
            let frame = model.frame(&crate::app_common::char_source());
            let bmp = crate::app_common::render_snapshot(&frame, scale)?;
            // Un-premultiply onto an opaque dark background so the PNG is viewable anywhere.
            let mut rgba = Vec::with_capacity(bmp.premultiplied.len() * 4);
            for px in &bmp.premultiplied {
                let (a, r, g, b) = (
                    (px >> 24) & 0xFF,
                    (px >> 16) & 0xFF,
                    (px >> 8) & 0xFF,
                    px & 0xFF,
                );
                let bg = |c: u32| (c + 0x3A * (255 - a) / 255).min(255) as u8;
                rgba.extend_from_slice(&[bg(r), bg(g), bg(b), 255]);
            }
            let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
            let mut enc = png::Encoder::new(
                std::io::BufWriter::new(file),
                bmp.width as u32,
                bmp.height as u32,
            );
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().map_err(|e| e.to_string())?;
            w.write_image_data(&rgba).map_err(|e| e.to_string())?;
            println!(
                "wrote {path} ({}x{}), status: {}",
                bmp.width, bmp.height, frame.status
            );
            Ok(())
        })()),
        _ => None,
    }
}

