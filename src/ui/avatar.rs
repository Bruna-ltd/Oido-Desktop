//! An agent's face when it is a picture: decoded once and kept for as long as the app runs, so the
//! sidebar, the chat header and the form draw it without decoding again. See [`crate::avatars`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use rune::{El, ImageCache, row};

use crate::avatars::{Avatar, preset};

thread_local! {
    static FACES: RefCell<Faces> = RefCell::new(Faces::default());
}

#[derive(Default)]
struct Faces {
    cache: Option<ImageCache>,
    /// By [`key`]; the cache frees pixels nothing holds, so the handles are kept here.
    loaded: HashMap<String, rune::Image>,
}

/// What tells one picture from another: a preset's id, or an upload's fingerprint.
fn key(face: &Avatar) -> String {
    match face {
        Avatar::Preset(id) => format!("preset:{id}"),
        Avatar::Upload(u) => format!("upload:{}", u.key),
    }
}

impl Faces {
    fn image(&mut self, face: &Avatar) -> Option<&rune::Image> {
        let key = key(face);
        if !self.loaded.contains_key(&key) {
            let bytes: Arc<[u8]> = match face {
                Avatar::Preset(id) => Arc::from(preset(id)?.png),
                Avatar::Upload(u) => Arc::from(u.png.as_slice()),
            };
            let cache = self.cache.get_or_insert_with(|| {
                // A picture that finishes decoding asks the window for another frame.
                ImageCache::new(Arc::new(|| {
                    if let Some(app) = crate::app::handle() {
                        app.update(|_, _| {});
                    }
                }))
            });
            self.loaded.insert(key.clone(), cache.load(bytes));
        }
        self.loaded.get(&key)
    }
}

/// The picture as a rounded tile `size` points square. Nothing is drawn for a preset this version
/// does not have, which cannot be saved (see [`Avatar::from_columns`]).
pub fn picture(face: &Avatar, size: f32, radius: f32) -> El {
    let tile = row().size(size, size).radius(radius);
    FACES.with(|f| match f.borrow_mut().image(face) {
        Some(image) => tile.pixels(image.pixels()),
        None => tile,
    })
}

/// Waits for these pictures to decode, so a headless picture of the screen has them.
#[cfg(debug_assertions)]
pub fn wait_all<'a>(faces: impl IntoIterator<Item = &'a Avatar>) {
    FACES.with(|f| {
        let mut f = f.borrow_mut();
        for face in faces {
            if let Some(image) = f.image(face) {
                let _ = image.wait();
            }
        }
    });
}
