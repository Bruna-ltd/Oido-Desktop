//! The file tools: reading, writing and editing text files for an agent. Every path is taken
//! relative to the agent's working folder and must stay inside it. A path that climbs out with `..`,
//! points elsewhere on disk, or goes through a link that leads out is refused. Results are always
//! text for the model, errors included: a failed call is something it can read and correct.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

/// A file read gives at most this many bytes of text, and a write or edit takes at most this many.
const READ_LIMIT: usize = 32 * 1024;
const WRITE_LIMIT: usize = 1024 * 1024;
/// A read gives this many lines unless it asks for fewer.
const LINES: usize = 2000;

pub const READ: &str = "read_file";
pub const WRITE: &str = "write_file";
pub const EDIT: &str = "edit_file";

pub fn is_file_tool(name: &str) -> bool {
    matches!(name, READ | WRITE | EDIT)
}

/// Runs a file tool in `root`. Always returns text.
pub fn run(root: &Path, name: &str, args: &str) -> String {
    let result = serde_json::from_str::<Value>(args)
        .map_err(|_| "The arguments were not valid JSON.".to_owned())
        .and_then(|v| match name {
            READ => read(root, &v),
            WRITE => write(root, &v),
            EDIT => edit(root, &v),
            _ => Err(format!("There is no tool called {name}.")),
        });
    result.unwrap_or_else(|why| format!("Error: {why}"))
}

/// The path a call is about, for lists.
pub fn path_of(args: &str) -> String {
    serde_json::from_str::<Value>(args)
        .ok()
        .and_then(|v| v.get("path").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| args.to_owned())
}

fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{key}` is required and must be text."))
}

fn count(v: &Value, key: &str) -> Option<usize> {
    v.get(key).and_then(Value::as_u64).map(|n| n as usize)
}

/// Where `given` is, if it is inside `root`. The file need not exist yet, but the nearest folder
/// that does must really be inside (links are followed, so one that leads out is caught).
pub(crate) fn resolve(root: &Path, given: &str) -> Result<PathBuf, String> {
    if given.trim().is_empty() {
        return Err("`path` is empty.".to_owned());
    }
    let root = root
        .canonicalize()
        .map_err(|e| format!("The working folder {} cannot be used: {e}.", root.display()))?;
    let given = Path::new(given);
    let joined = if given.is_absolute() {
        given.to_path_buf()
    } else {
        root.join(given)
    };
    let mut clean = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            other => clean.push(other),
        }
    }
    let outside = || {
        format!(
            "{} is outside the working folder {}. Files can only be reached inside it.",
            given.display(),
            root.display()
        )
    };
    let mut existing = clean.clone();
    let mut missing = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name().map(ToOwned::to_owned) else {
            return Err(outside());
        };
        missing.push(name);
        existing.pop();
    }
    let real = existing.canonicalize().map_err(|e| e.to_string())?;
    if !real.starts_with(&root) {
        return Err(outside());
    }
    Ok(missing.iter().rev().fold(real, |p, n| p.join(n)))
}

/// A path for the model to read: relative to the working folder when it is inside it.
fn shown(root: &Path, path: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    path.strip_prefix(&root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn read_text(path: &Path, root: &Path) -> Result<String, String> {
    if path.is_dir() {
        return Err(format!("{} is a folder, not a file.", shown(root, path)));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| format!("{} could not be read: {e}.", shown(root, path)))?;
    String::from_utf8(bytes).map_err(|_| format!("{} is not a text file.", shown(root, path)))
}

fn read(root: &Path, v: &Value) -> Result<String, String> {
    let path = resolve(root, text(v, "path")?)?;
    let body = read_text(&path, root)?;
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let total = lines.len();
    let start = count(v, "offset").unwrap_or(1).max(1) - 1;
    let want = count(v, "limit").unwrap_or(LINES).max(1);
    let mut out = String::new();
    let mut shown_lines = 0;
    for line in lines.iter().skip(start).take(want) {
        if out.len() + line.len() > READ_LIMIT {
            break;
        }
        out.push_str(line);
        shown_lines += 1;
    }
    // A single line longer than the limit would otherwise give nothing.
    if shown_lines == 0 && start < total {
        let (cut, _) = crate::runs::clip(lines[start], READ_LIMIT);
        out = cut;
        shown_lines = 1;
    }
    let end = start + shown_lines;
    if body.is_empty() {
        return Ok("(the file is empty)".to_owned());
    }
    if start >= total {
        return Err(format!("The file has only {total} lines."));
    }
    if start > 0 || end < total {
        out.push_str(&format!(
            "\n[showing lines {}-{end} of {total}; use `offset` and `limit` for more]",
            start + 1
        ));
    }
    Ok(out)
}

fn write(root: &Path, v: &Value) -> Result<String, String> {
    let path = resolve(root, text(v, "path")?)?;
    let content = text(v, "content")?;
    if content.len() > WRITE_LIMIT {
        return Err(format!(
            "The content is over {} KB; write it in parts.",
            WRITE_LIMIT / 1024
        ));
    }
    if path.is_dir() {
        return Err(format!("{} is a folder.", shown(root, &path)));
    }
    let existed = path.exists();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "The folder for {} could not be made: {e}.",
                shown(root, &path)
            )
        })?;
    }
    std::fs::write(&path, content)
        .map_err(|e| format!("{} could not be written: {e}.", shown(root, &path)))?;
    Ok(format!(
        "{} {} ({} bytes).",
        if existed { "Overwrote" } else { "Created" },
        shown(root, &path),
        content.len()
    ))
}

fn edit(root: &Path, v: &Value) -> Result<String, String> {
    let path = resolve(root, text(v, "path")?)?;
    let (old, new) = (text(v, "old_text")?, text(v, "new_text")?);
    let all = v
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if old.is_empty() {
        return Err("`old_text` is empty; to make a new file use write_file.".to_owned());
    }
    if !path.exists() {
        return Err(format!("{} does not exist.", shown(root, &path)));
    }
    let body = read_text(&path, root)?;
    let found = body.matches(old).count();
    match (found, all) {
        (0, _) => Err("`old_text` was not found. It must match the file exactly, including spaces and line breaks; read the file again.".to_owned()),
        (n, false) if n > 1 => Err(format!(
            "`old_text` appears {n} times. Include more around it so it is unique, or set `replace_all`."
        )),
        _ => {
            let changed = if all { body.replace(old, new) } else { body.replacen(old, new, 1) };
            if changed.len() > WRITE_LIMIT {
                return Err("The result would be over 1024 KB.".to_owned());
            }
            std::fs::write(&path, changed)
                .map_err(|e| format!("{} could not be written: {e}.", shown(root, &path)))?;
            Ok(format!(
                "Replaced {found} occurrence{} in {}.",
                if found == 1 { "" } else { "s" },
                shown(root, &path)
            ))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("oido-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.txt"), "one\ntwo\nthree\n").unwrap();
        root.canonicalize().unwrap()
    }

    fn call(root: &Path, name: &str, args: Value) -> String {
        run(root, name, &args.to_string())
    }

    #[test]
    fn paths_stay_inside_the_working_folder() {
        let r = root();
        assert!(resolve(&r, "src/a.txt").is_ok());
        assert!(
            resolve(&r, "new/deep/file.txt").is_ok(),
            "a new path inside is fine"
        );
        assert!(
            resolve(&r, &r.join("src/a.txt").to_string_lossy()).is_ok(),
            "absolute but inside"
        );
        assert!(
            resolve(&r, "src/../a.txt").is_ok(),
            "`..` that stays inside"
        );
        for bad in ["../x", "src/../../x", "/etc/passwd", "", "  "] {
            assert!(resolve(&r, bad).is_err(), "{bad}");
        }
        std::fs::remove_dir_all(r).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_leads_out_is_refused_for_reading_and_writing() {
        let r = root();
        let outside = std::env::temp_dir().join(format!("oido-outside-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(&outside, r.join("link")).unwrap();
        let got = call(&r, READ, json!({"path": "link/secret.txt"}));
        assert!(
            got.starts_with("Error:") && got.contains("outside"),
            "{got}"
        );
        let got = call(&r, WRITE, json!({"path": "link/new.txt", "content": "x"}));
        assert!(got.contains("outside"), "{got}");
        assert!(!outside.join("new.txt").exists());
        std::fs::remove_dir_all(r).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn reading_gives_the_text_whole_or_by_lines_and_explains_what_it_cannot() {
        let r = root();
        assert_eq!(
            call(&r, READ, json!({"path": "src/a.txt"})),
            "one\ntwo\nthree\n"
        );
        let part = call(
            &r,
            READ,
            json!({"path": "src/a.txt", "offset": 2, "limit": 1}),
        );
        assert!(
            part.starts_with("two\n") && part.contains("lines 2-2 of 3"),
            "{part}"
        );
        assert!(call(&r, READ, json!({"path": "src/a.txt", "offset": 9})).contains("only 3 lines"));
        assert!(call(&r, READ, json!({"path": "src"})).contains("is a folder"));
        assert!(call(&r, READ, json!({"path": "nope.txt"})).starts_with("Error:"));
        std::fs::write(r.join("bin"), [0xff, 0xfe, 0x00]).unwrap();
        assert!(call(&r, READ, json!({"path": "bin"})).contains("not a text file"));
        std::fs::write(r.join("empty"), "").unwrap();
        assert_eq!(
            call(&r, READ, json!({"path": "empty"})),
            "(the file is empty)"
        );
        std::fs::write(r.join("big"), "x".repeat(100_000) + "\n").unwrap();
        let big = call(&r, READ, json!({"path": "big"}));
        assert!(big.len() < READ_LIMIT + 200, "{}", big.len());
        assert!(call(&r, READ, json!({})).contains("`path` is required"));
        std::fs::remove_dir_all(r).unwrap();
    }

    #[test]
    fn writing_makes_folders_and_overwrites_and_says_which() {
        let r = root();
        let made = call(
            &r,
            WRITE,
            json!({"path": "out/dir/f.txt", "content": "hello"}),
        );
        assert_eq!(made, "Created out/dir/f.txt (5 bytes).");
        assert_eq!(
            std::fs::read_to_string(r.join("out/dir/f.txt")).unwrap(),
            "hello"
        );
        let again = call(
            &r,
            WRITE,
            json!({"path": "out/dir/f.txt", "content": "bye"}),
        );
        assert!(again.starts_with("Overwrote"), "{again}");
        assert_eq!(
            std::fs::read_to_string(r.join("out/dir/f.txt")).unwrap(),
            "bye"
        );
        assert!(call(&r, WRITE, json!({"path": "src", "content": "x"})).contains("is a folder"));
        assert!(call(&r, WRITE, json!({"path": "../x", "content": "x"})).contains("outside"));
        assert!(call(&r, WRITE, json!({"path": "f"})).contains("`content` is required"));
        std::fs::remove_dir_all(r).unwrap();
    }

    #[test]
    fn editing_replaces_exact_text_once_and_refuses_what_is_unclear() {
        let r = root();
        let got = call(
            &r,
            EDIT,
            json!({"path": "src/a.txt", "old_text": "two", "new_text": "2"}),
        );
        assert_eq!(got, "Replaced 1 occurrence in src/a.txt.");
        assert_eq!(
            std::fs::read_to_string(r.join("src/a.txt")).unwrap(),
            "one\n2\nthree\n"
        );

        std::fs::write(r.join("dup"), "a a a").unwrap();
        let dup = call(
            &r,
            EDIT,
            json!({"path": "dup", "old_text": "a", "new_text": "b"}),
        );
        assert!(dup.contains("appears 3 times"), "{dup}");
        assert_eq!(
            std::fs::read_to_string(r.join("dup")).unwrap(),
            "a a a",
            "left alone"
        );
        let all = call(
            &r,
            EDIT,
            json!({"path": "dup", "old_text": "a", "new_text": "b", "replace_all": true}),
        );
        assert_eq!(all, "Replaced 3 occurrences in dup.");
        assert_eq!(std::fs::read_to_string(r.join("dup")).unwrap(), "b b b");

        let missing = call(
            &r,
            EDIT,
            json!({"path": "dup", "old_text": "zzz", "new_text": "y"}),
        );
        assert!(missing.contains("was not found"), "{missing}");
        assert!(
            call(
                &r,
                EDIT,
                json!({"path": "dup", "old_text": "", "new_text": "y"})
            )
            .contains("empty")
        );
        assert!(
            call(
                &r,
                EDIT,
                json!({"path": "nope", "old_text": "a", "new_text": "b"})
            )
            .contains("does not exist")
        );
        assert!(
            call(
                &r,
                EDIT,
                json!({"path": "../x", "old_text": "a", "new_text": "b"})
            )
            .contains("outside")
        );
        std::fs::remove_dir_all(r).unwrap();
    }

    #[test]
    fn bad_arguments_and_unknown_names_are_errors_in_words() {
        let r = root();
        assert!(run(&r, READ, "not json").contains("not valid JSON"));
        assert!(run(&r, "other", "{}").contains("no tool called other"));
        assert_eq!(path_of(r#"{"path":"a/b"}"#), "a/b");
        std::fs::remove_dir_all(r).unwrap();
    }
}
