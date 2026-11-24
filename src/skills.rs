//! Skills: folders in `~/.agents/skills`, each with a `SKILL.md` that says what the skill is and
//! how to do it. An agent can be given some. The model is told each one's name and description
//! with the [`crate::tools::SKILL`] tool, and reads a skill's instructions with it when a task
//! needs them, so a skill costs only its description on every message. `OIDO_SKILLS` points
//! somewhere else.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::completion::ToolSpec;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
}

/// Where skills live.
pub fn dir() -> Option<PathBuf> {
    match std::env::var_os("OIDO_SKILLS") {
        Some(p) => Some(PathBuf::from(p)),
        None => dirs::home_dir().map(|h| h.join(".agents").join("skills")),
    }
}

/// Every folder a model may read skill files from, beside its working folder: the skills folder
/// and Claude's (`~/.claude/skills`), so a skill that points at its own scripts and references
/// can be followed. Reading only; nothing there can be changed.
pub fn read_roots() -> Vec<PathBuf> {
    let claude = dirs::home_dir().map(|h| h.join(".claude").join("skills"));
    dir().into_iter().chain(claude).collect()
}

/// Every skill in `dir` (a folder holding a `SKILL.md`), by name. A missing folder has none.
pub fn discover(dir: &Path) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut skills: Vec<Skill> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let folder = e.file_name().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(e.path().join("SKILL.md")).ok()?;
            let (front, _) = split(&text);
            Some(Skill {
                name: field(front, "name").unwrap_or(folder),
                description: field(front, "description").unwrap_or_default(),
            })
        })
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// The `SKILL.md` of the skill called `name`. `None` if it is gone.
fn read(dir: &Path, name: &str) -> Option<String> {
    // A name is a folder name; never a path out of the skills folder.
    if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return None;
    }
    std::fs::read_to_string(dir.join(name).join("SKILL.md")).ok()
}

/// A skill's instructions, without its front matter. `None` if it is gone.
pub fn body(dir: &Path, name: &str) -> Option<String> {
    Some(split(&read(dir, name)?).1.trim().to_owned())
}

/// Each chosen skill still in the folder, with its description, in the order chosen.
pub fn chosen(dir: Option<&Path>, names: &[String]) -> Vec<Skill> {
    names
        .iter()
        .filter_map(|name| {
            let text = read(dir?, name)?;
            Some(Skill {
                name: name.clone(),
                description: field(split(&text).0, "description").unwrap_or_default(),
            })
        })
        .collect()
}

/// The tool an agent with skills reads them with: each skill's name and description, and how to
/// use them.
pub fn spec(chosen: &[Skill]) -> ToolSpec {
    let listed: Vec<String> = chosen
        .iter()
        .map(|s| match s.description.trim() {
            "" => format!("- {}", s.name),
            d => format!("- {}: {d}", s.name),
        })
        .collect();
    ToolSpec {
        name: crate::tools::SKILL.into(),
        description: "Read the instructions of one of your skills. Each skill covers a kind of task: before doing a task one covers, read it and follow it. Read a skill once in a chat; if what it said was removed to save room, read it again.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "enum": chosen.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
                    "description": format!("Which skill:\n{}", listed.join("\n")),
                }
            },
            "required": ["name"]
        }),
    }
}

/// Answers a call of the skill tool: the instructions of the skill it names, if the agent has it
/// (`chosen`) and it is still in `dir`.
pub fn run(dir: Option<&Path>, chosen: &[String], args: &str) -> String {
    let v: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if !chosen.iter().any(|c| c == name) {
        return format!(
            "Error: you have no skill called \"{name}\". Yours are: {}.",
            chosen.join(", ")
        );
    }
    match dir.and_then(|d| body(d, name)) {
        Some(text) => text,
        None => format!("Error: the skill {name} is no longer installed."),
    }
}

/// Roughly how many tokens the chosen skills add to every message: their names and descriptions,
/// at about four bytes a token. Skills no longer in the folder add nothing.
pub fn estimate_tokens(all: &[Skill], chosen: &[String]) -> usize {
    all.iter()
        .filter(|s| chosen.contains(&s.name))
        .map(|s| (s.name.len() + s.description.len() + 4) / 4)
        .sum()
}

/// "about 800 tokens", "about 12.4k tokens".
pub fn tokens_label(n: usize) -> String {
    if n >= 1000 {
        format!("about {:.1}k tokens", n as f32 / 1000.0)
    } else {
        format!("about {n} tokens")
    }
}

/// Whether text has Chinese or Japanese characters. The text engine has no word-break data for
/// them and logs an error each time it lays them out, so the picker leaves such a description
/// out of its one-line rows (search still reads it).
pub fn needs_complex_text(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c, '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}'))
}

/// A file's `---` front matter and the rest.
fn split(text: &str) -> (&str, &str) {
    let Some(rest) = text.strip_prefix("---\n") else {
        return ("", text);
    };
    match rest.find("\n---") {
        Some(end) => (&rest[..end], rest[end + 4..].trim_start_matches('\n')),
        None => ("", text),
    }
}

/// A top-level `key: value` of the front matter, with the indented lines that continue it: a
/// quoted or plain value folded onto one line, or a `|` / `>` block.
fn field(front: &str, key: &str) -> Option<String> {
    let mut lines = front.lines();
    let first = lines
        .by_ref()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))?
        .trim();
    let more: Vec<&str> = lines
        .take_while(|l| l.trim().is_empty() || l.starts_with([' ', '\t']))
        .map(str::trim)
        .collect();
    let value = match first.chars().next() {
        Some(block @ ('|' | '>')) => {
            let joint = if block == '|' { "\n" } else { " " };
            let mut text = more.join(joint);
            text.truncate(text.trim_end().len());
            text
        }
        _ => {
            let words: Vec<&str> = std::iter::once(first)
                .chain(more)
                .filter(|l| !l.is_empty())
                .collect();
            unquote(&words.join(" "))
        }
    };
    (!value.is_empty()).then_some(value)
}

/// A YAML scalar without its quotes: `''` in single quotes and `\"` in double ones are a quote.
fn unquote(v: &str) -> String {
    if let Some(inner) = v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        inner.replace("''", "'")
    } else if let Some(inner) = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        inner.replace("\\\"", "\"")
    } else {
        v.to_owned()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn tree() -> PathBuf {
        let root = std::env::temp_dir().join(format!("oido-skills-{}", uuid::Uuid::new_v4()));
        for (folder, text) in [
            (
                "beta",
                "---\nname: beta\ndescription: Does B.\n---\n\n# B\nDo b.",
            ),
            ("alpha", "---\ndescription: \"Does A.\"\n---\nDo a."),
            ("plain", "No front matter."),
        ] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
            std::fs::write(root.join(folder).join("SKILL.md"), text).unwrap();
        }
        std::fs::create_dir_all(root.join("empty")).unwrap();
        root
    }

    #[test]
    fn skills_are_found_by_their_folders_sorted_with_name_and_description() {
        let root = tree();
        let found: Vec<_> = discover(&root)
            .into_iter()
            .map(|s| (s.name, s.description))
            .collect();
        assert_eq!(
            found,
            vec![
                ("alpha".into(), "Does A.".into()),
                ("beta".into(), "Does B.".into()),
                ("plain".into(), String::new()),
            ],
            "a folder with no SKILL.md is not a skill"
        );
        assert!(discover(&root.join("nowhere")).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_description_over_several_lines_is_read_whole() {
        let front = |value: &str| format!("name: x\ndescription:{value}\nmetadata:\n  a: b");
        let read = |value: &str| field(&front(value), "description");
        assert_eq!(
            read(" 'Spend audit: idle\n  spend, the user''s bill.'"),
            Some("Spend audit: idle spend, the user's bill.".into())
        );
        assert_eq!(
            read(" >-\n  Folded onto\n  one line.\n"),
            Some("Folded onto one line.".into())
        );
        assert_eq!(
            read(" |\n  Kept on\n  two lines.\n"),
            Some("Kept on\ntwo lines.".into())
        );
        assert_eq!(
            read(" Plain that\n  goes on."),
            Some("Plain that goes on.".into())
        );
        assert_eq!(read(" \"Say \\\"hi\\\".\""), Some("Say \"hi\".".into()));
        assert_eq!(read(""), None);
        assert_eq!(field(&front(" x"), "name"), Some("x".into()));
    }

    #[test]
    fn what_chosen_skills_add_is_estimated_from_their_descriptions() {
        let skill = |n: &str, d: &str| Skill {
            name: n.into(),
            description: d.into(),
        };
        let all = [
            skill("a", &"x".repeat(396)),
            skill("b", "bb"),
            skill("c", ""),
        ];
        let chosen = ["a".to_owned(), "c".to_owned(), "gone".to_owned()];
        assert_eq!(estimate_tokens(&all, &chosen), 100 + 1);
        assert_eq!(tokens_label(1010), "about 1.0k tokens");
        assert_eq!(tokens_label(80), "about 80 tokens");
        assert_eq!(estimate_tokens(&all, &[]), 0);
    }

    #[test]
    fn chinese_and_japanese_text_is_told_from_the_rest() {
        assert!(needs_complex_text("Use for 中文 text"));
        assert!(needs_complex_text("ひらがな"));
        assert!(!needs_complex_text("Plain ASCII, accents é and emoji 🎉"));
    }

    #[test]
    fn chosen_skills_are_offered_by_description_and_a_missing_or_escaping_one_is_left_out() {
        let root = tree();
        let names = ["beta".to_owned(), "gone".to_owned(), "../beta".to_owned()];
        let chosen = chosen(Some(&root), &names);
        assert_eq!(
            chosen,
            [Skill {
                name: "beta".into(),
                description: "Does B.".into()
            }]
        );
        let spec = spec(&chosen);
        assert_eq!(spec.name, crate::tools::SKILL);
        assert_eq!(spec.schema["properties"]["name"]["enum"], json!(["beta"]));
        assert!(
            spec.schema["properties"]["name"]["description"]
                .as_str()
                .unwrap()
                .contains("- beta: Does B."),
            "the model is told what each is for"
        );
        assert!(super::chosen(None, &names).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_skill_is_read_only_if_the_agent_has_it_and_it_is_still_there() {
        let root = tree();
        let mine = ["beta".to_owned(), "alpha".to_owned(), "gone".to_owned()];
        assert_eq!(run(Some(&root), &mine, r#"{"name":"beta"}"#), "# B\nDo b.");
        assert_eq!(run(Some(&root), &mine, r#"{"name":" alpha "}"#), "Do a.");
        assert!(
            run(Some(&root), &mine, r#"{"name":"plain"}"#).starts_with("Error: you have no skill")
        );
        assert!(run(Some(&root), &mine, r#"{"name":"gone"}"#).contains("no longer installed"));
        assert!(run(None, &mine, r#"{"name":"beta"}"#).starts_with("Error"));
        assert!(run(Some(&root), &mine, "nonsense").starts_with("Error"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
