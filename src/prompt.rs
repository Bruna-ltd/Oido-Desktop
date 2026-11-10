//! What you send an agent: your words and the files that go with them. A text file travels as its
//! contents, read when it was attached; a picture as PNG or JPEG bytes, shrunk to a size models
//! take. Saved inside the chat's message, so a chat reads the same tomorrow even if the file
//! changed or went.

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

/// The longest side a picture is sent at. Anthropic shrinks anything bigger itself, so sending
/// more only costs upload time; OpenAI's high detail tiles fit inside it as well.
const MAX_SIDE: u32 = 1568;
/// The biggest file read as a picture, before shrinking.
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// The biggest text file attached whole. Bigger ones would crowd the model's context out.
const MAX_TEXT_BYTES: u64 = 256 * 1024;

/// One message of yours.
#[derive(Clone, Default, PartialEq, Debug, Serialize, Deserialize)]
#[serde(from = "Stored", into = "Stored")]
pub struct Prompt {
    pub text: String,
    pub attachments: Vec<Attachment>,
}

/// How a prompt is saved: a message with nothing attached stays a bare string, as it always was,
/// so chats saved before attachments existed read back unchanged.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Stored {
    Text(String),
    Full {
        text: String,
        attachments: Vec<Attachment>,
    },
}

impl From<Stored> for Prompt {
    fn from(s: Stored) -> Prompt {
        match s {
            Stored::Text(text) => Prompt::from(text),
            Stored::Full { text, attachments } => Prompt { text, attachments },
        }
    }
}

impl From<Prompt> for Stored {
    fn from(p: Prompt) -> Stored {
        if p.attachments.is_empty() {
            Stored::Text(p.text)
        } else {
            Stored::Full {
                text: p.text,
                attachments: p.attachments,
            }
        }
    }
}

impl From<String> for Prompt {
    fn from(text: String) -> Prompt {
        Prompt {
            text,
            attachments: Vec::new(),
        }
    }
}

impl From<&str> for Prompt {
    fn from(text: &str) -> Prompt {
        Prompt::from(text.to_owned())
    }
}

impl Prompt {
    /// Whether there is nothing to send.
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.attachments.is_empty()
    }

    /// The words the model reads: yours, then each text file, fenced with its name.
    pub fn for_model(&self) -> String {
        let mut out = self.text.clone();
        for a in &self.attachments {
            if let Attachment::File { name, text } = a {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(&format!("<file name=\"{name}\">\n{text}\n</file>"));
            }
        }
        out
    }

    /// The pictures, in the order they were attached.
    pub fn images(&self) -> impl Iterator<Item = &Image> {
        self.attachments.iter().filter_map(|a| match a {
            Attachment::Image(i) => Some(i),
            Attachment::File { .. } => None,
        })
    }

    /// Adds another message's words and files to this one: two sent while the agent was busy
    /// go as one.
    pub fn append(&mut self, other: Prompt) {
        if !other.text.trim().is_empty() {
            if !self.text.is_empty() {
                self.text.push_str("\n\n");
            }
            self.text.push_str(&other.text);
        }
        for a in other.attachments {
            add(&mut self.attachments, a);
        }
    }

    /// A rough count of the tokens this costs: four characters a token, and pictures as
    /// Anthropic counts them (width × height / 750).
    pub fn estimated_tokens(&self) -> usize {
        let words = self.for_model().chars().count().div_ceil(4);
        let pictures: usize = self
            .images()
            .map(|i| (i.width as usize * i.height as usize).div_ceil(750))
            .sum();
        words + pictures
    }
}

/// Adds `a` to `list` unless one with the same name is there already, which it then replaces:
/// attaching a file twice keeps the newer copy once.
pub fn add(list: &mut Vec<Attachment>, a: Attachment) {
    match list.iter_mut().find(|b| b.name() == a.name()) {
        Some(slot) => *slot = a,
        None => list.push(a),
    }
}

/// Something sent along with a message.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Attachment {
    /// A text file and what it held when it was attached.
    File {
        name: String,
        text: String,
    },
    Image(Image),
}

/// A picture ready to send.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Image {
    pub name: String,
    /// `image/png` or `image/jpeg`.
    pub media_type: String,
    /// The bytes, base64.
    pub data: String,
    /// The size it is sent at.
    pub width: u32,
    pub height: u32,
}

impl Attachment {
    pub fn name(&self) -> &str {
        match self {
            Attachment::File { name, .. } => name,
            Attachment::Image(i) => &i.name,
        }
    }

    pub fn is_image(&self) -> bool {
        matches!(self, Attachment::Image(_))
    }

    /// A short note of its size, for its chip: lines for a text file, pixels for a picture.
    pub fn detail(&self) -> String {
        match self {
            Attachment::File { text, .. } => match text.lines().count() {
                1 => "1 line".to_owned(),
                n => format!("{n} lines"),
            },
            Attachment::Image(i) => format!("{}×{}", i.width, i.height),
        }
    }

    /// Reads the file at `path` as a picture (PNG or JPEG) or as text, named `name` (what the
    /// model and the chip show).
    ///
    /// # Errors
    ///
    /// The file cannot be read, is too big, or is neither a picture we can send nor UTF-8 text;
    /// the message says which, in words.
    pub fn read(path: &Path, name: &str) -> Result<Attachment, String> {
        let size = std::fs::metadata(path)
            .map_err(|e| format!("{name} could not be read: {e}."))?
            .len();
        if !path.is_file() {
            return Err(format!("{name} is a folder, not a file."));
        }
        if size > MAX_IMAGE_BYTES {
            return Err(format!(
                "{name} is too big to attach ({}).",
                megabytes(size)
            ));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{name} could not be read: {e}."))?;
        if image::guess_format(&bytes).is_ok() {
            return Image::fit(name, &bytes).map(Attachment::Image);
        }
        if size > MAX_TEXT_BYTES {
            return Err(format!(
                "{name} is too long to attach whole ({}). Ask the agent to read the part you need.",
                megabytes(size)
            ));
        }
        match String::from_utf8(bytes) {
            Ok(text) if !text.contains('\0') => Ok(Attachment::File {
                name: name.to_owned(),
                text,
            }),
            _ => Err(format!(
                "{name} is not a text file or a picture, so it cannot be attached."
            )),
        }
    }
}

impl Image {
    /// A picture from its file's bytes: sent as it is when it is a PNG or JPEG no bigger than
    /// [`MAX_SIDE`], else shrunk and sent as PNG.
    ///
    /// # Errors
    ///
    /// The bytes are not a picture this app can read (PNG or JPEG).
    pub fn fit(name: &str, bytes: &[u8]) -> Result<Image, String> {
        let format = image::guess_format(bytes).ok();
        let img = image::load_from_memory(bytes).map_err(|_| {
            format!("{name} is a kind of picture that cannot be sent. Use PNG or JPEG.")
        })?;
        let (width, height) = (img.width(), img.height());
        let kept = match format {
            Some(image::ImageFormat::Png) => Some("image/png"),
            Some(image::ImageFormat::Jpeg) => Some("image/jpeg"),
            _ => None,
        };
        if let Some(media_type) = kept.filter(|_| width.max(height) <= MAX_SIDE) {
            return Ok(Image {
                name: name.to_owned(),
                media_type: media_type.to_owned(),
                data: STANDARD.encode(bytes),
                width,
                height,
            });
        }
        let shrunk = if width.max(height) > MAX_SIDE {
            img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Lanczos3)
        } else {
            img
        };
        let mut out = std::io::Cursor::new(Vec::new());
        shrunk
            .write_to(&mut out, image::ImageFormat::Png)
            .map_err(|e| format!("{name} could not be resized: {e}."))?;
        Ok(Image {
            name: name.to_owned(),
            media_type: "image/png".to_owned(),
            data: STANDARD.encode(out.into_inner()),
            width: shrunk.width(),
            height: shrunk.height(),
        })
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(w, h)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn a_prompt_with_nothing_attached_is_saved_as_the_bare_string_it_always_was() {
        let p = Prompt::from("hello");
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            serde_json::json!("hello")
        );
        assert_eq!(serde_json::from_str::<Prompt>("\"hello\"").unwrap(), p);
    }

    #[test]
    fn a_prompt_with_attachments_round_trips() {
        let p = Prompt {
            text: "look".into(),
            attachments: vec![
                Attachment::File {
                    name: "a.rs".into(),
                    text: "fn a() {}".into(),
                },
                Attachment::Image(Image::fit("p.png", &png(4, 4)).unwrap()),
            ],
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Prompt>(&json).unwrap(), p);
    }

    #[test]
    fn text_files_follow_the_words_fenced_with_their_names_and_pictures_stay_apart() {
        let p = Prompt {
            text: "explain".into(),
            attachments: vec![
                Attachment::Image(Image::fit("p.png", &png(2, 2)).unwrap()),
                Attachment::File {
                    name: "src/a.rs".into(),
                    text: "fn a() {}".into(),
                },
            ],
        };
        assert_eq!(
            p.for_model(),
            "explain\n\n<file name=\"src/a.rs\">\nfn a() {}\n</file>"
        );
        assert_eq!(p.images().count(), 1);
    }

    #[test]
    fn a_big_picture_is_shrunk_to_png_and_a_small_one_is_sent_as_it_is() {
        let big = Image::fit("big.png", &png(3000, 1000)).unwrap();
        assert_eq!((big.width, big.height), (1568, 523));
        let small_bytes = png(300, 200);
        let small = Image::fit("s.png", &small_bytes).unwrap();
        assert_eq!(small.data, STANDARD.encode(&small_bytes));
        assert_eq!(small.media_type, "image/png");
    }

    #[test]
    fn reading_tells_text_pictures_and_binaries_apart() {
        let dir = std::env::temp_dir().join(format!("oido-prompt-{}", crate::model::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.md"), "# hi\nthere").unwrap();
        std::fs::write(dir.join("pic.png"), png(10, 10)).unwrap();
        std::fs::write(dir.join("blob.bin"), [0u8, 159, 146, 150]).unwrap();
        let read = |n: &str| Attachment::read(&dir.join(n), n);
        assert!(
            matches!(read("notes.md"), Ok(Attachment::File { text, .. }) if text == "# hi\nthere")
        );
        assert!(read("pic.png").unwrap().is_image());
        assert!(read("blob.bin").unwrap_err().contains("not a text file"));
        assert!(
            read("missing.txt")
                .unwrap_err()
                .contains("could not be read")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn attaching_the_same_name_twice_keeps_the_newer_copy_once() {
        let mut list = Vec::new();
        let file = |t: &str| Attachment::File {
            name: "a.txt".into(),
            text: t.into(),
        };
        add(&mut list, file("old"));
        add(&mut list, file("new"));
        assert_eq!(list, [file("new")]);
    }

    #[test]
    fn appending_joins_words_and_files() {
        let mut p = Prompt::from("one");
        p.append(Prompt {
            text: "two".into(),
            attachments: vec![Attachment::File {
                name: "x".into(),
                text: "y".into(),
            }],
        });
        assert_eq!(p.text, "one\n\ntwo");
        assert_eq!(p.attachments.len(), 1);
    }
}
