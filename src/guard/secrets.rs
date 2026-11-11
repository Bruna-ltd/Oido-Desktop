//! Finding secrets in text, and hiding them from a model behind numbered placeholders that are
//! turned back into the real values on this computer.
//!
//! Only well-known shapes are looked for (a provider's key prefix, a private key block, a password
//! written as `password = …`, the password in a URL), so ordinary text is left alone.

use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::Value;

/// What one pattern finds, which of its groups is the secret (0: all of it), and whether a find
/// really is one.
struct Detector {
    kind: &'static str,
    pattern: Regex,
    group: usize,
    accept: fn(&str) -> bool,
}

fn detector(kind: &'static str, pattern: &str, group: usize) -> Detector {
    Detector {
        kind,
        pattern: Regex::new(pattern).unwrap_or_else(|e| panic!("bad {kind} pattern: {e}")),
        group,
        accept: |_| true,
    }
}

static DETECTORS: LazyLock<Vec<Detector>> = LazyLock::new(|| {
    vec![
        detector(
            "private key",
            r"-----BEGIN[A-Z ]*PRIVATE KEY-----[\s\S]*?-----END[A-Z ]*PRIVATE KEY-----",
            0,
        ),
        // OpenAI, Anthropic, OpenRouter and others: `sk-` then a long token.
        detector("API key", r"\bsk-[A-Za-z0-9_\-]{20,}", 0),
        detector(
            "access token",
            r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,}|glpat-[A-Za-z0-9_\-]{20,}|hf_[A-Za-z0-9]{30,}|gsk_[A-Za-z0-9]{40,}|npm_[A-Za-z0-9]{36,}|xox[abposr]-[A-Za-z0-9\-]{10,}|xapp-[A-Za-z0-9\-]{10,})",
            0,
        ),
        detector("API key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", 0),
        detector("API key", r"\bAIza[0-9A-Za-z_\-]{35}", 0),
        detector("API key", r"\b(?:sk|rk)_(?:live|test)_[0-9A-Za-z]{16,}", 0),
        detector("bot token", r"\b\d{8,10}:[A-Za-z0-9_\-]{35}\b", 0),
        detector(
            "access token",
            r"\beyJ[A-Za-z0-9_\-]{8,}\.eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}",
            0,
        ),
        // `postgres://user:password@host`: only the password.
        detector(
            "password",
            r"\b[A-Za-z][A-Za-z0-9+.\-]*://[^\s:/@]+:([^\s@/]{3,})@",
            1,
        ),
        // `password = …`, `API_KEY: "…"`, `client_secret=…`: only the value, and only one that looks
        // like a secret (see `looks_secret`), so `token = self.token` is left alone.
        Detector {
            accept: looks_secret,
            ..detector(
                "password",
                r#"(?i)\b\w*?(?:password|passwd|secret|api[_\-]?key|apikey|access[_\-]?key|auth[_\-]?token|access[_\-]?token|token)\b["']?\s*[:=]\s*["']?([^\s"'`,;]{8,})"#,
                1,
            )
        },
    ]
});

/// `<<SECRET_3>>`, as the model sees a hidden value.
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<<SECRET_(\d+)>>").unwrap_or_else(|e| panic!("bad placeholder pattern: {e}"))
});

/// Whether a value caught by the `password = …` pattern is one: long enough, letters and digits,
/// and not code, a variable or a placeholder.
fn looks_secret(value: &str) -> bool {
    value.len() >= 8
        && value.chars().any(|c| c.is_ascii_digit())
        && value.chars().any(|c| c.is_ascii_alphabetic())
        && !value.contains(['(', ')'])
        && !value.starts_with(['$', '{', '<', '%', '['])
}

/// One secret found in a text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Found {
    pub kind: &'static str,
    pub value: String,
}

/// Every secret in `text`, in the order they appear, overlapping finds counted once (the longer).
pub fn find(text: &str) -> Vec<Found> {
    let mut spans: Vec<(usize, usize, &'static str)> = Vec::new();
    for d in DETECTORS.iter() {
        for caps in d.pattern.captures_iter(text) {
            let Some(m) = caps.get(d.group) else {
                continue;
            };
            if !(d.accept)(m.as_str()) || PLACEHOLDER.is_match(m.as_str()) {
                continue;
            }
            spans.push((m.start(), m.end(), d.kind));
        }
    }
    spans.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    let mut out = Vec::new();
    let mut end = 0;
    for (start, stop, kind) in spans {
        if start < end {
            continue;
        }
        end = stop;
        out.push(Found {
            kind,
            value: text[start..stop].to_owned(),
        });
    }
    out
}

/// The secrets hidden from a model in one chat, numbered in the order they were first seen there,
/// so the same value is always the same placeholder and what was sent before stays the same.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hidden {
    found: Vec<Found>,
}

impl Hidden {
    pub fn is_empty(&self) -> bool {
        self.found.is_empty()
    }

    pub fn len(&self) -> usize {
        self.found.len()
    }

    /// What kinds of secret are hidden, each with how many: "2 API keys, 1 password".
    pub fn describe(&self) -> String {
        let mut kinds: Vec<(&str, usize)> = Vec::new();
        for f in &self.found {
            match kinds.iter_mut().find(|(k, _)| *k == f.kind) {
                Some((_, n)) => *n += 1,
                None => kinds.push((f.kind, 1)),
            }
        }
        kinds
            .iter()
            .map(|(k, n)| match n {
                1 => format!("1 {k}"),
                n => format!("{n} {k}s"),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Notes the secrets in `text` not already known.
    pub fn learn(&mut self, text: &str) {
        for f in find(text) {
            if !self.found.iter().any(|k| k.value == f.value) {
                self.found.push(f);
            }
        }
    }

    /// Notes the secrets in the strings of a tool call's JSON arguments.
    pub fn learn_json(&mut self, args: &str) {
        match serde_json::from_str::<Value>(args) {
            Ok(v) => walk(&v, &mut |s| self.learn(s)),
            Err(_) => self.learn(args),
        }
    }

    /// Whether `value` is one of the hidden secrets.
    pub fn contains(&self, value: &str) -> bool {
        self.found.iter().any(|f| f.value == value)
    }

    /// The hidden secrets but those `keep` says to leave as they are.
    pub fn except(&self, keep: impl Fn(&str) -> bool) -> Hidden {
        Hidden {
            found: self
                .found
                .iter()
                .filter(|f| !keep(&f.value))
                .cloned()
                .collect(),
        }
    }

    /// Something that replaces every hidden value with its placeholder, made once for many texts.
    /// The placeholders keep this chat's numbering even when only some values are hidden.
    pub fn masker<'h>(&'h self, numbering: &'h Hidden) -> Masker<'h> {
        let mut values: Vec<&str> = self.found.iter().map(|f| f.value.as_str()).collect();
        // Longest first, so a value inside a longer one does not split it.
        values.sort_by_key(|v| std::cmp::Reverse(v.len()));
        let pattern = (!values.is_empty()).then(|| {
            let alternatives: Vec<String> = values.iter().map(|v| regex::escape(v)).collect();
            Regex::new(&alternatives.join("|"))
                .unwrap_or_else(|e| panic!("escaped values always make a pattern: {e}"))
        });
        Masker { pattern, numbering }
    }

    /// `text` with every hidden value replaced by its placeholder.
    pub fn hide(&self, text: &str) -> String {
        self.masker(self).hide(text)
    }

    /// `text` with every placeholder of a hidden value replaced by the value. A placeholder for a
    /// value that is not hidden here stays as it is.
    pub fn reveal(&self, text: &str) -> String {
        PLACEHOLDER
            .replace_all(text, |caps: &Captures| {
                caps[1]
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .and_then(|i| self.found.get(i))
                    .map_or_else(|| caps[0].to_owned(), |f| f.value.clone())
            })
            .into_owned()
    }

    /// [`Hidden::reveal`] in the strings of a tool call's JSON arguments, so a value with quotes or
    /// line breaks in it stays valid JSON.
    pub fn reveal_json(&self, args: &str) -> String {
        if !has_placeholder(args) {
            return args.to_owned();
        }
        match serde_json::from_str::<Value>(args) {
            Ok(mut v) => {
                map_strings(&mut v, &|s| self.reveal(s));
                v.to_string()
            }
            Err(_) => self.reveal(args),
        }
    }

    fn placeholder(&self, value: &str) -> Option<String> {
        self.found
            .iter()
            .position(|f| f.value == value)
            .map(|i| format!("<<SECRET_{}>>", i + 1))
    }
}

/// Replaces hidden values with placeholders; see [`Hidden::masker`].
pub struct Masker<'h> {
    pattern: Option<Regex>,
    numbering: &'h Hidden,
}

impl Masker<'_> {
    pub fn hide(&self, text: &str) -> String {
        match &self.pattern {
            Some(p) => p
                .replace_all(text, |caps: &Captures| {
                    self.numbering
                        .placeholder(&caps[0])
                        .unwrap_or_else(|| caps[0].to_owned())
                })
                .into_owned(),
            None => text.to_owned(),
        }
    }

    /// [`Masker::hide`] in the strings of a tool call's JSON arguments.
    pub fn hide_json(&self, args: &str) -> String {
        let Some(p) = &self.pattern else {
            return args.to_owned();
        };
        match serde_json::from_str::<Value>(args) {
            Ok(mut v) => {
                let mut changed = false;
                walk(&v, &mut |s| changed |= p.is_match(s));
                if !changed {
                    return args.to_owned();
                }
                map_strings(&mut v, &|s| self.hide(s));
                v.to_string()
            }
            Err(_) => self.hide(args),
        }
    }
}

/// Whether `text` holds a placeholder.
pub fn has_placeholder(text: &str) -> bool {
    PLACEHOLDER.is_match(text)
}

/// The first placeholder in `text`, if any.
pub fn first_placeholder(text: &str) -> Option<&str> {
    PLACEHOLDER.find(text).map(|m| m.as_str())
}

fn walk(v: &Value, f: &mut dyn FnMut(&str)) {
    match v {
        Value::String(s) => f(s),
        Value::Array(items) => items.iter().for_each(|i| walk(i, f)),
        Value::Object(map) => map.values().for_each(|i| walk(i, f)),
        _ => {}
    }
}

fn map_strings(v: &mut Value, f: &dyn Fn(&str) -> String) {
    match v {
        Value::String(s) => *s = f(s),
        Value::Array(items) => items.iter_mut().for_each(|i| map_strings(i, f)),
        Value::Object(map) => map.values_mut().for_each(|i| map_strings(i, f)),
        _ => {}
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const KEY: &str = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123";

    fn kinds(text: &str) -> Vec<(&'static str, String)> {
        find(text).into_iter().map(|f| (f.kind, f.value)).collect()
    }

    #[test]
    fn well_known_secrets_are_found_and_ordinary_text_is_not() {
        assert_eq!(
            kinds(&format!("use {KEY} please")),
            [("API key", KEY.to_owned())]
        );
        let gh = format!("ghp_{}", "a1".repeat(18));
        assert_eq!(kinds(&gh), [("access token", gh.clone())]);
        assert_eq!(
            kinds("AKIAABCDEFGHIJKLMNOP"),
            [("API key", "AKIAABCDEFGHIJKLMNOP".to_owned())]
        );
        let pem =
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3Blbn\nNzaC1\n-----END OPENSSH PRIVATE KEY-----";
        assert_eq!(
            kinds(&format!("key:\n{pem}\n")),
            [("private key", pem.to_owned())]
        );
        assert_eq!(
            kinds("postgres://ada:s3cret-pw@db.local/app"),
            [("password", "s3cret-pw".to_owned())]
        );
        for plain in [
            "sk-short",
            "the token = self.token",
            "password: hunter",
            "passwords = 12345678abc",
            "tokenizer = bpe2tok12",
            "api_key = os.environ(\"KEY1\")",
            "https://example.com/path",
            "an ordinary sentence with no secrets at all",
        ] {
            assert!(find(plain).is_empty(), "{plain:?} found {:?}", find(plain));
        }
    }

    #[test]
    fn a_password_assignment_hides_only_the_value() {
        assert_eq!(
            kinds("OPENAI_API_KEY=abc123def456ghi"),
            [("password", "abc123def456ghi".to_owned())]
        );
        assert_eq!(
            kinds(r#"{"password": "hunter2hunter2"}"#),
            [("password", "hunter2hunter2".to_owned())]
        );
    }

    #[test]
    fn values_are_numbered_by_first_sight_and_hide_and_reveal_round_trip() {
        let mut h = Hidden::default();
        h.learn(&format!("first {KEY}"));
        h.learn("password = hunter2hunter2 and again ");
        h.learn(&format!("{KEY} seen again"));
        assert_eq!(h.len(), 2);
        assert_eq!(h.describe(), "1 API key, 1 password");
        let text = format!("send {KEY} with hunter2hunter2");
        let hidden = h.hide(&text);
        assert_eq!(hidden, "send <<SECRET_1>> with <<SECRET_2>>");
        assert_eq!(h.reveal(&hidden), text);
        assert_eq!(
            h.reveal("<<SECRET_9>> stays"),
            "<<SECRET_9>> stays",
            "an unknown placeholder is left for the gate to refuse"
        );
    }

    #[test]
    fn json_arguments_stay_valid_when_a_value_has_line_breaks() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIE\"q\n-----END RSA PRIVATE KEY-----";
        let mut h = Hidden::default();
        h.learn(pem);
        let args = serde_json::json!({"path": "id_rsa", "content": "<<SECRET_1>>"}).to_string();
        let revealed: Value = serde_json::from_str(&h.reveal_json(&args)).unwrap();
        assert_eq!(revealed["content"], pem);
        let back = h.masker(&h).hide_json(&revealed.to_string());
        assert!(
            back.contains("<<SECRET_1>>") && !back.contains("MIIE"),
            "{back}"
        );
        assert_eq!(
            h.masker(&h).hide_json(r#"{"a":"plain"}"#),
            r#"{"a":"plain"}"#,
            "untouched arguments keep their exact text"
        );
    }

    #[test]
    fn hiding_some_values_keeps_the_chats_numbering() {
        let mut h = Hidden::default();
        h.learn(&format!("{KEY} and password=hunter2hunter2"));
        let theirs = h.except(|v| v == KEY);
        assert_eq!(
            theirs.masker(&h).hide(&format!("{KEY} / hunter2hunter2")),
            format!("{KEY} / <<SECRET_2>>")
        );
    }
}
