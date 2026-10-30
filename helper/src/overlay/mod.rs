//! The click-through overlay: window plumbing and the pixel surface handed to it.

pub mod appbar;
pub mod frame;
pub mod geometry;
pub mod placement;
pub mod render_d2d;
pub mod window;

/// Premultiplied BGRA pixels, row-major, top-down, as `UpdateLayeredWindow` wants them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BgraBitmap {
    pub width: i32,
    pub height: i32,
    pub premultiplied: Vec<u32>,
}

impl BgraBitmap {
    pub fn new(width: i32, height: i32) -> BgraBitmap {
        BgraBitmap {
            width,
            height,
            premultiplied: vec![0; (width.max(0) * height.max(0)) as usize],
        }
    }
}
