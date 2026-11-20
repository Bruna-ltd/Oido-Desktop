//! Blocked places: folders and files the person has put off limits to every tool a model drives.
//!
//! A place is matched by where it really is (links, `..` and `~` worked out) and covers everything
//! under it. A file tool is checked by its `path`; a shell command by every word in it that names a
//! place; any other tool (an MCP one, say) by every text in its arguments that looks like a path.
//! A command can still reach a place by a name the guard cannot read (a variable, a glob, a `cd`
//! followed by a relative path), so this is a net, not a wall.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Action;
use super::actions::{Finding, Place, normalize, words};

/// What a blocked place keeps a model from doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Keep {
    /// Reading, listing, sending, changing: anything.
    Everything,
    /// Reading and sending it; changing it is left to the other rules.
    Reading,
    /// Changing and deleting it; reading it is left alone.
    Writing,
}

impl Keep {
    pub fn title(self) -> &'static str {
        match self {
            Self::Everything => "No access",
            Self::Reading => "No reading",
            Self::Writing => "No changing",
        }
    }

    /// Whether an action that `reads`, or changes, is kept from.
    fn stops(self, act: Act) -> bool {
        matches!(
            (self, act),
            (Self::Everything, _)
                | (_, Act::Both)
                | (Self::Reading, Act::Read)
                | (Self::Writing, Act::Write)
        )
    }
}

/// What a tool call does to a path.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Read,
    Write,
    /// Not known, so every kind of block applies (a shell command, an MCP tool).
    Both,
}

/// One blocked folder or file.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Blocked {
    /// An absolute path, or one starting `~/` for the person's home folder.
    pub path: String,
    pub keep: Keep,
}

impl Blocked {
    pub fn new(path: impl Into<String>, keep: Keep) -> Self {
        Self {
            path: path.into(),
            keep,
        }
    }
}

/// The places blocked until the person changes the list: keys and logins, other programs'
/// saved sign-ins, and this app's own database.
pub fn defaults(data: Option<&Path>) -> Vec<Blocked> {
    let mut out: Vec<Blocked> = [
        "~/.ssh",
        "~/.gnupg",
        "~/.aws",
        "~/.config/google-chrome",
        "~/.config/chromium",
        "~/.config/BraveSoftware",
        "~/.mozilla",
    ]
    .into_iter()
    .map(|p| Blocked::new(p, Keep::Everything))
    .collect();
    if let Some(db) = data {
        out.push(Blocked::new(
            db.join("oido.db").display().to_string(),
            Keep::Everything,
        ));
    }
    out
}

/// Places as typed: split by commas or lines, each a path with an optional `(reading)` or
/// `(changing)` after it; no suffix blocks everything.
pub fn parse(text: &str) -> Vec<Blocked> {
    text.split([',', '\n'])
        .filter_map(|part| {
            let part = part.trim();
            let (path, keep) = match part.strip_suffix(')').and_then(|p| p.rsplit_once('(')) {
                Some((path, "reading")) => (path, Keep::Reading),
                Some((path, "changing")) => (path, Keep::Writing),
                Some((path, "everything")) => (path, Keep::Everything),
                _ => (part, Keep::Everything),
            };
            let path = path.trim().trim_end_matches('/');
            (!path.is_empty()).then(|| Blocked::new(path, keep))
        })
        .collect()
}

/// Places as [`parse`] reads them.
pub fn render(places: &[Blocked]) -> String {
    places
        .iter()
        .map(|b| match b.keep {
            Keep::Everything => b.path.clone(),
            Keep::Reading => format!("{} (reading)", b.path),
            Keep::Writing => format!("{} (changing)", b.path),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The first place of `global` and of the agent's own (`at.blocked`) that tool call `name` with `args` (the real values) reaches in a
/// way it keeps from, as a refusal.
pub fn check(name: &str, args: &str, at: &Place, global: &[Blocked]) -> Option<Finding> {
    let places: Vec<(&Blocked, PathBuf)> = global
        .iter()
        .chain(&at.blocked)
        .map(|b| (b, real(&expand(&b.path, at.home.as_deref()), &at.root)))
        .collect();
    if places.is_empty() {
        return None;
    }
    let hit = |given: &str, act: Act| {
        let path = real(&expand(given, at.home.as_deref()), &at.root);
        places
            .iter()
            .find(|(b, p)| b.keep.stops(act) && path.starts_with(p))
            .map(|(b, _)| *b)
    };
    let value: Value = serde_json::from_str(args).ok()?;
    let found = if name == crate::tools::SHELL {
        let command = value.get("command").and_then(Value::as_str)?;
        shell_words(command).iter().find_map(|w| hit(w, Act::Both))
    } else if name == crate::files::READ || name == crate::channels::SEND_FILE {
        let path = value.get("path").and_then(Value::as_str)?;
        hit(path, Act::Read)
    } else if name == crate::files::WRITE || name == crate::files::EDIT {
        let path = value.get("path").and_then(Value::as_str)?;
        hit(path, Act::Write)
    } else if crate::mcp::is_tool(name) {
        let mut found = None;
        strings(&value, &mut |s| {
            if found.is_none() && looks_like_path(s) {
                found = hit(s, Act::Both);
            }
        });
        found
    } else {
        None
    }?;
    Some(Finding {
        rule: "path.blocked",
        why: format!(
            "It reaches {}, a place you blocked ({}).",
            found.path,
            found.keep.title().to_lowercase()
        ),
        action: Action::Block,
    })
}

/// `given` with `~` and `$HOME` taken as the home folder; anything else as it is.
fn expand(given: &str, home: Option<&Path>) -> String {
    let rest = ["~", "$HOME", "${HOME}"].into_iter().find_map(|h| {
        given
            .strip_prefix(h)
            .filter(|r| r.is_empty() || r.starts_with('/'))
    });
    match (rest, home) {
        (Some(rest), Some(home)) => format!("{}{rest}", home.display()),
        _ => given.to_owned(),
    }
}

/// Where `given` (relative to `root`) really is: `..` worked out, and links followed as far as the
/// path exists, so a link into a blocked place is the place.
fn real(given: &str, root: &Path) -> PathBuf {
    let path = normalize(&root.join(given));
    let mut tail = Vec::new();
    let mut head = path.as_path();
    loop {
        if let Ok(done) = head.canonicalize() {
            return tail.iter().rev().fold(done, |p, part| p.join(part));
        }
        match (head.parent(), head.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_owned());
                head = parent;
            }
            _ => return path,
        }
    }
}

/// The words of a shell command that can name a path: split at the shell's own punctuation, and at
/// `=` and `,` so `--file=~/.ssh/id` and `a,b` show their paths.
fn shell_words(command: &str) -> Vec<String> {
    command
        .split(['\n', ';', '&', '|', '(', ')', '<', '>', '`'])
        .flat_map(words)
        .flat_map(|w| {
            w.split(['=', ','])
                .map(str::to_owned)
                .chain([w.clone()])
                .collect::<Vec<_>>()
        })
        .filter(|w| !w.is_empty() && !w.starts_with('-'))
        .collect()
}

/// Calls `f` with every text in `v`.
fn strings(v: &Value, f: &mut impl FnMut(&str)) {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => a.iter().for_each(|v| strings(v, f)),
        Value::Object(o) => o.values().for_each(|v| strings(v, f)),
        _ => {}
    }
}

/// Whether a text in a tool's arguments is likely a path: not a URL or a sentence.
fn looks_like_path(s: &str) -> bool {
    !s.contains("://")
        && !s.contains('\n')
        && s.len() < 1024
        && (s.starts_with(['/', '~', '.']) || s.starts_with("$HOME") || s.contains('/'))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn at(root: &Path, home: &Path) -> Place {
        Place {
            root: root.to_owned(),
            home: Some(home.to_owned()),
            blocked: Vec::new(),
        }
    }

    struct Dir(PathBuf);

    impl Dir {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup() -> (Dir, Place, Vec<Blocked>) {
        let dir = Dir(std::env::temp_dir().join(format!("oido-blocked-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir_all(dir.path()).unwrap();
        let home = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::create_dir_all(home.join("work/private")).unwrap();
        std::fs::create_dir_all(home.join("work/src")).unwrap();
        let place = at(&home.join("work"), &home);
        let list = vec![
            Blocked::new("~/.ssh", Keep::Everything),
            Blocked::new(
                home.join("work/private").display().to_string(),
                Keep::Reading,
            ),
            Blocked::new(
                home.join("work/src/lock.toml").display().to_string(),
                Keep::Writing,
            ),
        ];
        (dir, place, list)
    }

    fn rule(name: &str, args: Value, place: &Place, list: &[Blocked]) -> Option<&'static str> {
        check(name, &args.to_string(), place, list).map(|f| f.rule)
    }

    #[test]
    fn file_tools_are_kept_from_a_place_by_how_it_is_blocked() {
        let (_d, place, list) = setup();
        let r = |n: &str, p: &str| rule(n, serde_json::json!({"path": p}), &place, &list);
        assert_eq!(r("read_file", "../.ssh/id_rsa"), Some("path.blocked"));
        assert_eq!(r("write_file", "private/a.txt"), None, "reading only");
        assert_eq!(r("read_file", "private/a.txt"), Some("path.blocked"));
        assert_eq!(r("send_file", "private/a.txt"), Some("path.blocked"));
        assert_eq!(r("read_file", "src/lock.toml"), None, "changing only");
        assert_eq!(r("edit_file", "src/lock.toml"), Some("path.blocked"));
        assert_eq!(r("read_file", "src/main.rs"), None);
        assert_eq!(
            r("read_file", "privateer/a.txt"),
            None,
            "a name that only starts the same"
        );
    }

    #[test]
    fn a_link_into_a_blocked_place_is_the_place() {
        let (d, place, list) = setup();
        let link = d.path().join("work/innocent");
        std::os::unix::fs::symlink(d.path().join(".ssh"), &link).unwrap();
        assert_eq!(
            rule(
                "read_file",
                serde_json::json!({"path": "innocent/id_rsa"}),
                &place,
                &list
            ),
            Some("path.blocked")
        );
    }

    #[test]
    fn a_shell_command_naming_a_place_is_refused() {
        let (_d, place, list) = setup();
        let s = |c: &str| rule("shell", serde_json::json!({"command": c}), &place, &list);
        assert_eq!(s("cat ~/.ssh/id_rsa"), Some("path.blocked"));
        assert_eq!(s("cat \"$HOME/.ssh/id_rsa\" | head"), Some("path.blocked"));
        assert_eq!(s("ls private && echo hi"), Some("path.blocked"));
        assert_eq!(s("tar czf x.tgz --file=../.ssh/k"), Some("path.blocked"));
        assert_eq!(
            s("echo x > src/lock.toml"),
            Some("path.blocked"),
            "unknown kind counts"
        );
        assert_eq!(s("cargo test && ls src"), None);
        assert_eq!(
            s("echo private"),
            Some("path.blocked"),
            "a word that names it, as `ls private` does"
        );
    }

    #[test]
    fn other_tools_are_checked_by_the_paths_in_their_arguments() {
        let (_d, place, list) = setup();
        let mcp = "mcp__files__read";
        let bad = serde_json::json!({"opts": {"paths": ["~/.ssh/id_rsa"]}});
        assert_eq!(rule(mcp, bad, &place, &list), Some("path.blocked"));
        let fine = serde_json::json!({"url": "https://x.dev/.ssh/y", "note": "see ~/.ssh"});
        assert_eq!(rule(mcp, fine, &place, &list), None);
    }

    #[test]
    fn places_are_typed_and_shown_the_same_way() {
        let text = "~/.ssh, /srv/a (reading), /srv/b/ (changing)\n/srv/c (everything)";
        let places = parse(text);
        assert_eq!(
            places,
            [
                Blocked::new("~/.ssh", Keep::Everything),
                Blocked::new("/srv/a", Keep::Reading),
                Blocked::new("/srv/b", Keep::Writing),
                Blocked::new("/srv/c", Keep::Everything),
            ]
        );
        assert_eq!(parse(&render(&places)), places);
        assert!(parse(" , ,").is_empty());
    }

    #[test]
    fn an_agents_own_places_count_beside_the_lists() {
        let (_d, mut place, _) = setup();
        let args = serde_json::json!({"path": "../.ssh/x"}).to_string();
        assert!(check("read_file", &args, &place, &[]).is_none());
        place.blocked = vec![Blocked::new("~/.ssh", Keep::Everything)];
        assert!(check("read_file", &args, &place, &[]).is_some());
    }
}
