//! A workflow definition: what a run is given (inputs), the steps it takes, and how to tell it
//! worked. Modelled on `oido-core`'s so a definition reads the same in both. A step is one of
//! three kinds for now: a tool call, a piece of writing by the model, or a choice among fixed
//! options. A definition that passes [`validate`] can only fail when it runs, on data.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::template;

pub const TOOL: &str = "tool";
pub const GENERATE: &str = "generate";
pub const DECIDE: &str = "decide";

/// The tools a workflow step may call. A flow runs on its own, so it cannot stop to ask you.
pub const STEP_TOOLS: [&str; 4] = [
    crate::tools::SHELL,
    crate::files::READ,
    crate::files::WRITE,
    crate::files::EDIT,
];

const MAX_STEPS: usize = 50;
const MAX_OPTIONS: usize = 20;

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Definition {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, Input>,
    pub steps: Vec<Step>,
    /// What shows a run did its job, in plain words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub success: String,
}

/// A value a run starts with. One with no default is required.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Input {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub kind: String,
    /// Names the result so later steps can reference it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    // tool
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Map<String, Value>>,
    // generate
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    // decide
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
}

fn name_ok(s: &str, dash: bool) -> bool {
    let mut chars = s.chars();
    let first = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || (!dash && c == '_'));
    first
        && s.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || (dash && c == '-'))
}

fn type_ok(v: &Value, kind: &str) -> bool {
    match kind {
        "string" => v.is_string(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        _ => false,
    }
}

/// Every structural problem in `def`, all at once, so one who fixes it gets the whole list.
pub fn validate(def: &Definition) -> Result<(), Vec<String>> {
    let mut bad: Vec<String> = Vec::new();
    for (name, input) in &def.inputs {
        if !name_ok(name, false) {
            bad.push(format!("input {name:?}: a name is letters, digits and underscores, starting with a letter or underscore"));
        }
        if !matches!(input.kind.as_str(), "string" | "number" | "boolean") {
            bad.push(format!(
                "input {name:?}: type {:?} is not string, number or boolean",
                input.kind
            ));
        } else if input
            .default
            .as_ref()
            .is_some_and(|d| !type_ok(d, &input.kind))
        {
            bad.push(format!(
                "input {name:?}: the default is not a {}",
                input.kind
            ));
        }
    }
    if def.steps.is_empty() {
        bad.push("a workflow needs at least one step".to_owned());
    }
    if def.steps.len() > MAX_STEPS {
        bad.push(format!(
            "{} steps is more than the {MAX_STEPS} a workflow may have",
            def.steps.len()
        ));
    }
    let mut ids = BTreeSet::new();
    let mut binds: BTreeSet<String> = BTreeSet::new();
    for (i, s) in def.steps.iter().enumerate() {
        let at = format!("step {} ({:?})", i + 1, s.id);
        if !name_ok(&s.id, true) {
            bad.push(format!(
                "{at}: an id is letters, digits, - and _, starting with a letter"
            ));
        } else if !ids.insert(s.id.clone()) {
            bad.push(format!("{at}: another step has this id"));
        }
        let mut refs: Vec<String> = Vec::new();
        let stray = |field: &str, present: bool, bad: &mut Vec<String>| {
            if present {
                bad.push(format!("{at}: a {} step has no {field}", s.kind));
            }
        };
        match s.kind.as_str() {
            TOOL => {
                match s.tool.as_deref() {
                    None | Some("") => bad.push(format!("{at}: a tool step needs a tool")),
                    Some(t) if !STEP_TOOLS.contains(&t) => bad.push(format!(
                        "{at}: {t:?} is not a tool a workflow can use; they are {}",
                        STEP_TOOLS.join(", ")
                    )),
                    _ => {}
                }
                if let Some(args) = &s.args {
                    template::refs_in(&Value::Object(args.clone()), &mut refs);
                }
                stray("instruction", s.instruction.is_some(), &mut bad);
                stray(
                    "question",
                    s.question.is_some() || !s.options.is_empty() || s.input.is_some(),
                    &mut bad,
                );
            }
            GENERATE => {
                match s.instruction.as_deref() {
                    None | Some("") => {
                        bad.push(format!("{at}: a generate step needs an instruction"))
                    }
                    Some(t) => refs.extend(template::find_refs(t).into_iter().map(|r| r.path)),
                }
                stray("tool", s.tool.is_some() || s.args.is_some(), &mut bad);
                stray(
                    "question",
                    s.question.is_some() || !s.options.is_empty() || s.input.is_some(),
                    &mut bad,
                );
            }
            DECIDE => {
                match s.question.as_deref() {
                    None | Some("") => bad.push(format!("{at}: a decide step needs a question")),
                    Some(q) if !template::find_refs(q).is_empty() => {
                        bad.push(format!(
                            "{at}: the question is fixed text; put the value to judge in `input`"
                        ));
                    }
                    _ => {}
                }
                let opts: BTreeSet<&String> = s.options.iter().collect();
                if s.options.len() < 2 || s.options.len() > MAX_OPTIONS {
                    bad.push(format!(
                        "{at}: a decide step needs 2 to {MAX_OPTIONS} options"
                    ));
                } else if opts.len() != s.options.len()
                    || s.options.iter().any(|o| o.trim().is_empty())
                {
                    bad.push(format!("{at}: the options must be different and not empty"));
                }
                match s.input.as_deref() {
                    None | Some("") => {
                        bad.push(format!("{at}: a decide step needs an input to judge"))
                    }
                    Some(t) => refs.extend(template::find_refs(t).into_iter().map(|r| r.path)),
                }
                if s.bind.as_deref().is_none_or(str::is_empty) {
                    bad.push(format!("{at}: a decide step needs a bind: its answer is only useful when something reads it"));
                }
                stray("tool", s.tool.is_some() || s.args.is_some(), &mut bad);
                stray("instruction", s.instruction.is_some(), &mut bad);
            }
            other => bad.push(format!(
                "{at}: {other:?} is not a step kind; they are tool, generate and decide"
            )),
        }
        for path in refs {
            let root = path.split('.').next().unwrap_or_default();
            if root == "inputs" {
                let key = path.split('.').nth(1).unwrap_or_default();
                if !def.inputs.contains_key(key) {
                    bad.push(format!(
                        "{at}: {{{{{path}}}}} names an input that is not declared"
                    ));
                }
            } else if !binds.contains(root) {
                bad.push(format!(
                    "{at}: {{{{{path}}}}} refers to {root:?}, which no earlier step binds"
                ));
            }
        }
        if let Some(b) = s.bind.as_deref().filter(|b| !b.is_empty()) {
            if !name_ok(b, false) || b == "inputs" {
                bad.push(format!(
                    "{at}: bind {b:?} must be letters, digits and underscores, and not `inputs`"
                ));
            } else if !binds.insert(b.to_owned()) {
                bad.push(format!("{at}: another step binds {b:?}"));
            }
        }
    }
    if bad.is_empty() { Ok(()) } else { Err(bad) }
}

/// The tools a definition calls, without repeats.
pub fn tools_used(def: &Definition) -> Vec<String> {
    let mut seen = Vec::new();
    for t in def.steps.iter().filter_map(|s| s.tool.clone()) {
        if !seen.contains(&t) {
            seen.push(t);
        }
    }
    seen
}

/// A step as one line a person can read: what it does and to what.
pub fn describe(s: &Step) -> String {
    let short = |t: &str| {
        let line = t.lines().next().unwrap_or_default();
        let (cut, more) = crate::runs::clip(line, 90);
        if more || t.lines().count() > 1 {
            format!("{cut}…")
        } else {
            cut
        }
    };
    match s.kind.as_str() {
        TOOL => {
            let tool = s.tool.as_deref().unwrap_or("?");
            let arg = |k: &str| {
                s.args
                    .as_ref()
                    .and_then(|a| a.get(k))
                    .map(template::as_text)
            };
            let what = match tool {
                "shell" => arg("command").map(|c| format!("`{}`", short(&c))),
                _ => arg("path"),
            };
            match what {
                Some(w) => format!("{tool}: {w}"),
                None => tool.to_owned(),
            }
        }
        GENERATE => format!(
            "Write: {}",
            short(s.instruction.as_deref().unwrap_or_default())
        ),
        DECIDE => format!(
            "Decide: {} ({})",
            short(s.question.as_deref().unwrap_or_default()),
            s.options.join(" / ")
        ),
        other => other.to_owned(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(v: Value) -> Definition {
        serde_json::from_value(v).unwrap()
    }

    fn problems(v: Value) -> String {
        validate(&def(v)).unwrap_err().join("\n")
    }

    #[test]
    fn a_good_definition_passes_and_round_trips_through_json() {
        let d = def(json!({
            "inputs": {"folder": {"type": "string", "default": "notes"}},
            "steps": [
                {"id": "list", "kind": "tool", "tool": "shell", "args": {"command": "ls {{inputs.folder}}"}, "bind": "files"},
                {"id": "pick", "kind": "decide", "question": "Any files?", "options": ["yes", "no"], "input": "{{files.stdout}}", "bind": "any"},
                {"id": "say", "kind": "generate", "instruction": "Summarise {{files.stdout}} given {{any}}"}
            ],
            "success": "A summary was written."
        }));
        assert_eq!(validate(&d), Ok(()));
        let again: Definition = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(again, d);
        assert_eq!(tools_used(&d), ["shell"]);
        assert_eq!(describe(&d.steps[0]), "shell: `ls {{inputs.folder}}`");
        assert!(describe(&d.steps[1]).starts_with("Decide: Any files? (yes / no)"));
    }

    #[test]
    fn every_problem_is_listed_at_once() {
        let p = problems(json!({
            "inputs": {"bad name": {"type": "date"}, "n": {"type": "number", "default": "x"}},
            "steps": [
                {"id": "1bad", "kind": "tool", "tool": "rm_rf"},
                {"id": "a", "kind": "tool", "tool": "shell", "args": {"command": "{{ghost}} {{inputs.zzz}}"}, "bind": "r"},
                {"id": "a", "kind": "generate", "instruction": "", "bind": "r"},
                {"id": "d", "kind": "decide", "question": "Is {{r}}?", "options": ["only"], "input": "{{r}}"},
                {"id": "x", "kind": "foreach"},
                {"id": "ask", "kind": "tool", "tool": "ask_user_question"}
            ]
        }));
        for expected in [
            "bad name",
            "type \"date\"",
            "default is not a number",
            "1bad",
            "not a tool a workflow can use",
            "refers to \"ghost\"",
            "input that is not declared",
            "another step has this id",
            "needs an instruction",
            "another step binds \"r\"",
            "fixed text",
            "2 to 20 options",
            "needs a bind",
            "not a step kind",
            "\"ask_user_question\" is not a tool",
        ] {
            assert!(p.contains(expected), "missing {expected:?} in:\n{p}");
        }
        assert!(problems(json!({"steps": []})).contains("at least one step"));
    }

    #[test]
    fn a_field_that_belongs_to_another_kind_is_refused_and_a_reference_may_only_look_back() {
        let p = problems(json!({"steps": [
            {"id": "a", "kind": "generate", "instruction": "x {{later}}", "tool": "shell"},
            {"id": "b", "kind": "tool", "tool": "shell", "instruction": "x", "bind": "later"}
        ]}));
        assert!(p.contains("a generate step has no tool"), "{p}");
        assert!(p.contains("a tool step has no instruction"), "{p}");
        assert!(p.contains("no earlier step binds"), "{p}");
    }
}
