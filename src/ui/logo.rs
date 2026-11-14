//! Pictures of MCP servers: the logos Studio shows, for the servers that are presets. Others get
//! the usual tile with their initial.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use rune::{El, ImageCache, row};
use rune_kit::Kit;

use crate::mcp::catalog::{Logo, logo_for};

const SIDE: f32 = 36.0;
/// Marks are drawn at this many device pixels per point, so they stay sharp on a high-DPI screen.
const SCALE: f32 = 2.0;

type Loaded = rune::Image;

thread_local! {
    /// The decoder, and each logo's decoded image, kept for as long as the app runs: the cache
    /// only holds on to an image while something holds a handle on it.
    static LOGOS: RefCell<Logos> = RefCell::new(Logos::default());
}

#[derive(Default)]
struct Logos {
    cache: Option<ImageCache>,
    loaded: HashMap<&'static str, Loaded>,
}

impl Logos {
    fn image(&mut self, name: &'static str, logo: Logo) -> &Loaded {
        if !self.loaded.contains_key(name) {
            let cache = self.cache.get_or_insert_with(|| {
                // A logo that finishes decoding asks the window for another frame.
                ImageCache::new(Arc::new(|| {
                    if let Some(app) = crate::app::handle() {
                        app.update(|_, _| {});
                    }
                }))
            });
            let image = match logo {
                Logo::Image(bytes) => cache.load(bytes),
                Logo::Mark { color, path } => {
                    cache.svg(mark(color, path).into_bytes(), SIDE, SCALE)
                }
            };
            self.loaded.insert(name, image);
        }
        &self.loaded[name]
    }
}

/// A brand's mark, in its colour, on a white rounded tile (as Studio draws it).
fn mark(color: &str, path: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" rx="5" fill="#fff"/><g transform="translate(4.5 4.5) scale(0.625)"><path fill="{color}" d="{path}"/></g></svg>"##
    )
}

/// The picture of the preset at `url`, as a tile; `None` for a server that is not one.
pub fn tile(url: &str, k: &Kit) -> Option<El> {
    let (name, logo) = logo_for(url)?;
    Some(of(name, logo, k))
}

/// A tile drawn from `logo`, which is known by `name` (an extension's, say).
pub fn of(name: &'static str, logo: Logo, k: &Kit) -> El {
    LOGOS.with(|l| {
        let mut l = l.borrow_mut();
        let image = l.image(name, logo);
        row()
            .size(SIDE, SIDE)
            .radius(10.0)
            .border(1.0, k.c.border_muted)
            .pixels(image.pixels())
    })
}

/// Waits for these logos' pixels, so a headless picture of the page has them.
#[cfg(debug_assertions)]
pub fn wait_for(logos: &[(&'static str, Logo)]) {
    LOGOS.with(|l| {
        let mut l = l.borrow_mut();
        for (name, logo) in logos {
            let _ = l.image(name, *logo).wait();
        }
    });
}

/// Waits for every logo's pixels, so a headless picture of the page has them.
#[cfg(debug_assertions)]
pub fn wait_all(urls: &[&str]) {
    LOGOS.with(|l| {
        let mut l = l.borrow_mut();
        for url in urls {
            if let Some((name, logo)) = logo_for(url) {
                let _ = l.image(name, logo).wait();
            }
        }
    });
}
