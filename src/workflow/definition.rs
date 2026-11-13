//! A workflow definition: what a run is given (inputs), the steps it takes, and how to tell it
//! worked. Modelled on `oido-core`'s so a definition reads the same in both. A step is a tool call,
//! a piece of writing by the model, a choice among fixed options, a value built from others, or a
//! container that picks (`if`, `switch`) or repeats (`for_each`, `while`) steps inside it. A
//! definition that passes [`validate`] can only fail when it runs, on data.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{expr, template};

pub const TOOL: &str = "tool";
pub const GENERATE: &str = "generate";
pub const DECIDE: &str = "decide";
pub const IF: &str = "if";
pub const SWITCH: &str = "switch";
pub const FOR_EACH: &str = "for_each";
pub const WHILE: &str = "while";
pub const SET: &str = "set";

const KINDS: [&str; 8] = [TOOL, GENERATE, DECIDE, IF, SWITCH, FOR_EACH, WHILE, SET];

/// The tools a workflow step may call. A flow runs on its own, so it cannot stop to ask you.
pub const STEP_TOOLS: [&str; 5] = [
    crate::tools::SHELL,
    crate::web::NAME,
    crate::files::READ,
    crate::files::WRITE,
    crate::files::EDIT,
];

/// Steps in a definition, those inside containers counted.
pub const MAX_STEPS: usize = 50;
/// Containers inside containers.
pub const MAX_DEPTH: usize = 5;
const MAX_OPTIONS: usize = 20;
/// A loop's most rounds when it names no limit, and the most it may name.
pub const DEFAULT_ROUNDS: u32 = 100;
pub const MAX_ROUNDS: u32 = 1000;

/// Names a step may not bind: they mean something inside a body.
const RESERVED: [&str; 4] = ["inputs", "item", "index", "error"];

/// How a loop treats a round that fails.
pub const STOP: &str = "stop";
pub const CONTINUE: &str = "continue";

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

/// One case of a `switch`: the steps run when its value equals the one switched on.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Case {
    /// An expression; its value is compared with the `on` value.
    pub when: String,
    #[serde(default)]
    pub steps: Vec<Step>,
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
    // if, while: an expression that is true or false
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cond: Option<String>,
    // switch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cases: Vec<Case>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default: Vec<Step>,
    // for_each
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub over: Option<String>,
    /// What each item is called inside the body; `item` when not named.
    #[serde(default, rename = "as", skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_error: Option<String>,
    // while
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
    // set: each name is bound to an expression's value, as one object
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<BTreeMap<String, String>>,
    // bodies: if has then and else; for_each and while have steps
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub then: Vec<Step>,
    #[serde(default, rename = "else", skip_serializing_if = "Vec::is_empty")]
    pub otherwise: Vec<Step>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
}

impl Step {
    /// The name the body of a `for_each` calls its item.
    pub fn item_name(&self) -> &str {
        self.item
            .as_deref()
            .filter(|n| !n.is_empty())
            .unwrap_or("item")
    }

    /// Fields this step has filled in, by name, for telling which ones its kind does not take.
    fn filled(&self) -> Vec<&'static str> {
        let fields: [(&str, bool); 18] = [
            ("tool", self.tool.is_some()),
            ("args", self.args.is_some()),
            ("instruction", self.instruction.is_some()),
            ("question", self.question.is_some()),
            ("options", !self.options.is_empty()),
            ("input", self.input.is_some()),
            ("cond", self.cond.is_some()),
            ("on", self.on.is_some()),
            ("cases", !self.cases.is_empty()),
            ("default", !self.default.is_empty()),
            ("over", self.over.is_some()),
            ("as", self.item.is_some()),
            ("on_error", self.on_error.is_some()),
            ("max", self.max.is_some()),
            ("values", self.values.is_some()),
            ("then", !self.then.is_empty()),
            ("else", !self.otherwise.is_empty()),
            ("steps", !self.steps.is_empty()),
        ];
        fields
            .into_iter()
            .filter_map(|(name, on)| on.then_some(name))
            .collect()
    }
}

/// The fields each kind takes besides `id`, `kind` and `bind`.
fn takes(kind: &str) -> &'static [&'static str] {
    match kind {
        TOOL => &["tool", "args"],
        GENERATE => &["instruction"],
        DECIDE => &["question", "options", "input"],
        IF => &["cond", "then", "else"],
        SWITCH => &["on", "cases", "default"],
        FOR_EACH => &["over", "as", "on_error", "steps"],
        WHILE => &["cond", "max", "steps"],
        SET => &["values"],
        _ => &[],
    }
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

// ---- walking the tree

/// Where a block of steps sits: the path of a step is its position among its block's steps, and
/// of a step inside another it is the step's path, the branch, then the position (`2.then.0`).
pub fn child(block: &str, i: usize) -> String {
    if block.is_empty() {
        i.to_string()
    } else {
        format!("{block}.{i}")
    }
}

pub fn branch(step: &str, name: &str) -> String {
    format!("{step}.{name}")
}

/// The name of a `switch` case's branch.
pub fn case_branch(i: usize) -> String {
    format!("case{i}")
}

/// Every step in `steps`, containers before what is inside them.
pub fn each_step<'a>(steps: &'a [Step], f: &mut impl FnMut(&'a Step)) {
    for s in steps {
        f(s);
        each_step(&s.then, f);
        each_step(&s.otherwise, f);
        for c in &s.cases {
            each_step(&c.steps, f);
        }
        each_step(&s.default, f);
        each_step(&s.steps, f);
    }
}

// ---- checking

struct Check<'a> {
    def: &'a Definition,
    bad: Vec<String>,
    ids: BTreeSet<String>,
    binds: BTreeSet<String>,
    count: usize,
}

impl Check<'_> {
    /// Names an expression may use here: the inputs, what is bound so far, and `extra`.
    fn names<'v>(vis: &'v [String], extra: &[&'v str]) -> Vec<&'v str> {
        std::iter::once("inputs")
            .chain(vis.iter().map(String::as_str))
            .chain(extra.iter().copied())
            .collect()
    }

    fn expression(&mut self, at: &str, field: &str, src: Option<&str>, names: &[&str]) {
        match src {
            None | Some("") => self.bad.push(format!("{at}: a {field} is needed")),
            Some(src) => {
                if let Err(why) = expr::check(names, src) {
                    self.bad.push(format!("{at}: {field} {src:?}: {why}"));
                }
            }
        }
    }

    /// References in text must name an input that is declared or something bound earlier, and an
    /// expression in text must read as one over the same names.
    fn references(&mut self, at: &str, refs: &[template::Ref], vis: &[String]) {
        for r in refs {
            if r.expr {
                let names = Self::names(vis, &[]);
                if let Err(why) = expr::check(&names, &r.path) {
                    self.bad.push(format!("{at}: {{{{= {}}}}}: {why}", r.path));
                }
                continue;
            }
            let path = &r.path;
            let root = path.split('.').next().unwrap_or_default();
            if root == "inputs" {
                let key = path.split('.').nth(1).unwrap_or_default();
                if !self.def.inputs.contains_key(key) {
                    self.bad.push(format!(
                        "{at}: {{{{{path}}}}} names an input that is not declared"
                    ));
                }
            } else if !vis.iter().any(|v| v == root) {
                self.bad.push(format!(
                    "{at}: {{{{{path}}}}} refers to {root:?}, which no earlier step binds"
                ));
            }
        }
    }

    /// A block of steps. What they bind is seen by later steps of the block, and by nothing outside.
    fn block(
        &mut self,
        steps: &[Step],
        prefix: &str,
        mut vis: Vec<String>,
        depth: usize,
    ) -> Vec<String> {
        for (i, s) in steps.iter().enumerate() {
            let label = if prefix.is_empty() {
                (i + 1).to_string()
            } else {
                format!("{prefix}.{}", i + 1)
            };
            self.step(s, &label, &mut vis, depth);
        }
        vis
    }

    /// A body: a block that also sees `extra`, and that has to have something in it. Gives what is
    /// in sight at its end.
    fn body(
        &mut self,
        at: &str,
        steps: &[Step],
        label: &str,
        vis: &[String],
        extra: &[&str],
        depth: usize,
    ) -> Vec<String> {
        if steps.is_empty() {
            self.bad
                .push(format!("{at}: needs at least one step inside"));
        }
        let mut inside = vis.to_vec();
        inside.extend(extra.iter().map(|e| (*e).to_owned()));
        self.block(steps, label, inside, depth)
    }

    fn step(&mut self, s: &Step, label: &str, vis: &mut Vec<String>, depth: usize) {
        self.count += 1;
        let at = format!("step {label} ({:?})", s.id);
        if !name_ok(&s.id, true) {
            self.bad.push(format!(
                "{at}: an id is letters, digits, - and _, starting with a letter"
            ));
        } else if !self.ids.insert(s.id.clone()) {
            self.bad.push(format!("{at}: another step has this id"));
        }
        if !KINDS.contains(&s.kind.as_str()) {
            self.bad.push(format!(
                "{at}: {:?} is not a step kind; they are {}",
                s.kind,
                KINDS.join(", ")
            ));
        } else {
            let taken = takes(&s.kind);
            for field in s.filled() {
                if !taken.contains(&field) {
                    self.bad
                        .push(format!("{at}: a {} step has no {field}", s.kind));
                }
            }
        }
        let container = matches!(s.kind.as_str(), IF | SWITCH | FOR_EACH | WHILE);
        if container && depth >= MAX_DEPTH {
            self.bad.push(format!(
                "{at}: steps are inside each other more than {MAX_DEPTH} deep"
            ));
            return;
        }
        let below = depth + 1;
        let mut refs: Vec<template::Ref> = Vec::new();
        match s.kind.as_str() {
            TOOL => {
                match s.tool.as_deref() {
                    None | Some("") => self.bad.push(format!("{at}: a tool step needs a tool")),
                    Some(t) if !STEP_TOOLS.contains(&t) => self.bad.push(format!(
                        "{at}: {t:?} is not a tool a workflow can use; they are {}",
                        STEP_TOOLS.join(", ")
                    )),
                    _ => {}
                }
                if let Some(args) = &s.args {
                    template::refs_in(&Value::Object(args.clone()), &mut refs);
                }
            }
            GENERATE => match s.instruction.as_deref() {
                None | Some("") => self
                    .bad
                    .push(format!("{at}: a generate step needs an instruction")),
                Some(t) => refs.extend(template::find_refs(t)),
            },
            DECIDE => {
                match s.question.as_deref() {
                    None | Some("") => self
                        .bad
                        .push(format!("{at}: a decide step needs a question")),
                    Some(q) if !template::find_refs(q).is_empty() => {
                        self.bad.push(format!(
                            "{at}: the question is fixed text; put the value to judge in `input`"
                        ));
                    }
                    _ => {}
                }
                let opts: BTreeSet<&String> = s.options.iter().collect();
                if s.options.len() < 2 || s.options.len() > MAX_OPTIONS {
                    self.bad.push(format!(
                        "{at}: a decide step needs 2 to {MAX_OPTIONS} options"
                    ));
                } else if opts.len() != s.options.len()
                    || s.options.iter().any(|o| o.trim().is_empty())
                {
                    self.bad
                        .push(format!("{at}: the options must be different and not empty"));
                }
                match s.input.as_deref() {
                    None | Some("") => self
                        .bad
                        .push(format!("{at}: a decide step needs an input to judge")),
                    Some(t) => refs.extend(template::find_refs(t)),
                }
                if s.bind.as_deref().is_none_or(str::is_empty) {
                    self.bad.push(format!("{at}: a decide step needs a bind: its answer is only useful when something reads it"));
                }
            }
            IF => {
                let names = Self::names(vis, &[]);
                self.expression(&at, "cond", s.cond.as_deref(), &names);
                self.body(&at, &s.then, label, vis, &[], below);
                if !s.otherwise.is_empty() {
                    self.block(&s.otherwise, label, vis.clone(), below);
                }
            }
            SWITCH => {
                let names = Self::names(vis, &[]);
                self.expression(&at, "on", s.on.as_deref(), &names);
                if s.cases.is_empty() || s.cases.len() > MAX_OPTIONS {
                    self.bad
                        .push(format!("{at}: a switch needs 1 to {MAX_OPTIONS} cases"));
                }
                for (n, c) in s.cases.iter().enumerate() {
                    let at_case = format!("{at}, case {}", n + 1);
                    self.expression(&at_case, "when", Some(c.when.as_str()), &names);
                    self.body(&at_case, &c.steps, label, vis, &[], below);
                }
                if !s.default.is_empty() {
                    self.block(&s.default, label, vis.clone(), below);
                }
            }
            FOR_EACH => {
                let names = Self::names(vis, &[]);
                self.expression(&at, "over", s.over.as_deref(), &names);
                if let Some(n) = s.item.as_deref().filter(|n| !n.is_empty())
                    && (!name_ok(n, false) || RESERVED.contains(&n) && n != "item")
                {
                    self.bad.push(format!(
                        "{at}: the item name {n:?} must be letters, digits and underscores, and not one the engine uses"
                    ));
                }
                if let Some(v) = s.on_error.as_deref()
                    && v != STOP
                    && v != CONTINUE
                {
                    self.bad
                        .push(format!("{at}: on_error is {STOP} or {CONTINUE}, not {v:?}"));
                }
                let item = s.item_name().to_owned();
                self.body(&at, &s.steps, label, vis, &[item.as_str(), "index"], below);
            }
            WHILE => {
                // The body runs first and the condition then reads what it bound.
                let end = self.body(&at, &s.steps, label, vis, &["index"], below);
                let names: Vec<&str> = std::iter::once("inputs")
                    .chain(end.iter().map(String::as_str))
                    .collect();
                self.expression(&at, "cond", s.cond.as_deref(), &names);
                if s.max.is_some_and(|m| m == 0 || m > MAX_ROUNDS) {
                    self.bad
                        .push(format!("{at}: max is 1 to {MAX_ROUNDS} rounds"));
                }
            }
            SET => {
                let names = Self::names(vis, &[]);
                match &s.values {
                    None => self.bad.push(format!("{at}: a set step needs values")),
                    Some(v) if v.is_empty() => {
                        self.bad.push(format!("{at}: a set step needs values"));
                    }
                    Some(values) => {
                        for (name, src) in values {
                            if !name_ok(name, false) {
                                self.bad.push(format!("{at}: value name {name:?} must be letters, digits and underscores"));
                            }
                            self.expression(&at, &format!("value {name}"), Some(src), &names);
                        }
                    }
                }
                if s.bind.as_deref().is_none_or(str::is_empty) {
                    self.bad.push(format!(
                        "{at}: a set step needs a bind: its values are only useful when something reads them"
                    ));
                }
            }
            _ => {}
        }
        self.references(&at, &refs, vis);
        if let Some(b) = s.bind.as_deref().filter(|b| !b.is_empty()) {
            if !name_ok(b, false) || RESERVED.contains(&b) {
                self.bad.push(format!(
                    "{at}: bind {b:?} must be letters, digits and underscores, and not {}",
                    RESERVED.join(", ")
                ));
            } else if !self.binds.insert(b.to_owned()) {
                self.bad.push(format!("{at}: another step binds {b:?}"));
            } else {
                vis.push(b.to_owned());
            }
        }
    }
}

/// Every structural problem in `def`, all at once, so one who fixes it gets the whole list.
pub fn validate(def: &Definition) -> Result<(), Vec<String>> {
    let mut check = Check {
        def,
        bad: Vec::new(),
        ids: BTreeSet::new(),
        binds: BTreeSet::new(),
        count: 0,
    };
    for (name, input) in &def.inputs {
        if !name_ok(name, false) {
            check.bad.push(format!("input {name:?}: a name is letters, digits and underscores, starting with a letter or underscore"));
        }
        if !matches!(input.kind.as_str(), "string" | "number" | "boolean") {
            check.bad.push(format!(
                "input {name:?}: type {:?} is not string, number or boolean",
                input.kind
            ));
        } else if input
            .default
            .as_ref()
            .is_some_and(|d| !type_ok(d, &input.kind))
        {
            check.bad.push(format!(
                "input {name:?}: the default is not a {}",
                input.kind
            ));
        }
    }
    if def.steps.is_empty() {
        check
            .bad
            .push("a workflow needs at least one step".to_owned());
    }
    check.block(&def.steps, "", Vec::new(), 0);
    if check.count > MAX_STEPS {
        check.bad.push(format!(
            "{} steps is more than the {MAX_STEPS} a workflow may have",
            check.count
        ));
    }
    if check.bad.is_empty() {
        Ok(())
    } else {
        Err(check.bad)
    }
}

/// The tools a definition calls, without repeats.
pub fn tools_used(def: &Definition) -> Vec<String> {
    let mut seen = Vec::new();
    each_step(&def.steps, &mut |s| {
        if let Some(t) = &s.tool
            && !seen.contains(t)
        {
            seen.push(t.clone());
        }
    });
    seen
}

fn short(t: &str) -> String {
    let line = t.lines().next().unwrap_or_default();
    let (cut, more) = crate::runs::clip(line, 90);
    if more || t.lines().count() > 1 {
        format!("{cut}…")
    } else {
        cut
    }
}

/// A step as one line a person can read: what it does and to what.
pub fn describe(s: &Step) -> String {
    let text = |o: &Option<String>| short(o.as_deref().unwrap_or_default());
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
        GENERATE => format!("Write: {}", text(&s.instruction)),
        DECIDE => format!("Decide: {} ({})", text(&s.question), s.options.join(" / ")),
        IF => format!("If {}", text(&s.cond)),
        SWITCH => format!("Switch on {}", text(&s.on)),
        FOR_EACH => format!("For each {} in {}", s.item_name(), text(&s.over)),
        WHILE => format!(
            "Repeat while {} (at most {} rounds)",
            text(&s.cond),
            s.max.unwrap_or(DEFAULT_ROUNDS)
        ),
        SET => format!(
            "Set {}",
            s.values
                .as_ref()
                .map(|v| v.keys().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_default()
        ),
        other => other.to_owned(),
    }
}

/// One line of a workflow laid out as it nests: how deep, where it is (the path a run's step
/// log uses, or a branch's), and what to say.
#[derive(Clone, PartialEq, Debug)]
pub struct Row {
    pub depth: usize,
    pub path: String,
    pub label: String,
}

/// The steps as an outline, with a line for each branch of an `if` or `switch`.
pub fn outline(steps: &[Step]) -> Vec<Row> {
    fn walk(steps: &[Step], block: &str, depth: usize, out: &mut Vec<Row>) {
        for (i, s) in steps.iter().enumerate() {
            let path = child(block, i);
            out.push(Row {
                depth,
                path: path.clone(),
                label: describe(s),
            });
            let mut part = |name: &str, label: String, inner: &[Step]| {
                let at = branch(&path, name);
                out.push(Row {
                    depth: depth + 1,
                    path: at.clone(),
                    label,
                });
                walk(inner, &at, depth + 2, out);
            };
            match s.kind.as_str() {
                IF => {
                    part("then", "then".to_owned(), &s.then);
                    if !s.otherwise.is_empty() {
                        part("else", "otherwise".to_owned(), &s.otherwise);
                    }
                }
                SWITCH => {
                    for (n, c) in s.cases.iter().enumerate() {
                        part(
                            &case_branch(n),
                            format!("when {}", short(&c.when)),
                            &c.steps,
                        );
                    }
                    if !s.default.is_empty() {
                        part("default", "otherwise".to_owned(), &s.default);
                    }
                }
                FOR_EACH | WHILE => walk(&s.steps, &branch(&path, "body"), depth + 1, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(steps, "", 0, &mut out);
    out
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

    fn tree() -> Value {
        json!({
            "inputs": {"folder": {"type": "string", "default": "notes"}},
            "steps": [
                {"id": "ls", "kind": "tool", "tool": "shell", "args": {"command": "ls {{inputs.folder}}"}, "bind": "files"},
                {"id": "any", "kind": "if", "cond": "files.exit_code == 0",
                 "then": [{"id": "each", "kind": "for_each", "over": "files.stdout.split(\"\\n\")", "as": "name",
                           "steps": [{"id": "show", "kind": "tool", "tool": "shell", "args": {"command": "echo {{name}} {{index}}"}}]}],
                 "else": [{"id": "none", "kind": "generate", "instruction": "Say {{files.stdout}} is empty"}]},
                {"id": "pick", "kind": "switch", "on": "inputs.folder",
                 "cases": [{"when": "\"notes\"", "steps": [{"id": "n", "kind": "set", "values": {"a": "1"}, "bind": "made"}]}],
                 "default": [{"id": "d", "kind": "set", "values": {"a": "2"}, "bind": "other"}]},
                {"id": "wait", "kind": "while", "cond": "poll.exit_code != 0 && index < 3", "max": 5,
                 "steps": [{"id": "tick", "kind": "tool", "tool": "shell", "args": {"command": "check"}, "bind": "poll"}]}
            ]
        })
    }

    #[test]
    fn containers_validate_round_trip_and_show_as_an_outline() {
        let d = def(tree());
        assert_eq!(validate(&d), Ok(()));
        let again: Definition = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(again, d);
        assert_eq!(tools_used(&d), ["shell"]);
        let rows: Vec<(usize, String, String)> = outline(&d.steps)
            .into_iter()
            .map(|r| (r.depth, r.path, r.label))
            .collect();
        let labels: Vec<&str> = rows.iter().map(|r| r.2.as_str()).collect();
        assert_eq!(labels[1], "If files.exit_code == 0");
        assert_eq!(labels[2], "then");
        assert!(
            labels[3].starts_with("For each name in files.stdout.split"),
            "{labels:?}"
        );
        assert!(rows.iter().any(|r| r.1 == "1.then.0.body.0" && r.0 == 3));
        assert!(
            rows.iter()
                .any(|r| r.1 == "2.case0" && r.2 == "when \"notes\"")
        );
        assert!(
            rows.iter()
                .any(|r| r.1 == "2.default" && r.2 == "otherwise")
        );
    }

    #[test]
    fn what_a_body_binds_stays_inside_it_and_a_container_bind_comes_out() {
        let mut v = tree();
        v["steps"][1]["bind"] = json!("chosen");
        v["steps"].as_array_mut().unwrap().push(json!(
            {"id": "after", "kind": "generate", "instruction": "{{chosen}} and {{made}}"}
        ));
        let p = problems(v);
        assert!(p.contains("refers to \"made\""), "{p}");
        assert!(!p.contains("refers to \"chosen\""), "{p}");
        // A body sees what came before it, and an item only inside its own loop.
        let p = problems(json!({"steps": [
            {"id": "l", "kind": "for_each", "over": "[1]", "steps": [
                {"id": "a", "kind": "generate", "instruction": "{{item}}"}]},
            {"id": "b", "kind": "generate", "instruction": "{{item}}"}
        ]}));
        assert!(
            p.contains("step 2 (\"b\")") && p.contains("refers to \"item\""),
            "{p}"
        );
        assert!(!p.contains("step 1.1"), "{p}");
    }

    #[test]
    fn every_container_problem_is_listed_with_where_it_is() {
        let p = problems(json!({"steps": [
            {"id": "i", "kind": "if", "cond": "nope > 1", "then": []},
            {"id": "s", "kind": "switch", "on": "1 +", "cases": []},
            {"id": "f", "kind": "for_each", "as": "inputs", "on_error": "shrug", "steps": []},
            {"id": "w", "kind": "while", "cond": "true", "max": 5000,
             "steps": [{"id": "x", "kind": "generate", "instruction": "x", "cond": "true"}]},
            {"id": "t", "kind": "set"},
            {"id": "g", "kind": "tool", "tool": "shell", "cond": "true"}
        ]}));
        for expected in [
            "step 1 (\"i\"): cond \"nope > 1\"",
            "needs at least one step inside",
            "step 2 (\"s\"): on \"1 +\"",
            "a switch needs 1 to 20 cases",
            "item name \"inputs\"",
            "on_error is stop or continue",
            "max is 1 to 1000 rounds",
            "step 4.1 (\"x\"): a generate step has no cond",
            "a set step needs values",
            "a tool step has no cond",
        ] {
            assert!(p.contains(expected), "missing {expected:?} in:\n{p}");
        }
    }

    #[test]
    fn nesting_and_size_are_capped() {
        let mut inner = json!({"id": "leaf", "kind": "generate", "instruction": "x"});
        for n in 0..6 {
            inner = json!({"id": format!("c{n}"), "kind": "if", "cond": "true", "then": [inner]});
        }
        let p = problems(json!({"steps": [inner]}));
        assert!(p.contains("more than 5 deep"), "{p}");
        let many: Vec<Value> = (0..51)
            .map(|n| json!({"id": format!("s{n}"), "kind": "generate", "instruction": "x"}))
            .collect();
        assert!(problems(json!({"steps": many})).contains("51 steps is more than the 50"));
    }

    #[test]
    fn an_old_linear_definition_reads_and_writes_exactly_as_before() {
        let text = r#"{"inputs":{"n":{"type":"number","default":2}},"steps":[{"id":"a","kind":"tool","bind":"r","tool":"shell","args":{"command":"ls"}},{"id":"b","kind":"generate","instruction":"say {{r}}"}],"success":"done"}"#;
        let d: Definition = serde_json::from_str(text).unwrap();
        assert_eq!(validate(&d), Ok(()));
        assert_eq!(serde_json::to_string(&d).unwrap(), text);
    }
}
