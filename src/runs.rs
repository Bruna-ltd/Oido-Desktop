//! Runs: what each reply cost and how it went, kept so the Runs screen can show it. A run is one
//! message you sent and the model's answer to it; its steps are what happened inside: the brief
//! it was given, your message, the model call (exactly what was sent and what came back), and an
//! error if there was one. Modelled on `oido-core`'s run ledger.

use crate::models::ModelRef;

/// How long runs are kept. Older ones are removed when the app starts; chats are never touched.
pub const KEEP_SECS: i64 = 30 * 24 * 60 * 60;

/// A step's input is cut at this many bytes, and its output at the next: a long chat is sent in
/// full on every message, and would otherwise be saved in full every time.
pub const INPUT_LIMIT: usize = 8 * 1024;
pub const OUTPUT_LIMIT: usize = 32 * 1024;

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
    pub status: Status,
    pub error: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub duration_ms: Option<u32>,
    /// Seconds since the epoch.
    pub started_at: i64,
    pub ended_at: Option<i64>,
    /// The run that handed this one to a team member. Its token totals include this run's.
    pub parent: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepKind {
    /// The agent's brief.
    System,
    /// Your message.
    User,
    /// The model call: what was sent, and what came back.
    Llm,
    Error,
}

impl StepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Llm => "llm",
            Self::Error => "error",
        }
    }

    pub fn parse(s: &str) -> StepKind {
        match s {
            "system" => Self::System,
            "user" => Self::User,
            "error" => Self::Error,
            _ => Self::Llm,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "SYSTEM",
            Self::User => "USER",
            Self::Llm => "ASSISTANT",
            Self::Error => "ERROR",
        }
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
    pub duration_ms: Option<u32>,
    pub started_at: i64,
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
            duration_ms: None,
            started_at,
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

/// The record of one run: its steps in order, with the model call carrying what was sent and
/// what came back. `sent` is what the model was given as a readable transcript.
pub struct Record<'a> {
    pub system: Option<&'a str>,
    pub user: &'a str,
    /// What the user step is called when it is not a message of yours (an answer to a question).
    pub user_name: Option<&'a str>,
    pub calls: &'a [crate::model::ToolCall],
    pub sent: &'a str,
    pub reply: &'a str,
    pub reasoning: &'a str,
    pub failure: Option<&'a str>,
    pub status: Status,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub duration_ms: u32,
    pub started_at: i64,
}

impl Record<'_> {
    pub fn steps(&self) -> Vec<Step> {
        let mut steps = Vec::new();
        let (mut seq, started_at) = (0u32, self.started_at);
        let mut next = |kind| {
            seq += 1;
            Step::new(seq, kind, started_at)
        };
        if let Some(system) = self.system.filter(|s| !s.is_empty()) {
            let mut s = next(StepKind::System);
            let (text, cut) = clip(system, INPUT_LIMIT);
            (s.output, s.truncated) = (Some(text), cut);
            steps.push(s);
        }
        let mut user = next(StepKind::User);
        user.name = self.user_name.unwrap_or_default().to_owned();
        let (text, cut) = clip(self.user, INPUT_LIMIT);
        (user.output, user.truncated) = (Some(text), cut);
        steps.push(user);

        let mut call = next(StepKind::Llm);
        call.status = self.status;
        let (sent, sent_cut) = clip(self.sent, INPUT_LIMIT);
        let mut said = self.reply.to_owned();
        for c in self.calls {
            if !said.is_empty() {
                said.push_str("\n\n");
            }
            said.push_str(&format!("[tool call] {} {}", c.name, c.args));
        }
        let (reply, reply_cut) = clip(&said, OUTPUT_LIMIT);
        call.input = Some(sent);
        call.output = Some(reply);
        call.reasoning = Some(self.reasoning.to_owned()).filter(|r| !r.is_empty());
        call.input_tokens = self.input_tokens;
        call.output_tokens = self.output_tokens;
        call.duration_ms = Some(self.duration_ms);
        call.truncated = sent_cut || reply_cut;
        steps.push(call);

        if let Some(why) = self.failure {
            let mut e = next(StepKind::Error);
            e.status = Status::Error;
            e.output = Some(why.to_owned());
            steps.push(e);
        }
        steps
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn record(status: Status, failure: Option<&str>) -> Record<'_> {
        Record {
            system: Some("You are brief."),
            user: "Hi",
            user_name: None,
            calls: &[],
            sent: "user: Hi",
            reply: "Hello!",
            reasoning: "",
            failure,
            status,
            input_tokens: Some(12),
            output_tokens: Some(3),
            duration_ms: 820,
            started_at: 1000,
        }
    }

    #[test]
    fn a_run_records_the_brief_the_message_the_model_call_and_a_failure_if_any() {
        let steps = record(Status::Ok, None).steps();
        let kinds: Vec<StepKind> = steps.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, vec![StepKind::System, StepKind::User, StepKind::Llm]);
        assert_eq!(
            steps.iter().map(|s| s.seq).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "numbered in order"
        );
        let call = &steps[2];
        assert_eq!(
            (call.input.as_deref(), call.output.as_deref()),
            (Some("user: Hi"), Some("Hello!"))
        );
        assert_eq!(
            (call.input_tokens, call.output_tokens, call.duration_ms),
            (Some(12), Some(3), Some(820))
        );

        let steps = record(Status::Error, Some("Rate limited.")).steps();
        assert_eq!(steps.last().unwrap().kind, StepKind::Error);
        assert_eq!(
            steps.last().unwrap().output.as_deref(),
            Some("Rate limited.")
        );
        assert_eq!(
            steps[2].status,
            Status::Error,
            "the model call carries the failure too"
        );
    }

    #[test]
    fn an_agent_with_no_brief_has_no_system_step_and_empty_reasoning_is_not_kept() {
        let mut r = record(Status::Ok, None);
        r.system = None;
        let steps = r.steps();
        assert_eq!(steps[0].kind, StepKind::User);
        assert_eq!(steps[0].seq, 1);
        assert!(steps.last().unwrap().reasoning.is_none());
        r.reasoning = "Let me think.";
        assert_eq!(
            r.steps().last().unwrap().reasoning.as_deref(),
            Some("Let me think.")
        );
    }

    #[test]
    fn long_text_is_cut_at_a_character_boundary_and_marked() {
        let (kept, cut) = clip("héllo wörld", 2);
        assert_eq!(kept, "h", "never in the middle of é");
        assert!(cut);
        assert_eq!(clip("short", 100), ("short".to_owned(), false));
        let long = "x".repeat(INPUT_LIMIT + 50);
        let mut r = record(Status::Ok, None);
        r.sent = &long;
        let call = r
            .steps()
            .into_iter()
            .find(|s| s.kind == StepKind::Llm)
            .unwrap();
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
        for k in [
            StepKind::System,
            StepKind::User,
            StepKind::Llm,
            StepKind::Error,
        ] {
            assert_eq!(StepKind::parse(k.as_str()), k);
        }
    }
}
