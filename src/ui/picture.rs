//! Pictures as the screen shows them: those attached to a message, decoded once from the bytes it
//! carries, and those a reply shows (`![alt](src)`). Kept for as long as the app runs, so the
//! composer and the chat draw them without decoding again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use percent_encoding::percent_decode_str;
use rune::{ImageCache, Pixels};
use sha2::{Digest, Sha256};

use crate::prompt::Image;

thread_local! {
    static PICTURES: RefCell<Pictures> = RefCell::new(Pictures::default());
}

#[derive(Default)]
struct Pictures {
    cache: Option<ImageCache>,
    /// By [`key`]; the cache frees pixels nothing holds, so the handles are kept here.
    loaded: HashMap<String, rune::Image>,
    /// Pictures replies show, by a hash of their folder and source; `None` for a source that
    /// will never show (a web address, a file outside the folder).
    shown: HashMap<[u8; 32], Option<rune::Image>>,
    /// Web pictures you asked to load, by address. Kept until the app quits.
    web: HashMap<String, Fetch>,
    /// What a web picture shows in place of itself until it loads (see [`Web`]).
    placeholder: Option<rune::Image>,
}

/// A web picture you asked to load.
enum Fetch {
    Loading,
    Loaded(rune::Image),
    Failed(String),
}

impl Pictures {
    fn cache(&mut self) -> &ImageCache {
        self.cache.get_or_insert_with(|| {
            // A picture that finishes decoding asks the window for another frame.
            ImageCache::new(Arc::new(|| {
                if let Some(app) = crate::app::handle() {
                    app.update(|_, _| {});
                }
            }))
        })
    }

    fn placeholder(&mut self) -> rune::Image {
        if let Some(image) = &self.placeholder {
            return image.clone();
        }
        let image = self.cache().rgba(1, 1, vec![0u8; 4]);
        self.placeholder = Some(image.clone());
        image
    }
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
            let image = p.cache().load(bytes);
            p.loaded.insert(k.clone(), image);
        }
        p.loaded.get(&k).cloned()
    })
}

/// The picture's pixels, drawn once decoded; `None` when its bytes are not base64.
pub fn pixels(img: &Image) -> Option<Pixels> {
    handle(img).map(|h| h.pixels().clone())
}

/// The widest a reply's picture draws, in points: an SVG rasterizes at this size.
const REPLY_WIDTH: f32 = 760.0;
/// Device pixels per point an SVG rasterizes at, so it stays crisp on a 2× display.
const SVG_SCALE: f32 = 2.0;

/// The picture for an image a reply shows (`![alt](src)`): a `data:` URL, or a file inside
/// `folder`, by a path relative to it, absolute, or as a `file://` URL. Web addresses are not
/// fetched: the reply's text is not ours, and fetching one would tell its server whatever the
/// address was made to carry.
pub fn markdown(src: &str, folder: &Path) -> Option<rune::Image> {
    if is_web(src) {
        // Shown once you let it load; until then a stand-in, which the chat draws as an offer
        // to load it (see [`web`]).
        return PICTURES.with(|p| {
            let mut p = p.borrow_mut();
            match p.web.get(src) {
                Some(Fetch::Loaded(image)) if image.is_ready() && image.error().is_none() => {
                    Some(image.clone())
                }
                _ => Some(p.placeholder()),
            }
        });
    }
    let mut hash = Sha256::new();
    hash.update(folder.as_os_str().as_encoded_bytes());
    hash.update([0]);
    hash.update(src.as_bytes());
    let id: [u8; 32] = hash.finalize().into();
    PICTURES.with(|p| {
        let mut p = p.borrow_mut();
        if let Some(known) = p.shown.get(&id) {
            return known.clone();
        }
        let image = match Source::of(src, folder) {
            Source::Data { mime, bytes } => {
                Some(p.cache().load_mime(bytes, &mime, REPLY_WIDTH, SVG_SCALE))
            }
            Source::File(path) => Some(p.cache().file(path, REPLY_WIDTH, SVG_SCALE)),
            // Not there yet: the agent may be about to write it, so it is looked for again.
            Source::Missing => return None,
            Source::Refused => None,
        };
        p.shown.insert(id, image.clone());
        image
    })
}

/// Whether a reply's picture is on the web, and so loads only when you ask.
pub fn is_web(src: &str) -> bool {
    let scheme = src.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
    matches!(scheme.as_deref(), Some("http" | "https"))
}

/// How far a web picture in a reply has loaded.
#[derive(Clone, Debug, PartialEq)]
pub enum Web {
    /// You have not asked for it.
    NotAsked,
    Loading,
    Shown,
    /// It could not be loaded, in words.
    Failed(String),
}

/// How far the web picture at `url` has loaded.
pub fn web(url: &str) -> Web {
    PICTURES.with(|p| match p.borrow().web.get(url) {
        None => Web::NotAsked,
        Some(Fetch::Loading) => Web::Loading,
        Some(Fetch::Failed(why)) => Web::Failed(why.clone()),
        Some(Fetch::Loaded(image)) => match (image.is_ready(), image.error()) {
            (false, _) => Web::Loading,
            (true, None) => Web::Shown,
            (true, Some(_)) => Web::Failed("it is not a picture this app can show".into()),
        },
    })
}

/// Loads the web picture at `url`, which you asked for, off the UI thread.
pub fn load(app: &mut crate::app::App, url: &str) {
    if !is_web(url) {
        return;
    }
    PICTURES.with(|p| p.borrow_mut().web.insert(url.to_owned(), Fetch::Loading));
    let address = url.to_owned();
    let url = url.to_owned();
    app.spawn(
        move || fetch(&address),
        move |_, got| {
            PICTURES.with(|p| {
                let mut p = p.borrow_mut();
                let state = match got {
                    Ok((mime, bytes)) => {
                        Fetch::Loaded(p.cache().load_mime(bytes, &mime, REPLY_WIDTH, SVG_SCALE))
                    }
                    Err(why) => Fetch::Failed(why),
                };
                p.web.insert(url, state);
            });
        },
    );
}

/// The most a web picture may weigh.
const WEB_LIMIT: u64 = 10 * 1024 * 1024;
/// How long a web picture may take to arrive.
const WEB_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The picture at `url`, as its type and bytes. Anything that is not an image is refused.
fn fetch(url: &str) -> Result<(String, Vec<u8>), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(WEB_TIMEOUT))
        .user_agent(format!("oido-desktop/{}", crate::update::VERSION))
        .build()
        .into();
    let resp = agent
        .get(url)
        .header("Accept", "image/*")
        .call()
        .map_err(|e| format!("it could not be fetched: {e}"))?;
    let mime = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !mime.starts_with("image/") {
        return Err(format!(
            "the address gave {}, not a picture",
            if mime.is_empty() {
                "something unnamed"
            } else {
                &mime
            }
        ));
    }
    let bytes = resp
        .into_body()
        .with_config()
        .limit(WEB_LIMIT)
        .read_to_vec()
        .map_err(|e| format!("it could not be read: {e}"))?;
    Ok((mime, bytes))
}

/// Where a reply's picture comes from.
#[derive(Debug, PartialEq)]
enum Source {
    Data {
        mime: String,
        bytes: Vec<u8>,
    },
    File(std::path::PathBuf),
    /// A file path that does not lead to a file, yet.
    Missing,
    /// A web address, a file outside the folder, or a `data:` URL that does not decode.
    Refused,
}

impl Source {
    fn of(src: &str, folder: &Path) -> Source {
        if let Some(rest) = src.strip_prefix("data:") {
            return data_url(rest).map_or(Source::Refused, |(mime, bytes)| Source::Data {
                mime,
                bytes,
            });
        }
        let path = match src.strip_prefix("file://") {
            Some(p) => percent_decode_str(p).decode_utf8_lossy().into_owned(),
            None if src.contains("://") => return Source::Refused,
            None => src.to_owned(),
        };
        let (Ok(file), Ok(folder)) = (folder.join(path).canonicalize(), folder.canonicalize())
        else {
            return Source::Missing;
        };
        if file.starts_with(&folder) && file.is_file() {
            Source::File(file)
        } else {
            Source::Refused
        }
    }
}

/// A `data:` URL after its scheme, as its type and bytes: `image/png;base64,iVBO…`, or
/// percent-encoded text such as `image/svg+xml,%3Csvg…`. Only images.
fn data_url(rest: &str) -> Option<(String, Vec<u8>)> {
    let (head, body) = rest.split_once(',')?;
    let (mime, base64) = match head.strip_suffix(";base64") {
        Some(m) => (m, true),
        None => (head, false),
    };
    let mime = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !mime.starts_with("image/") {
        return None;
    }
    let bytes = if base64 {
        STANDARD.decode(body.trim()).ok()?
    } else {
        percent_decode_str(body).collect()
    };
    Some((mime, bytes))
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
#[allow(clippy::unwrap_used)]
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

    #[test]
    fn a_data_url_gives_its_image_bytes() {
        assert_eq!(
            data_url("image/png;base64,aGk="),
            Some(("image/png".into(), b"hi".to_vec()))
        );
        assert_eq!(
            data_url("image/svg+xml;charset=utf-8,%3Csvg%3E"),
            Some(("image/svg+xml".into(), b"<svg>".to_vec()))
        );
        assert_eq!(data_url("text/html,<script>"), None);
        assert_eq!(data_url("image/png;base64,***"), None);
    }

    #[test]
    fn web_pictures_wait_to_be_asked_for_and_only_pictures_load() {
        assert!(is_web("https://example.com/a.png") && is_web("HTTP://x/y"));
        assert!(!is_web("plot.png") && !is_web("file:///a.png") && !is_web("data:image/png,"));

        let (base, _seen) = crate::testing::serve_as(vec![
            (
                200,
                "image/svg+xml",
                "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'/>",
            ),
            (200, "text/html", "<html></html>"),
        ]);
        let url = format!("{base}/pic.svg");
        let image = markdown(&url, Path::new(".")).unwrap();
        let _ = image.wait();
        assert_eq!(image.size(), Some([1, 1]), "a stand-in until you ask");
        assert_eq!(web(&url), Web::NotAsked);

        let mut app = crate::testing::samples();
        load(&mut app, &url);
        // Decoded on the cache's worker.
        for _ in 0..500 {
            if web(&url) != Web::Loading {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(web(&url), Web::Shown);
        assert_ne!(markdown(&url, Path::new(".")).unwrap().size(), Some([1, 1]));

        let page = format!("{base}/page");
        load(&mut app, &page);
        assert_eq!(
            web(&page),
            Web::Failed("the address gave text/html, not a picture".into())
        );
    }

    #[test]
    fn a_data_url_picture_decodes() {
        let src = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='12'%3E%3Crect width='24' height='12' fill='red'/%3E%3C/svg%3E";
        let image = markdown(src, Path::new(".")).unwrap();
        let _ = image.wait();
        assert_eq!(image.error().map(|e| e.to_string()), None);
        assert!(image.size().is_some());
    }

    #[test]
    fn a_reply_shows_files_in_its_folder_and_never_fetches() {
        let root = std::env::temp_dir().join(format!("oido-pic-{}", uuid::Uuid::new_v4()));
        let folder = root.join("work");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("plot.png"), b"x").unwrap();
        std::fs::write(root.join("secret.png"), b"x").unwrap();
        let inside = folder.canonicalize().unwrap().join("plot.png");

        assert_eq!(
            Source::of("plot.png", &folder),
            Source::File(inside.clone())
        );
        let absolute = inside.to_string_lossy().into_owned();
        assert_eq!(Source::of(&absolute, &folder), Source::File(inside.clone()));
        assert_eq!(
            Source::of(&format!("file://{absolute}"), &folder),
            Source::File(inside)
        );
        assert_eq!(Source::of("../secret.png", &folder), Source::Refused);
        assert_eq!(Source::of("later.png", &folder), Source::Missing);
        assert_eq!(
            Source::of("https://example.com/a.png?q=secret", &folder),
            Source::Refused
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
