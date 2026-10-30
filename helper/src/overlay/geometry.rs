//! Where every key and case sits, in unscaled logical pixels (spec §6.2).
//!
//! Coordinates are "panel coordinates": the left half occupies `0..HALF_W`, then a `GAP`, then the
//! mirrored right half; the renderer adds `PAD` and the status bar on top.

use crate::keymap::{Hand, PhysicalKey};

pub const KEY: f32 = 64.0;
pub const PITCH: f32 = 72.0;
pub const RADIUS: f32 = 9.0;
pub const HALF_W: f32 = 470.0;
pub const HALF_H: f32 = 400.0;
/// Space between the halves; wide enough for the find-key recipe column so the layout never moves.
pub const GAP: f32 = 250.0;
pub const PAD: f32 = 10.0;
/// The status pill sits inside the gap between the halves, so it costs no vertical space.
pub const STATUS_H: f32 = 0.0;
pub const RECIPE_ROW_H: f32 = 34.0;
pub const THUMB_ROT_DEG: f32 = 15.0;
/// Stroke width that turns the convex hull of the keys into the case silhouette.
pub const CASE_MARGIN: f32 = 30.0;

const COL_X: [f32; 5] = [40.0, 112.0, 184.0, 256.0, 328.0];
const COL_DY: [f32; 5] = [40.0, 10.0, 0.0, 10.0, 20.0];
const ROW_Y0: f32 = 40.0;
/// Left-half thumb centres, outer → inner.
const THUMB: [(f32, f32); 3] = [(236.0, 300.0), (308.0, 312.0), (380.0, 330.0)];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyRect {
    pub cx: f32,
    pub cy: f32,
    pub size: f32,
    pub rot_deg: f32,
}

impl KeyRect {
    /// The four corners after rotation, in panel coordinates.
    pub fn corners(&self) -> [(f32, f32); 4] {
        let (s, c) = self.rot_deg.to_radians().sin_cos();
        let h = self.size / 2.0;
        [(-h, -h), (h, -h), (h, h), (-h, h)]
            .map(|(dx, dy)| (self.cx + dx * c - dy * s, self.cy + dx * s + dy * c))
    }
}

fn mirror_x(x: f32) -> f32 {
    2.0 * HALF_W + GAP - x
}

/// Centre, size and rotation of a key in panel coordinates.
pub fn key_rect(key: &PhysicalKey) -> KeyRect {
    let (lx, cy, rot) = match (key.column, key.thumb) {
        (Some(c), _) => {
            let c = c as usize;
            let row = (key.row % 3) as f32;
            (COL_X[c] + KEY / 2.0, ROW_Y0 + COL_DY[c] + row * PITCH + KEY / 2.0, 0.0)
        }
        (None, Some(t)) => {
            let (tx, ty) = THUMB[t as usize];
            (tx, ty, THUMB_ROT_DEG)
        }
        (None, None) => (0.0, 0.0, 0.0),
    };
    match key.hand {
        Hand::Left => KeyRect { cx: lx, cy, size: KEY, rot_deg: rot },
        Hand::Right => KeyRect {
            cx: mirror_x(lx),
            cy,
            size: KEY,
            rot_deg: -rot,
        },
    }
}

/// Battery indicator rectangle `(x, y, w, h)`: on the case beside the inner column, level with
/// the home row (right of G on the left half, left of H on the right half, as Boris asked).
pub fn battery_rect(hand: Hand) -> (f32, f32, f32, f32) {
    let (w, h) = (BATTERY_W, BATTERY_H);
    let x_left = COL_X[4] + KEY + 8.0;
    let y = ROW_Y0 + COL_DY[4] + PITCH + KEY / 2.0 - h / 2.0;
    match hand {
        Hand::Left => (x_left, y, w, h),
        Hand::Right => (mirror_x(x_left + w), y, w, h),
    }
}

pub const BATTERY_W: f32 = 56.0;
pub const BATTERY_H: f32 = 22.0;

/// Square slot for the USB / Bluetooth glyph, centred under the battery of each half
/// (`(x, y, w, h)`); it is drawn on whichever half is the master.
pub fn link_glyph_rect(hand: Hand) -> (f32, f32, f32, f32) {
    let (bx, by, bw, bh) = battery_rect(Hand::Left);
    let side = 24.0;
    let x_left = bx + (bw - side) / 2.0;
    let y = by + bh + 4.0;
    match hand {
        Hand::Left => (x_left, y, side, side),
        Hand::Right => (mirror_x(x_left + side), y, side, side),
    }
}

/// Case silhouette of one half in the vendor's documentation style: a stepped top edge that
/// follows the column stagger, an inner side that bulges around the battery and the thumb cluster,
/// and a bottom edge sloping from the thumbs back up to the pinky column. Stroke it `CASE_MARGIN`
/// wide with round joins to get the rounded case; the polygon is not convex.
pub fn case_outline(hand: Hand) -> Vec<(f32, f32)> {
    let top = |c: usize| ROW_Y0 + COL_DY[c];
    let bottom_pinky = ROW_Y0 + COL_DY[0] + 3.0 * PITCH - (PITCH - KEY);
    let (bx, by, bw, _) = battery_rect(Hand::Left);
    let (_, gy, _, gh) = link_glyph_rect(Hand::Left);
    let inner = bx + bw + 4.0;
    let space = key_rect(&crate::keymap::OP36_KEYS[thumb_index(Hand::Left, 2)]).corners();
    let bksp = key_rect(&crate::keymap::OP36_KEYS[thumb_index(Hand::Left, 0)]).corners();
    let mut pts: Vec<(f32, f32)> = Vec::new();
    // top edge follows the stagger with slanted connectors (the round-joined stroke smooths them)
    for c in 0..5 {
        pts.push((COL_X[c] + 6.0, top(c)));
        pts.push((COL_X[c] + KEY - 6.0, top(c)));
    }
    // inner side: down past the battery and the glyph, then around the Space thumb
    pts.push((COL_X[4] + KEY, by - 12.0));
    pts.push((inner, by - 4.0));
    pts.push((inner, gy + gh + 4.0));
    pts.push((space[1].0 + 6.0, space[1].1 - 10.0));
    pts.push((space[2].0 + 4.0, space[2].1 + 2.0));
    // bottom: thumb cluster, then up to the pinky column
    pts.push((bksp[3].0 - 2.0, bksp[3].1 + 2.0));
    pts.push((COL_X[0] + KEY, bottom_pinky));
    pts.push((COL_X[0], bottom_pinky));
    match hand {
        Hand::Left => pts,
        Hand::Right => pts.into_iter().map(|(x, y)| (mirror_x(x), y)).collect(),
    }
}

/// Index into `OP36_KEYS` of thumb `t` (outer → inner) of `hand`.
fn thumb_index(hand: Hand, t: u8) -> usize {
    crate::keymap::OP36_KEYS
        .iter()
        .position(|k| k.hand == hand && k.thumb == Some(t))
        .expect("thumb key exists")
}

/// `true` when `(x, y)` is inside the outline (even-odd rule) or within `CASE_MARGIN / 2` of an
/// edge: the area the stroked case covers.
#[cfg(test)] // only tests use this
pub fn point_on_case(outline: &[(f32, f32)], x: f32, y: f32) -> bool {
    let n = outline.len();
    let mut inside = false;
    let mut near = false;
    for i in 0..n {
        let (ax, ay) = outline[i];
        let (bx, by) = outline[(i + 1) % n];
        if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
            inside = !inside;
        }
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
        let (px, py) = (ax + t * dx, ay + t * dy);
        if ((x - px).powi(2) + (y - py).powi(2)).sqrt() <= CASE_MARGIN / 2.0 {
            near = true;
        }
    }
    inside || near
}

/// Overall panel size in device pixels. Recipes live in the gap between the halves, so the size does
/// not depend on how many there are (the overlay must never jump).
pub fn panel_size(scale: f32, _recipe_rows: usize) -> (f32, f32) {
    let w = 2.0 * HALF_W + GAP + 2.0 * PAD;
    let h = STATUS_H + HALF_H + PAD;
    (w * scale, h * scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{Hand, OP36_KEYS};

    #[test]
    fn thumb_keys_are_rotated_15_degrees() {
        assert_eq!(key_rect(&OP36_KEYS[30]).rot_deg, 15.0);
        assert_eq!(key_rect(&OP36_KEYS[35]).rot_deg, -15.0);
        assert_eq!(key_rect(&OP36_KEYS[0]).rot_deg, 0.0);
    }

    #[test]
    fn pinky_column_sits_40px_lower_than_middle() {
        let q = key_rect(&OP36_KEYS[0]);
        let e = key_rect(&OP36_KEYS[2]);
        assert_eq!(q.cy - e.cy, 40.0);
        assert_eq!(e.cx - q.cx, 2.0 * PITCH);
    }

    #[test]
    fn right_half_mirrors_left() {
        let q = key_rect(&OP36_KEYS[0]);
        let p = key_rect(&OP36_KEYS[9]);
        assert_eq!(q.cx + p.cx, 2.0 * HALF_W + GAP);
        assert_eq!(q.cy, p.cy);
        let enter = key_rect(&OP36_KEYS[33]);
        let space = key_rect(&OP36_KEYS[32]);
        assert_eq!(space.cx + enter.cx, 2.0 * HALF_W + GAP, "inner thumbs mirror each other");
    }

    #[test]
    fn case_outline_contains_all_keys() {
        for hand in [Hand::Left, Hand::Right] {
            let hull = case_outline(hand);
            assert!(hull.len() >= 12, "stepped top, inner bulge, thumb bottom");
            for k in OP36_KEYS.iter().filter(|k| k.hand == hand) {
                for (x, y) in key_rect(k).corners() {
                    assert!(point_on_case(&hull, x, y), "key {} corner ({x},{y}) outside {hand:?} case", k.index);
                }
            }
        }
    }

    #[test]
    fn link_glyph_rect_mirrors_like_the_battery() {
        let l = link_glyph_rect(Hand::Left);
        let r = link_glyph_rect(Hand::Right);
        assert_eq!(l.0 + l.2 + r.0, 2.0 * HALF_W + GAP, "mirrored");
        let (_, by, _, bh) = battery_rect(Hand::Left);
        assert!(l.1 >= by + bh, "under the battery");
        for (hand, (x, y, w, h)) in [(Hand::Left, l), (Hand::Right, r)] {
            let hull = case_outline(hand);
            for (px, py) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
                assert!(point_on_case(&hull, px, py), "{hand:?} glyph corner ({px},{py}) off the case");
            }
        }
    }

    #[test]
    fn battery_rect_sits_beside_the_inner_column_at_the_home_row() {
        for hand in [Hand::Left, Hand::Right] {
            let hull = case_outline(hand);
            let (x, y, w, h) = battery_rect(hand);
            for (px, py) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
                assert!(point_on_case(&hull, px, py), "{hand:?} battery corner ({px},{py}) off the case");
            }
            // Level with G / H, clear of the inner column's keys.
            let inner_col = 4; // column indices are mirrored with the half: 4 is the inner column on both
            let g = key_rect(
                OP36_KEYS
                    .iter()
                    .find(|k| k.hand == hand && k.row % 3 == 1 && k.column == Some(inner_col))
                    .expect("home-row inner key"),
            );
            assert!((y + h / 2.0 - g.cy).abs() < 1.0, "battery centred on the home row");
            if hand == Hand::Left {
                assert!(x >= g.cx + KEY / 2.0 + 4.0, "right of G");
            } else {
                assert!(x + w <= g.cx - KEY / 2.0 - 4.0, "left of H");
            }
        }
        let l = battery_rect(Hand::Left);
        let r = battery_rect(Hand::Right);
        assert_eq!(l.0 + l.2 + r.0, 2.0 * HALF_W + GAP, "mirrored");
    }

    #[test]
    fn panel_size_is_independent_of_recipe_rows() {
        assert_eq!(panel_size(1.0, 3), panel_size(1.0, 0));
        assert!(GAP >= 3.0 * RECIPE_ROW_H + 100.0, "gap must hold three recipe rows of chips");
        let (w, h) = panel_size(0.5, 0);
        assert_eq!(w, (2.0 * HALF_W + GAP + 2.0 * PAD) * 0.5);
        assert_eq!(h, (STATUS_H + HALF_H + PAD) * 0.5);
    }
}
