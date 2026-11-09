//! References in a workflow: `{{inputs.since}}` for a value the run started with, `{{emails}}` or
//! `{{emails.0.subject}}` for an earlier step's result. A path is names and list positions joined
//! by dots.

use std::collections::BTreeMap;

use serde_json::Value;

/// What a step can reference: each input under `inputs`, and each bound result by name.
pub type Scope = BTreeMap<String, Value>;

/// A `{{path}}` found in text: where it sits and what it names.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ref {
    pub start: usize,
    pub end: usize,
    pub path: String,
}

fn path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// Every well-formed `{{path}}` in `text`, in order. Spaces inside the braces are allowed.
pub fn find_refs(text: &str) -> Vec<Ref> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find("{{") {
        let start = from + open;
        let Some(close) = text[start + 2..].find("}}") else {
            break;
        };
        let end = start + 2 + close + 2;
        let inner = text[start + 2..end - 2].trim();
        if !inner.is_empty() && inner.chars().all(path_char) {
            out.push(Ref {
                start,
                end,
                path: inner.to_owned(),
            });
        }
        from = end;
    }
    out
}

/// The value a path names in `scope`.
pub fn lookup(scope: &Scope, path: &str) -> Result<Value, String> {
    let mut parts = path.split('.');
    let root = parts.next().unwrap_or_default();
    let mut value = scope
        .get(root)
        .ok_or_else(|| format!("{{{{{path}}}}}: nothing is called {root}"))?;
    for part in parts {
        value = match value {
            Value::Array(items) => part
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i))
                .ok_or_else(|| format!("{{{{{path}}}}}: the list has no item {part}"))?,
            Value::Object(map) => map
                .get(part)
                .ok_or_else(|| format!("{{{{{path}}}}}: there is no field {part}"))?,
            _ => return Err(format!("{{{{{path}}}}}: {part} is inside a plain value")),
        };
    }
    Ok(value.clone())
}

/// A value as text: text as it is, anything else as JSON.
pub fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `text` with each reference replaced by its value as text.
pub fn render(scope: &Scope, text: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut last = 0;
    for r in find_refs(text) {
        out.push_str(&text[last..r.start]);
        out.push_str(&as_text(&lookup(scope, &r.path)?));
        last = r.end;
    }
    out.push_str(&text[last..]);
    Ok(out)
}

/// A tool argument with its references filled in. A string that is only one reference keeps the
/// value's own type (a number stays a number); references inside longer text become text.
pub fn resolve(scope: &Scope, v: &Value) -> Result<Value, String> {
    match v {
        Value::String(s) => {
            let refs = find_refs(s);
            if let [only] = refs.as_slice()
                && only.start == 0
                && only.end == s.len()
            {
                return lookup(scope, &only.path);
            }
            render(scope, s).map(Value::String)
        }
        Value::Array(items) => items
            .iter()
            .map(|i| resolve(scope, i))
            .collect::<Result<_, _>>()
            .map(Value::Array),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| resolve(scope, v).map(|v| (k.clone(), v)))
            .collect::<Result<_, _>>()
            .map(Value::Object),
        other => Ok(other.clone()),
    }
}

/// Every reference inside a JSON value's strings.
pub fn refs_in(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.extend(find_refs(s).into_iter().map(|r| r.path)),
        Value::Array(items) => items.iter().for_each(|i| refs_in(i, out)),
        Value::Object(map) => map.values().for_each(|v| refs_in(v, out)),
        _ => {}
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scope() -> Scope {
        Scope::from([
            ("inputs".into(), json!({"since": "monday", "n": 3})),
            ("mails".into(), json!([{"subject": "Hi", "ids": [7, 8]}])),
            ("out".into(), json!("plain")),
        ])
    }

    #[test]
    fn references_are_found_with_their_places_and_malformed_ones_are_left_alone() {
        let t = "a {{ inputs.since }} b {{mails.0.subject}} {{ bad path }} {{unclosed";
        let refs = find_refs(t);
        assert_eq!(
            refs.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["inputs.since", "mails.0.subject"]
        );
        assert_eq!(&t[refs[0].start..refs[0].end], "{{ inputs.since }}");
    }

    #[test]
    fn a_path_walks_fields_and_list_positions_and_says_where_it_failed() {
        let s = scope();
        assert_eq!(lookup(&s, "inputs.n").unwrap(), json!(3));
        assert_eq!(lookup(&s, "mails.0.ids.1").unwrap(), json!(8));
        assert!(
            lookup(&s, "nope")
                .unwrap_err()
                .contains("nothing is called nope")
        );
        assert!(lookup(&s, "mails.5").unwrap_err().contains("no item 5"));
        assert!(
            lookup(&s, "mails.0.zzz")
                .unwrap_err()
                .contains("no field zzz")
        );
        assert!(lookup(&s, "out.x").unwrap_err().contains("plain value"));
    }

    #[test]
    fn text_is_rendered_and_a_lone_reference_keeps_its_type() {
        let s = scope();
        assert_eq!(
            render(&s, "since {{inputs.since}}: {{mails.0.ids}}").unwrap(),
            "since monday: [7,8]"
        );
        assert_eq!(resolve(&s, &json!("{{inputs.n}}")).unwrap(), json!(3));
        assert_eq!(resolve(&s, &json!("n={{inputs.n}}")).unwrap(), json!("n=3"));
        assert_eq!(
            resolve(&s, &json!({"a": ["{{out}}", 1, true]})).unwrap(),
            json!({"a": ["plain", 1, true]})
        );
        let mut found = Vec::new();
        refs_in(&json!({"a": ["{{x.y}}", "{{z}}"]}), &mut found);
        assert_eq!(found, ["x.y", "z"]);
    }
}
