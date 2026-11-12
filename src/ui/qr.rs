//! QR codes as the screen shows them: dark on white whatever the theme, with the quiet border
//! scanners need, drawn from the code's modules. Kept while the app runs, a few at a time (a
//! WhatsApp link's code changes every few seconds).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use qrcode::{Color, QrCode};
use rune::{El, ImageCache, row, text};
use rune_kit::Kit;

/// Device pixels per module.
const SCALE: usize = 8;
/// Modules of white around the code, as the standard asks.
const QUIET: usize = 4;
/// The most codes kept at once.
const KEPT: usize = 4;

thread_local! {
    static CODES: RefCell<Codes> = RefCell::new(Codes::default());
}

#[derive(Default)]
struct Codes {
    cache: Option<ImageCache>,
    /// By the text each holds; the cache frees pixels nothing holds, so the handles are kept.
    shown: HashMap<String, rune::Image>,
}

/// The QR code holding `data`, `side` points square; words in its place if it cannot be made.
pub fn code(data: &str, side: f32, k: &Kit) -> El {
    let Ok(qr) = QrCode::new(data.as_bytes()) else {
        return text("This code could not be drawn.")
            .font(k.ty.small)
            .color(k.c.text_muted);
    };
    let image = CODES.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(image) = c.shown.get(data) {
            return image.clone();
        }
        if c.shown.len() >= KEPT {
            c.shown.clear();
        }
        let cache = c.cache.get_or_insert_with(|| {
            ImageCache::new(Arc::new(|| {
                if let Some(app) = crate::app::handle() {
                    app.update(|_, _| {});
                }
            }))
        });
        let (px, rgba) = pixels(qr.width(), &qr.to_colors());
        let image = cache.rgba(px, px, rgba);
        c.shown.insert(data.to_owned(), image.clone());
        image
    });
    row().size(side, side).radius(8.0).pixels(image.pixels())
}

/// The code's modules (`width` square) as RGBA pixels with the quiet border: its side in pixels
/// and the bytes.
fn pixels(width: usize, modules: &[Color]) -> (u32, Vec<u8>) {
    let side = (width + 2 * QUIET) * SCALE;
    let mut rgba = vec![255u8; side * side * 4];
    for (i, m) in modules.iter().enumerate() {
        if *m != Color::Dark {
            continue;
        }
        let (mx, my) = (i % width + QUIET, i / width + QUIET);
        for y in my * SCALE..(my + 1) * SCALE {
            for x in mx * SCALE..(mx + 1) * SCALE {
                let at = (y * side + x) * 4;
                rgba[at..at + 3].fill(0);
            }
        }
    }
    (u32::try_from(side).unwrap_or(u32::MAX), rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_become_dark_squares_inside_a_white_border() {
        let (side, rgba) = pixels(2, &[Color::Dark, Color::Light, Color::Light, Color::Dark]);
        let side = side as usize;
        assert_eq!(side, (2 + 2 * QUIET) * SCALE);
        let at = |x: usize, y: usize| rgba[(y * side + x) * 4];
        let first = QUIET * SCALE;
        assert_eq!(at(0, 0), 255, "the border is white");
        assert_eq!(at(first, first), 0, "the first module is dark");
        assert_eq!(at(first + SCALE, first), 255, "the second is light");
        assert_eq!(at(first + SCALE, first + SCALE), 0, "the last is dark");
    }
}
