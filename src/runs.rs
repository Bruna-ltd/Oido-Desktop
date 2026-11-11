//! Runs: what an agent did with something you said, kept so the Runs screen can show it. A run
//! starts with your message (or your answer to its question) and lasts until the agent stops or
//! waits on you again: every model call it made and every tool it used in between is a step of
//! that one run. Its first steps are what the model was given besides the chat: the brief, each
//! skill, the context, and the tools on offer. Modelled on `oido-core`'s run ledger.

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::completion::{Tokens, ToolSpec};
use crate::model::ToolCall;
use crate::models::ModelRef;

/// How long runs are kept. Older ones are removed when the app starts; chats are never touched.
pub const KEEP_SECS: i64 = 30 * 24 * 60 * 60;

/// A step's input is cut at this many bytes, and its output at the next: a long chat is sent in
/// full on every model call, and would otherwise be saved in full every time.
pub const INPUT_LIMIT: usize = 8 * 1024;
pub const OUTPUT_LIMIT: usize = 32 * 1024;

/// A run is named after the first line of what you said, up to this many characters.
const TITLE: usize = 80;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Running,
    Ok,
    Error,
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Status {
        match s {
            "ok" => Self::Ok,
            "error" => Self::Error,
            "cancelled" => Self::Cancelled,
            _ => Self::Running,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Ok => "done",
            Self::Error => "failed",
            Self::Cancelled => "stopped",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Run {
    pub id: String,
    /// The agent and chat it ran in. No link is kept: a run outlives both.
    pub agent_id: String,
    pub agent_name: String,
    pub session_id: String,
    pub model: ModelRef,
    /// The first line of what you said.
    pub title: String,
    pub status: Status,
    pub error: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    /// Of the input, how much was read from and written to the provider's cache.
    pub cache_read_tokens: Option<u32>,
    pub cache_write_tokens: Option<u32>,
    pub duration_ms: Option<u32>,
    /// Seconds since the epoch.
    pub started_at: i64,
    pub ended_at: Option<i64>,
    /// The run that handed this one to a team member. Its token totals include this run's.
    pub parent: Option<String>,
    /// How many model calls and tool uses it has recorded.
    pub calls: u32,
    pub tools: u32,
}

impl Run {
    /// A run that has just started, with nothing recorded yet.
    pub fn new(
        id: &str,
        agent_id: &str,
        agent_name: &str,
        session_id: &str,
        model: ModelRef,
    ) -> Run {
        Run {
            id: id.to_owned(),
            agent_id: agent_id.to_owned(),
            agent_name: agent_name.to_owned(),
            session_id: session_id.to_owned(),
            model,
            title: String::new(),
            status: Status::Running,
            error: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            duration_ms: None,
            started_at: crate::app::now(),
            ended_at: None,
            parent: None,
            calls: 0,
            tools: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepKind {
    /// The agent's brief.
    System,
    /// One of the agent's skills, sent with its brief. Its name is the skill's.
    Skill,
    /// What the agent is told about the moment.
    Context,
    /// The tools the model was offered.
    Tools,
    /// Your message, or your answer to a question.
    User,
    /// A model call: what was sent, and what came back.
    Llm,
    /// A tool the model called: its arguments, and what it gave back. Its name is the tool's.
    Tool,
    /// A guardrail at work: secrets hidden from the model, or a tool call refused or held. Its
    /// name says which.
    Guard,
    Error,
}

impl StepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Skill => "skill",
            Self::Context => "context",
            Self::Tools => "tools",
            Self::User => "user",
            Self::Llm => "llm",
            Self::Tool => "tool",
            Self::Guard => "guard",
            Self::Error => "error",
        }
    }

    pub fn parse(s: &str) -> StepKind {
        match s {
            "system" => Self::System,
            "skill" => Self::Skill,
            "context" => Self::Context,
            "tools" => Self::Tools,
            "user" => Self::User,
            "tool" => Self::Tool,
            "guard" => Self::Guard,
            "error" => Self::Error,
            _ => Self::Llm,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "SYSTEM",
            Self::Skill => "SKILL",
            Self::Context => "CONTEXT",
            Self::Tools => "TOOLS",
            Self::User => "USER",
            Self::Llm => "LLM",
            Self::Tool => "TOOL",
            Self::Guard => "GUARD",
            Self::Error => "ERROR",
        }
    }

    /// Whether the step is part of what the model was given before anything happened.
    pub fn is_setup(self) -> bool {
        matches!(
            self,
            Self::System | Self::Skill | Self::Context | Self::Tools
        )
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Step {
    /// 1-based, in the order they happened.
    pub seq: u32,
    pub kind: StepKind,
    pub name: String,
    pub status: Status,
    pub input: Option<String>,
    pub output: Option<String>,
    /// What the model said while thinking, when it said it.
    pub reasoning: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    /// Of the input, how much was read from and written to the provider's cache.
    pub cache_read_tokens: Option<u32>,
    pub cache_write_tokens: Option<u32>,
    pub duration_ms: Option<u32>,
    pub started_at: i64,
    /// When it began, in milliseconds after the run began.
    pub at_ms: Option<u32>,
    /// A model call: how long until the first of its reply arrived.
    pub first_ms: Option<u32>,
    /// Whether input or output was cut at its limit.
    pub truncated: bool,
}

impl Step {
    /// A step of `kind` with nothing filled in yet.
    pub fn new(seq: u32, kind: StepKind, started_at: i64) -> Step {
        Step {
            seq,
            kind,
            name: String::new(),
            status: Status::Ok,
            input: None,
            output: None,
            reasoning: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            duration_ms: None,
            started_at,
            at_ms: None,
            first_ms: None,
            truncated: false,
        }
    }
}

/// `text` cut to at most `limit` bytes at a character boundary, and whether it was cut.
pub fn clip(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_owned(), false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

/// The last `limit` bytes of `text` at a character boundary, and whether the start was cut: what
/// was sent to a model ends with what it answered.
pub fn clip_start(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_owned(), false);
    }
    let mut start = text.len() - limit;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    (text[start..].to_owned(), true)
}

/// What a run is called: the first line of what you said that has words in it.
pub fn title_of(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let mut title: String = line.chars().take(TITLE).collect();
    if line.chars().count() > TITLE {
        title.push('…');
    }
    title
}

/// Puts each handed-over run right after the run that handed it over, oldest first, and leaves the
/// rest in their order. A run whose parent is not in the list stands on its own.
pub fn nest(runs: Vec<Run>) -> Vec<Run> {
    let (children, mut top): (Vec<Run>, Vec<Run>) =
        runs.into_iter().partition(|r| r.parent.is_some());
    let mut children = children;
    children.sort_by_key(|r| r.started_at);
    let mut out = Vec::new();
    while let Some(run) = (!top.is_empty()).then(|| top.remove(0)) {
        let id = run.id.clone();
        out.push(run);
        let (mine, rest): (Vec<Run>, Vec<Run>) = children
            .into_iter()
            .partition(|c| c.parent.as_deref() == Some(id.as_str()));
        children = rest;
        out.extend(mine);
    }
    out.extend(children);
    out
}

/// The tools on offer as the record shows them: each one's name and arguments (`?` when it may be
/// left out), and what it is for.
pub fn offered(tools: &[ToolSpec]) -> String {
    tools
        .iter()
        .map(|t| {
            let required: Vec<&str> = t.schema["required"]
                .as_array()
                .map(|r| r.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let args: Vec<String> = t.schema["properties"]
                .as_object()
                .map(|p| {
                    p.keys()
                        .map(|k| {
                            if required.contains(&k.as_str()) {
                                k.clone()
                            } else {
                                format!("{k}?")
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            format!("{}({})\n{}", t.name, args.join(", "), t.description.trim())
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// What the model was given before the chat, and what started the run.
pub struct Setup<'a> {
    pub brief: &'a str,
    /// Each chosen skill's name and instructions.
    pub skills: &'a [(String, String)],
    pub context: &'a str,
    pub tools: &'a [ToolSpec],
    pub user: &'a str,
    /// What the user step is called when it is not a message of yours (an answer to a question).
    pub user_name: Option<&'a str>,
}

/// One model call, as it ended.
pub struct Reply<'a> {
    /// The messages it was sent, as readable text.
    pub sent: &'a str,
    pub text: &'a str,
    pub reasoning: &'a str,
    /// The tools it called; those with a result already (a tool that does not exist) are recorded
    /// at once, the rest when their result comes.
    pub calls: &'a [ToolCall],
    pub failure: Option<&'a str>,
    pub status: Status,
    pub tokens: Tokens,
    pub began: Instant,
    pub first: Option<Duration>,
}

/// A tool call that has no result yet.
struct Pending {
    id: String,
    name: String,
    args: String,
    since: Instant,
}

/// A run being recorded. Each method returns the steps it added, for saving with [`Trace::run`].
pub struct Trace {
    pub run: Run,
    began: Instant,
    seq: u32,
    pending: Vec<Pending>,
    /// How many secrets the run has said it hid from the model, so each is said once.
    hidden: usize,
    /// Whether the run has said outside text was in view.
    outside: bool,
}

fn ms(d: Duration) -> u32 {
    d.as_millis().min(u128::from(u32::MAX)) as u32
}

impl Trace {
    pub fn new(run: Run) -> Trace {
        Trace {
            run,
            began: Instant::now(),
            seq: 0,
            pending: Vec::new(),
            hidden: 0,
            outside: false,
        }
    }

    fn step(&mut self, kind: StepKind, at: Instant) -> Step {
        self.seq += 1;
        let offset = at.saturating_duration_since(self.began);
        let mut s = Step::new(
            self.seq,
            kind,
            self.run.started_at + offset.as_secs() as i64,
        );
        s.at_ms = Some(ms(offset));
        s
    }

    fn note(&mut self, kind: StepKind, name: &str, text: &str) -> Step {
        let mut s = self.step(kind, Instant::now());
        s.name = name.to_owned();
        let (text, cut) = clip(text, INPUT_LIMIT);
        (s.output, s.truncated) = (Some(text), cut);
        s
    }

    /// A guardrail at work, named for what it did.
    pub fn guard(&mut self, name: &str, text: &str) -> Step {
        self.note(StepKind::Guard, name, text)
    }

    /// The model call about to go hides `count` secrets, of the kinds `what` names: a step saying
    /// so when that is more than the run has said already.
    pub fn hid(&mut self, count: usize, what: &str) -> Option<Step> {
        if count <= self.hidden {
            return None;
        }
        self.hidden = count;
        let name = match count {
            1 => "1 secret hidden from the model".to_owned(),
            n => format!("{n} secrets hidden from the model"),
        };
        Some(self.guard(
            &name,
            &format!("Replaced with placeholders before sending: {what}. The model never sees them; tools get them back on this computer."),
        ))
    }

    /// What the model is about to be sent holds outside text (`what`, in words): a step saying so,
    /// once a run.
    pub fn outside(&mut self, what: &str) -> Option<Step> {
        if self.outside {
            return None;
        }
        self.outside = true;
        Some(self.guard(
            "Outside text in view",
            &format!("What the model is sent holds {what}, which could be steering it. While it does, its calls that run, change or send something wait for your go-ahead."),
        ))
    }

    /// The run's time and end move on to now.
    fn touch(&mut self) {
        self.run.duration_ms = Some(ms(self.began.elapsed()));
        self.run.ended_at = Some(crate::app::now());
    }

    /// The first steps: the brief, each skill, the context, the tools on offer, and what you said.
    pub fn setup(&mut self, s: &Setup) -> Vec<Step> {
        let mut steps = Vec::new();
        if !s.brief.trim().is_empty() {
            steps.push(self.note(StepKind::System, "", s.brief.trim()));
        }
        for (name, text) in s.skills {
            steps.push(self.note(StepKind::Skill, name, text));
        }
        if !s.context.is_empty() {
            steps.push(self.note(StepKind::Context, "", s.context));
        }
        if !s.tools.is_empty() {
            let names = s.tools.len().to_string();
            steps.push(self.note(StepKind::Tools, &names, &offered(s.tools)));
        }
        steps.push(self.note(StepKind::User, s.user_name.unwrap_or_default(), s.user));
        self.run.title = title_of(s.user);
        steps
    }

    /// A model call ended: its step, an error if it failed, and the tools it called that already
    /// have a result. The run is still running while a tool it called works.
    pub fn reply(&mut self, r: &Reply) -> Vec<Step> {
        let mut steps = Vec::new();
        let mut call = self.step(StepKind::Llm, r.began);
        call.status = r.status;
        let mut said = r.text.to_owned();
        for c in r.calls {
            if !said.is_empty() {
                said.push_str("\n\n");
            }
            said.push_str(&format!("[tool call] {} {}", c.name, c.args));
        }
        let ((sent, sent_cut), (said, said_cut)) =
            (clip_start(r.sent, INPUT_LIMIT), clip(&said, OUTPUT_LIMIT));
        (call.input, call.output) = (Some(sent), Some(said));
        call.reasoning = Some(clip(r.reasoning, OUTPUT_LIMIT).0).filter(|t| !t.is_empty());
        let t = r.tokens;
        (call.input_tokens, call.output_tokens) = (t.input, t.output);
        (call.cache_read_tokens, call.cache_write_tokens) = (t.cache_read, t.cache_write);
        call.duration_ms = Some(ms(r.began.elapsed()));
        call.first_ms = r.first.map(ms);
        call.truncated = sent_cut || said_cut;
        steps.push(call);
        self.run.calls += 1;
        let add = |total: Option<u32>, more: Option<u32>| match (total, more) {
            (None, None) => None,
            (t, m) => Some(t.unwrap_or(0).saturating_add(m.unwrap_or(0))),
        };
        let run = &mut self.run;
        run.input_tokens = add(run.input_tokens, t.input);
        run.output_tokens = add(run.output_tokens, t.output);
        run.cache_read_tokens = add(run.cache_read_tokens, t.cache_read);
        run.cache_write_tokens = add(run.cache_write_tokens, t.cache_write);

        if let Some(why) = r.failure {
            let mut e = self.note(StepKind::Error, "", why);
            e.status = Status::Error;
            steps.push(e);
        }
        if r.status == Status::Ok {
            for c in r.calls.iter().filter(|c| !c.id.is_empty()) {
                self.pending.push(Pending {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    args: c.args.clone(),
                    since: Instant::now(),
                });
                if let Some(out) = &c.output {
                    steps.extend(self.tool_done(&c.id, out));
                }
            }
        }
        self.run.status = match r.status {
            Status::Ok if self.working() => Status::Running,
            s => s,
        };
        self.run.error = r.failure.map(str::to_owned);
        self.touch();
        steps
    }

    /// A tool's result came back: its step. Your answer to a question does not move the run's
    /// end on: the time it waited for you is not the agent's.
    pub fn tool_done(&mut self, call_id: &str, output: &str) -> Option<Step> {
        let at = self.pending.iter().position(|p| p.id == call_id)?;
        let p = self.pending.remove(at);
        let mut s = self.step(StepKind::Tool, p.since);
        s.name = p.name;
        s.status = if output.starts_with("Error") {
            Status::Error
        } else {
            Status::Ok
        };
        let ((args, args_cut), (out, out_cut)) =
            (clip(&p.args, INPUT_LIMIT), clip(output, OUTPUT_LIMIT));
        (s.input, s.output, s.truncated) = (Some(args), Some(out), args_cut || out_cut);
        s.duration_ms = Some(ms(p.since.elapsed()));
        self.run.tools += 1;
        if s.name != crate::tools::ASK {
            if !self.working() && self.run.status == Status::Running {
                self.run.status = Status::Ok;
            }
            self.touch();
        }
        Some(s)
    }

    /// The model is called again with what its tools gave back.
    pub fn resume(&mut self) {
        self.run.status = Status::Running;
        self.run.error = None;
    }

    /// You stopped it while a tool worked.
    pub fn stop(&mut self) {
        if self.run.status == Status::Running {
            self.run.status = Status::Cancelled;
            self.touch();
        }
    }

    /// Whether a tool it called is still working (a question waiting for you does not count).
    pub fn working(&self) -> bool {
        self.pending.iter().any(|p| p.name != crate::tools::ASK)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn trace() -> Trace {
        Trace::new(Run::new(
            "r",
            "a",
            "Atlas",
            "s",
            ModelRef::new("openai", "gpt-x"),
        ))
    }

    fn call(id: &str, name: &str, output: Option<&str>) -> ToolCall {
        ToolCall {
            name: name.into(),
            summary: String::new(),
            output: output.map(str::to_owned),
            id: id.into(),
            args: r#"{"path":"a.txt"}"#.into(),
            trip: None,
            outside: None,
        }
    }

    fn reply<'a>(text: &'a str, calls: &'a [ToolCall]) -> Reply<'a> {
        Reply {
            sent: "[user]\nHi",
            text,
            reasoning: "",
            calls,
            failure: None,
            status: Status::Ok,
            tokens: Tokens {
                input: Some(12),
                output: Some(3),
                cache_read: Some(8),
                cache_write: None,
            },
            began: Instant::now(),
            first: Some(Duration::from_millis(40)),
        }
    }

    fn spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: "Reads a file.".into(),
            schema: json!({
                "type": "object",
                "properties": {"path": {"type": "string"}, "lines": {"type": "integer"}},
                "required": ["path"]
            }),
        }
    }

    #[test]
    fn a_run_opens_with_the_brief_each_skill_the_context_the_tools_and_your_message() {
        let mut t = trace();
        let skills = [("pdf".to_owned(), "Use pdftotext.".to_owned())];
        let steps = t.setup(&Setup {
            brief: "You are brief.",
            skills: &skills,
            context: "## Context\nIt is Monday.",
            tools: &[spec("read_file")],
            user: "\n  Read a.txt\nthen sum it",
            user_name: None,
        });
        let kinds: Vec<StepKind> = steps.iter().map(|s| s.kind).collect();
        use StepKind::*;
        assert_eq!(kinds, [System, Skill, Context, Tools, User]);
        assert_eq!(
            steps.iter().map(|s| s.seq).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5]
        );
        assert_eq!(steps[1].name, "pdf");
        assert_eq!(
            steps[3].output.as_deref(),
            Some("read_file(lines?, path)\nReads a file.")
        );
        assert_eq!(t.run.title, "Read a.txt", "named after its first line");

        let mut t = trace();
        let steps = t.setup(&Setup {
            brief: " ",
            skills: &[],
            context: "",
            tools: &[],
            user: "Hi",
            user_name: Some("Answer to your question"),
        });
        assert_eq!(steps.len(), 1, "nothing given, nothing recorded");
        assert_eq!(steps[0].name, "Answer to your question");
    }

    #[test]
    fn model_calls_and_their_tools_are_steps_of_one_run_whose_totals_add_up() {
        let mut t = trace();
        let calls = [
            call("c1", "read_file", None),
            call("c2", "nope", Some("Error: no tool")),
        ];
        let steps = t.reply(&reply("Reading.", &calls));
        assert_eq!(
            steps.iter().map(|s| s.kind).collect::<Vec<_>>(),
            [StepKind::Llm, StepKind::Tool],
            "a call with its result already is recorded at once"
        );
        assert_eq!(steps[1].status, Status::Error);
        assert_eq!(steps[0].first_ms, Some(40));
        assert!(
            steps[0]
                .output
                .as_deref()
                .unwrap()
                .contains("[tool call] read_file")
        );
        assert_eq!(t.run.status, Status::Running, "a tool is still working");

        let done = t.tool_done("c1", "hello").unwrap();
        assert_eq!(
            (done.kind, done.name.as_str()),
            (StepKind::Tool, "read_file")
        );
        assert_eq!(done.input.as_deref(), Some(r#"{"path":"a.txt"}"#));
        assert_eq!(done.output.as_deref(), Some("hello"));
        assert_eq!(t.run.status, Status::Ok, "nothing left working");
        assert!(t.tool_done("c1", "again").is_none(), "recorded once");

        t.resume();
        let steps = t.reply(&reply("The sum is 3.", &[]));
        assert_eq!(steps[0].seq, 4, "numbered on from the first call");
        assert_eq!((t.run.status, t.run.calls, t.run.tools), (Status::Ok, 2, 2));
        assert_eq!(
            (t.run.input_tokens, t.run.output_tokens),
            (Some(24), Some(6))
        );
        assert_eq!(
            (t.run.cache_read_tokens, t.run.cache_write_tokens),
            (Some(16), None),
            "cache counts add up like the rest, and stay unknown when never said"
        );
    }

    #[test]
    fn a_question_ends_the_run_for_now_and_a_failure_or_a_stop_ends_it_for_good() {
        let mut t = trace();
        t.reply(&reply("", &[call("q", crate::tools::ASK, None)]));
        assert_eq!(t.run.status, Status::Ok, "it waits on you, not on a tool");
        assert!(!t.working());
        let ended = t.run.duration_ms;
        std::thread::sleep(Duration::from_millis(5));
        assert!(t.tool_done("q", "Blue").is_some());
        assert_eq!(t.run.duration_ms, ended, "waiting for you is not its time");

        let mut t = trace();
        let mut r = reply("", &[]);
        (r.status, r.failure) = (Status::Error, Some("Rate limited."));
        let steps = t.reply(&r);
        assert_eq!(steps.last().unwrap().kind, StepKind::Error);
        assert_eq!(
            (t.run.status, t.run.error.as_deref()),
            (Status::Error, Some("Rate limited."))
        );

        let mut t = trace();
        let calls = [call("c", "shell", None)];
        t.reply(&reply("", &calls));
        t.stop();
        assert_eq!(t.run.status, Status::Cancelled);
    }

    #[test]
    fn long_text_is_cut_at_a_character_boundary_and_marked() {
        let (kept, cut) = clip("héllo wörld", 2);
        assert_eq!(kept, "h", "never in the middle of é");
        assert!(cut);
        assert_eq!(clip("short", 100), ("short".to_owned(), false));
        let (kept, cut) = clip_start("héllo wörld", 4);
        assert_eq!(kept, "rld", "the end is kept, never half of ö");
        assert!(cut);

        let long = format!("{}[user]\nlast", "x".repeat(INPUT_LIMIT));
        let mut t = trace();
        let mut r = reply("ok", &[]);
        r.sent = &long;
        let call = t.reply(&r).remove(0);
        assert!(call.input.as_deref().unwrap().ends_with("[user]\nlast"));
        assert_eq!(call.input.unwrap().len(), INPUT_LIMIT);
        assert!(call.truncated);
    }

    #[test]
    fn statuses_and_kinds_round_trip_through_their_saved_names() {
        for s in [
            Status::Running,
            Status::Ok,
            Status::Error,
            Status::Cancelled,
        ] {
            assert_eq!(Status::parse(s.as_str()), s);
        }
        use StepKind::*;
        for k in [System, Skill, Context, Tools, User, Llm, Tool, Error] {
            assert_eq!(StepKind::parse(k.as_str()), k);
        }
    }
}
