//! Hooks: commands you set to run around an agent's tool calls, on this computer.
//!
//! A hook is a shell command. It is given the event as JSON on its input and answers with its exit
//! code and what it prints: nothing, or exit 0, lets things go on; exit 2 refuses (what it wrote to
//! stderr is the reason); or it prints a JSON verdict, `{"action": "allow" | "block" | "ask" |
//! "replace", ...}` (see [`Verdict`]).
//!
//! A hook never loosens the guardrails (see [`crate::guard`]): they look at a call first, and a
//! hook only sees one they let through, which it may refuse, hold for your go-ahead or change.
//! Hooks are yours: they are set on the Hooks screen, never by a model or a channel person.

mod gate;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::command::{Ended, Spec};
use crate::store::Store;

/// Where the hooks are saved.
const SAVED: &str = "hooks.list";

/// The longest a hook may print and still be understood, in bytes: a replaced result is in it.
const OUTPUT_LIMIT: usize = 256 * 1024;

/// How long a hook may run when none is set, in seconds, and the longest that can be set.
pub const DEFAULT_SECS: u64 = 10;
pub const MAX_SECS: u64 = 60;

/// When a hook runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// Before a tool call runs, once the guardrails let it through: it can be refused, held for
    /// your go-ahead, or have its arguments changed.
    BeforeTool,
    /// When a tool call has its result, before the model sees it: it can be withheld or replaced.
    AfterTool,
}

impl Event {
    pub const ALL: [Event; 2] = [Event::BeforeTool, Event::AfterTool];

    pub fn title(self) -> &'static str {
        match self {
            Self::BeforeTool => "Before a tool runs",
            Self::AfterTool => "After a tool runs",
        }
    }

    /// The name the hook is told, and the docs use.
    pub fn name(self) -> &'static str {
        match self {
            Self::BeforeTool => "before_tool",
            Self::AfterTool => "after_tool",
        }
    }
}

/// What happens to the call when a hook cannot give an answer (it failed to start, ran past its
/// limit, exited with an error, or printed something that is not a verdict).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fail {
    /// The call goes on, and the failure is noted in the run.
    #[default]
    Open,
    /// The call is refused (before a tool) or its result withheld (after one).
    Closed,
}

/// A command run at an [`Event`].
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Hook {
    pub id: String,
    pub name: String,
    pub event: Event,
    /// The tools it is for: names split by `|`, `*` standing for any characters (`shell`,
    /// `mcp__*`, `write_file|edit_file`). Empty: every tool.
    #[serde(default)]
    pub tools: String,
    pub command: String,
    #[serde(default = "default_secs")]
    pub secs: u64,
    #[serde(default)]
    pub fail: Fail,
    #[serde(default = "yes")]
    pub on: bool,
}

fn default_secs() -> u64 {
    DEFAULT_SECS
}

fn yes() -> bool {
    true
}

impl Hook {
    pub fn new(event: Event) -> Hook {
        Hook {
            id: crate::model::new_id(),
            name: String::new(),
            event,
            tools: String::new(),
            command: String::new(),
            secs: DEFAULT_SECS,
            fail: Fail::Open,
            on: true,
        }
    }

    /// What it is called: its name, or its command when it has none.
    pub fn label(&self) -> &str {
        let name = self.name.trim();
        if name.is_empty() {
            self.command.trim()
        } else {
            name
        }
    }

    /// Whether it is for tool `name`.
    pub fn is_for(&self, name: &str) -> bool {
        let tools = self.tools.trim();
        tools.is_empty() || tools.split('|').any(|p| glob(p.trim(), name))
    }
}

/// Whether `text` matches `pattern`, where `*` stands for any characters.
fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let mut parts = parts.peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.is_empty()
}

/// The hooks you have set, in the order they run.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Hooks {
    list: Vec<Hook>,
}

impl Hooks {
    pub fn load(store: Option<&Store>) -> Hooks {
        let list = store
            .and_then(|db| {
                db.setting::<Vec<Hook>>(SAVED).unwrap_or_else(|e| {
                    eprintln!("oido: could not read the hooks: {e}");
                    None
                })
            })
            .unwrap_or_default();
        Hooks { list }
    }

    pub fn list(&self) -> &[Hook] {
        &self.list
    }

    /// The hooks that are on for `event` and tool `tool`, in order.
    pub fn for_tool(&self, event: Event, tool: &str) -> Vec<Hook> {
        self.list
            .iter()
            .filter(|h| h.on && h.event == event && !h.command.trim().is_empty() && h.is_for(tool))
            .cloned()
            .collect()
    }

    /// Sets the list, dropping what cannot run: a hook with no command, and a time limit out of range
    /// is brought into it.
    pub(crate) fn set(&mut self, list: Vec<Hook>) {
        self.list = list;
        for h in &mut self.list {
            h.command = h.command.trim().to_owned();
            h.secs = h.secs.clamp(1, MAX_SECS);
        }
    }
}

/// The Hooks screen's form: a new hook, or one being changed (`id`).
pub struct Form {
    pub id: Option<String>,
    pub event: Event,
    pub name: rune::TextEdit,
    pub tools: rune::TextEdit,
    pub command: rune::TextEdit,
    pub secs: u64,
    pub fail: Fail,
}

impl Default for Form {
    fn default() -> Form {
        Form {
            id: None,
            event: Event::BeforeTool,
            name: rune::TextEdit::new(""),
            tools: rune::TextEdit::new(""),
            command: rune::TextEdit::new(""),
            secs: DEFAULT_SECS,
            fail: Fail::Open,
        }
    }
}

impl Form {
    /// The form filled in with `hook`, to change it.
    pub fn of(hook: &Hook) -> Form {
        Form {
            id: Some(hook.id.clone()),
            event: hook.event,
            name: rune::TextEdit::new(hook.name.clone()),
            tools: rune::TextEdit::new(hook.tools.clone()),
            command: rune::TextEdit::new(hook.command.clone()),
            secs: hook.secs,
            fail: hook.fail,
        }
    }

    /// Whether it holds anything to save: a command to run.
    pub fn ready(&self) -> bool {
        !self.command.text.trim().is_empty()
    }

    /// The hook it describes, on or off as `on` says.
    pub fn hook(&self, on: bool) -> Hook {
        let mut hook = Hook::new(self.event);
        if let Some(id) = &self.id {
            hook.id.clone_from(id);
        }
        hook.name = self.name.text.trim().to_owned();
        hook.tools = self.tools.text.trim().to_owned();
        hook.command = self.command.text.trim().to_owned();
        hook.secs = self.secs;
        hook.fail = self.fail;
        hook.on = on;
        hook
    }
}

/// What a hook decided.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Go on.
    Allow,
    /// Refuse the call, or withhold its result; the reason is told to the model.
    Block(String),
    /// Hold the call for your go-ahead (before a tool only).
    Ask(String),
    /// Go on with these arguments (before a tool) or this result (after one).
    Replace(String),
}

/// Reads what a hook that ended with `code` printed. `Err` says why there is no verdict.
fn verdict(event: Event, code: Option<i32>, stdout: &str, stderr: &str) -> Result<Verdict, String> {
    match code {
        Some(2) => {
            let why = stderr.trim();
            return Ok(Verdict::Block(if why.is_empty() {
                "refused by the hook".to_owned()
            } else {
                why.to_owned()
            }));
        }
        Some(0) => {}
        Some(code) => {
            let why = stderr.trim();
            return Err(if why.is_empty() {
                format!("it exited with code {code}")
            } else {
                format!("it exited with code {code}: {why}")
            });
        }
        None => return Err("it was ended by a signal".to_owned()),
    }
    let out = stdout.trim();
    if out.is_empty() {
        return Ok(Verdict::Allow);
    }
    let v: Value = serde_json::from_str(out)
        .map_err(|_| "what it printed is not JSON like {\"action\": \"allow\"}".to_owned())?;
    let reason = v
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let or = |fallback: &str| {
        if reason.is_empty() {
            fallback.to_owned()
        } else {
            reason.clone()
        }
    };
    match v.get("action").and_then(Value::as_str) {
        Some("allow") => Ok(Verdict::Allow),
        Some("block") => Ok(Verdict::Block(or("refused by the hook"))),
        Some("ask") if event == Event::BeforeTool => {
            Ok(Verdict::Ask(or("the hook wants your go-ahead")))
        }
        Some("ask") => Err("\"ask\" only works before a tool runs".to_owned()),
        Some("replace") => {
            let key = match event {
                Event::BeforeTool => "args",
                Event::AfterTool => "result",
            };
            match v.get(key) {
                Some(Value::String(s)) => Ok(Verdict::Replace(s.clone())),
                Some(other) if event == Event::BeforeTool && other.is_object() => {
                    Ok(Verdict::Replace(other.to_string()))
                }
                _ => Err(format!("\"replace\" needs \"{key}\"")),
            }
        }
        Some(other) => Err(format!("\"{other}\" is not an action it knows")),
        None => Err("what it printed has no \"action\"".to_owned()),
    }
}

/// Runs `hook` in `dir` with `event` as its input, and waits for it. Blocks, so call it off the UI
/// thread. `Err` says why it gave no verdict.
pub fn run(hook: &Hook, event: &Value, dir: &Path) -> Result<Verdict, String> {
    let spec = Spec {
        command: hook.command.clone(),
        reason: String::new(),
        timeout: Duration::from_secs(hook.secs.clamp(1, MAX_SECS)),
        background: false,
        input: Some(event.to_string()),
        limit: OUTPUT_LIMIT,
    };
    let raw = crate::command::run_raw(&spec, dir, &AtomicBool::new(false));
    match raw.end {
        Ended::Exit(code) => {
            if raw.stdout_cut && code == Some(0) {
                return Err("it printed more than it may".to_owned());
            }
            verdict(hook.event, code, &raw.stdout, &raw.stderr)
        }
        Ended::TimedOut => Err(format!(
            "it ran past its {} second limit",
            spec.timeout.as_secs()
        )),
        Ended::Stopped => Err("it was stopped".to_owned()),
        Ended::Failed(why) => Err(why),
    }
}

/// The input a hook is given before tool `tool` runs.
pub fn before_event(chat: &str, agent: &str, dir: &Path, tool: &str, args: &str) -> Value {
    json!({
        "event": Event::BeforeTool.name(),
        "chat": chat,
        "agent": agent,
        "cwd": dir,
        "tool": tool,
        "args": serde_json::from_str::<Value>(args).unwrap_or_else(|_| Value::String(args.to_owned())),
    })
}

/// The input a hook is given once tool `tool` has `result`.
pub fn after_event(
    chat: &str,
    agent: &str,
    dir: &Path,
    tool: &str,
    args: &str,
    result: &str,
) -> Value {
    let mut event = before_event(chat, agent, dir, tool, args);
    event["event"] = json!(Event::AfterTool.name());
    event["result"] = json!(result);
    event
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn hook(event: Event, command: &str) -> Hook {
        let mut h = Hook::new(event);
        h.command = command.to_owned();
        h
    }

    fn run_in_tmp(h: &Hook) -> Result<Verdict, String> {
        run(h, &json!({"tool": "shell"}), Path::new("/tmp"))
    }

    #[test]
    fn tools_match_by_name_with_stars_and_bars() {
        let mut h = Hook::new(Event::BeforeTool);
        assert!(h.is_for("anything"));
        h.tools = "shell| write_file ".into();
        assert!(h.is_for("shell") && h.is_for("write_file") && !h.is_for("web"));
        h.tools = "mcp__*".into();
        assert!(h.is_for("mcp__gmail__send") && !h.is_for("shell"));
        h.tools = "*_file".into();
        assert!(h.is_for("read_file") && !h.is_for("read_files"));
        h.tools = "mcp__*__send".into();
        assert!(h.is_for("mcp__gmail__send") && !h.is_for("mcp__gmail__read"));
    }

    #[test]
    fn exit_codes_and_verdicts() {
        let before = Event::BeforeTool;
        assert_eq!(verdict(before, Some(0), "", ""), Ok(Verdict::Allow));
        assert_eq!(
            verdict(before, Some(2), "", " no deploys on Friday \n"),
            Ok(Verdict::Block("no deploys on Friday".into()))
        );
        assert!(
            verdict(before, Some(1), "", "oops")
                .unwrap_err()
                .contains("code 1: oops")
        );
        assert_eq!(
            verdict(before, Some(0), r#"{"action":"ask","reason":"prod"}"#, ""),
            Ok(Verdict::Ask("prod".into()))
        );
        assert_eq!(
            verdict(
                before,
                Some(0),
                r#"{"action":"replace","args":{"command":"ls"}}"#,
                ""
            ),
            Ok(Verdict::Replace(r#"{"command":"ls"}"#.into()))
        );
        assert_eq!(
            verdict(
                Event::AfterTool,
                Some(0),
                r#"{"action":"replace","result":"short"}"#,
                ""
            ),
            Ok(Verdict::Replace("short".into()))
        );
        assert!(verdict(Event::AfterTool, Some(0), r#"{"action":"ask"}"#, "").is_err());
        assert!(verdict(before, Some(0), "not json", "").is_err());
        assert!(verdict(before, Some(0), r#"{"action":"explode"}"#, "").is_err());
    }

    #[test]
    fn a_hook_reads_the_event_and_answers() {
        let blocks = hook(
            Event::BeforeTool,
            r#"grep -q '"tool":"shell"' && echo '{"action":"block","reason":"no shell"}'"#,
        );
        assert_eq!(run_in_tmp(&blocks), Ok(Verdict::Block("no shell".into())));
        let refuses = hook(Event::BeforeTool, "echo nope >&2; exit 2");
        assert_eq!(run_in_tmp(&refuses), Ok(Verdict::Block("nope".into())));
    }

    #[test]
    fn a_hook_that_hangs_or_breaks_gives_no_verdict() {
        let mut slow = hook(Event::BeforeTool, "sleep 5");
        slow.secs = 1;
        assert!(run_in_tmp(&slow).unwrap_err().contains("limit"));
        assert!(run_in_tmp(&hook(Event::BeforeTool, "exit 7")).is_err());
    }
}
