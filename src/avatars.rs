//! What an agent's face can be besides its initial on a colour: one of the pictures that come with
//! the app, or one the person gave it. A picture is always a square PNG [`SIDE`] points across,
//! so the screen draws every one the same way and an upload costs the database a few kilobytes.
//!
//! The presets are `assets/avatars/*.png`, cut from one sheet of twelve. Each has an id that is
//! saved with the agent, so the files can be reordered or renamed without changing anyone's face.

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use sha2::{Digest, Sha256};

/// How many pixels a picture is on each side.
pub const SIDE: u32 = 192;
/// The biggest file read for an upload, as for a picture attached to a message.
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// A picture that comes with the app.
#[derive(Debug)]
pub struct Preset {
    pub id: &'static str,
    /// What it shows, for the screen reader and the hover hint.
    pub label: &'static str,
    pub png: &'static [u8],
}

macro_rules! preset {
    ($id:literal, $label:literal) => {
        Preset {
            id: $id,
            label: $label,
            png: include_bytes!(concat!("../assets/avatars/", $id, ".png")),
        }
    };
}

/// In the order the picker lists them.
pub const PRESETS: [Preset; 12] = [
    preset!("support", "Headset"),
    preset!("reader", "Glasses and a book"),
    preset!("analyst", "Rising chart"),
    preset!("engineer", "Hard hat and a gear"),
    preset!("developer", "Code"),
    preset!("researcher", "Magnifying glass"),
    preset!("security", "Hat and a fingerprint"),
    preset!("chef", "Chef's hat"),
    preset!("health", "Nurse's cap and a stethoscope"),
    preset!("planner", "Calendar and a clock"),
    preset!("designer", "Paintbrush and a picture"),
    preset!("marketer", "Megaphone"),
];

/// The preset with this id; `None` for one this version does not have.
pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// A picture the person gave: already cropped and shrunk to [`SIDE`], as PNG.
#[derive(Debug, PartialEq)]
pub struct Upload {
    /// A short fingerprint of the bytes, so the screen can tell pictures apart without comparing
    /// them on every frame. Compared first.
    pub key: String,
    pub png: Vec<u8>,
}

impl Upload {
    /// Wraps PNG bytes this app made (or saved before).
    pub fn new(png: Vec<u8>) -> Upload {
        let digest = Sha256::digest(&png);
        let key = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
        Upload { key, png }
    }
}

/// A face that is a picture. An agent without one shows its initial on a colour.
#[derive(Clone, PartialEq, Debug)]
pub enum Avatar {
    /// By [`Preset::id`].
    Preset(&'static str),
    Upload(Arc<Upload>),
}

impl Avatar {
    /// The two database columns it is saved in: a preset's id, or a picture's bytes. At most one.
    pub fn columns(avatar: Option<&Avatar>) -> (Option<&str>, Option<&[u8]>) {
        match avatar {
            None => (None, None),
            Some(Avatar::Preset(id)) => (Some(id), None),
            Some(Avatar::Upload(u)) => (None, Some(&u.png)),
        }
    }

    /// The face the two columns hold. A preset this version does not know gives none, so the
    /// agent shows its initial instead of failing to load.
    pub fn from_columns(preset_id: Option<String>, image: Option<Vec<u8>>) -> Option<Avatar> {
        match (preset_id, image) {
            (_, Some(png)) => Some(Avatar::Upload(Arc::new(Upload::new(png)))),
            (Some(id), None) => preset(&id).map(|p| Avatar::Preset(p.id)),
            (None, None) => None,
        }
    }

    /// A picture from a file on disk: the middle square of it, shrunk to [`SIDE`]. Meant to run off
    /// the UI thread.
    ///
    /// # Errors
    ///
    /// The file cannot be read, is too big, or is not a PNG or JPEG; the message says which.
    pub fn from_file(path: &Path) -> Result<Avatar, String> {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let size = std::fs::metadata(path)
            .map_err(|e| format!("{name} could not be read: {e}."))?
            .len();
        if !path.is_file() {
            return Err(format!("{name} is a folder, not a picture."));
        }
        if size > MAX_FILE_BYTES {
            return Err(format!("{name} is too big to use as a picture."));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{name} could not be read: {e}."))?;
        Avatar::from_bytes(&name, &bytes)
    }

    /// [`Avatar::from_file`], from the bytes.
    ///
    /// # Errors
    ///
    /// The bytes are not a PNG or JPEG, or could not be shrunk.
    pub fn from_bytes(name: &str, bytes: &[u8]) -> Result<Avatar, String> {
        let img = image::load_from_memory(bytes)
            .map_err(|_| format!("{name} is not a picture this app can read. Use PNG or JPEG."))?;
        let square = img.resize_to_fill(SIDE, SIDE, image::imageops::FilterType::Lanczos3);
        let mut out = Cursor::new(Vec::new());
        square
            .write_to(&mut out, image::ImageFormat::Png)
            .map_err(|e| format!("{name} could not be resized: {e}."))?;
        Ok(Avatar::Upload(Arc::new(Upload::new(out.into_inner()))))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::RgbImage::new(w, h)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn every_preset_is_a_square_picture_with_an_id_of_its_own() {
        let mut seen = HashSet::new();
        for p in &PRESETS {
            assert!(seen.insert(p.id), "{} twice", p.id);
            let img = image::load_from_memory(p.png).unwrap();
            assert_eq!((img.width(), img.height()), (SIDE, SIDE), "{}", p.id);
            assert_eq!(preset(p.id).unwrap().id, p.id);
        }
        assert!(preset("nobody").is_none());
    }

    #[test]
    fn an_upload_is_cropped_to_its_middle_square_and_shrunk() {
        for (w, h) in [(600, 300), (300, 600), (50, 50)] {
            let Avatar::Upload(u) = Avatar::from_bytes("me.png", &png(w, h)).unwrap() else {
                panic!("an upload is an upload");
            };
            let img = image::load_from_memory(&u.png).unwrap();
            assert_eq!((img.width(), img.height()), (SIDE, SIDE), "{w}x{h}");
        }
    }

    #[test]
    fn what_is_not_a_picture_is_refused_in_words() {
        let why = Avatar::from_bytes("notes.txt", b"hello").unwrap_err();
        assert!(
            why.contains("notes.txt") && why.contains("PNG or JPEG"),
            "{why}"
        );
        let why = Avatar::from_file(Path::new("/nonexistent/me.png")).unwrap_err();
        assert!(why.contains("me.png"), "{why}");
        let why = Avatar::from_file(Path::new("/tmp")).unwrap_err();
        assert!(why.contains("folder"), "{why}");
    }

    #[test]
    fn a_face_goes_into_its_columns_and_comes_back() {
        let upload = Avatar::from_bytes("me.png", &png(40, 40)).unwrap();
        for face in [None, Some(Avatar::Preset("chef")), Some(upload)] {
            let (preset_id, image) = Avatar::columns(face.as_ref());
            assert!(preset_id.is_none() || image.is_none(), "at most one");
            let back =
                Avatar::from_columns(preset_id.map(str::to_owned), image.map(<[u8]>::to_vec));
            assert_eq!(back, face);
        }
        // A preset from a newer version is no face, not an error.
        assert_eq!(Avatar::from_columns(Some("gone".into()), None), None);
    }
}
