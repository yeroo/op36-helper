//! Background thread that owns the Vial connection: reconnect loop, keymap read, layer
//! subscription. Results reach the UI thread as `WM_APP_HID` messages carrying a boxed
//! [`HidEvent`].

use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::model::HidEvent;
use crate::vial::layer_client::{LayerClient, LayerPacket};
use crate::vial::protocol::{parse_battery_halves, req_battery_halves};
use crate::vial::reader::{read_keymap, request_collect};
use crate::vial::transport::{HidTransport, VialTransport};
use crate::win::msgwindow::WM_APP_HID;

const MATRIX_ROWS: u8 = 6;
const MATRIX_COLS: u8 = 6;
const RECONNECT_EVERY: Duration = Duration::from_secs(2);
const BATTERY_EVERY: Duration = Duration::from_secs(30);
/// How often a Bluetooth session looks for a USB collection to switch to.
const USB_UPGRADE_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum HidCmd {
    ReloadKeymap,
    Quit,
}

/// Wrapper so the UI thread can pass raw `HWND` into the thread (handles are plain integers).
#[derive(Clone, Copy)]
struct UiWindow(isize);
// SAFETY: an HWND is an opaque integer handle; PostMessageW is thread-safe.
unsafe impl Send for UiWindow {}

/// The model event for a layer-notify packet (state update or master handoff).
pub fn route_layer_packet(packet: &LayerPacket) -> HidEvent {
    match packet {
        LayerPacket::State(state) => HidEvent::Layer(*state),
        LayerPacket::Handoff(side) => HidEvent::Switching(*side),
        LayerPacket::Firmware(side, build) => HidEvent::Firmware(*side, build.clone()),
    }
}

fn post(ui: UiWindow, event: HidEvent) {
    let boxed = Box::new(event);
    // SAFETY: the UI thread's handler takes ownership back with `Box::from_raw` (see `take_event`).
    unsafe {
        let _ = PostMessageW(
            Some(HWND(ui.0 as *mut _)),
            WM_APP_HID,
            WPARAM(0),
            LPARAM(Box::into_raw(boxed) as isize),
        );
    }
}

/// Reclaim the event posted by [`post`]. Must be called exactly once per `WM_APP_HID`.
pub fn take_event(lparam: LPARAM) -> Box<HidEvent> {
    // SAFETY: `lparam` is a pointer produced by `Box::into_raw` in `post`.
    unsafe { Box::from_raw(lparam.0 as *mut HidEvent) }
}

pub fn spawn(ui_hwnd: HWND) -> Sender<HidCmd> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ui = UiWindow(ui_hwnd.0 as isize);
    std::thread::Builder::new()
        .name("hid".into())
        .spawn(move || run(ui, rx))
        .expect("spawn hid thread");
    tx
}

fn now_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

fn run(ui: UiWindow, rx: Receiver<HidCmd>) {
    let start = Instant::now();
    let api = match hidapi::HidApi::new() {
        Ok(api) => api,
        Err(e) => {
            post(ui, HidEvent::Error(format!("hidapi init failed: {e}")));
            return;
        }
    };
    let mut api = api;
    loop {
        // ---- disconnected: look for the keyboard every 2 s ----
        let mut transport = loop {
            match rx.try_recv() {
                Ok(HidCmd::Quit) | Err(TryRecvError::Disconnected) => return,
                _ => {}
            }
            let _ = api.refresh_devices();
            if let Some(t) = HidTransport::open(&api) {
                break t;
            }
            std::thread::sleep(RECONNECT_EVERY);
        };
        post(ui, HidEvent::Connected(transport.transport));
        log::info!("keyboard connected over {}", transport.transport.label());
        if let Err(e) = session(ui, &rx, &mut transport, start, &mut api) {
            log::warn!("hid session ended: {e}");
        }
        post(ui, HidEvent::Disconnected);
        log::info!("keyboard disconnected");
        std::thread::sleep(RECONNECT_EVERY);
    }
}

/// One connected session; returns when the device errors or `Quit` arrives (as `Err`).
fn session(
    ui: UiWindow,
    rx: &Receiver<HidCmd>,
    t: &mut HidTransport,
    start: Instant,
    api: &mut hidapi::HidApi,
) -> Result<(), String> {
    let mut next_usb_check = Instant::now() + USB_UPGRADE_EVERY;
    let mut client = LayerClient::new();
    let mut read_pending = true;
    let mut subscribed = false;
    let mut next_battery = Instant::now();
    loop {
        if Instant::now() >= next_battery {
            next_battery = Instant::now() + BATTERY_EVERY;
            let mut stray = Vec::new();
            match request_collect(t, &req_battery_halves(), &mut stray) {
                Ok(reply) => {
                    if let Some(b) = parse_battery_halves(&reply) {
                        post(ui, HidEvent::Battery(b));
                    }
                }
                Err(e) => log::debug!("battery query: {e}"),
            }
            for pkt in &stray {
                if let Some(packet) = client.on_packet(pkt) {
                    post(ui, route_layer_packet(&packet));
                }
            }
        }
        if t.transport == crate::vial::transport::Transport::Ble && Instant::now() >= next_usb_check {
            next_usb_check = Instant::now() + USB_UPGRADE_EVERY;
            let _ = api.refresh_devices();
            if t.usb_available(api) {
                return Err("USB collection appeared, reconnecting over USB".into());
            }
        }
        match rx.try_recv() {
            Ok(HidCmd::Quit) | Err(TryRecvError::Disconnected) => return Err("quit".into()),
            Ok(HidCmd::ReloadKeymap) => read_pending = true,
            Err(TryRecvError::Empty) => {}
        }
        if read_pending {
            read_pending = false;
            match read_keymap(t, MATRIX_ROWS, MATRIX_COLS) {
                Ok(km) => post(ui, HidEvent::Keymap(km)),
                Err(e) => {
                    post(ui, HidEvent::Error(format!("keymap read failed: {e}")));
                    return Err(e);
                }
            }
            if !subscribed {
                subscribed = true;
                for pkt in client.on_connect(now_ms(start)) {
                    t.write(&pkt)?;
                }
            }
        }
        if let Some(pkt) = client.tick(now_ms(start)) {
            t.write(&pkt)?;
        }
        match t.read(50)? {
            Some(pkt) => {
                let was_supported = client.supported;
                if let Some(packet) = client.on_packet(&pkt) {
                    if was_supported != Some(true) {
                        post(ui, HidEvent::LayerNotifySupported(true));
                    }
                    post(ui, route_layer_packet(&packet));
                }
            }
            None => {
                let before = client.supported;
                // Over Bluetooth the first layer packet can take longer than the 1 s USB budget.
                let budget = t.reply_timeout().as_millis() as u64;
                client.on_timeout(now_ms(start).saturating_sub(budget.saturating_sub(500)));
                if before.is_none() && client.supported == Some(false) {
                    post(ui, HidEvent::LayerNotifySupported(false));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vial::protocol::Side;

    #[test]
    fn handoff_in_stray_packets_is_delivered() {
        assert_eq!(route_layer_packet(&LayerPacket::Handoff(Side::Left)), HidEvent::Switching(Side::Left));
        let state = crate::vial::protocol::LayerState { active: 2, ..Default::default() };
        assert_eq!(route_layer_packet(&LayerPacket::State(state)), HidEvent::Layer(state));
    }
}
