//! tiny-skia rendering of a [`Frame`] into a premultiplied BGRA bitmap: the Linux counterpart of
//! `render_d2d.rs`, drawing the same shapes in the same logical coordinates. Text is laid out from
//! system fonts (a semibold UI face plus symbol fallbacks) and filled as glyph outlines, so it goes
//! through the same transforms as the keys (rotated thumbs included).

use std::cell::RefCell;
use std::collections::HashMap;

use ab_glyph::{Font, FontVec, GlyphId, OutlineCurve, VariableFont};
use tiny_skia::{
    FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform,
};

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Center,
    TopLeft,
    BottomLeft,
    BottomRight,
}

/// `(left, top, right, bottom)` in the current logical coordinates.
#[derive(Clone, Copy)]
struct RectF {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> RectF {
    RectF {
        left,
        top,
        right,
        bottom,
    }
}

// ---------------------------------------------------------------------------------------------
// Fonts

/// UI faces tried first for letters and digits (semibold, like the Windows build's Segoe UI).
const PRIMARY_FAMILIES: &[&str] = &[
    "Ubuntu Sans",
    "Noto Sans",
    "Cantarell",
    "Open Sans",
    "Liberation Sans",
    "DejaVu Sans",
];
/// Faces tried, in order, for characters the primary face lacks (key symbols, arrows, Cyrillic).
const FALLBACK_FAMILIES: &[&str] = &[
    "DejaVu Sans",
    "Noto Sans Symbols2",
    "Noto Sans Symbols",
    "Noto Sans Math",
    "DejaVu Sans Mono",
    "FreeSans",
];

struct LoadedFont {
    font: FontVec,
}

/// The primary face, an ordered fallback list, and a per-character cache of which face draws it.
pub struct Fonts {
    faces: Vec<LoadedFont>,
    db: fontdb::Database,
    /// Faces from the whole system tried once each for characters nothing above covers.
    scanned: bool,
    pick: RefCell<HashMap<char, Option<usize>>>,
}

impl Fonts {
    pub fn load() -> Fonts {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let mut faces = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let semibold = fontdb::Weight(600);
        for (i, family) in PRIMARY_FAMILIES
            .iter()
            .chain(FALLBACK_FAMILIES.iter())
            .enumerate()
        {
            let weight = if i < PRIMARY_FAMILIES.len() {
                semibold
            } else {
                fontdb::Weight::NORMAL
            };
            let query = fontdb::Query {
                families: &[fontdb::Family::Name(family)],
                weight,
                ..Default::default()
            };
            let Some(id) = db.query(&query) else { continue };
            if !seen.insert(id) {
                continue;
            }
            if let Some(font) = load_face(&db, id, weight.0 as f32) {
                faces.push(LoadedFont { font });
            }
        }
        if faces.is_empty() {
            log::warn!("no usable system font found; key labels will be missing");
        } else {
            log::info!("renderer: {} font faces loaded", faces.len());
        }
        Fonts {
            faces,
            db,
            scanned: false,
            pick: RefCell::new(HashMap::new()),
        }
    }

    /// Index of the face that draws `c`, scanning the rest of the system once if needed.
    fn face_for(&mut self, c: char) -> Option<usize> {
        if let Some(hit) = self.pick.borrow().get(&c) {
            return *hit;
        }
        let mut found = self
            .faces
            .iter()
            .position(|f| f.font.glyph_id(c).0 != 0);
        if found.is_none() && !self.scanned {
            // Last resort: any installed face with the glyph (loaded on demand, then kept).
            let ids: Vec<fontdb::ID> = self.db.faces().map(|f| f.id).collect();
            for id in ids {
                let covers = self
                    .db
                    .with_face_data(id, |data, index| {
                        ab_glyph::FontRef::try_from_slice_and_index(data, index)
                            .map(|f| f.glyph_id(c).0 != 0)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if covers && let Some(font) = load_face(&self.db, id, 400.0) {
                    self.faces.push(LoadedFont { font });
                    found = Some(self.faces.len() - 1);
                    break;
                }
            }
        }
        self.pick.borrow_mut().insert(c, found);
        found
    }
}

fn load_face(db: &fontdb::Database, id: fontdb::ID, weight: f32) -> Option<FontVec> {
    db.with_face_data(id, |data, index| {
        let mut font = FontVec::try_from_vec_and_index(data.to_vec(), index).ok()?;
        // Variable faces (Ubuntu Sans ships as one) default to their lightest instance.
        font.set_variation(b"wght", weight);
        Some(font)
    })
    .flatten()
}

/// One positioned glyph in em-relative units: `x` is the pen position in px at the requested size.
struct PlacedGlyph {
    face: usize,
    id: GlyphId,
    x: f32,
}

// ---------------------------------------------------------------------------------------------
// Renderer

pub struct Renderer {
    /// Draw the dark palette (set by the caller from the theme).
    pub dark: bool,
    /// Blink phase of the "switching master" arrow (toggled by the caller every 500 ms).
    pub switch_phase: bool,
    fonts: Fonts,
    pixmap: Pixmap,
    transform: Transform,
}

impl Renderer {
    pub fn new() -> Renderer {
        Renderer {
            dark: false,
            switch_phase: false,
            fonts: Fonts::load(),
            pixmap: Pixmap::new(1, 1).expect("1x1 pixmap"),
            transform: Transform::identity(),
        }
    }

    fn paint(c: Rgba) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color_rgba8(
            (c.0 * 255.0).round() as u8,
            (c.1 * 255.0).round() as u8,
            (c.2 * 255.0).round() as u8,
            (c.3 * 255.0).round().clamp(0.0, 255.0) as u8,
        );
        p.anti_alias = true;
        p
    }

    fn round_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
        let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
        // Cubic approximation of a quarter circle.
        let k = 0.552_284_8 * r;
        let mut pb = PathBuilder::new();
        pb.move_to(x + r, y);
        pb.line_to(x + w - r, y);
        pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
        pb.line_to(x + w, y + h - r);
        pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
        pb.line_to(x + r, y + h);
        pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
        pb.line_to(x, y + r);
        pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
        pb.close();
        pb.finish()
    }

    fn fill_round(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgba) {
        if let Some(path) = Self::round_rect_path(x, y, w, h, r) {
            self.pixmap.fill_path(
                &path,
                &Self::paint(c),
                FillRule::Winding,
                self.transform,
                None,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stroke_round(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, c: Rgba) {
        if let Some(path) = Self::round_rect_path(x, y, w, h, r) {
            let stroke = Stroke {
                width,
                ..Default::default()
            };
            self.pixmap
                .stroke_path(&path, &Self::paint(c), &stroke, self.transform, None);
        }
    }

    fn round_stroke(width: f32) -> Stroke {
        Stroke {
            width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            miter_limit: 1.0,
            ..Default::default()
        }
    }

    /// Filled polygon, also stroked `stroke_width` wide with round joins (how the case outline
    /// grows into the rounded case).
    fn polygon(&mut self, pts: &[(f32, f32)], fill: Rgba, stroke_width: f32) {
        if pts.len() < 3 {
            return;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0, pts[0].1);
        for (x, y) in &pts[1..] {
            pb.line_to(*x, *y);
        }
        pb.close();
        let Some(path) = pb.finish() else { return };
        let paint = Self::paint(fill);
        if stroke_width > 0.0 && fill.3 < 1.0 {
            // A translucent fill plus stroke would double-blend where they overlap (Direct2D has
            // the same effect, but its wash is drawn once per half): draw fill + stroke into a mask
            // layer so the union is blended once.
            let (w, h) = (self.pixmap.width(), self.pixmap.height());
            if let Some(mut layer) = Pixmap::new(w, h) {
                let opaque = Self::paint(Rgba(fill.0, fill.1, fill.2, 1.0));
                layer.fill_path(&path, &opaque, FillRule::Winding, self.transform, None);
                layer.stroke_path(
                    &path,
                    &opaque,
                    &Self::round_stroke(stroke_width),
                    self.transform,
                    None,
                );
                let pp = tiny_skia::PixmapPaint {
                    opacity: fill.3,
                    ..Default::default()
                };
                self.pixmap
                    .draw_pixmap(0, 0, layer.as_ref(), &pp, Transform::identity(), None);
                return;
            }
        }
        self.pixmap
            .fill_path(&path, &paint, FillRule::Winding, self.transform, None);
        if stroke_width > 0.0 {
            self.pixmap.stroke_path(
                &path,
                &paint,
                &Self::round_stroke(stroke_width),
                self.transform,
                None,
            );
        }
    }

    fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, c: Rgba) {
        let mut pb = PathBuilder::new();
        pb.move_to(a.0, a.1);
        pb.line_to(b.0, b.1);
        if let Some(path) = pb.finish() {
            let stroke = Stroke {
                width,
                ..Default::default()
            };
            self.pixmap
                .stroke_path(&path, &Self::paint(c), &stroke, self.transform, None);
        }
    }

    fn polyline_round(&mut self, pts: &[(f32, f32)], width: f32, c: Rgba) {
        if pts.len() < 2 {
            return;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0, pts[0].1);
        for (x, y) in &pts[1..] {
            pb.line_to(*x, *y);
        }
        if let Some(path) = pb.finish() {
            self.pixmap.stroke_path(
                &path,
                &Self::paint(c),
                &Self::round_stroke(width),
                self.transform,
                None,
            );
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgba) {
        if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
            self.pixmap.fill_path(
                &path,
                &Self::paint(c),
                FillRule::Winding,
                self.transform,
                None,
            );
        }
    }

    /// Lay `s` out on one line at `size` px (em), aligned in `r` like DirectWrite's text and
    /// paragraph alignment, no wrapping, and fill the glyph outlines.
    fn text(&mut self, s: &str, size: f32, align: Align, r: RectF, c: Rgba) {
        if s.is_empty() || self.fonts.faces.is_empty() {
            return;
        }
        // Pen positions with per-face scaling and kerning between neighbours of the same face.
        let mut glyphs: Vec<PlacedGlyph> = Vec::new();
        let mut pen = 0.0f32;
        let mut prev: Option<(usize, GlyphId)> = None;
        for ch in s.chars() {
            let Some(face) = self.fonts.face_for(ch).or(Some(0)) else {
                continue;
            };
            let font = &self.fonts.faces[face].font;
            let upem = font.units_per_em().unwrap_or(1000.0);
            let k = size / upem;
            let id = font.glyph_id(ch);
            if let Some((pf, pid)) = prev
                && pf == face
            {
                pen += font.kern_unscaled(pid, id) * k;
            }
            glyphs.push(PlacedGlyph { face, id, x: pen });
            pen += font.h_advance_unscaled(id) * k;
            prev = Some((face, id));
        }
        let width = pen;
        // Line box from the primary face, as DirectWrite does with the format's font.
        let primary = &self.fonts.faces[0].font;
        let upem0 = primary.units_per_em().unwrap_or(1000.0);
        let ascent = primary.ascent_unscaled() * size / upem0;
        let descent = -primary.descent_unscaled() * size / upem0;
        let line_h = ascent + descent;
        let x0 = match align {
            Align::Center => (r.left + r.right - width) / 2.0,
            Align::TopLeft | Align::BottomLeft => r.left,
            Align::BottomRight => r.right - width,
        };
        let top = match align {
            Align::Center => (r.top + r.bottom - line_h) / 2.0,
            Align::TopLeft => r.top,
            Align::BottomLeft | Align::BottomRight => r.bottom - line_h,
        };
        let baseline = top + ascent;
        let mut pb = PathBuilder::new();
        for g in &glyphs {
            let font = &self.fonts.faces[g.face].font;
            let k = size / font.units_per_em().unwrap_or(1000.0);
            let Some(outline) = font.outline(g.id) else {
                continue;
            };
            let map = |p: ab_glyph::Point| (x0 + g.x + p.x * k, baseline - p.y * k);
            let mut last: Option<(f32, f32)> = None;
            for curve in &outline.curves {
                let (start, end) = match curve {
                    OutlineCurve::Line(a, b) => (map(*a), map(*b)),
                    OutlineCurve::Quad(a, _, b) => (map(*a), map(*b)),
                    OutlineCurve::Cubic(a, _, _, b) => (map(*a), map(*b)),
                };
                if last.is_none_or(|l| (l.0 - start.0).abs() > 1e-3 || (l.1 - start.1).abs() > 1e-3)
                {
                    if last.is_some() {
                        pb.close();
                    }
                    pb.move_to(start.0, start.1);
                }
                match curve {
                    OutlineCurve::Line(_, _) => pb.line_to(end.0, end.1),
                    OutlineCurve::Quad(_, c1, _) => {
                        let c1 = map(*c1);
                        pb.quad_to(c1.0, c1.1, end.0, end.1)
                    }
                    OutlineCurve::Cubic(_, c1, c2, _) => {
                        let (c1, c2) = (map(*c1), map(*c2));
                        pb.cubic_to(c1.0, c1.1, c2.0, c2.1, end.0, end.1)
                    }
                }
                last = Some(end);
            }
            if last.is_some() {
                pb.close();
            }
        }
        if let Some(path) = pb.finish() {
            self.pixmap.fill_path(
                &path,
                &Self::paint(c),
                FillRule::Winding,
                self.transform,
                None,
            );
        }
    }

    /// Two stacked plates like the vendor's layer icon: a solid front plate over an outlined,
    /// lightly tinted back plate.
    fn layer_icon(&mut self, cx: f32, cy: f32, c: Rgba, s: f32) {
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
            for k in 0..4 {
                self.line(
                    pts[k],
                    pts[(k + 1) % 4],
                    1.5 * s,
                    Rgba(1.0, 1.0, 1.0, 0.9),
                );
            }
        }
    }

    fn draw_key(&mut self, frame: &Frame, draw: &super::frame::KeyDraw, base: Transform) {
        let key = &OP36_KEYS[draw.index as usize];
        let kr = key_rect(key);
        let (fill, text, sub, empty) = palette(draw.color_layer, self.dark);
        let legend = &draw.legend;
        self.transform = base.pre_concat(Transform::from_rotate_at(kr.rot_deg, kr.cx, kr.cy));
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
        let r = |l: f32, t: f32, rr: f32, b: f32| rect(x + l, y + t, x + KEY - rr, y + KEY - b);
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
                    r(7.0, 5.0, 4.0, 4.0),
                    text_color,
                );
                self.text(hint, 18.0, Align::BottomRight, r(4.0, 4.0, 7.0, 6.0), sub);
            }
            Some(shifted) => {
                // Like a real keycap: the Shift symbol centred above, the plain symbol centred below.
                self.text(
                    shifted,
                    15.0,
                    Align::Center,
                    r(2.0, 5.0, 2.0, KEY * 0.52),
                    text_color,
                );
                self.text(
                    &legend.primary,
                    17.0,
                    Align::Center,
                    r(2.0, KEY * 0.46, 2.0, 5.0),
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
                    r(2.0, 3.0, 2.0, KEY * 0.48),
                    text_color,
                );
                self.text(
                    &legend.primary,
                    13.0,
                    Align::Center,
                    r(2.0, KEY * 0.55, 2.0, 4.0),
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
                    r(2.0, 0.0, 2.0, shift * 2.0),
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
            self.text(icon, 17.0, Align::BottomLeft, r(6.0, 4.0, 4.0, 3.0), c);
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
            rect(text_left, y, x + body_w, y + h),
            text_color,
        );
    }

    /// Grey wash plus a word (`offline`, `off`) over a half that is not there.
    fn draw_offline_half(&mut self, side: crate::vial::protocol::Side, label: &str) {
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
            label,
            13.0,
            Align::Center,
            rect(bx, by, bx + bw, by + bh),
            text_color,
        );
    }

    /// Pulsing double arrow in the gap between the halves while the keyboard swaps masters.
    fn draw_switching(&mut self) {
        let (_, y, _, h) = battery_rect(Hand::Left);
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
            rect(cx - 40.0, y - 8.0, cx + 40.0, y + h + 8.0),
            color,
        );
    }

    /// USB or Bluetooth glyph beside the battery of the master half, drawn as vector strokes (no
    /// icon font on Linux).
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
        let c = if self.dark {
            rgb(0xDDDDDD)
        } else {
            rgb(0x333333)
        };
        let p = |fx: f32, fy: f32| (x + fx * w, y + fy * h);
        match link {
            crate::vial::transport::Transport::Ble => {
                // The Bluetooth rune.
                self.polyline_round(
                    &[
                        p(0.30, 0.30),
                        p(0.70, 0.68),
                        p(0.50, 0.87),
                        p(0.50, 0.13),
                        p(0.70, 0.32),
                        p(0.30, 0.70),
                    ],
                    1.8,
                    c,
                );
            }
            crate::vial::transport::Transport::Usb => {
                // The USB trident: stem with an arrow head, a round and a square branch, a base disc.
                self.polyline_round(&[p(0.5, 0.20), p(0.5, 0.80)], 1.8, c);
                self.polygon(&[p(0.5, 0.08), p(0.40, 0.24), p(0.60, 0.24)], c, 0.0);
                self.polyline_round(&[p(0.5, 0.66), p(0.27, 0.52), p(0.27, 0.40)], 1.6, c);
                self.circle(p(0.27, 0.36).0, p(0.27, 0.36).1, w * 0.07, c);
                self.polyline_round(&[p(0.5, 0.58), p(0.73, 0.45), p(0.73, 0.34)], 1.6, c);
                let (sx, sy) = p(0.66, 0.24);
                self.fill_round(sx, sy, w * 0.14, h * 0.12, 0.5, c);
                self.circle(p(0.5, 0.84).0, p(0.5, 0.84).1, w * 0.09, c);
            }
        }
    }

    /// Recipes stack vertically in the gap between the halves, centred, never resizing the overlay.
    fn draw_recipes(&mut self, frame: &Frame, board: Transform) {
        self.transform = board;
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
                        rect(x, y, x + 16.0, y + chip),
                        STATUS,
                    );
                    x += 18.0;
                }
                match c {
                    Chip::Layer(l) => {
                        let bg = if self.dark { HOME_DARK } else { WHITE };
                        self.fill_round(x, y, chip, chip, 6.0, bg);
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
                        self.text(m, 14.0, Align::Center, rect(x, y, x + chip, y + chip), mt);
                        x += chip + 4.0;
                    }
                    Chip::Key(label, layer) => {
                        let (fill, text, _, _) = palette(*layer, self.dark);
                        let w = chip.max(10.0 * label.chars().count() as f32 + 12.0);
                        self.fill_round(x, y, w, chip, 6.0, fill);
                        self.text(label, 14.0, Align::Center, rect(x, y, x + w, y + chip), text);
                        x += w + 4.0;
                    }
                    Chip::Text(t) => {
                        let w = 8.0 * t.chars().count() as f32 + 8.0;
                        self.text(t, 13.0, Align::Center, rect(x, y, x + w, y + chip), STATUS);
                        x += w + 4.0;
                    }
                }
            }
        }
    }

    /// Render `frame` at `scale` (user scale × display scale) into a premultiplied BGRA bitmap.
    pub fn render(&mut self, frame: &Frame, scale: f32, opacity: f32) -> BgraBitmap {
        let (wf, hf) = panel_size(scale, frame.recipes.len());
        let (w, h) = (wf.ceil().max(1.0) as u32, hf.ceil().max(1.0) as u32);
        self.pixmap = Pixmap::new(w, h).expect("pixmap");
        let base = Transform::from_scale(scale, scale);
        let board = base.pre_translate(PAD, STATUS_H);
        self.transform = board;
        let case = if self.dark { CASE_DARK } else { CASE };
        for hand in [Hand::Left, Hand::Right] {
            let hull = case_outline(hand);
            self.polygon(&hull, case, CASE_MARGIN);
        }
        for draw in &frame.keys {
            self.draw_key(frame, draw, board);
        }
        self.transform = board;
        if !frame.connected {
            // The whole keyboard is off (or not found yet): both halves washed out.
            for side in [
                crate::vial::protocol::Side::Left,
                crate::vial::protocol::Side::Right,
            ] {
                self.draw_offline_half(side, if frame.access_denied { "no access" } else { "off" });
            }
        } else if let Some(side) = frame.offline_half {
            self.draw_offline_half(side, "offline");
        }
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
        if !frame.recipes.is_empty() {
            self.draw_recipes(frame, board);
        }
        // Premultiplied RGBA bytes → premultiplied 0xAARRGGBB words, scaled by the overlay opacity.
        let k = opacity.clamp(0.0, 1.0);
        let mut out = BgraBitmap::new(w as i32, h as i32);
        for (dst, px) in out
            .premultiplied
            .iter_mut()
            .zip(self.pixmap.data().chunks_exact(4))
        {
            let s = |v: u8| ((v as f32 * k).round() as u32).min(255);
            *dst = (s(px[3]) << 24) | (s(px[0]) << 16) | (s(px[1]) << 8) | s(px[2]);
        }
        out
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}
