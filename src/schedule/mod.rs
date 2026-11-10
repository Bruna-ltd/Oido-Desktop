//! Schedules: an agent's prompt or one of its workflows, fired on a [`Timing`]. Each occurrence is
//! a [`Fire`]. A schedule an agent proposes waits for approval as a [`Proposal`].

pub mod timing;
pub mod tool;

use chrono::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub use timing::Timing;

/// What a schedule does when it fires.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// Send this text to the agent in a new chat.
    Prompt(String),
    /// Run one of the agent's saved workflows with these inputs.
    Workflow {
        id: String,
        inputs: Map<String, Value>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    pub target: Target,
    pub timing: Timing,
    pub enabled: bool,
    /// The next fire; `None` once a "once" schedule has fired.
    pub next_at: Option<i64>,
    pub created_at: i64,
}

impl Schedule {
    /// A new schedule, on, first firing after `now`. Errs, in words, on an empty name or prompt,
    /// or a timing with nothing left to fire (a "once" in the past).
    pub fn new<Tz: TimeZone>(
        agent_id: &str,
        name: &str,
        target: Target,
        timing: Timing,
        now: i64,
        tz: &Tz,
    ) -> Result<Schedule, String> {
        let mut s = Schedule {
            id: crate::model::new_id(),
            agent_id: agent_id.to_owned(),
            name: String::new(),
            target,
            timing,
            enabled: true,
            next_at: None,
            created_at: now,
        };
        s.change(name, now, tz)?;
        Ok(s)
    }

    /// Takes a new name and recomputes the next fire from `now`, checking as [`Schedule::new`]
    /// does. Used after the target or timing was changed too.
    pub fn change<Tz: TimeZone>(&mut self, name: &str, now: i64, tz: &Tz) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a schedule needs a name".to_owned());
        }
        if let Target::Prompt(p) = &self.target
            && p.trim().is_empty()
        {
            return Err("the prompt is empty".to_owned());
        }
        let next = self
            .timing
            .next_after(now, tz)
            .ok_or_else(|| "that time has already passed".to_owned())?;
        self.name = name.to_owned();
        self.next_at = Some(next);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireStatus {
    Running,
    Succeeded,
    Failed,
    /// Came due while the previous fire was still going.
    Skipped,
}

impl FireStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    /// Reads a stored status; the table's `CHECK` allows no others, so anything else reads as
    /// failed rather than as still running.
    pub fn parse(s: &str) -> FireStatus {
        match s {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "skipped" => Self::Skipped,
            _ => Self::Failed,
        }
    }
}

/// One occurrence of a schedule.
#[derive(Clone, Debug, PartialEq)]
pub struct Fire {
    pub id: String,
    pub schedule_id: String,
    /// When it was due (the schedule's `next_at` at the time).
    pub due_at: i64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: FireStatus,
    /// Why it failed or was skipped.
    pub note: String,
    /// The chat it opened.
    pub session_id: Option<String>,
    /// The workflow run it made, for a workflow schedule.
    pub workflow_run_id: Option<String>,
}

/// A schedule an agent asked for with the `schedule` tool, shown in its chat to approve or reject.
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    /// The tool call that asked for it.
    pub call_id: String,
    pub session_id: String,
    /// It changes the saved schedule with the same id, rather than adding one.
    pub updates: bool,
    pub schedule: Schedule,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn prompt(text: &str) -> Target {
        Target::Prompt(text.into())
    }

    #[test]
    fn a_new_schedule_is_on_and_fires_after_now() {
        let t = Timing::every(1_000, 60).unwrap();
        let s = Schedule::new("a1", "  Digest ", prompt("Sum up"), t, 1_030, &Utc).unwrap();
        assert_eq!(
            (s.name.as_str(), s.enabled, s.next_at),
            ("Digest", true, Some(1_060))
        );
    }

    #[test]
    fn a_schedule_needs_a_name_a_prompt_and_a_time_to_come() {
        let every = || Timing::every(0, 60).unwrap();
        assert!(Schedule::new("a1", " ", prompt("x"), every(), 0, &Utc).is_err());
        assert!(Schedule::new("a1", "n", prompt(" \n"), every(), 0, &Utc).is_err());
        assert!(Schedule::new("a1", "n", prompt("x"), Timing::Once(5), 10, &Utc).is_err());
        let wf = Target::Workflow {
            id: "w1".into(),
            inputs: Map::new(),
        };
        assert!(Schedule::new("a1", "n", wf, every(), 0, &Utc).is_ok());
    }

    #[test]
    fn statuses_read_back_and_unknown_ones_read_as_failed() {
        use FireStatus::*;
        for s in [Running, Succeeded, Failed, Skipped] {
            assert_eq!(FireStatus::parse(s.as_str()), s);
        }
        assert_eq!(FireStatus::parse("??"), Failed);
    }
}
