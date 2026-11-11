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

#[derive(Clone, Default, PartialEq, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub summary: String,
    /// What came back. `None` while it is still waiting (for a question, for your answer).
    pub output: Option<String>,
    /// The model's id for the call and its arguments as written, with any value hidden from the
    /// model put back; empty on the sample data.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub args: String,
    /// The guardrail it tripped, if any: refused, or held for your go-ahead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trip: Option<crate::guard::Trip>,
    /// What its result is, in words, when it is outside text (a web page, an MCP tool's result):
    /// while that is in what the model is sent, its calls that act wait (see `guard::taint`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outside: Option<String>,
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
    /// The tokens the model had in view by the end of the call, as the provider counted them:
    /// everything sent (cached or not) and the reply. How full the chat was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u32>,
    /// The tokens, estimated, trimmed from what the call sent to make it fit; `None` if it went
    /// whole. See `docs/context.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trimmed: Option<u32>,
}

/// How a reply ended. `Working` is the reply as it is being written and is never saved.
#[derive(Clone, Default, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "state", content = "why", rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    Done,
    Working,
    /// You pressed Stop; what had been written stays.
    Stopped,
    /// It failed, with the reason in words; what had been written stays.
    Failed(String),
}

/// One entry in a conversation. Stored as JSON, tagged by `type`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Message {
    User(crate::prompt::Prompt),
    /// "Start fresh": the agent forgets everything above, the screen keeps it.
    Divider(String),
    /// Older messages written in brief to make room (see `docs/context.md`). The model is sent
    /// this in place of everything above it, then the `keeps` messages right above it word for
    /// word, then what follows. The screen keeps them all.
    Summary {
        text: String,
        keeps: usize,
        /// What outside text it was written from, if any: it carries that on (see `guard::taint`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outside: Option<String>,
    },
    Assistant {
        text: String,
        /// What the model said while thinking, for the screen only: never sent back to it.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        thinking: String,
        thought_secs: Option<f32>,
        tools: Vec<ToolCall>,
        approval: Option<Approval>,
        stats: Option<Stats>,
        /// The run that wrote it, so the chat can open it in Runs.
        #[serde(default)]
        run: Option<String>,
        #[serde(default)]
        outcome: Outcome,
    },
}

impl Message {
    /// A reply that is just text, as a model writes it: no tools, no approval.
    pub fn reply(text: impl Into<String>, run: Option<String>, outcome: Outcome) -> Message {
        Message::Assistant {
            text: text.into(),
            thinking: String::new(),
            thought_secs: None,
            tools: Vec::new(),
            approval: None,
            stats: None,
            run,
            outcome,
        }
    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn replies_saved_before_thinking_was_kept_still_load_and_none_is_not_written() {
        let old = r#"{"type":"assistant","data":{"text":"hi","thought_secs":null,"tools":[],"approval":null,"stats":null}}"#;
        let m: Message = serde_json::from_str(old).unwrap();
        assert_eq!(m, Message::reply("hi", None, Outcome::Done));
        let saved = serde_json::to_string(&m).unwrap();
        assert!(!saved.contains("thinking"), "{saved}");
    }
}
