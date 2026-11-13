//! Workflows: something an agent did once, saved as steps that can be run again without a model
//! working out what to do. Modelled on `oido-core`'s workflows (taught from a run, shown as a
//! draft, saved, run), cut down: three step kinds, one version, runs started by you or by an
//! agent, no schedule.

pub mod compile;
pub mod definition;
pub mod editor;
pub mod engine;
pub mod expr;
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
    Succeeded,
    Failed,
    Stopped,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
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
            _ => Self::Running,
        }
    }
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
}
