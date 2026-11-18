//! Pictures of MCP servers: the logos Studio shows, for the servers that are presets. Others get
//! the usual tile with their initial.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use rune::color::hex;
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
    loaded: HashMap<usize, Loaded>,
}

/// The decoder: a logo that finishes decoding asks the window for another frame.
fn new_cache() -> ImageCache {
    ImageCache::new(Arc::new(|| {
        if let Some(app) = crate::app::handle() {
            app.update(|_, _| {});
        }
    }))
}

impl Logos {
    fn image(&mut self, name: &'static str, logo: Logo) -> &Loaded {
        let key = name.as_ptr() as usize;
        if !self.loaded.contains_key(&key) {
            let cache = self.cache.get_or_insert_with(new_cache);
            let image = match logo {
                Logo::Image(bytes) => cache.load(bytes),
                Logo::Mark { color, path } => {
                    cache.svg(mark(color, path).into_bytes(), SIDE, SCALE)
                }
            };
            self.loaded.insert(key, image);
        }
        &self.loaded[&key]
    }
}

impl Logos {
    /// A provider's logo, an SVG (drawn at the size it shows at) or a PNG, decoded once.
    fn art(&mut self, bytes: &'static [u8]) -> &Loaded {
        let cache = self.cache.get_or_insert_with(new_cache);
        self.loaded
            .entry(bytes.as_ptr() as usize)
            .or_insert_with(|| {
                if bytes.starts_with(b"\x89PNG") {
                    cache.load(bytes)
                } else {
                    cache.svg(bytes, GLYPH, SCALE)
                }
            })
    }
}

/// A brand's mark, in its colour, on a white rounded tile (as Studio draws it).
fn mark(color: &str, path: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" rx="5" fill="#fff"/><g transform="translate(4.5 4.5) scale(0.625)"><path fill="{color}" d="{path}"/></g></svg>"##
    )
}

/// Inside a provider's tile, the logo is drawn this big.
const GLYPH: f32 = 22.0;

/// A provider's logo: its SVG or PNG, and the tile it sits on (`None`: the provider's own colour).
fn provider_art(id: &str) -> Option<(&'static [u8], Option<&'static str>)> {
    Some(match id {
        "openai" => (include_bytes!("../../assets/providers/openai.svg"), None),
        "codex" => (
            include_bytes!("../../assets/providers/codex.svg"),
            Some("#ffffff"),
        ),
        "anthropic" => (include_bytes!("../../assets/providers/anthropic.svg"), None),
        "gemini" => (
            include_bytes!("../../assets/providers/gemini.svg"),
            Some("#ffffff"),
        ),
        "openrouter" => (
            include_bytes!("../../assets/providers/openrouter.svg"),
            None,
        ),
        "nvidia" => (
            include_bytes!("../../assets/providers/nvidia.svg"),
            Some("#000000"),
        ),
        "deepseek" => (
            include_bytes!("../../assets/providers/deepseek.svg"),
            Some("#ffffff"),
        ),
        "mistral" => (
            include_bytes!("../../assets/providers/mistral.svg"),
            Some("#ffffff"),
        ),
        "kimi" => (
            include_bytes!("../../assets/providers/kimi.svg"),
            Some("#000000"),
        ),
        "qwen" => (include_bytes!("../../assets/providers/qwen.svg"), None),
        "typesafe" => (
            include_bytes!("../../assets/providers/typesafe.png"),
            Some("#ffffff"),
        ),
        _ => return None,
    })
}

/// The tile of the provider `id`, whose own colour is `color`, with its logo on it; `None` for a
/// provider with no logo, which gets its initial instead.
pub fn provider(id: &str, color: &str, k: &Kit) -> Option<El> {
    let (svg, tile) = provider_art(id)?;
    let image = LOGOS.with(|l| l.borrow_mut().art(svg).pixels().clone());
    Some(
        row()
            .size(SIDE, SIDE)
            .radius(10.0)
            .center()
            .fill(hex(tile.unwrap_or(color)).unwrap_or(k.c.layer_03))
            .border(1.0, k.c.border_muted)
            .child(row().size(GLYPH, GLYPH).pixels(&image)),
    )
}

/// The picture of the preset at `url`, as a tile; `None` for a server that is not one.
pub fn tile(url: &str, k: &Kit) -> Option<El> {
    let (name, logo) = logo_for(url)?;
    Some(of(name, logo, k))
}

/// A tile drawn from `logo`, which is known by `name` (an extension's, say).
pub fn of(name: &'static str, logo: Logo, k: &Kit) -> El {
    sized(name, logo, SIDE, k)
}

/// The picture of the server whose tool is `tool` (`mcp__server__name`): an extension's, or a
/// preset's. `None` for a built-in tool or a server with no picture of its own.
pub fn of_tool(tool: &str, side: f32, k: &Kit) -> Option<El> {
    let server = tool.strip_prefix(crate::mcp::PREFIX)?.split("__").next()?;
    let (name, logo) = crate::extensions::ALL
        .iter()
        .find(|e| crate::extensions::id_of(**e) == server)
        .map(|e| (e.title(), e.logo()))
        .or_else(|| {
            crate::mcp::catalog::PRESETS
                .iter()
                .find(|p| crate::mcp::slug(p.name) == server)
                .map(|p| (p.name, p.logo))
        })?;
    Some(sized(name, logo, side, k))
}

fn sized(name: &'static str, logo: Logo, side: f32, k: &Kit) -> El {
    LOGOS.with(|l| {
        let mut l = l.borrow_mut();
        let image = l.image(name, logo);
        row()
            .size(side, side)
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

/// Waits for the providers' logos, so a headless picture of the page has them.
#[cfg(debug_assertions)]
pub fn wait_providers(ids: &[&str]) {
    LOGOS.with(|l| {
        let mut l = l.borrow_mut();
        for (svg, _) in ids.iter().filter_map(|id| provider_art(id)) {
            let _ = l.art(svg).wait();
        }
    });
}
