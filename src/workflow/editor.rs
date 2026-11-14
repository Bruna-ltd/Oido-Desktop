//! The workflow editor's state: a [`Definition`] as the text boxes and lists a person edits, and
//! back. Nothing here draws; `ui/workflows.rs` shows it. Turning it back into a definition says what
//! is wrong in words, so the screen can list the problems beside the steps.

use std::collections::BTreeMap;

use rune::TextEdit;
use serde_json::Value;

use super::Saved;
use super::definition::{
    Branch, CODE, CONTINUE, Case, DECIDE, Definition, FLOW, FOR_EACH, GENERATE, IF, Input,
    PARALLEL, SET, STEP_TOOLS, SWITCH, Step, TOOL, TRY, WAIT, WHILE, each_step, validate,
};

/// How a text box is checked and what can be put into it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refs {
    /// Plain text.
    None,
    /// Text that may hold `{{name}}` and `{{= expression}}`.
    Braces,
    /// An expression: a name goes in as it is.
    Bare,
}

/// One text box of a kind of step.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub key: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    /// Lines tall; one is a single-line box.
    pub lines: usize,
    pub refs: Refs,
}

const fn spec(
    key: &'static str,
    label: &'static str,
    hint: &'static str,
    lines: usize,
    refs: Refs,
) -> Spec {
    Spec {
        key,
        label,
        hint,
        lines,
        refs,
    }
}

static TOOL_BOXES: [Spec; 1] = [spec(
    "args",
    "Arguments (JSON)",
    r#"{"command": "ls {{inputs.folder}}"}"#,
    3,
    Refs::Braces,
)];
static GENERATE_BOXES: [Spec; 1] = [spec(
    "instruction",
    "Instruction",
    "What to write, with {{name}} for values",
    3,
    Refs::Braces,
)];
static DECIDE_BOXES: [Spec; 3] = [
    spec("question", "Question", "A fixed question", 1, Refs::None),
    spec("options", "Options, one per line", "yes\nno", 3, Refs::None),
    spec("input", "What to judge", "{{name}}", 1, Refs::Braces),
];
static IF_BOXES: [Spec; 1] = [spec(
    "cond",
    "Condition",
    "files.exit_code == 0",
    1,
    Refs::Bare,
)];
static SWITCH_BOXES: [Spec; 1] = [spec("on", "Switch on", "inputs.kind", 1, Refs::Bare)];
static FOR_EACH_BOXES: [Spec; 2] = [
    spec(
        "over",
        "Go over",
        "files.stdout.split(\"\\n\")",
        1,
        Refs::Bare,
    ),
    spec("as", "Call each item", "item", 1, Refs::None),
];
static WHILE_BOXES: [Spec; 2] = [
    spec(
        "cond",
        "Repeat while",
        "poll.stdout != \"ready\"",
        1,
        Refs::Bare,
    ),
    spec("max", "At most this many rounds", "100", 1, Refs::None),
];
static SET_BOXES: [Spec; 1] = [spec(
    "values",
    "Values, one per line: name = expression",
    "total = inputs.a + inputs.b",
    3,
    Refs::Bare,
)];

static TRY_BOXES: [Spec; 2] = [
    spec("retries", "Try again this many times", "0", 1, Refs::None),
    spec(
        "backoff_ms",
        "Wait between tries (milliseconds)",
        "0",
        1,
        Refs::None,
    ),
];

static FLOW_BOXES: [Spec; 2] = [
    spec(
        "workflow",
        "Workflow to run (its name)",
        "Weekly report",
        1,
        Refs::None,
    ),
    spec(
        "inputs",
        "What to give it (JSON)",
        r#"{"folder": "{{inputs.folder}}"}"#,
        3,
        Refs::Braces,
    ),
];

static CODE_BOXES: [Spec; 1] = [spec(
    "script",
    "Script (its last value is the result)",
    "let total = 0;\nfor n in files.stdout.split(\"\\n\") { total += n.len(); }\ntotal",
    8,
    Refs::Bare,
)];

static WAIT_BOXES: [Spec; 2] = [
    spec(
        "seconds",
        "Wait this many seconds (a long wait is saved, and the run goes on when it is over)",
        "3600",
        1,
        Refs::None,
    ),
    spec(
        "approve",
        "Or ask you first (shown with Approve and Decline)",
        "Send the report to {{inputs.who}}?",
        2,
        Refs::Braces,
    ),
];

/// The text boxes each kind of step has, in order.
pub fn specs(kind: &str) -> &'static [Spec] {
    match kind {
        TOOL => &TOOL_BOXES,
        GENERATE => &GENERATE_BOXES,
        DECIDE => &DECIDE_BOXES,
        IF => &IF_BOXES,
        SWITCH => &SWITCH_BOXES,
        FOR_EACH => &FOR_EACH_BOXES,
        WHILE => &WHILE_BOXES,
        SET => &SET_BOXES,
        TRY => &TRY_BOXES,
        FLOW => &FLOW_BOXES,
        CODE => &CODE_BOXES,
        WAIT => &WAIT_BOXES,
        _ => &[],
    }
}

/// The kinds of step a person can add, in the order the menu lists them.
pub const KINDS: [(&str, &str); 13] = [
    (TOOL, "Run a tool"),
    (GENERATE, "Write with the model"),
    (DECIDE, "Let the model choose"),
    (IF, "If"),
    (SWITCH, "Switch"),
    (FOR_EACH, "For each"),
    (WHILE, "Repeat while"),
    (SET, "Set values"),
    (TRY, "Try, and recover"),
    (FLOW, "Run another workflow"),
    (CODE, "Run a script"),
    (WAIT, "Wait or ask first"),
    (PARALLEL, "Run branches at once"),
];

/// What a list of steps inside a step is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Then,
    Else,
    Case,
    Default,
    Body,
    /// What a `try` tries.
    Try,
    /// What a `try` does when that still fails.
    Catch,
    /// One branch of a `parallel` step; its name is the block's `when`.
    Branch,
}

/// A list of steps inside a step, and for a `switch` case the value it is for.
#[derive(Clone, PartialEq, Debug)]
pub struct Block {
    pub part: Part,
    pub when: TextEdit,
    pub steps: Vec<Node>,
}

impl Block {
    fn new(part: Part) -> Block {
        Block {
            part,
            when: TextEdit::new(""),
            steps: Vec::new(),
        }
    }
}

/// One step as it is edited.
#[derive(Clone, PartialEq, Debug)]
pub struct Node {
    pub kind: String,
    pub id: TextEdit,
    pub bind: TextEdit,
    /// For a tool step.
    pub tool: String,
    /// For a loop: carry on when a round fails.
    pub go_on: bool,
    /// One per [`specs`] of the kind.
    pub fields: Vec<TextEdit>,
    pub blocks: Vec<Block>,
}

/// Where a list of steps is: through which step and which of its blocks, from the top. Empty is
/// the workflow's own steps.
pub type Place = Vec<(usize, usize)>;

/// A step: the list it is in, and where in it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Loc {
    pub place: Place,
    pub index: usize,
}

fn lines_of(t: &TextEdit) -> Vec<String> {
    t.text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

fn opt(t: &TextEdit) -> Option<String> {
    (!t.text.trim().is_empty()).then(|| t.text.clone())
}

impl Node {
    /// A new, empty step of `kind`.
    pub fn new(kind: &str, id: String) -> Node {
        let blocks = match kind {
            IF => vec![Block::new(Part::Then), Block::new(Part::Else)],
            SWITCH => vec![Block::new(Part::Case), Block::new(Part::Default)],
            FOR_EACH | WHILE => vec![Block::new(Part::Body)],
            TRY => vec![Block::new(Part::Try), Block::new(Part::Catch)],
            PARALLEL => vec![Block::new(Part::Branch), Block::new(Part::Branch)],
            _ => Vec::new(),
        };
        Node {
            kind: kind.to_owned(),
            id: TextEdit::new(id),
            bind: TextEdit::new(""),
            tool: if kind == TOOL {
                STEP_TOOLS[0].to_owned()
            } else {
                String::new()
            },
            go_on: false,
            fields: specs(kind).iter().map(|_| TextEdit::new("")).collect(),
            blocks,
        }
    }

    fn field_at(&self, key: &str) -> Option<&TextEdit> {
        let i = specs(&self.kind).iter().position(|s| s.key == key)?;
        self.fields.get(i)
    }

    fn set_field(&mut self, key: &str, text: impl Into<String>) {
        if let Some(i) = specs(&self.kind).iter().position(|s| s.key == key) {
            self.fields[i] = TextEdit::new(text);
        }
    }

    /// What a loop's body calls each item.
    pub fn item_name(&self) -> String {
        self.field_at("as")
            .map(|t| t.text.trim())
            .filter(|t| !t.is_empty())
            .unwrap_or("item")
            .to_owned()
    }

    pub fn from_step(s: &Step) -> Node {
        let mut n = Node::new(&s.kind, s.id.clone());
        n.bind = TextEdit::new(s.bind.clone().unwrap_or_default());
        let text = |o: &Option<String>| o.clone().unwrap_or_default();
        match s.kind.as_str() {
            TOOL => {
                n.tool = s.tool.clone().unwrap_or_default();
                let args = s.args.clone().unwrap_or_default();
                n.set_field(
                    "args",
                    if args.is_empty() {
                        String::new()
                    } else {
                        serde_json::to_string_pretty(&args).unwrap_or_default()
                    },
                );
            }
            GENERATE => n.set_field("instruction", text(&s.instruction)),
            DECIDE => {
                n.set_field("question", text(&s.question));
                n.set_field("options", s.options.join("\n"));
                n.set_field("input", text(&s.input));
            }
            IF => {
                n.set_field("cond", text(&s.cond));
                n.blocks[0].steps = s.then.iter().map(Node::from_step).collect();
                n.blocks[1].steps = s.otherwise.iter().map(Node::from_step).collect();
            }
            SWITCH => {
                n.set_field("on", text(&s.on));
                n.blocks = s
                    .cases
                    .iter()
                    .map(|c| Block {
                        part: Part::Case,
                        when: TextEdit::new(c.when.clone()),
                        steps: c.steps.iter().map(Node::from_step).collect(),
                    })
                    .collect();
                n.blocks.push(Block {
                    part: Part::Default,
                    when: TextEdit::new(""),
                    steps: s.default.iter().map(Node::from_step).collect(),
                });
            }
            FOR_EACH => {
                n.set_field("over", text(&s.over));
                n.set_field("as", text(&s.item));
                n.go_on = s.on_error.as_deref() == Some(CONTINUE);
                n.blocks[0].steps = s.steps.iter().map(Node::from_step).collect();
            }
            WHILE => {
                n.set_field("cond", text(&s.cond));
                n.set_field("max", s.max.map(|m| m.to_string()).unwrap_or_default());
                n.blocks[0].steps = s.steps.iter().map(Node::from_step).collect();
            }
            TRY => {
                n.set_field(
                    "retries",
                    s.retries.map(|r| r.to_string()).unwrap_or_default(),
                );
                n.set_field(
                    "backoff_ms",
                    s.backoff_ms.map(|r| r.to_string()).unwrap_or_default(),
                );
                n.blocks[0].steps = s.steps.iter().map(Node::from_step).collect();
                n.blocks[1].steps = s.catch.iter().map(Node::from_step).collect();
            }
            CODE => n.set_field("script", text(&s.script)),
            PARALLEL => {
                n.go_on = s.on_error.as_deref() == Some(CONTINUE);
                n.blocks = s
                    .branches
                    .iter()
                    .map(|b| Block {
                        part: Part::Branch,
                        when: TextEdit::new(b.name.clone()),
                        steps: b.steps.iter().map(Node::from_step).collect(),
                    })
                    .collect();
            }
            WAIT => {
                n.set_field(
                    "seconds",
                    s.seconds.map(|n| n.to_string()).unwrap_or_default(),
                );
                n.set_field("approve", text(&s.approve));
            }
            FLOW => {
                n.set_field("workflow", text(&s.workflow));
                let given = s.inputs.clone().unwrap_or_default();
                n.set_field(
                    "inputs",
                    if given.is_empty() {
                        String::new()
                    } else {
                        serde_json::to_string_pretty(&given).unwrap_or_default()
                    },
                );
            }
            SET => {
                let lines: Vec<String> = s
                    .values
                    .iter()
                    .flatten()
                    .map(|(k, v)| format!("{k} = {v}"))
                    .collect();
                n.set_field("values", lines.join("\n"));
            }
            _ => {}
        }
        n
    }

    /// The step this is, with what is wrong with its text boxes added to `bad`.
    pub fn to_step(&self, at: &str, bad: &mut Vec<String>) -> Step {
        let get = |key: &str| self.field_at(key).and_then(opt);
        let steps = |b: &Block, bad: &mut Vec<String>, label: &str| {
            b.steps
                .iter()
                .enumerate()
                .map(|(i, n)| n.to_step(&format!("{label}.{}", i + 1), bad))
                .collect::<Vec<_>>()
        };
        let mut s = Step {
            id: self.id.text.trim().to_owned(),
            kind: self.kind.clone(),
            bind: opt(&self.bind).map(|b| b.trim().to_owned()),
            ..Step::default()
        };
        match self.kind.as_str() {
            TOOL => {
                s.tool = Some(self.tool.clone()).filter(|t| !t.is_empty());
                if let Some(text) = get("args") {
                    match serde_json::from_str::<Value>(&text) {
                        Ok(Value::Object(m)) => s.args = Some(m).filter(|m| !m.is_empty()),
                        Ok(_) => bad.push(format!("step {at}: the arguments are a JSON object, like {{\"command\": \"ls\"}}")),
                        Err(e) => bad.push(format!("step {at}: the arguments are not JSON: {e}")),
                    }
                }
            }
            GENERATE => s.instruction = get("instruction"),
            DECIDE => {
                s.question = get("question");
                s.options = self.field_at("options").map(lines_of).unwrap_or_default();
                s.input = get("input");
            }
            IF => {
                s.cond = get("cond");
                s.then = steps(&self.blocks[0], bad, at);
                s.otherwise = steps(&self.blocks[1], bad, at);
            }
            SWITCH => {
                s.on = get("on");
                for b in &self.blocks {
                    if b.part == Part::Default {
                        s.default = steps(b, bad, at);
                    } else {
                        s.cases.push(Case {
                            when: b.when.text.clone(),
                            steps: steps(b, bad, at),
                        });
                    }
                }
            }
            FOR_EACH => {
                s.over = get("over");
                s.item = get("as").map(|a| a.trim().to_owned());
                s.on_error = self.go_on.then(|| CONTINUE.to_owned());
                s.steps = steps(&self.blocks[0], bad, at);
            }
            WHILE => {
                s.cond = get("cond");
                if let Some(m) = get("max") {
                    match m.trim().parse::<u32>() {
                        Ok(n) => s.max = Some(n),
                        Err(_) => bad.push(format!("step {at}: the most rounds is a whole number")),
                    }
                }
                s.steps = steps(&self.blocks[0], bad, at);
            }
            TRY => {
                let whole = |key: &str, what: &str, bad: &mut Vec<String>| {
                    let text = get(key)?;
                    text.trim()
                        .parse::<u32>()
                        .map_err(|_| bad.push(format!("step {at}: {what} is a whole number")))
                        .ok()
                };
                s.retries = whole("retries", "how many times to try again", bad);
                s.backoff_ms = whole("backoff_ms", "the wait between tries", bad);
                s.steps = steps(&self.blocks[0], bad, at);
                s.catch = steps(&self.blocks[1], bad, at);
            }
            CODE => s.script = get("script"),
            PARALLEL => {
                s.on_error = self.go_on.then(|| CONTINUE.to_owned());
                for b in &self.blocks {
                    s.branches.push(Branch {
                        name: b.when.text.trim().to_owned(),
                        steps: steps(b, bad, at),
                    });
                }
            }
            WAIT => {
                s.approve = get("approve");
                if let Some(n) = get("seconds") {
                    match n.trim().parse::<u32>() {
                        Ok(n) => s.seconds = Some(n),
                        Err(_) => bad.push(format!("step {at}: the seconds are a whole number")),
                    }
                }
            }
            FLOW => {
                s.workflow = get("workflow").map(|w| w.trim().to_owned());
                if let Some(text) = get("inputs") {
                    match serde_json::from_str::<Value>(&text) {
                        Ok(Value::Object(m)) => s.inputs = Some(m).filter(|m| !m.is_empty()),
                        Ok(_) => bad.push(format!("step {at}: what to give is a JSON object, like {{\"folder\": \"notes\"}}")),
                        Err(e) => bad.push(format!("step {at}: what to give is not JSON: {e}")),
                    }
                }
            }
            SET => {
                let mut values = BTreeMap::new();
                for line in self.field_at("values").map(lines_of).unwrap_or_default() {
                    match line.split_once('=') {
                        Some((name, src)) => {
                            values.insert(name.trim().to_owned(), src.trim().to_owned());
                        }
                        None => bad.push(format!(
                            "step {at}: write each value as name = expression, not {line:?}"
                        )),
                    }
                }
                s.values = Some(values).filter(|v| !v.is_empty());
            }
            _ => {}
        }
        s
    }
}

/// One of a workflow's inputs as it is edited.
#[derive(Clone, PartialEq, Debug)]
pub struct InputRow {
    pub name: TextEdit,
    /// `string`, `number` or `boolean`.
    pub kind: &'static str,
    pub description: TextEdit,
    pub default: TextEdit,
}

pub const INPUT_KINDS: [&str; 3] = ["string", "number", "boolean"];

/// What a box of the editor is, for finding it again when it is typed in.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Target {
    Name,
    Goal,
    Success,
    InputName(usize),
    InputDescription(usize),
    InputDefault(usize),
    Id(Loc),
    Bind(Loc),
    Field(Loc, usize),
    /// The value a `switch` case is for: its block.
    When(Loc, usize),
}

/// Which pop-up list of the editor is open; one at a time.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Menu {
    Agent,
    InputKind(usize),
    /// Add a step at the end of this list.
    Add(Place),
    /// Add a step at this position in this list.
    AddAt(Place, usize),
    Tool(Loc),
    /// Names to put into a box.
    Names(Target),
}

/// A workflow being made or changed.
#[derive(Debug)]
pub struct Editor {
    /// The workflow being changed; `None` for a new one.
    pub workflow: Option<String>,
    pub agent_id: String,
    pub name: TextEdit,
    pub goal: TextEdit,
    pub success: TextEdit,
    pub inputs: Vec<InputRow>,
    pub steps: Vec<Node>,
    /// What is wrong, as of the last check.
    pub problems: Vec<String>,
    /// The steps as they were before each change to their structure, newest last, with what the
    /// change was. Typing in a box is not one: it has the box's own undo.
    undo: Vec<(String, Vec<Node>)>,
    redo: Vec<(String, Vec<Node>)>,
}

/// How many changes can be taken back.
const HISTORY: usize = 200;

impl Editor {
    pub fn blank(agent_id: &str) -> Editor {
        Editor {
            workflow: None,
            agent_id: agent_id.to_owned(),
            name: TextEdit::new(""),
            goal: TextEdit::new(""),
            success: TextEdit::new(""),
            inputs: Vec::new(),
            steps: Vec::new(),
            problems: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn from_saved(w: &Saved) -> Editor {
        let d = &w.definition;
        Editor {
            workflow: Some(w.id.clone()),
            agent_id: w.agent_id.clone(),
            name: TextEdit::new(w.name.clone()),
            goal: TextEdit::new(w.goal.clone()),
            success: TextEdit::new(d.success.clone()),
            inputs: d
                .inputs
                .iter()
                .map(|(name, i)| InputRow {
                    name: TextEdit::new(name.clone()),
                    kind: INPUT_KINDS
                        .into_iter()
                        .find(|k| *k == i.kind)
                        .unwrap_or("string"),
                    description: TextEdit::new(i.description.clone()),
                    default: TextEdit::new(
                        i.default
                            .as_ref()
                            .map(super::template::as_text)
                            .unwrap_or_default(),
                    ),
                })
                .collect(),
            steps: d.steps.iter().map(Node::from_step).collect(),
            problems: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// The definition the boxes say, and every problem with it: what could not be read, then what
    /// [`validate`] finds.
    pub fn definition(&self) -> Result<Definition, Vec<String>> {
        let mut bad = Vec::new();
        let mut inputs = BTreeMap::new();
        for row in &self.inputs {
            let name = row.name.text.trim().to_owned();
            let default = match (opt(&row.default), row.kind) {
                (None, _) => None,
                (Some(t), "number") => match t.trim().parse::<f64>() {
                    Ok(n) if n.fract() == 0.0 && n.abs() < 1e15 => Some(Value::from(n as i64)),
                    Ok(n) => serde_json::Number::from_f64(n).map(Value::Number),
                    Err(_) => {
                        bad.push(format!("input {name:?}: the default is not a number"));
                        None
                    }
                },
                (Some(t), "boolean") => match t.trim().to_lowercase().as_str() {
                    "true" | "yes" => Some(Value::Bool(true)),
                    "false" | "no" => Some(Value::Bool(false)),
                    _ => {
                        bad.push(format!("input {name:?}: the default is true or false"));
                        None
                    }
                },
                (Some(t), _) => Some(Value::String(t)),
            };
            if inputs
                .insert(
                    name.clone(),
                    Input {
                        kind: row.kind.to_owned(),
                        description: row.description.text.trim().to_owned(),
                        default,
                    },
                )
                .is_some()
            {
                bad.push(format!("input {name:?} is there twice"));
            }
        }
        let steps = self
            .steps
            .iter()
            .enumerate()
            .map(|(i, n)| n.to_step(&(i + 1).to_string(), &mut bad))
            .collect();
        let def = Definition {
            inputs,
            steps,
            success: self.success.text.trim().to_owned(),
        };
        if let Err(more) = validate(&def) {
            bad.extend(more);
        }
        if bad.is_empty() { Ok(def) } else { Err(bad) }
    }

    /// Looks again at what is typed and keeps what is wrong with it.
    pub fn check(&mut self) {
        self.problems = self.definition().err().unwrap_or_default();
        if let Some(why) = self.basics() {
            self.problems.insert(0, why);
        }
    }

    /// What is missing before anything else: a name, and the agent it belongs to.
    fn basics(&self) -> Option<String> {
        if self.name.text.trim().is_empty() {
            Some("The workflow needs a name.".to_owned())
        } else if self.agent_id.is_empty() {
            Some("Choose the agent this workflow belongs to.".to_owned())
        } else {
            None
        }
    }

    /// The workflow, if it is good to save. `created_at` is for a new one.
    pub fn saved(&self, id: &str, created_at: i64) -> Result<Saved, Vec<String>> {
        let definition = self.definition()?;
        if let Some(why) = self.basics() {
            return Err(vec![why]);
        }
        Ok(Saved {
            id: id.to_owned(),
            agent_id: self.agent_id.clone(),
            name: self.name.text.trim().to_owned(),
            goal: self.goal.text.trim().to_owned(),
            definition,
            created_at,
        })
    }

    // ---- changing the tree

    pub fn block_mut(&mut self, place: &Place) -> Option<&mut Vec<Node>> {
        let mut list = &mut self.steps;
        for &(step, block) in place {
            list = &mut list.get_mut(step)?.blocks.get_mut(block)?.steps;
        }
        Some(list)
    }

    pub fn node_mut(&mut self, at: &Loc) -> Option<&mut Node> {
        self.block_mut(&at.place)?.get_mut(at.index)
    }

    /// An id no step has.
    fn fresh_id(&self) -> String {
        let mut used = Vec::new();
        collect_ids(&self.steps, &mut used);
        (1..)
            .map(|n| format!("step-{n}"))
            .find(|id| !used.contains(id))
            .unwrap_or_default()
    }

    /// Remembers the steps as they are, before a change called `what`.
    fn record(&mut self, what: &str) {
        self.undo.push((what.to_owned(), self.steps.clone()));
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// What undo would take back, if anything.
    pub fn undoes(&self) -> Option<&str> {
        self.undo.last().map(|(what, _)| what.as_str())
    }

    /// What redo would do again, if anything.
    pub fn redoes(&self) -> Option<&str> {
        self.redo.last().map(|(what, _)| what.as_str())
    }

    /// Takes back the last change to the steps. Gives what it was.
    pub fn undo(&mut self) -> Option<String> {
        let (what, steps) = self.undo.pop()?;
        self.redo
            .push((what.clone(), std::mem::replace(&mut self.steps, steps)));
        Some(what)
    }

    /// Does again what undo took back. Gives what it was.
    pub fn redo(&mut self) -> Option<String> {
        let (what, steps) = self.redo.pop()?;
        self.undo
            .push((what.clone(), std::mem::replace(&mut self.steps, steps)));
        Some(what)
    }

    /// Adds a step of `kind` at the end of the list at `place`.
    pub fn add(&mut self, place: &Place, kind: &str) {
        self.add_at(place, usize::MAX, kind);
    }

    /// Adds a step of `kind` to the list at `place`, at `index` (the end if that is past it). Gives
    /// where it went.
    pub fn add_at(&mut self, place: &Place, index: usize, kind: &str) -> Option<Loc> {
        let len = self.block_mut(place)?.len();
        let index = index.min(len);
        let node = Node::new(kind, self.fresh_id());
        self.record("Add step");
        self.block_mut(place)?.insert(index, node);
        Some(Loc {
            place: place.clone(),
            index,
        })
    }

    pub fn remove(&mut self, at: &Loc) {
        let there = self
            .block_mut(&at.place)
            .is_some_and(|list| at.index < list.len());
        if there {
            self.record("Delete step");
            if let Some(list) = self.block_mut(&at.place) {
                list.remove(at.index);
            }
        }
    }

    /// Moves a step up (`-1`) or down (`1`) among its list.
    pub fn shift(&mut self, at: &Loc, by: isize) {
        let Some(list) = self.block_mut(&at.place) else {
            return;
        };
        let to = at.index.checked_add_signed(by);
        if let Some(to) = to.filter(|t| *t < list.len() && at.index < list.len()) {
            self.record("Move step");
            if let Some(list) = self.block_mut(&at.place) {
                list.swap(at.index, to);
            }
        }
    }

    /// Whether `to` is a list inside the step at `from`, or the step's own: where it cannot go.
    fn within(from: &Loc, to: &Place) -> bool {
        let depth = from.place.len();
        to.len() > depth && to[..depth] == from.place[..] && to[depth].0 == from.index
    }

    /// Moves the step at `from` to the position `to` (a list and an index in it, counted as the
    /// lists are now). Gives where the step ends up. It cannot go inside itself, and a move that
    /// changes nothing is not recorded.
    pub fn move_to(&mut self, from: &Loc, to: &Loc) -> Result<Loc, &'static str> {
        if self.node_mut(from).is_none() {
            return Err("There is no such step.");
        }
        if Self::within(from, &to.place) {
            return Err("A step cannot go inside itself.");
        }
        let len = self
            .block_mut(&to.place)
            .map(|l| l.len())
            .ok_or("There is nowhere to put it.")?;
        if to.index > len {
            return Err("There is nowhere to put it.");
        }
        if to.place == from.place && (to.index == from.index || to.index == from.index + 1) {
            return Ok(from.clone());
        }
        self.record("Move step");
        let node = self
            .block_mut(&from.place)
            .map(|l| l.remove(from.index))
            .ok_or("There is no such step.")?;
        // Taking it out moved what came after it up by one, in its own list and in the path to
        // the new place if that goes through one of them.
        let mut place = to.place.clone();
        let mut index = to.index;
        let depth = from.place.len();
        if place == from.place && index > from.index {
            index -= 1;
        } else if place.len() > depth
            && place[..depth] == from.place[..]
            && place[depth].0 > from.index
        {
            place[depth].0 -= 1;
        }
        let list = self
            .block_mut(&place)
            .ok_or("There is nowhere to put it.")?;
        list.insert(index.min(list.len()), node);
        Ok(Loc { place, index })
    }

    /// Puts a copy of the step at `from` at the position `to`, with ids of its own and its results
    /// kept under new names so none is taken twice. Gives where the copy is.
    pub fn duplicate(&mut self, from: &Loc, to: &Loc) -> Result<Loc, &'static str> {
        let mut copy = self.node_mut(from).ok_or("There is no such step.")?.clone();
        let len = self
            .block_mut(&to.place)
            .map(|l| l.len())
            .ok_or("There is nowhere to put it.")?;
        let mut ids = Vec::new();
        collect_ids(&self.steps, &mut ids);
        let mut binds = Vec::new();
        collect_binds(&self.steps, &mut binds);
        renew(&mut copy, &mut ids, &mut binds);
        self.record("Duplicate step");
        let index = to.index.min(len);
        self.block_mut(&to.place)
            .ok_or("There is nowhere to put it.")?
            .insert(index, copy);
        Ok(Loc {
            place: to.place.clone(),
            index,
        })
    }

    /// Adds a case before the default of the `switch` at `at`.
    pub fn add_case(&mut self, at: &Loc) {
        if self.node_mut(at).is_some_and(|n| n.kind == SWITCH) {
            self.record("Add case");
            if let Some(n) = self.node_mut(at) {
                let at = n.blocks.len().saturating_sub(1);
                n.blocks.insert(at, Block::new(Part::Case));
            }
        }
    }

    /// Adds a branch at the end of the `parallel` step at `at`.
    pub fn add_branch(&mut self, at: &Loc) {
        if self.node_mut(at).is_some_and(|n| n.kind == PARALLEL) {
            self.record("Add branch");
            if let Some(n) = self.node_mut(at) {
                n.blocks.push(Block::new(Part::Branch));
            }
        }
    }

    pub fn remove_branch(&mut self, at: &Loc, block: usize) {
        let there = self.node_mut(at).is_some_and(|n| {
            n.kind == PARALLEL && n.blocks.get(block).is_some_and(|b| b.part == Part::Branch)
        });
        if there {
            self.record("Delete branch");
            if let Some(n) = self.node_mut(at) {
                n.blocks.remove(block);
            }
        }
    }

    pub fn remove_case(&mut self, at: &Loc, block: usize) {
        let there = self.node_mut(at).is_some_and(|n| {
            n.kind == SWITCH && n.blocks.get(block).is_some_and(|b| b.part == Part::Case)
        });
        if there {
            self.record("Delete case");
            if let Some(n) = self.node_mut(at) {
                n.blocks.remove(block);
            }
        }
    }

    /// The text box a [`Target`] names.
    pub fn text_mut(&mut self, which: &Target) -> Option<&mut TextEdit> {
        match which {
            Target::Name => Some(&mut self.name),
            Target::Goal => Some(&mut self.goal),
            Target::Success => Some(&mut self.success),
            Target::InputName(i) => self.inputs.get_mut(*i).map(|r| &mut r.name),
            Target::InputDescription(i) => self.inputs.get_mut(*i).map(|r| &mut r.description),
            Target::InputDefault(i) => self.inputs.get_mut(*i).map(|r| &mut r.default),
            Target::Id(at) => self.node_mut(at).map(|n| &mut n.id),
            Target::Bind(at) => self.node_mut(at).map(|n| &mut n.bind),
            Target::Field(at, i) => self.node_mut(at).and_then(|n| n.fields.get_mut(*i)),
            Target::When(at, b) => self
                .node_mut(at)
                .and_then(|n| n.blocks.get_mut(*b))
                .map(|b| &mut b.when),
        }
    }

    /// What a box can name: the inputs as `inputs.name`, what the steps before bind, and inside a
    /// loop its item and `index`. A `while` condition also sees what its body binds.
    pub fn names(&self, at: &Loc, key: Option<&str>) -> Vec<String> {
        let mut names: Vec<String> = self
            .inputs
            .iter()
            .map(|r| r.name.text.trim())
            .filter(|n| !n.is_empty())
            .map(|n| format!("inputs.{n}"))
            .collect();
        let bound = |list: &[Node], names: &mut Vec<String>| {
            for n in list {
                let b = n.bind.text.trim();
                if !b.is_empty() && !names.iter().any(|x| x == b) {
                    names.push(b.to_owned());
                }
            }
        };
        let mut list = &self.steps;
        for &(step, block) in &at.place {
            let Some(node) = list.get(step) else {
                return names;
            };
            bound(&list[..step], &mut names);
            match node.kind.as_str() {
                FOR_EACH => {
                    names.push(node.item_name());
                    names.push("index".to_owned());
                }
                WHILE => names.push("index".to_owned()),
                TRY if block == 1 => names.push("error".to_owned()),
                _ => {}
            }
            let Some(b) = node.blocks.get(block) else {
                return names;
            };
            list = &b.steps;
        }
        bound(&list[..at.index.min(list.len())], &mut names);
        if let Some(node) = list.get(at.index)
            && node.kind == WHILE
            && key == Some("cond")
        {
            names.push("index".to_owned());
            bound(&node.blocks[0].steps, &mut names);
        }
        names
    }
}

fn collect_binds(list: &[Node], out: &mut Vec<String>) {
    for n in list {
        let b = n.bind.text.trim();
        if !b.is_empty() {
            out.push(b.to_owned());
        }
        for block in &n.blocks {
            collect_binds(&block.steps, out);
        }
    }
}

/// Gives `node` and the steps inside it ids no step has, and results under names no step uses.
fn renew(node: &mut Node, ids: &mut Vec<String>, binds: &mut Vec<String>) {
    let id = (1..)
        .map(|n| format!("step-{n}"))
        .find(|id| !ids.contains(id))
        .unwrap_or_default();
    ids.push(id.clone());
    node.id = TextEdit::new(id);
    let bind = node.bind.text.trim().to_owned();
    if !bind.is_empty() {
        let fresh = (2..)
            .map(|n| format!("{bind}_{n}"))
            .find(|b| !binds.contains(b))
            .unwrap_or_default();
        binds.push(fresh.clone());
        node.bind = TextEdit::new(fresh);
    }
    for block in &mut node.blocks {
        for inner in &mut block.steps {
            renew(inner, ids, binds);
        }
    }
}

fn collect_ids(list: &[Node], out: &mut Vec<String>) {
    for n in list {
        out.push(n.id.text.trim().to_owned());
        for b in &n.blocks {
            collect_ids(&b.steps, out);
        }
    }
}

/// How many steps a definition has, those inside containers counted.
pub fn step_count(def: &Definition) -> usize {
    let mut n = 0;
    each_step(&def.steps, &mut |_| n += 1);
    n
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    fn saved(v: Value) -> Saved {
        Saved {
            id: "w1".into(),
            agent_id: "a1".into(),
            name: "Daily".into(),
            goal: "Does it.".into(),
            definition: serde_json::from_value(v).unwrap(),
            created_at: 5,
        }
    }

    fn every_kind() -> Value {
        json!({
            "inputs": {
                "folder": {"type": "string", "description": "where", "default": "notes"},
                "n": {"type": "number", "default": 2},
                "loud": {"type": "boolean"}
            },
            "steps": [
                {"id": "ls", "kind": "tool", "tool": "shell", "bind": "files",
                 "args": {"command": "ls {{inputs.folder}}"}},
                {"id": "ask", "kind": "decide", "question": "Any?", "options": ["yes", "no"],
                 "input": "{{files.stdout}}", "bind": "any"},
                {"id": "say", "kind": "generate", "instruction": "Say {{any}}"},
                {"id": "if", "kind": "if", "cond": "files.exit_code == 0", "bind": "r",
                 "then": [{"id": "each", "kind": "for_each", "over": "files.stdout.split(\"\\n\")", "as": "name",
                           "on_error": "continue",
                           "steps": [{"id": "show", "kind": "tool", "tool": "shell", "args": {"command": "echo {{name}}"}}]}],
                 "else": [{"id": "none", "kind": "generate", "instruction": "empty"}]},
                {"id": "sw", "kind": "switch", "on": "inputs.folder",
                 "cases": [{"when": "\"notes\"", "steps": [{"id": "a", "kind": "set", "values": {"x": "1", "y": "inputs.n + 1"}, "bind": "made"}]},
                           {"when": "\"docs\"", "steps": [{"id": "b", "kind": "generate", "instruction": "b"}]}],
                 "default": [{"id": "d", "kind": "generate", "instruction": "d"}]},
                {"id": "wait", "kind": "while", "cond": "poll.exit_code != 0", "max": 5,
                 "steps": [{"id": "tick", "kind": "tool", "tool": "shell", "args": {"command": "check"}, "bind": "poll"}]},
                {"id": "safe", "kind": "try", "retries": 2, "backoff_ms": 500, "bind": "pushed",
                 "steps": [{"id": "push", "kind": "tool", "tool": "shell", "args": {"command": "git push"}}],
                 "catch": [{"id": "note", "kind": "generate", "instruction": "Push failed: {{error}}"}]},
                {"id": "sub", "kind": "flow", "workflow": "Weekly report", "bind": "report",
                 "inputs": {"folder": "{{inputs.folder}}"}},
                {"id": "calc", "kind": "code", "bind": "sum", "script": "let t = 0;\nfor i in [1, 2] { t += i; }\nt"},
                {"id": "ok", "kind": "wait", "approve": "Send {{sum}} on?"},
                {"id": "nap", "kind": "wait", "seconds": 3600},
                {"id": "both", "kind": "parallel", "on_error": "continue", "bind": "pair", "branches": [
                    {"name": "left", "steps": [{"id": "l", "kind": "tool", "tool": "shell", "args": {"command": "echo l"}}]},
                    {"name": "right", "steps": [{"id": "r", "kind": "generate", "instruction": "Say {{sum}}"}]}
                ]}
            ],
            "success": "All done."
        })
    }

    #[test]
    fn a_workflow_goes_through_the_editor_and_comes_back_the_same() {
        let w = saved(every_kind());
        assert_eq!(validate(&w.definition), Ok(()));
        let e = Editor::from_saved(&w);
        let back = e.saved("w1", 5);
        assert_eq!(back, Ok(w));
        assert_eq!(
            step_count(
                &Editor::from_saved(&saved(every_kind()))
                    .definition()
                    .unwrap()
            ),
            23
        );
    }

    #[test]
    fn a_problem_in_a_box_or_in_the_whole_is_said_in_words() {
        let mut e = Editor::from_saved(&saved(every_kind()));
        e.steps[0].fields[0] = TextEdit::new("{not json");
        let wait = 5;
        e.steps[wait].fields[1] = TextEdit::new("lots");
        e.steps[4].blocks[0].steps[0].fields[0] = TextEdit::new("no equals sign");
        e.inputs[2].default = TextEdit::new("two");
        e.steps[7].fields[1] = TextEdit::new("[1, 2]");
        e.name = TextEdit::new("");
        e.check();
        let p = e.problems.join("\n");
        for expected in [
            "The workflow needs a name.",
            "step 1: the arguments are not JSON",
            "step 6: the most rounds is a whole number",
            "step 5.1: write each value as name = expression",
            "the default is not a number",
            "step 8: what to give is a JSON object",
        ] {
            assert!(p.contains(expected), "missing {expected:?} in:\n{p}");
        }
        // What the whole definition gets wrong is added: a misspelt name in an expression.
        let mut e = Editor::from_saved(&saved(every_kind()));
        e.steps[3].fields[0] = TextEdit::new("fles.exit_code == 0");
        e.check();
        assert!(
            e.problems.iter().any(|p| p.contains("fles")),
            "{:?}",
            e.problems
        );
        e.steps[3].fields[0] = TextEdit::new("files.exit_code == 0");
        e.check();
        assert!(e.problems.is_empty(), "{:?}", e.problems);
    }

    fn at(place: &[(usize, usize)], index: usize) -> Loc {
        Loc {
            place: place.to_vec(),
            index,
        }
    }

    #[test]
    fn steps_are_added_to_any_list_moved_and_removed_with_ids_that_are_never_reused() {
        let mut e = Editor::blank("a1");
        e.add(&vec![], TOOL);
        e.add(&vec![], IF);
        e.add(&vec![(1, 0)], GENERATE);
        e.add(&vec![(1, 1)], SWITCH);
        e.add(&vec![(1, 1), (0, 0)], FOR_EACH);
        assert_eq!(e.steps[1].blocks[0].steps[0].id.text, "step-3");
        assert_eq!(
            e.steps[1].blocks[1].steps[0].blocks[0].steps[0].id.text,
            "step-5"
        );
        // A new tool step starts with a tool chosen; a switch with a case and its default.
        assert_eq!(e.steps[0].tool, "shell");
        assert_eq!(
            e.steps[1].blocks[1].steps[0]
                .blocks
                .iter()
                .map(|b| b.part)
                .collect::<Vec<_>>(),
            [Part::Case, Part::Default]
        );
        e.shift(&at(&[], 0), 1);
        assert_eq!(e.steps[0].kind, IF);
        e.shift(&at(&[], 0), -1);
        assert_eq!(e.steps[0].kind, IF, "the first cannot go up");
        e.shift(&at(&[], 1), 1);
        assert_eq!(e.steps[1].kind, TOOL, "the last cannot go down");
        e.remove(&at(&[], 1));
        e.add(&vec![], TOOL);
        assert_eq!(
            e.steps[1].id.text, "step-1",
            "a freed id comes back, but none in use does"
        );
        e.remove(&at(&[(9, 9)], 0));
        e.add_case(&at(&[(0, 1)], 0));
        let sw = &e.steps[0].blocks[1].steps[0];
        assert_eq!(
            sw.blocks.iter().map(|b| b.part).collect::<Vec<_>>(),
            [Part::Case, Part::Case, Part::Default]
        );
        e.remove_case(&at(&[(0, 1)], 0), 2);
        e.remove_case(&at(&[(0, 1)], 0), 0);
        assert_eq!(e.steps[0].blocks[1].steps[0].blocks.len(), 2);
    }

    #[test]
    fn a_box_can_be_found_again_by_where_it_is() {
        let mut e = Editor::from_saved(&saved(every_kind()));
        let inner = at(&[(3, 0)], 0);
        e.text_mut(&Target::Field(inner.clone(), 0)).unwrap().text = "[1]".into();
        assert_eq!(e.steps[3].blocks[0].steps[0].fields[0].text, "[1]");
        assert!(
            e.text_mut(&Target::When(at(&[], 4), 1))
                .unwrap()
                .text
                .contains("docs")
        );
        assert!(e.text_mut(&Target::Id(at(&[(7, 7)], 0))).is_none());
        assert_eq!(e.text_mut(&Target::InputName(0)).unwrap().text, "folder");
    }

    #[test]
    fn the_names_a_box_can_use_are_what_comes_before_it_and_what_its_loop_gives() {
        let e = Editor::from_saved(&saved(every_kind()));
        let top = e.names(&at(&[], 2), None);
        assert_eq!(
            top,
            ["inputs.folder", "inputs.loud", "inputs.n", "files", "any"]
        );
        // Inside the for_each: everything before the if, then the item under its own name.
        let inside = e.names(&at(&[(3, 0)], 0), None);
        assert!(
            inside.contains(&"files".to_owned()) && inside.contains(&"any".to_owned()),
            "{inside:?}"
        );
        assert!(
            !inside.contains(&"r".to_owned()),
            "the if's own bind is not yet there"
        );
        let body = e.names(&at(&[(3, 0), (0, 0)], 0), None);
        assert!(
            body.ends_with(&["name".to_owned(), "index".to_owned()]),
            "{body:?}"
        );
        // A while's condition also sees what its body binds.
        let cond = e.names(&at(&[], 5), Some("cond"));
        assert!(
            cond.contains(&"poll".to_owned()) && cond.contains(&"index".to_owned()),
            "{cond:?}"
        );
        assert!(!e.names(&at(&[], 5), None).contains(&"poll".to_owned()));
        // A branch sees what comes before the step, not its sibling branches.
        let left = e.names(&at(&[(11, 0)], 0), None);
        assert!(left.contains(&"sum".to_owned()), "{left:?}");
        // Only what runs after a try fails can see its error.
        assert!(
            !e.names(&at(&[(6, 0)], 0), None)
                .contains(&"error".to_owned())
        );
        assert!(
            e.names(&at(&[(6, 1)], 0), None)
                .contains(&"error".to_owned())
        );
    }

    #[test]
    fn branches_are_added_and_removed_and_each_is_a_list_of_steps() {
        let mut e = Editor::blank("a1");
        e.add(&vec![], PARALLEL);
        assert_eq!(
            e.steps[0].blocks.len(),
            2,
            "a new parallel step starts with two"
        );
        e.add_branch(&at(&[], 0));
        e.add(&vec![(0, 2)], GENERATE);
        assert_eq!(e.steps[0].blocks[2].steps.len(), 1);
        e.remove_branch(&at(&[], 0), 0);
        assert_eq!(e.steps[0].blocks.len(), 2);
        assert_eq!(
            e.steps[0].blocks[1].steps.len(),
            1,
            "what was the third is the second"
        );
        // Only a parallel step has branches.
        e.add(&vec![], TOOL);
        e.add_branch(&at(&[], 1));
        assert!(e.steps[1].blocks.is_empty());
    }

    fn ids(e: &Editor, place: &[(usize, usize)]) -> Vec<String> {
        let mut list = &e.steps;
        for &(step, block) in place {
            list = &list[step].blocks[block].steps;
        }
        list.iter().map(|n| n.id.text.clone()).collect()
    }

    fn tool(id: &str) -> Value {
        json!({"id": id, "kind": "tool", "tool": "shell", "args": {"command": "x"}})
    }

    fn tree() -> Editor {
        Editor::from_saved(&saved(json!({"steps": [
            tool("a"), tool("b"),
            {"id": "i", "kind": "if", "cond": "true", "then": [tool("t1"), tool("t2")], "else": [tool("e1")]},
            tool("c")
        ]})))
    }

    #[test]
    fn a_step_moves_within_its_list_and_into_and_out_of_frames_with_the_indexes_right() {
        // `to` is a position counted as the lists are now: before step 3 of the top list is 3.
        let mut e = tree();
        let moved = e.move_to(&at(&[], 0), &at(&[], 3)).unwrap();
        assert_eq!(
            (ids(&e, &[]), moved),
            (["b", "i", "a", "c"].map(String::from).to_vec(), at(&[], 2))
        );
        // Into the then list, between t1 and t2: the frame moved up one by taking b... a was first.
        let mut e = tree();
        let moved = e.move_to(&at(&[], 0), &at(&[(2, 0)], 1)).unwrap();
        assert_eq!(ids(&e, &[(1, 0)]), ["t1", "a", "t2"]);
        assert_eq!(
            moved,
            at(&[(1, 0)], 1),
            "the frame is step 1 once a has left"
        );
        assert_eq!(ids(&e, &[]), ["b", "i", "c"]);
        // Out of a frame, to the top, before the last step.
        let mut e = tree();
        let moved = e.move_to(&at(&[(2, 0)], 1), &at(&[], 3)).unwrap();
        assert_eq!(
            (ids(&e, &[]), moved),
            (
                ["a", "b", "i", "t2", "c"].map(String::from).to_vec(),
                at(&[], 3)
            )
        );
        assert_eq!(ids(&e, &[(2, 0)]), ["t1"]);
        // Across frames: from the else list into the then list.
        let mut e = tree();
        e.move_to(&at(&[(2, 1)], 0), &at(&[(2, 0)], 0)).unwrap();
        assert_eq!(
            (ids(&e, &[(2, 0)]), ids(&e, &[(2, 1)])),
            (["e1", "t1", "t2"].map(String::from).to_vec(), vec![])
        );
    }

    #[test]
    fn a_step_cannot_go_inside_itself_and_a_move_that_changes_nothing_is_not_remembered() {
        let mut e = tree();
        let err = e.move_to(&at(&[], 2), &at(&[(2, 0)], 0)).unwrap_err();
        assert!(err.contains("inside itself"), "{err}");
        assert_eq!(e.undoes(), None, "nothing happened");
        // Where it already is: before itself, or just after.
        for to in [at(&[], 1), at(&[], 2)] {
            assert_eq!(e.move_to(&at(&[], 1), &to), Ok(at(&[], 1)));
        }
        assert_eq!(e.undoes(), None);
        assert!(e.move_to(&at(&[], 9), &at(&[], 0)).is_err());
        assert!(e.move_to(&at(&[], 0), &at(&[], 9)).is_err());
        assert!(e.move_to(&at(&[], 0), &at(&[(7, 0)], 0)).is_err());
    }

    #[test]
    fn changes_to_the_steps_can_be_undone_and_done_again_and_a_new_change_ends_the_redo() {
        let mut e = tree();
        let before = ids(&e, &[]);
        e.add_at(&vec![], 1, "set").unwrap();
        e.remove(&at(&[], 0));
        e.move_to(&at(&[], 0), &at(&[], 3)).unwrap();
        assert_eq!(e.undoes(), Some("Move step"));
        assert_eq!(e.undo().as_deref(), Some("Move step"));
        assert_eq!(e.undo().as_deref(), Some("Delete step"));
        assert_eq!(e.redoes(), Some("Delete step"));
        assert_eq!(e.undo().as_deref(), Some("Add step"));
        assert_eq!(ids(&e, &[]), before, "back to the start");
        assert_eq!(e.undo(), None);
        assert_eq!(e.redo().as_deref(), Some("Add step"));
        assert_eq!(e.steps.len(), 5);
        // A new change after an undo forgets what was undone.
        e.undo();
        e.remove(&at(&[], 3));
        assert_eq!(e.redoes(), None);
        // Every kind of structural change is one step of history, and one that does nothing (a
        // case added to something that is not a switch) is none.
        let mut e = tree();
        e.add_case(&at(&[], 2));
        e.shift(&at(&[], 0), 1);
        e.duplicate(&at(&[], 0), &at(&[], 0)).unwrap();
        let mut seen = Vec::new();
        while let Some(what) = e.undo() {
            seen.push(what);
        }
        assert_eq!(seen, ["Duplicate step", "Move step"]);
    }

    #[test]
    fn a_copy_has_ids_and_result_names_of_its_own_all_through() {
        let mut e = Editor::from_saved(&saved(json!({"steps": [
            {"id": "step-1", "kind": "for_each", "over": "[1]", "bind": "all",
             "steps": [{"id": "step-2", "kind": "tool", "tool": "shell", "args": {"command": "x"}, "bind": "out"}]}
        ]})));
        let copy = e.duplicate(&at(&[], 0), &at(&[], 1)).unwrap();
        assert_eq!(copy, at(&[], 1));
        let (a, b) = (&e.steps[0], &e.steps[1]);
        assert_eq!(
            (b.id.text.as_str(), b.bind.text.as_str()),
            ("step-3", "all_2")
        );
        assert_eq!(
            (
                b.blocks[0].steps[0].id.text.as_str(),
                b.blocks[0].steps[0].bind.text.as_str()
            ),
            ("step-4", "out_2")
        );
        assert_eq!(a.id.text, "step-1", "the original is untouched");
        // Copied again, the names go on from where they are.
        e.duplicate(&at(&[], 0), &at(&[], 2)).unwrap();
        assert_eq!(e.steps[2].bind.text, "all_3");
        assert!(e.definition().is_ok(), "{:?}", e.definition().err());
    }

    #[test]
    fn a_step_is_added_at_a_position() {
        let mut e = tree();
        let at_two = e.add_at(&vec![], 2, "generate").unwrap();
        assert_eq!((at_two, e.steps[2].kind.as_str()), (at(&[], 2), "generate"));
        let inner = e.add_at(&vec![(3, 1)], 9, "set").unwrap();
        assert_eq!(inner, at(&[(3, 1)], 1), "past the end is the end");
        assert_eq!(e.add_at(&vec![(8, 0)], 0, "set"), None);
    }
}
