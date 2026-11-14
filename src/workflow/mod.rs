//! Workflows: something an agent did once, saved as steps that can be run again without a model
//! working out what to do. Modelled on `oido-core`'s workflows (taught from a run, shown as a
//! draft, saved, run), cut down: three step kinds, one version, runs started by you or by an
//! agent, no schedule.

pub mod compile;
pub mod definition;
pub mod editor;
pub mod engine;
pub mod expr;
pub mod layout;
pub mod pause;
pub mod template;

use definition::Definition;
use engine::StepLog;

/// A saved workflow. It belongs to the agent that learned it.
#[derive(Clone, PartialEq, Debug)]
pub struct Saved {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    pub goal: String,
    pub definition: Definition,
    pub created_at: i64,
}

/// A workflow just learned from a chat and not saved yet. Nothing runs from a draft.
#[derive(Clone, PartialEq, Debug)]
pub struct Draft {
    pub session_id: String,
    pub agent_id: String,
    pub name: String,
    pub goal: String,
    pub definition: Definition,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RunStatus {
    Running,
    /// Stopped at a `wait`. Not stored as a status: a run is waiting if it has a [`Pause`].
    Waiting,
    Succeeded,
    Failed,
    Stopped,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }

    pub fn parse(s: &str) -> RunStatus {
        match s {
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "stopped" => Self::Stopped,
            "waiting" => Self::Waiting,
            _ => Self::Running,
        }
    }
}

/// What a waiting run is waiting for.
#[derive(Clone, PartialEq, Debug)]
pub struct Pause {
    /// The id of the `wait` step.
    pub step: String,
    /// What it asks you, or says it waits for.
    pub prompt: String,
    /// When a delay is over, in seconds since the epoch; none for an approval.
    pub until: Option<i64>,
}

/// One execution of a workflow.
#[derive(Clone, PartialEq, Debug)]
pub struct RunRecord {
    pub id: String,
    pub workflow_id: String,
    /// What started it: "you" (the Workflows page) or "agent".
    pub started_by: String,
    pub status: RunStatus,
    /// The values it started with, as JSON text.
    pub inputs: String,
    pub output: String,
    pub failed_step: String,
    pub error: String,
    pub steps: Vec<StepLog>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// Set while it waits at a `wait`.
    pub pause: Option<Pause>,
}
