//! Workflows: something an agent did once, saved as steps that can be run again without a model
//! working out what to do. Modelled on `oido-core`'s workflows (taught from a run, shown as a
//! draft, saved, run), cut down: three step kinds, one version, runs started by you or by an
//! agent, no schedule.

pub mod compile;
pub mod cond;
pub mod definition;
pub mod discover;
pub mod editor;
pub mod engine;
pub mod examples;
pub mod expr;
pub mod layout;
pub mod pause;
pub mod template;
pub mod trace;

use definition::Definition;
use engine::StepLog;

/// A saved workflow. It belongs to the agent that learned it. `definition` is the draft: what the
/// editor changes and a test run uses. Only `published` runs for real (a schedule, an agent).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Saved {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    pub goal: String,
    pub definition: Definition,
    pub created_at: i64,
    pub published: Option<Published>,
}

/// How the Workflows page orders its list.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sort {
    #[default]
    Newest,
    Oldest,
    Name,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::Newest, Sort::Oldest, Sort::Name];

    pub fn label(self) -> &'static str {
        match self {
            Self::Newest => "Newest",
            Self::Oldest => "Oldest",
            Self::Name => "Name",
        }
    }

    /// The workflows whose name or purpose holds `query` (any case), in this order.
    pub fn of<'a>(self, all: &'a [Saved], query: &str) -> Vec<&'a Saved> {
        let q = query.trim().to_lowercase();
        let mut out: Vec<&Saved> = all
            .iter()
            .filter(|w| {
                q.is_empty()
                    || w.name.to_lowercase().contains(&q)
                    || w.goal.to_lowercase().contains(&q)
            })
            .collect();
        match self {
            Self::Newest => out.sort_by_key(|w| std::cmp::Reverse(w.created_at)),
            Self::Oldest => out.sort_by_key(|w| w.created_at),
            Self::Name => out.sort_by_key(|w| w.name.to_lowercase()),
        }
        out
    }
}

/// The copy of a workflow that is live, and when it went live.
#[derive(Clone, PartialEq, Debug)]
pub struct Published {
    pub definition: Definition,
    pub at: i64,
}

impl Saved {
    /// What runs for real: the published copy, or nothing if it was never published.
    pub fn live(&self) -> Option<&Definition> {
        self.published.as_ref().map(|p| &p.definition)
    }

    /// This workflow with its draft published as it is, for tests that run one.
    #[cfg(test)]
    pub fn published_as_is(mut self) -> Saved {
        self.published = Some(Published {
            definition: self.definition.clone(),
            at: 0,
        });
        self
    }

    /// The draft has changed since it was published (or was never published).
    pub fn has_unpublished_changes(&self) -> bool {
        self.live() != Some(&self.definition)
    }
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

#[cfg(test)]
mod sort_tests {
    use super::*;

    fn named(name: &str, goal: &str, at: i64) -> Saved {
        Saved {
            name: name.into(),
            goal: goal.into(),
            created_at: at,
            ..Saved::default()
        }
    }

    #[test]
    fn the_list_is_searched_by_name_or_purpose_and_put_in_order() {
        let all = [
            named("Tidy downloads", "", 2),
            named("backup notes", "copies the Notes folder", 3),
            named("Changelog", "", 1),
        ];
        let names = |s: Sort, q: &str| -> Vec<&str> {
            s.of(&all, q).iter().map(|w| w.name.as_str()).collect()
        };
        assert_eq!(
            names(Sort::Newest, ""),
            ["backup notes", "Tidy downloads", "Changelog"]
        );
        assert_eq!(
            names(Sort::Oldest, ""),
            ["Changelog", "Tidy downloads", "backup notes"]
        );
        assert_eq!(
            names(Sort::Name, ""),
            ["backup notes", "Changelog", "Tidy downloads"]
        );
        assert_eq!(names(Sort::Name, " NOTES "), ["backup notes"]);
        assert!(names(Sort::Name, "zzz").is_empty());
    }
}
