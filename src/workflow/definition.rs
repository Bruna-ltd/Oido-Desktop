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
pub const TRY: &str = "try";
pub const FLOW: &str = "flow";
pub const CODE: &str = "code";
pub const WAIT: &str = "wait";
pub const PARALLEL: &str = "parallel";

const KINDS: [&str; 13] = [
    TOOL, GENERATE, DECIDE, IF, SWITCH, FOR_EACH, WHILE, SET, TRY, FLOW, CODE, WAIT, PARALLEL,
];

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
/// Most times a `try` may run its steps again after they fail.
pub const MAX_RETRIES: u32 = 10;
/// Longest a `try` may wait between attempts, in milliseconds.
pub const MAX_BACKOFF_MS: u32 = 60_000;
/// How deep workflows may call each other.
pub const MAX_FLOW_DEPTH: usize = 5;
/// A delay up to this long is waited out where the run is; a longer one is saved and the run goes
/// on when it is over, even after the app has been closed.
pub const INLINE_WAIT: u32 = 120;
/// The longest a `wait` may be, in seconds: a week.
pub const MAX_WAIT: u32 = 604_800;
/// Most branches one `parallel` step may run at once.
pub const MAX_BRANCHES: usize = 8;

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

/// One branch of a `parallel` step: a name for its result, and the steps it runs.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Branch {
    pub name: String,
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
    // try: steps is what is tried; catch runs when it still fails, and sees `error`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub catch: Vec<Step>,
    // flow: another workflow of the same agent, by name, and what to give it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Map<String, Value>>,
    // code: a script over what is in scope; its last value is the result
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    // wait: a delay in seconds, or a question to answer before going on
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approve: Option<String>,
    // parallel: each branch runs at once; the result is an object of branch name to its result
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<Branch>,
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
        let fields: [(&str, bool); 27] = [
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
            ("retries", self.retries.is_some()),
            ("backoff_ms", self.backoff_ms.is_some()),
            ("catch", !self.catch.is_empty()),
            ("workflow", self.workflow.is_some()),
            ("inputs", self.inputs.is_some()),
            ("script", self.script.is_some()),
            ("seconds", self.seconds.is_some()),
            ("approve", self.approve.is_some()),
            ("branches", !self.branches.is_empty()),
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
        TRY => &["retries", "backoff_ms", "steps", "catch"],
        FLOW => &["workflow", "inputs"],
        CODE => &["script"],
        WAIT => &["seconds", "approve"],
        PARALLEL => &["branches", "on_error"],
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
        each_step(&s.catch, f);
        for b in &s.branches {
            each_step(&b.steps, f);
        }
    }
}

// ---- checking

struct Check<'a> {
    def: &'a Definition,
    bad: Vec<String>,
    ids: BTreeSet<String>,
    binds: BTreeSet<String>,
    count: usize,
    /// How many `parallel` branches the step being checked is inside.
    parallel: usize,
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
        let container = matches!(
            s.kind.as_str(),
            IF | SWITCH | FOR_EACH | WHILE | TRY | FLOW | PARALLEL
        );
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
                    Some(t)
                        if !STEP_TOOLS.contains(&t)
                            && !crate::mcp::is_tool(t)
                            && t != crate::tools::DELEGATE =>
                    {
                        self.bad.push(format!(
                            "{at}: {t:?} is not a tool a workflow can use; they are {}, or an MCP tool such as mcp__server__tool",
                            STEP_TOOLS.join(", ")
                        ));
                    }
                    _ => {}
                }
                if let Some(args) = &s.args {
                    template::refs_in(&Value::Object(args.clone()), &mut refs);
                }
                if s.tool.as_deref() == Some(crate::tools::DELEGATE) {
                    // The team member is fixed, so it is known and ready before the run starts.
                    let who = s
                        .args
                        .as_ref()
                        .and_then(|a| a.get("agent"))
                        .and_then(Value::as_str);
                    if who.is_none_or(|w| w.trim().is_empty() || !template::find_refs(w).is_empty())
                    {
                        self.bad.push(format!(
                            "{at}: delegate needs `agent`, the name of a team member, as fixed text"
                        ));
                    }
                    if s.args
                        .as_ref()
                        .and_then(|a| a.get("task"))
                        .and_then(Value::as_str)
                        .is_none_or(|t| t.trim().is_empty())
                    {
                        self.bad.push(format!(
                            "{at}: delegate needs `task`, what the team member is to do"
                        ));
                    }
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
            TRY => {
                if s.retries.is_some_and(|r| r > MAX_RETRIES) {
                    self.bad
                        .push(format!("{at}: retries is 0 to {MAX_RETRIES}"));
                }
                if s.backoff_ms.is_some_and(|b| b > MAX_BACKOFF_MS) {
                    self.bad.push(format!(
                        "{at}: the wait between attempts is at most {MAX_BACKOFF_MS} milliseconds"
                    ));
                }
                self.body(&at, &s.steps, label, vis, &[], below);
                if !s.catch.is_empty() {
                    self.block(
                        &s.catch,
                        label,
                        [vis.as_slice(), &["error".to_owned()]].concat(),
                        below,
                    );
                }
            }
            PARALLEL => {
                if s.branches.is_empty() || s.branches.len() > MAX_BRANCHES {
                    self.bad.push(format!(
                        "{at}: a parallel step needs 1 to {MAX_BRANCHES} branches"
                    ));
                }
                if let Some(v) = s.on_error.as_deref()
                    && v != STOP
                    && v != CONTINUE
                {
                    self.bad
                        .push(format!("{at}: on_error is {STOP} or {CONTINUE}, not {v:?}"));
                }
                let mut named = BTreeSet::new();
                for (n, b) in s.branches.iter().enumerate() {
                    let at_branch = format!("{at}, branch {}", n + 1);
                    if !name_ok(&b.name, false) {
                        self.bad.push(format!(
                            "{at_branch}: a name is letters, digits and underscores, starting with a letter or underscore"
                        ));
                    } else if !named.insert(b.name.as_str()) {
                        self.bad
                            .push(format!("{at_branch}: another branch has this name"));
                    }
                    self.parallel += 1;
                    self.body(&at_branch, &b.steps, label, vis, &[], below);
                    self.parallel -= 1;
                }
            }
            WAIT => match (s.seconds, s.approve.as_deref().map(str::trim)) {
                (Some(_), Some(_)) => self
                    .bad
                    .push(format!("{at}: a wait is a delay or an approval, not both")),
                (None, None | Some("")) => self.bad.push(format!(
                    "{at}: a wait needs seconds to wait, or text to ask you before going on"
                )),
                (Some(n), None) if n == 0 || n > MAX_WAIT => self
                    .bad
                    .push(format!("{at}: seconds is 1 to {MAX_WAIT}, a week")),
                (Some(n), None) if n > INLINE_WAIT && self.parallel > 0 => self.bad.push(format!(
                    "{at}: a wait longer than {INLINE_WAIT} seconds pauses the run, which a parallel branch cannot do"
                )),
                (Some(_), None) => {}
                (None, Some(_)) if self.parallel > 0 => self.bad.push(format!(
                    "{at}: asking you pauses the run, which a parallel branch cannot do"
                )),
                (None, Some(_)) => refs.extend(template::find_refs(
                    s.approve.as_deref().unwrap_or_default(),
                )),
            },
            CODE => {
                let names = Self::names(vis, &[]);
                match s.script.as_deref() {
                    None => self.bad.push(format!("{at}: a code step needs a script")),
                    Some(src) => {
                        if let Err(why) = expr::check_script(&names, src) {
                            self.bad.push(format!("{at}: script: {why}"));
                        }
                    }
                }
            }
            FLOW => {
                match s.workflow.as_deref().map(str::trim) {
                    None | Some("") => self
                        .bad
                        .push(format!("{at}: a flow step needs the name of a workflow")),
                    Some(_) => {}
                }
                if let Some(given) = &s.inputs {
                    template::refs_in(&Value::Object(given.clone()), &mut refs);
                }
                if s.bind.as_deref().is_none_or(str::is_empty) {
                    self.bad.push(format!(
                        "{at}: a flow step needs a bind: what the workflow gives is only useful when something reads it"
                    ));
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
        parallel: 0,
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

/// Names in an expression, for telling what it reads. Words inside quotes count too, which only
/// makes a check that uses this more careful.
fn words(src: &str, out: &mut BTreeSet<String>) {
    let mut word = String::new();
    for c in src.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            if word.starts_with(|f: char| f.is_ascii_alphabetic() || f == '_') {
                out.insert(std::mem::take(&mut word));
            }
            word.clear();
        }
    }
}

/// What a step's own fields read: the first name of each `{{path}}`, and every name in an
/// expression. Steps inside it are not looked into.
pub fn reads(s: &Step) -> BTreeSet<String> {
    let mut refs = Vec::new();
    for given in [&s.args, &s.inputs].into_iter().flatten() {
        template::refs_in(&Value::Object(given.clone()), &mut refs);
    }
    for t in [&s.instruction, &s.input, &s.approve].into_iter().flatten() {
        refs.extend(template::find_refs(t));
    }
    let mut out = BTreeSet::new();
    for r in refs {
        if r.expr {
            words(&r.path, &mut out);
        } else if let Some(root) = r.path.split('.').next() {
            out.insert(root.to_owned());
        }
    }
    for e in [&s.cond, &s.on, &s.over].into_iter().flatten() {
        words(e, &mut out);
    }
    for c in &s.cases {
        words(&c.when, &mut out);
    }
    for e in s.values.iter().flat_map(|v| v.values()) {
        words(e, &mut out);
    }
    if let Some(script) = &s.script {
        words(script, &mut out);
    }
    out
}

/// The workflows a definition runs, by the name each step gives, without repeats.
pub fn flows_used(def: &Definition) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    each_step(&def.steps, &mut |s| {
        if s.kind == FLOW
            && let Some(w) = s
                .workflow
                .as_deref()
                .map(str::trim)
                .filter(|w| !w.is_empty())
            && !seen.iter().any(|x| x.eq_ignore_ascii_case(w))
        {
            seen.push(w.to_owned());
        }
    });
    seen
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
                crate::tools::DELEGATE => match (arg("agent"), arg("task")) {
                    (Some(who), Some(task)) => Some(format!("{who}: {}", short(&task))),
                    (who, _) => who,
                },
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
        TRY => match (s.retries.unwrap_or(0), s.catch.is_empty()) {
            (0, true) => "Try".to_owned(),
            (0, false) => "Try, and handle a failure".to_owned(),
            (n, true) => format!("Try, again up to {n} times"),
            (n, false) => format!("Try, again up to {n} times, then handle a failure"),
        },
        FLOW => format!("Run the workflow {}", text(&s.workflow)),
        CODE => format!("Script: {}", text(&s.script)),
        PARALLEL => format!(
            "At once: {}",
            s.branches
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        WAIT => match s.seconds {
            Some(n) => format!("Wait {}", super::pause::span(n)),
            None => format!("Wait for your approval: {}", text(&s.approve)),
        },
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
                PARALLEL => {
                    for b in &s.branches {
                        part(&b.name, format!("branch {}", b.name), &b.steps);
                    }
                }
                TRY => {
                    part("body", "try".to_owned(), &s.steps);
                    if !s.catch.is_empty() {
                        part("catch", "on failure".to_owned(), &s.catch);
                    }
                }
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

    #[test]
    fn a_flow_step_needs_a_workflow_and_a_bind_and_its_inputs_may_only_look_back() {
        let ok = def(json!({"steps": [
            {"id": "a", "kind": "set", "bind": "x", "values": {"v": "1"}},
            {"id": "f", "kind": "flow", "workflow": "Other", "bind": "r", "inputs": {"n": "{{x.v}}"}}
        ]}));
        assert_eq!(validate(&ok), Ok(()));
        assert_eq!(flows_used(&ok), ["Other"]);
        assert_eq!(describe(&ok.steps[1]), "Run the workflow Other");
        let why = problems(json!({"steps": [
            {"id": "f", "kind": "flow", "inputs": {"n": "{{later}}"}},
            {"id": "b", "kind": "set", "bind": "later", "values": {"v": "1"}}
        ]}));
        for expected in ["needs the name of a workflow", "needs a bind", "later"] {
            assert!(why.contains(expected), "missing {expected:?} in {why}");
        }
        let why = problems(json!({"steps": [
            {"id": "f", "kind": "flow", "workflow": "W", "bind": "r", "cond": "true"}
        ]}));
        assert!(why.contains("a flow step has no cond"), "{why}");
    }

    #[test]
    fn handing_a_task_over_names_a_team_member_in_fixed_text_and_says_what_to_do() {
        let ok = def(json!({"steps": [
            {"id": "a", "kind": "tool", "tool": "delegate", "bind": "r",
             "args": {"agent": "Ann", "task": "Do {{inputs.x}}"}}
        ], "inputs": {"x": {"type": "string"}}}));
        assert_eq!(validate(&ok), Ok(()));
        assert_eq!(describe(&ok.steps[0]), "delegate: Ann: Do {{inputs.x}}");
        let why = problems(json!({"steps": [
            {"id": "a", "kind": "tool", "tool": "delegate", "args": {"agent": "{{inputs.x}}"}}
        ], "inputs": {"x": {"type": "string"}}}));
        assert!(
            why.contains("fixed text") && why.contains("needs `task`"),
            "{why}"
        );
    }

    #[test]
    fn a_code_step_needs_a_script_that_reads_and_names_only_what_is_in_sight() {
        let ok = def(json!({"steps": [
            {"id": "a", "kind": "set", "bind": "x", "values": {"v": "1"}},
            {"id": "c", "kind": "code", "bind": "r", "script": "let y = x.v + 1; y"}
        ]}));
        assert_eq!(validate(&ok), Ok(()));
        assert_eq!(describe(&ok.steps[1]), "Script: let y = x.v + 1; y");
        let why = problems(json!({"steps": [
            {"id": "c", "kind": "code", "script": "later + 1"},
            {"id": "d", "kind": "code", "script": "let = "},
            {"id": "e", "kind": "code"},
            {"id": "b", "kind": "set", "bind": "later", "values": {"v": "1"}}
        ]}));
        for expected in ["later", "script:", "needs a script"] {
            assert!(why.contains(expected), "missing {expected:?} in {why}");
        }
    }

    #[test]
    fn a_wait_is_a_delay_or_an_approval_with_text_that_may_only_look_back() {
        let ok = def(json!({"steps": [
            {"id": "a", "kind": "set", "bind": "x", "values": {"v": "1"}},
            {"id": "ask", "kind": "wait", "approve": "Go on with {{x.v}}?"},
            {"id": "nap", "kind": "wait", "seconds": 90}
        ]}));
        assert_eq!(validate(&ok), Ok(()));
        assert_eq!(
            describe(&ok.steps[1]),
            "Wait for your approval: Go on with {{x.v}}?"
        );
        assert_eq!(describe(&ok.steps[2]), "Wait 90 seconds");
        let why = problems(json!({"steps": [
            {"id": "a", "kind": "wait"},
            {"id": "b", "kind": "wait", "seconds": 5, "approve": "ok?"},
            {"id": "c", "kind": "wait", "seconds": 0},
            {"id": "d", "kind": "wait", "seconds": 700000},
            {"id": "e", "kind": "wait", "approve": "{{later}}?"},
            {"id": "f", "kind": "set", "bind": "later", "values": {"v": "1"}}
        ]}));
        for expected in [
            "a delay or an approval",
            "needs seconds",
            "1 to 604800",
            "later",
        ] {
            assert!(why.contains(expected), "missing {expected:?} in {why}");
        }
    }
}
