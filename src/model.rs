//! What the screens show. Mock data until `oido-core` is wired in: each type mirrors a Studio
//! shape (`ChatBox/types.ts`, the run trace kinds) so the logic layer can fill it later.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentStatus {
    Idle,
    Running,
    /// Blocked on an approval gate.
    Waiting,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approve,
    Reject,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub summary: String,
    pub output: Option<String>,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Approval {
    pub action: String,
    pub decision: Option<Decision>,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Stats {
    pub model: String,
    pub tokens: u32,
    pub seconds: f32,
}

/// One entry in a conversation. Stored as JSON, tagged by `type`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Message {
    User(String),
    /// "Start fresh": the agent forgets everything above, the screen keeps it.
    Divider(String),
    Assistant {
        text: String,
        thought_secs: Option<f32>,
        tools: Vec<ToolCall>,
        approval: Option<Approval>,
        stats: Option<Stats>,
    },
}

/// One tab: a conversation with an agent. The id is stable across launches.
#[derive(Clone, PartialEq, Debug)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub messages: Vec<Message>,
}

pub fn chat(title: &str, messages: Vec<Message>) -> Session {
    Session {
        id: new_id(),
        title: title.into(),
        messages,
    }
}

/// A fresh opaque id, safe to persist: random, so it never collides with an earlier launch's.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Done,
    Running,
    Failed,
    Waiting,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    System,
    User,
    Context,
    Assistant,
    Tool,
    Approval,
    Subagent,
}

impl RecordKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "SYSTEM",
            Self::User => "USER",
            Self::Context => "CONTEXT",
            Self::Assistant => "ASSISTANT",
            Self::Tool => "TOOL",
            Self::Approval => "APPROVAL",
            Self::Subagent => "SUBAGENT",
        }
    }
}

pub struct Record {
    pub kind: RecordKind,
    pub title: &'static str,
    pub millis: u32,
    pub detail: &'static str,
}

pub struct Run {
    pub id: &'static str,
    pub agent: &'static str,
    pub status: RunStatus,
    pub started: &'static str,
    pub seconds: f32,
    pub tokens: u32,
    pub records: Vec<Record>,
}

pub fn runs() -> Vec<Run> {
    let rec = |kind, title, millis, detail| Record {
        kind,
        title,
        millis,
        detail,
    };
    vec![
        Run {
            id: "run_7f3a",
            agent: "Atlas",
            status: RunStatus::Waiting,
            started: "2 min ago",
            seconds: 8.9,
            tokens: 3120,
            records: vec![
                rec(
                    RecordKind::System,
                    "System prompt · 14 tools",
                    0,
                    "You are Atlas, the research agent for this organization.\nBusiness profile and org wiki index are injected.",
                ),
                rec(
                    RecordKind::User,
                    "Pull last week's churn numbers…",
                    0,
                    "Pull last week's churn numbers and draft the Monday brief.",
                ),
                rec(
                    RecordKind::Context,
                    "Business profile, wiki index",
                    40,
                    "Org: Oido Labs · Timezone: Europe/Nicosia",
                ),
                rec(
                    RecordKind::Assistant,
                    "Plan: read export, search wiki",
                    1900,
                    "Reading the usage export first, then the churn definition.",
                ),
                rec(
                    RecordKind::Tool,
                    "storage.read",
                    310,
                    "path: exports/usage-week-44.csv\n→ 1,284 rows · 9 columns",
                ),
                rec(
                    RecordKind::Tool,
                    "wiki.search",
                    220,
                    "query: churn definition\n→ Churn = paid orgs with no run in 30 days.",
                ),
                rec(
                    RecordKind::Assistant,
                    "Draft brief",
                    3800,
                    "## Monday brief\n- Churn: 2.1%, down from 2.8%",
                ),
                rec(
                    RecordKind::Approval,
                    "Post to #leadership",
                    0,
                    "Waiting for a decision in chat.",
                ),
            ],
        },
        Run {
            id: "run_7f39",
            agent: "Forge",
            status: RunStatus::Running,
            started: "5 min ago",
            seconds: 41.0,
            tokens: 8890,
            records: vec![
                rec(
                    RecordKind::User,
                    "Review PR 482 and run the test suite.",
                    0,
                    "Review PR 482 and run the test suite.",
                ),
                rec(
                    RecordKind::Tool,
                    "github.diff",
                    480,
                    "PR 482 → 6 files · +212 −48",
                ),
                rec(
                    RecordKind::Subagent,
                    "reviewer",
                    22000,
                    "Reading the diff for unsafe blocks and missing tests.",
                ),
                rec(RecordKind::Tool, "shell.run", 0, "cargo test (running)"),
            ],
        },
        Run {
            id: "run_7f2c",
            agent: "Scout",
            status: RunStatus::Done,
            started: "1 h ago",
            seconds: 4.1,
            tokens: 960,
            records: vec![
                rec(RecordKind::User, "Triage the inbox", 0, "Triage the inbox."),
                rec(
                    RecordKind::Tool,
                    "gmail.list",
                    520,
                    "→ 18 unread, 3 flagged",
                ),
                rec(
                    RecordKind::Assistant,
                    "Summary",
                    1300,
                    "Three messages need a reply today.",
                ),
            ],
        },
        Run {
            id: "run_7f11",
            agent: "Ledger",
            status: RunStatus::Failed,
            started: "yesterday",
            seconds: 2.3,
            tokens: 410,
            records: vec![
                rec(
                    RecordKind::User,
                    "Reconcile October invoices",
                    0,
                    "Reconcile October invoices.",
                ),
                rec(
                    RecordKind::Tool,
                    "stripe.list_invoices",
                    900,
                    "Error: provider key expired (401)",
                ),
            ],
        },
    ]
}
