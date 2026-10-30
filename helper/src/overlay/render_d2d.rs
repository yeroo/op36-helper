//! Direct2D + DirectWrite rendering of a [`Frame`] into a premultiplied BGRA bitmap (spec §6.2).

use std::collections::HashMap;
use std::ffi::c_void;

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_END_CLOSED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_CAP_STYLE_ROUND, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_LINE_JOIN_ROUND, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_STROKE_STYLE_PROPERTIES, D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory,
    ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_PARAGRAPH_ALIGNMENT_FAR, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GdiFlush, SelectObject,
};
use windows::core::w;
use windows_numerics::{Matrix3x2, Vector2};

use super::BgraBitmap;
use super::frame::{Chip, Frame};
use super::geometry::{
    CASE_MARGIN, GAP, HALF_H, HALF_W, KEY, PAD, RADIUS, RECIPE_ROW_H, STATUS_H, battery_rect,
    case_outline, key_rect, link_glyph_rect, panel_size,
};
use crate::keymap::{Hand, OP36_KEYS};
use crate::legend::KeyKind;

#[derive(Clone, Copy)]
struct Rgba(f32, f32, f32, f32);

const fn rgb(hex: u32) -> Rgba {
    Rgba(
        ((hex >> 16) & 0xFF) as f32 / 255.0,
        ((hex >> 8) & 0xFF) as f32 / 255.0,
        (hex & 0xFF) as f32 / 255.0,
        1.0,
    )
}

const CASE: Rgba = rgb(0xEDEDED);
const HOME: Rgba = rgb(0xDADADA);
const WHITE: Rgba = rgb(0xFFFFFF);
const TEXT_DARK: Rgba = rgb(0x222222);
const TEXT_LIGHT: Rgba = rgb(0xFFFFFF);
const HINT_RED: Rgba = rgb(0xD63A3A);
const HALO: Rgba = rgb(0xFFD34D);
const STATUS: Rgba = Rgba(1.0, 1.0, 1.0, 0.75);

const CASE_DARK: Rgba = rgb(0x3A3D44);
const HOME_DARK: Rgba = rgb(0x4A4E57);

/// Key colour, text colour, secondary-text colour and "empty key" tint per layer.
fn palette(layer: u8, dark: bool) -> (Rgba, Rgba, Rgba, Rgba) {
    match (layer, dark) {
        (0, true) => (CASE_DARK, rgb(0xEDEDED), rgb(0xFF8A8A), rgb(0x30333A)),
        (0, false) => (CASE, TEXT_DARK, HINT_RED, rgb(0xF3F3F3)),
        (layer, _) => palette_colored(layer),
    }
}

fn palette_colored(layer: u8) -> (Rgba, Rgba, Rgba, Rgba) {
    match layer {
        0 => (CASE, TEXT_DARK, HINT_RED, rgb(0xF3F3F3)),
        1 => (rgb(0xD9414C), TEXT_LIGHT, rgb(0xFFD9DC), rgb(0xF3AEB3)),
        2 => (rgb(0x1B8DA3), TEXT_LIGHT, rgb(0xCDEEF4), rgb(0xB9DFE8)),
        3 => (rgb(0xF3E39A), rgb(0x333333), rgb(0x7A6A10), rgb(0xFAF2CF)),
        _ => (rgb(0x9A8FC7), TEXT_LIGHT, rgb(0xE6E0F7), rgb(0xD8D1EC)),
    }
}

/// Symbol drawn above the word on the thumb and edge keys (`⌫ Bksp`, `␣ Space`, `↵ Enter`, …).
fn key_glyph(label: &str) -> Option<&'static str> {
    Some(match label {
        "Bksp" => "\u{232B}",
        "Space" => "\u{2423}",
        "Enter" => "\u{21B5}",
        "Tab" => "\u{21E5}",
        "Esc" => "\u{238B}",
        "Del" => "\u{2326}",
        _ => return None,
    })
}

fn layer_icon_color(layer: u8) -> Rgba {
    match layer {
        0 => TEXT_DARK,
        1 => rgb(0xD9414C),
        2 => rgb(0x1B8DA3),
        3 => rgb(0xE4C94A),
        _ => rgb(0x9A8FC7),
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
enum Align {
    Center,
    TopLeft,
    BottomLeft,
    BottomRight,
}

pub struct Renderer {
    /// Draw the dark palette (set by the caller from the theme).
    pub dark: bool,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    target: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
    round_stroke: ID2D1StrokeStyle,
    formats: HashMap<(u32, Align), IDWriteTextFormat>,
    /// Symbol-font format for the USB / Bluetooth glyph, created on first use.
    icon_format: Option<IDWriteTextFormat>,
    /// Blink phase of the "switching master" arrow (toggled by the caller every 500 ms).
    pub switch_phase: bool,
}

impl Renderer {
    pub fn new() -> windows::core::Result<Renderer> {
        // SAFETY: factory creation with documented arguments; all objects are kept alive by `self`.
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                // Software: a DC target is copied out through GDI anyway, and a software target
                // cannot be lost (no D2DERR_RECREATE_TARGET after driver resets or sleep).
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let target = d2d.CreateDCRenderTarget(&props)?;
            let brush = target.CreateSolidColorBrush(&color(WHITE), None)?;
            let round_stroke = d2d.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 1.0,
                    ..Default::default()
                },
                None,
            )?;
            Ok(Renderer {
                dark: false,
                d2d,
                dwrite,
                target,
                brush,
                round_stroke,
                formats: HashMap::new(),
                icon_format: None,
                switch_phase: false,
            })
        }
    }

    fn format(&mut self, size: f32, align: Align) -> Option<IDWriteTextFormat> {
        let key = (size.to_bits(), align);
        if let Some(f) = self.formats.get(&key) {
            return Some(f.clone());
        }
        // SAFETY: text format creation; Segoe UI is always present on Windows.
        let f = unsafe {
            let f = self
                .dwrite
                .CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )
                .ok()?;
            let (ta, pa) = match align {
                Align::Center => (
                    DWRITE_TEXT_ALIGNMENT_CENTER,
                    DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                ),
                Align::TopLeft => (
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
                ),
                Align::BottomLeft => (
                    DWRITE_TEXT_ALIGNMENT_LEADING,
                    DWRITE_PARAGRAPH_ALIGNMENT_FAR,
                ),
                Align::BottomRight => (
                    DWRITE_TEXT_ALIGNMENT_TRAILING,
                    DWRITE_PARAGRAPH_ALIGNMENT_FAR,
                ),
            };
            let _ = f.SetTextAlignment(ta);
            let _ = f.SetParagraphAlignment(pa);
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            f
        };
        self.formats.insert(key, f.clone());
        Some(f)
    }

    fn set_color(&self, c: Rgba) {
        // SAFETY: brush owned by self.
        unsafe { self.brush.SetColor(&color(c)) };
    }

    fn text(&mut self, s: &str, size: f32, align: Align, rect: D2D_RECT_F, c: Rgba) {
        if s.is_empty() {
            return;
        }
        let Some(fmt) = self.format(size, align) else {
            return;
        };
        self.set_color(c);
        let utf16: Vec<u16> = s.encode_utf16().collect();
        // SAFETY: drawing between BeginDraw/EndDraw with owned resources.
        unsafe {
            self.target.DrawText(
                &utf16,
                &fmt,
                &rect,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    fn fill_round(&self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgba) {
        self.set_color(c);
        let rr = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: x,
                top: y,
                right: x + w,
                bottom: y + h,
            },
            radiusX: r,
            radiusY: r,
        };
        // SAFETY: drawing between BeginDraw/EndDraw.
        unsafe { self.target.FillRoundedRectangle(&rr, &self.brush) };
    }

    fn stroke_round(&self, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, c: Rgba) {
        self.set_color(c);
        let rr = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: x,
                top: y,
                right: x + w,
                bottom: y + h,
            },
            radiusX: r,
            radiusY: r,
        };
        // SAFETY: drawing between BeginDraw/EndDraw.
        unsafe {
            self.target
                .DrawRoundedRectangle(&rr, &self.brush, width, None)
        };
    }

    fn polygon(&self, pts: &[(f32, f32)], fill: Rgba, stroke_width: f32) {
        if pts.len() < 3 {
            return;
        }
        // SAFETY: path geometry built and closed before use; drawn between BeginDraw/EndDraw.
        unsafe {
            let Ok(geometry) = self.d2d.CreatePathGeometry() else {
                return;
            };
            let Ok(sink) = geometry.Open() else { return };
            sink.BeginFigure(
                Vector2 {
                    X: pts[0].0,
                    Y: pts[0].1,
                },
                D2D1_FIGURE_BEGIN_FILLED,
            );
            let rest: Vec<Vector2> = pts[1..]
                .iter()
                .map(|(x, y)| Vector2 { X: *x, Y: *y })
                .collect();
            sink.AddLines(&rest);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            self.set_color(fill);
            self.target.FillGeometry(&geometry, &self.brush, None);
            self.target
                .DrawGeometry(&geometry, &self.brush, stroke_width, &self.round_stroke);
        }
    }

    /// Three stacked rhombi, like the layer icon on the vendor's keycaps.
    /// Two stacked plates like the vendor's layer icon: a solid front plate over an outlined,
    /// lightly tinted back plate.
    fn layer_icon(&self, cx: f32, cy: f32, c: Rgba, s: f32) {
        for (i, dy) in [6.0f32, -5.0].into_iter().enumerate() {
            let a = if i == 1 { 1.0 } else { 0.3 };
            let y = cy + dy * s;
            let pts = [
                (cx, y - 9.0 * s),
                (cx + 15.0 * s, y),
                (cx, y + 9.0 * s),
                (cx - 15.0 * s, y),
            ];
            self.polygon(&pts, Rgba(c.0, c.1, c.2, a), 0.0);
            self.set_color(Rgba(1.0, 1.0, 1.0, 0.9));
            // SAFETY: drawing between BeginDraw/EndDraw.
            unsafe {
                for k in 0..4 {
                    let (x0, y0) = pts[k];
                    let (x1, y1) = pts[(k + 1) % 4];
                    self.target.DrawLine(
                        Vector2 { X: x0, Y: y0 },
                        Vector2 { X: x1, Y: y1 },
                        &self.brush,
                        1.5 * s,
                        None,
                    );
                }
            }
        }
    }

    fn draw_key(&mut self, frame: &Frame, draw: &super::frame::KeyDraw, base: Matrix3x2) {
        let key = &OP36_KEYS[draw.index as usize];
        let kr = key_rect(key);
        let (fill, text, sub, empty) = palette(draw.color_layer, self.dark);
        let legend = &draw.legend;
        let rot = Matrix3x2::rotation_around(kr.rot_deg, Vector2 { X: kr.cx, Y: kr.cy });
        // SAFETY: transform set between BeginDraw/EndDraw.
        unsafe { self.target.SetTransform(&(rot * base)) };
        let (x, y) = (kr.cx - KEY / 2.0, kr.cy - KEY / 2.0);
        // A pure layer thumb is a keycap in its layer's colour with a white icon, like the vendor's
        // drawings; the held one stays white so it reads as "pressed".
        let solid_layer = legend
            .layer_icon
            .filter(|_| legend.primary.is_empty() && legend.kind != KeyKind::HeldLayer);
        let fill = match legend.kind {
            KeyKind::Empty => empty,
            KeyKind::Home if frame.layer_color == 0 => {
                if self.dark {
                    HOME_DARK
                } else {
                    HOME
                }
            }
            KeyKind::HeldLayer => {
                if self.dark {
                    rgb(0x5A5E68)
                } else {
                    WHITE
                }
            }
            _ if solid_layer.is_some() => layer_icon_color(solid_layer.unwrap_or(0)),
            _ => fill,
        };
        if draw.halo {
            self.stroke_round(
                x - 5.0,
                y - 5.0,
                KEY + 10.0,
                KEY + 10.0,
                12.0,
                10.0,
                Rgba(HALO.0, HALO.1, HALO.2, 0.35),
            );
            self.stroke_round(x - 5.0, y - 5.0, KEY + 10.0, KEY + 10.0, 12.0, 4.0, HALO);
        }
        self.fill_round(x, y, KEY, KEY, RADIUS, fill);
        let border = match (draw.color_layer == 0, self.dark) {
            (true, true) => Rgba(1.0, 1.0, 1.0, 0.35),
            (true, false) => Rgba(0.0, 0.0, 0.0, 0.45),
            _ => Rgba(0.0, 0.0, 0.0, 0.12),
        };
        self.stroke_round(x, y, KEY, KEY, RADIUS, 1.2, border);
        if let Some(layer) = legend.layer_icon {
            let scale = if legend.primary.is_empty() { 1.0 } else { 0.55 };
            let (ix, iy) = if legend.primary.is_empty() {
                (kr.cx, kr.cy)
            } else {
                (x + KEY - 14.0, y + 14.0)
            };
            let color = if solid_layer.is_some() {
                WHITE
            } else {
                layer_icon_color(layer)
            };
            self.layer_icon(ix, iy, color, scale);
        }
        let rect = |l: f32, t: f32, r: f32, b: f32| D2D_RECT_F {
            left: x + l,
            top: y + t,
            right: x + KEY - r,
            bottom: y + KEY - b,
        };
        let text_color = if legend.kind == KeyKind::HeldLayer {
            if self.dark { TEXT_LIGHT } else { TEXT_DARK }
        } else {
            text
        };
        match &legend.secondary {
            Some(hint) if legend.secondary_is_hint => {
                self.text(
                    &legend.primary,
                    18.0,
                    Align::TopLeft,
                    rect(7.0, 5.0, 4.0, 4.0),
                    text_color,
                );
                self.text(
                    hint,
                    18.0,
                    Align::BottomRight,
                    rect(4.0, 4.0, 7.0, 6.0),
                    sub,
                );
            }
            Some(shifted) => {
                // Like a real keycap: the Shift symbol centred above, the plain symbol centred below.
                self.text(
                    shifted,
                    15.0,
                    Align::Center,
                    rect(2.0, 5.0, 2.0, KEY * 0.52),
                    text_color,
                );
                self.text(
                    &legend.primary,
                    17.0,
                    Align::Center,
                    rect(2.0, KEY * 0.46, 2.0, 5.0),
                    text_color,
                );
            }
            None if key_glyph(&legend.primary).is_some() => {
                // Thumb keys in the vendor's style: the symbol above, the word below.
                let glyph = key_glyph(&legend.primary).unwrap_or("");
                self.text(
                    glyph,
                    20.0,
                    Align::Center,
                    rect(2.0, 3.0, 2.0, KEY * 0.48),
                    text_color,
                );
                self.text(
                    &legend.primary,
                    13.0,
                    Align::Center,
                    rect(2.0, KEY * 0.55, 2.0, 4.0),
                    text_color,
                );
            }
            None => {
                let n = legend.primary.chars().count();
                let size = if n <= 2 {
                    20.0
                } else if n <= 5 {
                    13.0
                } else {
                    11.0
                };
                let shift = if legend.mod_icon.is_some() { 6.0 } else { 0.0 };
                self.text(
                    &legend.primary,
                    size,
                    Align::Center,
                    rect(2.0, 0.0, 2.0, shift * 2.0),
                    text_color,
                );
            }
        }
        if let Some(icon) = &legend.mod_icon {
            let c = if draw.color_layer == 0 {
                if self.dark {
                    rgb(0xDDDDDD)
                } else {
                    rgb(0x333333)
                }
            } else {
                sub
            };
            self.text(icon, 17.0, Align::BottomLeft, rect(6.0, 4.0, 4.0, 3.0), c);
        }
    }

    /// iPhone-style battery: outline with a nub, a fill proportional to the charge (green, red when
    /// low), and the bare percentage inside.
    fn draw_battery(&mut self, hand: Hand, percent: u8, charging: bool) {
        let (x, y, w, h) = battery_rect(hand);
        let outline = if self.dark {
            rgb(0xDDDDDD)
        } else {
            rgb(0x333333)
        };
        let fill = if percent <= 20 {
            rgb(0xFF3B30)
        } else {
            rgb(0x34C759)
        };
        let body_w = w - 4.0;
        self.stroke_round(x, y, body_w, h, 5.0, 1.5, outline);
        self.fill_round(x + body_w + 1.0, y + h * 0.3, 3.0, h * 0.4, 1.5, outline);
        let inner = (body_w - 4.0) * percent.min(100) as f32 / 100.0;
        if inner > 0.5 {
            self.fill_round(x + 2.0, y + 2.0, inner, h - 4.0, 3.0, fill);
        }
        // iPhone style: a white bolt at the left while on USB power, the number moves right a bit.
        let text_left = if charging {
            let (bx, by) = (x + 5.0, y + 4.0);
            self.polygon(
                &[
                    (bx + 5.0, by),
                    (bx, by + 8.0),
                    (bx + 4.0, by + 8.0),
                    (bx + 2.0, by + 14.0),
                    (bx + 8.0, by + 5.0),
                    (bx + 4.0, by + 5.0),
                    (bx + 6.0, by),
                ],
                WHITE,
                1.0,
            );
            x + 8.0
        } else {
            x
        };
        let text_color = if self.dark { TEXT_LIGHT } else { TEXT_DARK };
        self.text(
            &percent.to_string(),
            12.0,
            Align::Center,
            D2D_RECT_F {
                left: text_left,
                top: y,
                right: x + body_w,
                bottom: y + h,
            },
            text_color,
        );
    }

    /// Grey wash plus `offline` over a half that is not linked to the master.
    fn draw_offline_half(&mut self, side: crate::vial::protocol::Side) {
        let hand = match side {
            crate::vial::protocol::Side::Left => Hand::Left,
            crate::vial::protocol::Side::Right => Hand::Right,
        };
        let hull = case_outline(hand);
        let wash = if self.dark {
            Rgba(0.12, 0.12, 0.12, 0.62)
        } else {
            Rgba(0.82, 0.82, 0.82, 0.70)
        };
        self.polygon(&hull, wash, CASE_MARGIN);
        let (bx, by, bw, bh) = battery_rect(hand);
        let text_color = if self.dark { TEXT_LIGHT } else { TEXT_DARK };
        self.text(
            "offline",
            13.0,
            Align::Center,
            D2D_RECT_F {
                left: bx,
                top: by,
                right: bx + bw,
                bottom: by + bh,
            },
            text_color,
        );
    }

    /// Pulsing double arrow in the gap between the halves while the keyboard swaps masters.
    fn draw_switching(&mut self) {
        let (_, y, _, h) = battery_rect(Hand::Left);
        // Accent colour so the arrow reads on light and dark backgrounds alike.
        let color = Rgba(
            HALO.0,
            HALO.1,
            HALO.2,
            if self.switch_phase { 1.0 } else { 0.45 },
        );
        let cx = HALF_W + GAP / 2.0;
        self.text(
            "\u{21C4}",
            34.0,
            Align::Center,
            D2D_RECT_F {
                left: cx - 40.0,
                top: y - 8.0,
                right: cx + 40.0,
                bottom: y + h + 8.0,
            },
            color,
        );
    }

    /// USB or Bluetooth glyph (Segoe MDL2 Assets) beside the battery of the master half: the
    /// half that is actually connected to the PC.
    fn draw_link(
        &mut self,
        link: crate::vial::transport::Transport,
        side: crate::vial::protocol::Side,
    ) {
        let hand = match side {
            crate::vial::protocol::Side::Left => Hand::Left,
            crate::vial::protocol::Side::Right => Hand::Right,
        };
        let (x, y, w, h) = link_glyph_rect(hand);
        let glyph = match link {
            crate::vial::transport::Transport::Usb => "\u{E88E}",
            crate::vial::transport::Transport::Ble => "\u{E702}",
        };
        let color = if self.dark {
            rgb(0xDDDDDD)
        } else {
            rgb(0x333333)
        };
        let rect = D2D_RECT_F {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        };
        self.icon_text(glyph, h - 6.0, rect, color);
    }

    /// Text in the symbol font (`Segoe MDL2 Assets`), centred in `rect`.
    fn icon_text(&mut self, s: &str, size: f32, rect: D2D_RECT_F, c: Rgba) {
        let fmt = match &self.icon_format {
            Some(f) => f.clone(),
            None => {
                // SAFETY: text format creation; the symbol font ships with Windows 10 and 11.
                let f = unsafe {
                    let Ok(f) = self.dwrite.CreateTextFormat(
                        w!("Segoe MDL2 Assets"),
                        None,
                        DWRITE_FONT_WEIGHT_SEMI_BOLD,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        size,
                        w!("en-us"),
                    ) else {
                        return;
                    };
                    let _ = f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
                    let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                    f
                };
                self.icon_format = Some(f.clone());
                f
            }
        };
        self.set_color(c);
        let utf16: Vec<u16> = s.encode_utf16().collect();
        // SAFETY: drawing between BeginDraw/EndDraw with owned resources.
        unsafe {
            self.target.DrawText(
                &utf16,
                &fmt,
                &rect,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// Recipes stack vertically in the gap between the halves, centred, never resizing the overlay.
    fn draw_recipes(&mut self, frame: &Frame, board: Matrix3x2) {
        // SAFETY: transform set between BeginDraw/EndDraw.
        unsafe { self.target.SetTransform(&board) };
        let chip = 26.0f32;
        let rows = frame.recipes.len() as f32;
        let column_x = HALF_W + 12.0;
        let column_w = GAP - 24.0;
        let first_y = ((HALF_H - rows * RECIPE_ROW_H) / 2.0).max(44.0);
        for (row, recipe) in frame.recipes.iter().enumerate() {
            let y = first_y + row as f32 * RECIPE_ROW_H + (RECIPE_ROW_H - chip) / 2.0;
            let width: f32 = recipe
                .chips
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let w = match c {
                        Chip::Layer(_) | Chip::Mod(_) => chip,
                        Chip::Key(label, _) => chip.max(10.0 * label.chars().count() as f32 + 12.0),
                        Chip::Text(t) => 8.0 * t.chars().count() as f32 + 8.0,
                    };
                    w + 4.0 + if i > 0 { 18.0 } else { 0.0 }
                })
                .sum();
            let mut x = column_x + ((column_w - width) / 2.0).max(0.0);
            for (i, c) in recipe.chips.iter().enumerate() {
                if i > 0 {
                    self.text(
                        "+",
                        14.0,
                        Align::Center,
                        D2D_RECT_F {
                            left: x,
                            top: y,
                            right: x + 16.0,
                            bottom: y + chip,
                        },
                        STATUS,
                    );
                    x += 18.0;
                }
                match c {
                    Chip::Layer(l) => {
                        self.fill_round(
                            x,
                            y,
                            chip,
                            chip,
                            6.0,
                            if self.dark { HOME_DARK } else { WHITE },
                        );
                        self.layer_icon(x + chip / 2.0, y + chip / 2.0, layer_icon_color(*l), 0.6);
                        x += chip + 4.0;
                    }
                    Chip::Mod(m) => {
                        let (mf, mt) = if self.dark {
                            (HOME_DARK, TEXT_LIGHT)
                        } else {
                            (rgb(0xE2E2E2), TEXT_DARK)
                        };
                        self.fill_round(x, y, chip, chip, 6.0, mf);
                        self.text(
                            m,
                            14.0,
                            Align::Center,
                            D2D_RECT_F {
                                left: x,
                                top: y,
                                right: x + chip,
                                bottom: y + chip,
                            },
                            mt,
                        );
                        x += chip + 4.0;
                    }
                    Chip::Key(label, layer) => {
                        let (fill, text, _, _) = palette(*layer, self.dark);
                        let w = chip.max(10.0 * label.chars().count() as f32 + 12.0);
                        self.fill_round(x, y, w, chip, 6.0, fill);
                        self.text(
                            label,
                            14.0,
                            Align::Center,
                            D2D_RECT_F {
                                left: x,
                                top: y,
                                right: x + w,
                                bottom: y + chip,
                            },
                            text,
                        );
                        x += w + 4.0;
                    }
                    Chip::Text(t) => {
                        let w = 8.0 * t.chars().count() as f32 + 8.0;
                        self.text(
                            t,
                            13.0,
                            Align::Center,
                            D2D_RECT_F {
                                left: x,
                                top: y,
                                right: x + w,
                                bottom: y + chip,
                            },
                            STATUS,
                        );
                        x += w + 4.0;
                    }
                }
            }
        }
    }

    /// Render `frame` at `scale` (user scale × DPI factor) into a premultiplied BGRA bitmap.
    pub fn render(
        &mut self,
        frame: &Frame,
        scale: f32,
        opacity: f32,
    ) -> windows::core::Result<BgraBitmap> {
        let (wf, hf) = panel_size(scale, frame.recipes.len());
        let (w, h) = (wf.ceil().max(1.0) as i32, hf.ceil().max(1.0) as i32);
        let mut out = BgraBitmap::new(w, h);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: GDI objects are created, used and released within this function; Direct2D calls
        // happen between BeginDraw and EndDraw on a target bound to the live DC.
        unsafe {
            let dc = CreateCompatibleDC(None);
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let old = SelectObject(dc, bitmap.into());
            let rect = RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            if let Err(e) = self.target.BindDC(dc, &rect) {
                SelectObject(dc, old);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(dc);
                return Err(e);
            }
            self.target.BeginDraw();
            self.target.Clear(Some(&color(Rgba(0.0, 0.0, 0.0, 0.0))));
            let base = Matrix3x2::scale(scale, scale);
            self.target.SetTransform(&base);

            // No panel and no title: only the keyboard shows (logical coordinates from here on).
            let _ = panel_size(1.0, frame.recipes.len());
            let board = Matrix3x2::translation(PAD, STATUS_H) * base;
            self.target.SetTransform(&board);
            let case = if self.dark { CASE_DARK } else { CASE };
            for hand in [Hand::Left, Hand::Right] {
                let hull = case_outline(hand);
                self.polygon(&hull, case, CASE_MARGIN);
            }
            for draw in &frame.keys {
                self.draw_key(frame, draw, board);
            }
            if let Some(side) = frame.offline_half {
                self.target.SetTransform(&board);
                self.draw_offline_half(side);
            }
            self.target.SetTransform(&board);
            let hidden = frame.offline_half;
            if let Some(p) = frame
                .battery_left
                .filter(|_| hidden != Some(crate::vial::protocol::Side::Left))
            {
                self.draw_battery(Hand::Left, p, frame.battery_left_charging);
            }
            if let Some(p) = frame
                .battery_right
                .filter(|_| hidden != Some(crate::vial::protocol::Side::Right))
            {
                self.draw_battery(Hand::Right, p, frame.battery_right_charging);
            }
            if frame.switching {
                self.draw_switching();
            } else if let Some(link) = frame.link {
                self.draw_link(link, frame.link_side);
            }
            let _ = (HALF_W, HALF_H, GAP); // layout constants used through geometry
            if !frame.recipes.is_empty() {
                self.draw_recipes(frame, board);
            }
            let result = self.target.EndDraw(None, None);
            let _ = GdiFlush();
            std::ptr::copy_nonoverlapping(
                bits as *const u32,
                out.premultiplied.as_mut_ptr(),
                (w * h) as usize,
            );
            // Whole-overlay opacity: scale every premultiplied channel (keyboard included).
            let k = opacity.clamp(0.0, 1.0);
            if k < 1.0 {
                for px in out.premultiplied.iter_mut() {
                    let scale = |v: u32| ((v as f32 * k).round() as u32).min(255);
                    *px = (scale(*px >> 24) << 24)
                        | (scale((*px >> 16) & 0xFF) << 16)
                        | (scale((*px >> 8) & 0xFF) << 8)
                        | scale(*px & 0xFF);
                }
            }
            SelectObject(dc, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            result?;
        }
        Ok(out)
    }
}

fn color(c: Rgba) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.0,
        g: c.1,
        b: c.2,
        a: c.3,
    }
}
