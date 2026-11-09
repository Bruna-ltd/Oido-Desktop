//! Skills: folders in `~/.agents/skills`, each with a `SKILL.md` that says what the skill is and
//! how to do it. An agent can be given some; their text then goes to the model with its brief.
//! `OIDO_SKILLS` points somewhere else.

use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Size of its instructions in bytes, for estimating what it adds to a request.
    pub bytes: usize,
}

/// Where skills live.
pub fn dir() -> Option<PathBuf> {
    match std::env::var_os("OIDO_SKILLS") {
        Some(p) => Some(PathBuf::from(p)),
        None => dirs::home_dir().map(|h| h.join(".agents").join("skills")),
    }
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
            let (front, rest) = split(&text);
            Some(Skill {
                name: field(front, "name").unwrap_or(folder),
                description: field(front, "description").unwrap_or_default(),
                bytes: rest.trim().len(),
            })
        })
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// A skill's instructions, without its front matter. `None` if it is gone.
pub fn body(dir: &Path, name: &str) -> Option<String> {
    // A name is a folder name; never a path out of the skills folder.
    if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return None;
    }
    let text = std::fs::read_to_string(dir.join(name).join("SKILL.md")).ok()?;
    Some(split(&text).1.trim().to_owned())
}

/// The brief with the chosen skills' instructions after it.
pub fn with_skills(brief: &str, dir: Option<&Path>, chosen: &[String]) -> String {
    let mut out = brief.trim().to_owned();
    for name in chosen {
        if let Some(text) = dir.and_then(|d| body(d, name)) {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&format!("## Skill: {name}\n\n{text}"));
        }
    }
    out
}

/// Roughly how many tokens the chosen skills add to every message: about four bytes a token.
/// Skills no longer in the folder add nothing.
pub fn estimate_tokens(all: &[Skill], chosen: &[String]) -> usize {
    all.iter()
        .filter(|s| chosen.contains(&s.name))
        .map(|s| s.bytes / 4)
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

/// A top-level `key: value` line of the front matter.
fn field(front: &str, key: &str) -> Option<String> {
    front
        .lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .map(|v| v.trim().trim_matches(['"', '\'']).to_owned())
        .filter(|v| !v.is_empty())
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
    fn what_chosen_skills_add_is_estimated_from_their_size() {
        let skill = |n: &str, bytes| Skill {
            name: n.into(),
            description: String::new(),
            bytes,
        };
        let all = [skill("a", 4000), skill("b", 400), skill("c", 40)];
        let chosen = ["a".to_owned(), "c".to_owned(), "gone".to_owned()];
        assert_eq!(estimate_tokens(&all, &chosen), 1010);
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
    fn chosen_skills_follow_the_brief_and_a_missing_or_escaping_one_adds_nothing() {
        let root = tree();
        let chosen = ["beta".to_owned(), "gone".to_owned(), "../beta".to_owned()];
        let text = with_skills("Be brief.", Some(&root), &chosen);
        assert_eq!(text, "Be brief.\n\n## Skill: beta\n\n# B\nDo b.");
        assert_eq!(
            with_skills("", Some(&root), &["alpha".to_owned()]),
            "## Skill: alpha\n\nDo a."
        );
        assert_eq!(with_skills("Hi", None, &chosen), "Hi");
        std::fs::remove_dir_all(root).unwrap();
    }
}
