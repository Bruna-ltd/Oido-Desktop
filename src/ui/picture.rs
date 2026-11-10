//! Attached pictures as the screen shows them: decoded once from the bytes a message carries and
//! kept for as long as the app runs, so the composer and the chat draw them without decoding again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rune::{ImageCache, Pixels};

use crate::prompt::Image;

thread_local! {
    static PICTURES: RefCell<Pictures> = RefCell::new(Pictures::default());
}

#[derive(Default)]
struct Pictures {
    cache: Option<ImageCache>,
    /// By [`key`]; the cache frees pixels nothing holds, so the handles are kept here.
    loaded: HashMap<String, rune::Image>,
}

/// A cheap key for a picture's bytes: their length and both ends of the base64. Two pictures that
/// share all three are, for any picture a person attaches, the same picture.
fn key(img: &Image) -> String {
    let d = &img.data;
    let n = d.len().min(64);
    format!("{}:{}:{}", d.len(), &d[..n], &d[d.len() - n..])
}

fn handle(img: &Image) -> Option<rune::Image> {
    PICTURES.with(|p| {
        let mut p = p.borrow_mut();
        let k = key(img);
        if !p.loaded.contains_key(&k) {
            let bytes = STANDARD.decode(&img.data).ok()?;
            let cache = p.cache.get_or_insert_with(|| {
                // A picture that finishes decoding asks the window for another frame.
                ImageCache::new(Arc::new(|| {
                    if let Some(app) = crate::app::handle() {
                        app.update(|_, _| {});
                    }
                }))
            });
            let image = cache.load(bytes);
            p.loaded.insert(k.clone(), image);
        }
        p.loaded.get(&k).cloned()
    })
}

/// The picture's pixels, drawn once decoded; `None` when its bytes are not base64.
pub fn pixels(img: &Image) -> Option<Pixels> {
    handle(img).map(|h| h.pixels().clone())
}

/// Waits for the pictures to decode, for a headless picture of the screen.
#[cfg(debug_assertions)]
pub fn wait(images: &[&Image]) {
    for img in images {
        if let Some(h) = handle(img) {
            let _ = h.wait();
        }
    }
}

/// The size to show a `width`×`height` picture at, fitting inside `max_w`×`max_h` and never
/// larger than it is.
pub fn fit(width: u32, height: u32, max_w: f32, max_h: f32) -> (f32, f32) {
    let (w, h) = (width.max(1) as f32, height.max(1) as f32);
    let scale = (max_w / w).min(max_h / h).min(1.0);
    (w * scale, h * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_shrinks_to_fit_keeping_its_shape_and_never_grows() {
        let fitted = |w, h| {
            let (a, b) = fit(w, h, 320.0, 240.0);
            (a.round(), b.round())
        };
        assert_eq!(fitted(1280, 800), (320.0, 200.0));
        assert_eq!(fitted(800, 1600), (120.0, 240.0));
        assert_eq!(fitted(100, 50), (100.0, 50.0));
    }
}
