//! TypeSafe tools: an agent hands a piece of text, and optionally images, to a System One model and
//! gets back a typed judgment with probabilities (yes/no, one of some options, a score, where
//! something is) instead of more prose. They are only offered while TypeSafe is connected (see
//! [`crate::tools::offered`]).

use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Map, Value, json};

use crate::completion::ToolSpec;

pub const YES_NO: &str = "typesafe_yes_no";
pub const CHOOSE: &str = "typesafe_choose";
pub const SCORE: &str = "typesafe_score";
pub const LOCATE: &str = "typesafe_locate";

pub const BASE: &str = "https://api.typesafe.ai/v1";
const MODEL: &str = "jev-latest";
const TIMEOUT: Duration = Duration::from_secs(60);
/// The one question every call asks, by the name its answer comes back under.
const Q: &str = "q";
/// TypeSafe refuses an image over 16 MiB, and a request over 32 MiB; a few images stay far inside.
const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_IMAGES: usize = 4;

pub fn is_tool(name: &str) -> bool {
    matches!(name, YES_NO | CHOOSE | SCORE | LOCATE)
}

pub fn specs() -> Vec<ToolSpec> {
    let content = json!({"type": "string", "description": "The text to judge, with whatever context the judgment needs. Optional when images are given."});
    let images = json!({
        "type": "array",
        "items": {"type": "string"},
        "description": "Optional images to judge, as PNG or JPEG files in the working folder (too big is shrunk to fit, too small is refused). They are called `image_1`, `image_2`, and so on: name them in the question."
    });
    let tool = |name, description, extra: Value, required: &[&str]| {
        let mut props = json!({"content": content, "images": images});
        props
            .as_object_mut()
            .into_iter()
            .for_each(|p| p.extend(extra.as_object().cloned().unwrap_or_default()));
        ToolSpec {
            name,
            description,
            schema: json!({"type": "object", "properties": props, "required": required}),
        }
    };
    vec![
        tool(
            YES_NO,
            "Ask TypeSafe whether something is true of a text or images, and get the probability of yes (0 to 1). Fast and calibrated: use it to classify or check, instead of reasoning it out. Near 0.5 means unsure.",
            json!({"question": {"type": "string", "description": "A yes/no question or a statement about the content."}}),
            &["question"],
        ),
        tool(
            CHOOSE,
            "Ask TypeSafe to pick the one option that best fits a text or images, and get the choice, its confidence and every option's probability. Use it to route or label.",
            json!({
                "question": {"type": "string", "description": "What to decide about the content."},
                "options": {
                    "type": "object",
                    "description": "The choices: each name with a short description of when it applies.",
                    "additionalProperties": {"type": "string"}
                }
            }),
            &["question", "options"],
        ),
        tool(
            SCORE,
            "Ask TypeSafe to rate a text or images on a scale you describe, and get the score (it can fall between levels), its confidence and each level's probability.",
            json!({
                "question": {"type": "string", "description": "What to rate."},
                "levels": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Up to 10 descriptions, lowest first. The first scores 0, the next 1, and so on."
                }
            }),
            &["question", "levels"],
        ),
        tool(
            LOCATE,
            "Ask TypeSafe where something is in an image, and get a box around it: top, bottom, left and right as fractions (0 to 1) of the image's height and width, from the top-left corner, with a confidence, and the same box in the image's own pixels. Screenshots work: take one into the working folder first (on Wayland, `grim shot.png` with the shell tool).",
            json!({"question": {"type": "string", "description": "What to find, and in which image, for example: the coffee mug in `image_1`. For a screenshot, a first step toward clicking on something."}}),
            &["question", "images"],
        ),
    ]
}

/// What a call is, for lists: the question it asks.
pub fn summary(args: &str) -> String {
    serde_json::from_str::<Value>(args)
        .ok()
        .and_then(|v| v.get("question").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| args.to_owned())
}

/// An image ready to send: its format and its bytes, base64, and the size the file really has
/// (what it was sent at may be smaller, see [`fit`]).
struct Picture {
    format: &'static str,
    data: String,
    size: (u32, u32),
}

impl Picture {
    fn new(format: &'static str, bytes: &[u8], size: (u32, u32)) -> Picture {
        Picture {
            format,
            data: STANDARD.encode(bytes),
            size,
        }
    }
}

/// TypeSafe takes images with both sides from 280 to 1568 pixels and refuses others rather than
/// resizing them; a screenshot is usually bigger.
const MIN_SIDE: u32 = 280;
const MAX_SIDE: u32 = 1568;

/// The bytes to send for an image: as they are when they fit, else shrunk to fit and saved as PNG.
/// One too small is refused, since stretching it would only blur it.
fn fit(given: &str, bytes: Vec<u8>) -> Result<Picture, String> {
    let unreadable = |e: image::ImageError| format!("{given} could not be read as an image: {e}.");
    let img = image::load_from_memory(&bytes).map_err(unreadable)?;
    let size = (img.width(), img.height());
    let (small, big) = (size.0.min(size.1), size.0.max(size.1));
    if big <= MAX_SIDE && small >= MIN_SIDE {
        let format = if bytes.starts_with(b"\x89PNG") {
            "png"
        } else {
            "jpg"
        };
        return Ok(Picture::new(format, &bytes, size));
    }
    let scale = f64::from(MAX_SIDE) / f64::from(big);
    if small < MIN_SIDE && (big <= MAX_SIDE || f64::from(small) * scale < f64::from(MIN_SIDE)) {
        return Err(format!(
            "{given} is {}x{} pixels, too narrow for TypeSafe, which needs both sides between {MIN_SIDE} and {MAX_SIDE}.",
            size.0, size.1
        ));
    }
    let shrunk = img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Lanczos3);
    let mut out = std::io::Cursor::new(Vec::new());
    shrunk
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| format!("{given} could not be resized: {e}."))?;
    Ok(Picture::new("png", &out.into_inner(), size))
}

/// The image paths a call named, in order.
fn image_paths(v: &Value) -> Result<Vec<String>, String> {
    let Some(list) = v.get("images") else {
        return Ok(Vec::new());
    };
    let paths: Vec<String> = list
        .as_array()
        .ok_or("`images` must be a list of file paths.")?
        .iter()
        .map(|p| p.as_str().map(str::to_owned))
        .collect::<Option<_>>()
        .ok_or("`images` must be a list of file paths.")?;
    if paths.len() > MAX_IMAGES {
        return Err(format!("At most {MAX_IMAGES} images can be sent at once."));
    }
    Ok(paths)
}

/// Reads the images from inside the agent's working folder, naming the first problem in words.
fn read_pictures(root: Option<&Path>, paths: &[String]) -> Result<Vec<Picture>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let root = root.ok_or("this agent has no working folder, so there are no image files to read. The person can choose one in the agent's settings.")?;
    paths
        .iter()
        .map(|given| {
            let path = crate::files::resolve(root, given)?;
            let size = std::fs::metadata(&path)
                .map_err(|e| format!("{given} could not be read: {e}."))?
                .len();
            if size > MAX_IMAGE_BYTES {
                return Err(format!("{given} is over 16 MiB, which is too large."));
            }
            let bytes =
                std::fs::read(&path).map_err(|e| format!("{given} could not be read: {e}."))?;
            if !bytes.starts_with(b"\x89PNG") && !bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                return Err(format!("{given} is not a PNG or JPEG image."));
            }
            fit(given, bytes)
        })
        .collect()
}

/// The `/systemone` request for a call, or what is wrong with its arguments.
fn request(name: &str, v: &Value, pictures: &[Picture]) -> Result<Value, String> {
    let text = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let question = text("question").ok_or("`question` is required and must be text.")?;
    let q = match name {
        YES_NO => json!({"type": "noul", "instructions": question}),
        CHOOSE => {
            let options = v
                .get("options")
                .and_then(Value::as_object)
                .filter(|o| !o.is_empty())
                .ok_or("`options` is required: an object of choice names and descriptions.")?;
            json!({"type": "choice", "instructions": question, "criteria": options})
        }
        SCORE => {
            let levels = v
                .get("levels")
                .and_then(Value::as_array)
                .filter(|l| (1..=10).contains(&l.len()))
                .ok_or("`levels` is required: 1 to 10 descriptions, lowest first.")?;
            json!({"type": "score", "instructions": question, "criteria": levels})
        }
        LOCATE => {
            if pictures.len() != 1 {
                return Err("`images` is required: exactly one image to look in.".into());
            }
            json!({"type": "box", "instructions": question})
        }
        other => return Err(format!("{other} is not a TypeSafe tool.")),
    };
    let content = text("content");
    let mut request = json!({"model": MODEL, "questions": {Q: q}});
    if pictures.is_empty() {
        request["state"] = json!(content.ok_or("`content` is required and must be text.")?);
        return Ok(request);
    }
    let mut state = Map::new();
    let mut assets = Map::new();
    state.extend(content.map(|c| ("text".to_owned(), json!(c))));
    for (i, p) in pictures.iter().enumerate() {
        let reference = crate::model::new_id();
        state.insert(format!("image_{}", i + 1), json!(reference));
        assets.insert(
            reference,
            json!({"type": "image", "format": p.format, "data_base64": p.data}),
        );
    }
    request["state"] = Value::Object(state);
    request["assets"] = Value::Object(assets);
    Ok(request)
}

/// The answer to give the agent, from the service's reply. For a box, `looked_in` is the size of
/// the image searched, so the answer can also say where the box is in its pixels.
fn answer(body: &str, looked_in: Option<(u32, u32)>) -> Result<String, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "the reply was not JSON.")?;
    let mut a = v
        .pointer(&format!("/answers/{Q}"))
        .cloned()
        .ok_or("the reply had no answer.")?;
    let Some(o) = a.as_object_mut() else {
        return Err("the answer was not understood.".into());
    };
    o.remove("type");
    if let Some(p) = o.remove("noul") {
        o.insert("yes_probability".into(), p);
    }
    if let (Some((w, h)), Some(b)) = (looked_in, o.get("box").cloned()) {
        let edge = |k: &str, of: u32| (b[k].as_f64().unwrap_or(0.0) * f64::from(of)).round() as i64;
        let (left, right, top, bottom) = (
            edge("left", w),
            edge("right", w),
            edge("top", h),
            edge("bottom", h),
        );
        o.insert("image_size".into(), json!({"width": w, "height": h}));
        o.insert(
            "pixels".into(),
            json!({
                "left": left, "top": top, "right": right, "bottom": bottom,
                "center_x": (left + right) / 2, "center_y": (top + bottom) / 2
            }),
        );
    }
    Ok(a.to_string())
}

/// Asks TypeSafe, in words for the agent: the answer as JSON, or `Error: …`. Images are read from
/// inside `root`, the agent's working folder. Blocks on the network and the disk, so call it from
/// a worker thread.
pub fn run(base: &str, key: &str, root: Option<&Path>, name: &str, args: &str) -> String {
    ask(base, key, root, name, args).unwrap_or_else(|why| format!("Error: {why}"))
}

fn ask(
    base: &str,
    key: &str,
    root: Option<&Path>,
    name: &str,
    args: &str,
) -> Result<String, String> {
    let v: Value = serde_json::from_str(args).map_err(|_| "the arguments were not JSON.")?;
    let pictures = read_pictures(root, &image_paths(&v)?)?;
    let body = request(name, &v, &pictures)?.to_string();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut call = agent
        .post(&format!("{base}/systemone"))
        .header("Content-Type", "application/json")
        .header("Authorization", &format!("Bearer {key}"));
    if !pictures.is_empty() {
        // Images are off unless each request that has one says so.
        call = call.header("TypeSafe-Danger-Zone-Images", "true");
    }
    let mut resp = call
        .send(body.as_str())
        .map_err(|_| "TypeSafe could not be reached.".to_owned())?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|_| "TypeSafe could not be reached.".to_owned())?;
    match status {
        200..=299 => answer(&text, (name == LOCATE).then(|| pictures[0].size)),
        401 | 403 => Err("TypeSafe did not accept the key. The person can connect it again on the Providers page.".into()),
        400 | 413 | 422 => Err(format!("TypeSafe refused the question: {}", detail(&text))),
        other => Err(format!("TypeSafe answered HTTP {other}.")),
    }
}

/// The service's own words for what was wrong, cut short.
fn detail(body: &str) -> String {
    body.chars().take(300).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testing::serve;

    fn req(name: &str, args: &str) -> Result<Value, String> {
        request(
            name,
            &serde_json::from_str(args).unwrap_or(Value::Null),
            &[],
        )
    }

    /// A folder of real images, since they are decoded: a fine PNG and JPEG, a big one, and two
    /// that cannot fit, and a text file.
    fn folder() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(crate::model::new_id());
        std::fs::create_dir_all(&dir).unwrap();
        let blank = |name: &str, w, h| image::RgbImage::new(w, h).save(dir.join(name)).unwrap();
        blank("a.png", 400, 300);
        blank("b.jpg", 300, 300);
        blank("big.png", 3000, 1500);
        blank("tiny.png", 100, 100);
        blank("thin.png", 3000, 100);
        std::fs::write(dir.join("c.txt"), "hello").unwrap();
        dir
    }

    #[test]
    fn each_tool_builds_its_own_kind_of_question() {
        let q = |name, args: &str| req(name, args).unwrap()["questions"][Q].clone();
        let yes = q(YES_NO, r#"{"content":"c","question":"Is it spam?"}"#);
        assert_eq!(yes, json!({"type": "noul", "instructions": "Is it spam?"}));
        let pick = q(
            CHOOSE,
            r#"{"content":"c","question":"Tone?","options":{"angry":"upset","calm":"neutral"}}"#,
        );
        assert_eq!(pick["type"], "choice");
        assert_eq!(pick["criteria"]["calm"], "neutral");
        let rate = q(
            SCORE,
            r#"{"content":"c","question":"Urgency?","levels":["later","now"]}"#,
        );
        assert_eq!(rate["criteria"], json!(["later", "now"]));
        let whole = req(YES_NO, r#"{"content":" hi ","question":"q"}"#).unwrap();
        assert_eq!(
            (&whole["model"], &whole["state"]),
            (&json!(MODEL), &json!("hi"))
        );
    }

    #[test]
    fn bad_arguments_are_told_to_the_agent_not_sent() {
        for (name, args) in [
            (YES_NO, "nope"),
            (YES_NO, r#"{"content":"c"}"#),
            (CHOOSE, r#"{"content":"c","question":"q","options":{}}"#),
            (SCORE, r#"{"content":"c","question":"q","levels":[]}"#),
        ] {
            assert!(req(name, args).is_err(), "{name} {args}");
        }
        assert!(run("http://127.0.0.1:1", "k", None, YES_NO, "{}").starts_with("Error: "));
    }

    #[test]
    fn answers_come_back_without_the_wire_noise() {
        let yes =
            r#"{"model":"jev-latest","answers":{"q":{"type":"noul","noul":0.98}},"usage":{}}"#;
        assert_eq!(answer(yes, None).unwrap(), r#"{"yes_probability":0.98}"#);
        let pick = r#"{"answers":{"q":{"type":"choice","choice":"calm","confidence":0.9,"probabilities":{"calm":0.9}}}}"#;
        let v: Value = serde_json::from_str(&answer(pick, None).unwrap()).unwrap();
        assert_eq!((v["choice"].as_str(), v.get("type")), (Some("calm"), None));
        assert!(answer("{}", None).is_err());
    }

    #[test]
    fn a_call_is_posted_with_the_key_and_its_failures_are_explained() {
        let (base, seen) = serve(vec![(
            200,
            r#"{"answers":{"q":{"type":"noul","noul":0.1}}}"#,
        )]);
        let args = r#"{"content":"hello","question":"Is it spam?"}"#;
        assert_eq!(
            run(&format!("{base}/v1"), "ts-key", None, YES_NO, args),
            r#"{"yes_probability":0.1}"#
        );
        let request = seen.recv().unwrap();
        assert!(request.starts_with("post /v1/systemone "), "{request}");
        assert!(
            request.contains("authorization: bearer ts-key"),
            "{request}"
        );
        assert!(request.contains(r#""type":"noul""#), "{request}");

        let (base, _) = serve(vec![(401, "{}")]);
        let told = run(&format!("{base}/v1"), "bad", None, YES_NO, args);
        assert!(told.contains("did not accept the key"), "{told}");
    }

    #[test]
    fn images_are_read_from_the_working_folder_and_sent_by_reference() {
        let dir = folder();
        let paths = |l: &[&str]| l.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
        let pics = read_pictures(Some(&dir), &paths(&["a.png", "b.jpg"])).unwrap();
        assert_eq!((pics[0].format, pics[1].format), ("png", "jpg"));

        let args = json!({"question": "Is `image_1` a cat?", "images": ["a.png"]});
        let sent = request(YES_NO, &args, &pics[..1]).unwrap();
        let reference = sent["state"]["image_1"].as_str().unwrap();
        assert_eq!(sent["assets"][reference]["type"], "image");
        assert_eq!(sent["assets"][reference]["data_base64"], pics[0].data);
        assert!(
            sent["state"].get("text").is_none(),
            "images alone need no text"
        );

        for (given, why) in [
            ("c.txt", "not a PNG or JPEG"),
            ("../escape.png", "outside the working folder"),
            ("missing.png", "could not be read"),
        ] {
            let e = read_pictures(Some(&dir), &paths(&[given])).err().unwrap();
            assert!(e.contains(why), "{given}: {e}");
        }
        assert!(
            read_pictures(None, &paths(&["a.png"]))
                .err()
                .unwrap()
                .contains("no working folder")
        );
        let big = read_pictures(Some(&dir), &paths(&["big.png"])).unwrap();
        assert_eq!(
            big[0].size,
            (3000, 1500),
            "its real size is kept for pixel positions"
        );
        let sent = image::load_from_memory(&STANDARD.decode(&big[0].data).unwrap()).unwrap();
        assert_eq!(
            (sent.width(), sent.height()),
            (1568, 784),
            "but it is sent shrunk to fit"
        );
        for given in ["tiny.png", "thin.png"] {
            let e = read_pictures(Some(&dir), &paths(&[given])).err().unwrap();
            assert!(e.contains("too narrow"), "{given}: {e}");
        }
        let many = paths(&["a.png"; 5]);
        assert!(image_paths(&json!({"images": many})).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn locating_needs_an_image_and_a_picture_call_sends_the_images_header() {
        assert!(
            req(LOCATE, r#"{"question":"the mug"}"#)
                .unwrap_err()
                .contains("images")
        );
        let dir = folder();
        let (base, seen) = serve(vec![(
            200,
            r#"{"answers":{"q":{"type":"box","box":{"top":0.2,"bottom":0.6,"left":0.1,"right":0.5},"confidence":0.9}}}"#,
        )]);
        let told = run(
            &format!("{base}/v1"),
            "ts-key",
            Some(&dir),
            LOCATE,
            r#"{"question":"the mug in `image_1`","images":["a.png"]}"#,
        );
        let v: Value = serde_json::from_str(&told).unwrap();
        assert_eq!(
            (v["box"]["left"].as_f64(), v.get("type")),
            (Some(0.1), None)
        );
        let request = seen.recv().unwrap();
        assert!(
            request.contains("typesafe-danger-zone-images: true"),
            "{request}"
        );
        assert!(request.contains(r#""type":"box""#), "{request}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
