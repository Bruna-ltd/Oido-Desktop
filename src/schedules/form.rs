//! The schedule form on the Schedules screen: what is typed, filled from a schedule or a draft, and
//! checked into the parts of a schedule. Times are typed as the `schedule` tool takes them.

use chrono::TimeZone;
use rune::TextEdit;
use serde_json::{Map, Value};

use crate::schedule::timing::{AtTime, Days};
use crate::schedule::tool::{clock, local_time};
use crate::schedule::{Schedule, Target, Timing};
use crate::workflow::Saved;

/// What a schedule does, as the form offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Prompt,
    Workflow,
}

/// When a schedule fires, as the form offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Once,
    Every,
    Days,
    Cron,
}

impl When {
    pub const ALL: [When; 4] = [When::Days, When::Every, When::Once, When::Cron];

    pub fn label(self) -> &'static str {
        match self {
            When::Days => "Daily / weekly",
            When::Every => "Every",
            When::Once => "Once",
            When::Cron => "Cron",
        }
    }
}

/// Where the form came from, which says what saving it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    New,
    /// Changes this saved schedule.
    Edit(String),
    /// Saves an agent's draft (the tool call that proposed it), as you changed it: approving it.
    Draft(String),
}

#[derive(Debug)]
pub struct Form {
    pub source: Source,
    pub agent_id: Option<String>,
    pub name: TextEdit,
    pub kind: Kind,
    pub prompt: TextEdit,
    pub workflow_id: Option<String>,
    pub inputs: Vec<(String, TextEdit)>,
    pub when: When,
    pub once: TextEdit,
    pub every: TextEdit,
    /// An "every" schedule keeps its grid when edited.
    pub every_start: Option<i64>,
    pub at: TextEdit,
    pub days: Days,
    pub cron: TextEdit,
    pub agent_open: bool,
    pub workflow_open: bool,
    /// Why the last save did not go through.
    pub notice: Option<String>,
}

impl Form {
    /// An empty form for `agent_id`: a prompt, every day at 09:00.
    pub fn new(agent_id: Option<String>) -> Form {
        Form {
            source: Source::New,
            agent_id,
            name: TextEdit::new(""),
            kind: Kind::Prompt,
            prompt: TextEdit::new(""),
            workflow_id: None,
            inputs: Vec::new(),
            when: When::Days,
            once: TextEdit::new(""),
            every: TextEdit::new("60"),
            every_start: None,
            at: TextEdit::new("09:00"),
            days: Days::EVERY,
            cron: TextEdit::new(""),
            agent_open: false,
            workflow_open: false,
            notice: None,
        }
    }

    /// The form filled in from `s`, to save as `source`.
    pub fn of<Tz: TimeZone>(s: &Schedule, source: Source, workflows: &[Saved], tz: &Tz) -> Form
    where
        Tz::Offset: std::fmt::Display,
    {
        let mut f = Form::new(Some(s.agent_id.clone()));
        f.source = source;
        f.name = TextEdit::new(&s.name);
        match &s.target {
            Target::Prompt(p) => f.prompt = TextEdit::new(p),
            Target::Workflow { id, inputs } => {
                f.kind = Kind::Workflow;
                f.pick_workflow(id, workflows);
                for (name, edit) in &mut f.inputs {
                    if let Some(v) = inputs.get(name) {
                        *edit = TextEdit::new(crate::workflow::template::as_text(v));
                    }
                }
            }
        }
        match &s.timing {
            Timing::Once(t) => {
                f.when = When::Once;
                let at = tz.timestamp_opt(*t, 0).earliest();
                f.once = TextEdit::new(
                    at.map_or_else(String::new, |d| d.format("%Y-%m-%d %H:%M").to_string()),
                );
            }
            Timing::Every { start, secs } => {
                f.when = When::Every;
                f.every = TextEdit::new((secs / 60).to_string());
                f.every_start = Some(*start);
            }
            t @ Timing::Cron { expr, .. } => match t.preset() {
                Some(at) => {
                    f.when = When::Days;
                    f.at = TextEdit::new(format!("{:02}:{:02}", at.hour, at.minute));
                    f.days = at.days;
                }
                None => {
                    f.when = When::Cron;
                    f.cron = TextEdit::new(expr);
                }
            },
        }
        f
    }

    /// Chooses the workflow to run, and lays out its inputs (with their defaults).
    pub fn pick_workflow(&mut self, id: &str, workflows: &[Saved]) {
        self.workflow_id = Some(id.to_owned());
        self.inputs = workflows
            .iter()
            .find(|w| w.id == id)
            .map(|w| {
                w.definition
                    .inputs
                    .iter()
                    .map(|(name, i)| {
                        let default = i.default.as_ref().map(crate::workflow::template::as_text);
                        (name.clone(), TextEdit::new(default.unwrap_or_default()))
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    /// The schedule's parts as typed: its agent, name, target and timing. Errs, in words, on
    /// anything missing or unreadable. `now` starts a new "every" schedule.
    pub fn read<Tz: TimeZone>(
        &self,
        workflows: &[Saved],
        now: i64,
        tz: &Tz,
    ) -> Result<(String, String, Target, Timing), String> {
        let agent = self.agent_id.clone().ok_or("Choose an agent.")?;
        let name = self.name.text.trim().to_owned();
        if name.is_empty() {
            return Err("Give it a name.".to_owned());
        }
        let target = match self.kind {
            Kind::Prompt => Target::Prompt(self.prompt.text.clone()),
            Kind::Workflow => {
                let id = self.workflow_id.clone().ok_or("Choose a workflow.")?;
                let wf = workflows
                    .iter()
                    .find(|w| w.id == id && w.agent_id == agent)
                    .ok_or("Choose one of this agent's workflows.")?;
                let inputs: Map<String, Value> = self
                    .inputs
                    .iter()
                    .filter(|(_, e)| !e.text.trim().is_empty())
                    .map(|(n, e)| (n.clone(), Value::String(e.text.clone())))
                    .collect();
                crate::workflow::engine::bind_inputs(&wf.definition, &inputs)?;
                Target::Workflow { id, inputs }
            }
        };
        let timing = match self.when {
            When::Once => Timing::Once(local_time(&self.once.text, tz)?),
            When::Every => {
                let minutes: i64 = self
                    .every
                    .text
                    .trim()
                    .parse()
                    .map_err(|_| "Give the minutes as a whole number.")?;
                Timing::every(self.every_start.unwrap_or(now), minutes.saturating_mul(60))?
            }
            When::Days => {
                if self.days.is_empty() {
                    return Err("Choose at least one day.".to_owned());
                }
                let (hour, minute) = clock(&self.at.text)?;
                Timing::at(AtTime {
                    hour,
                    minute,
                    days: self.days,
                })?
            }
            When::Cron => Timing::cron(&self.cron.text)?,
        };
        Ok((agent, name, target, timing))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;
    use crate::workflow::definition::Definition;

    fn workflow() -> Saved {
        Saved {
            id: "w1".into(),
            agent_id: "a1".into(),
            name: "Digest".into(),
            goal: String::new(),
            definition: Definition {
                inputs: serde_json::from_value(
                    json!({"folder": {"type": "string", "default": "notes"}}),
                )
                .unwrap(),
                ..Definition::default()
            },
            created_at: 0,
        }
    }

    fn round_trip(s: &Schedule) -> Schedule {
        let wfs = [workflow()];
        let f = Form::of(s, Source::Edit(s.id.clone()), &wfs, &Utc);
        let (agent, name, target, timing) = f.read(&wfs, 0, &Utc).unwrap();
        Schedule::new(&agent, &name, target, timing, 0, &Utc).unwrap()
    }

    #[test]
    fn a_schedule_reads_back_from_its_form_unchanged() {
        let target = Target::Workflow {
            id: "w1".into(),
            inputs: json!({"folder": "inbox"}).as_object().unwrap().clone(),
        };
        for timing in [
            Timing::Once(1_899_999_960),
            Timing::every(120, 15 * 60).unwrap(),
            Timing::at(AtTime {
                hour: 7,
                minute: 5,
                days: Days::WEEKDAYS,
            })
            .unwrap(),
            Timing::cron("*/10 9-17 * * 1-5").unwrap(),
        ] {
            let s = Schedule::new("a1", "Digest", target.clone(), timing, 0, &Utc).unwrap();
            let back = round_trip(&s);
            assert_eq!((back.target, back.timing), (s.target, s.timing));
        }
        let p = Schedule::new(
            "a1",
            "Note",
            Target::Prompt("Hi".into()),
            Timing::Once(1_899_999_960),
            0,
            &Utc,
        )
        .unwrap();
        assert_eq!(round_trip(&p).target, p.target);
    }

    #[test]
    fn a_new_form_starts_every_day_at_nine_and_says_what_is_missing() {
        let wfs = [workflow()];
        let mut f = Form::new(None);
        assert_eq!(f.read(&wfs, 0, &Utc).unwrap_err(), "Choose an agent.");
        f.agent_id = Some("a1".into());
        assert_eq!(f.read(&wfs, 0, &Utc).unwrap_err(), "Give it a name.");
        f.name = TextEdit::new("Morning");
        f.prompt = TextEdit::new("Plan my day");
        let (_, _, _, timing) = f.read(&wfs, 0, &Utc).unwrap();
        assert_eq!(timing.describe(&Utc), "every day at 09:00");
        f.days = Days::NONE;
        assert!(f.read(&wfs, 0, &Utc).unwrap_err().contains("day"));
        f.kind = Kind::Workflow;
        assert_eq!(f.read(&wfs, 0, &Utc).unwrap_err(), "Choose a workflow.");
    }

    #[test]
    fn picking_a_workflow_lays_out_its_inputs_with_their_defaults() {
        let mut f = Form::new(Some("a1".into()));
        f.pick_workflow("w1", &[workflow()]);
        assert_eq!(f.inputs.len(), 1);
        assert_eq!(
            (f.inputs[0].0.as_str(), f.inputs[0].1.text.as_str()),
            ("folder", "notes")
        );
        let mut other = workflow();
        other.agent_id = "a2".into();
        f.name = TextEdit::new("n");
        f.kind = Kind::Workflow;
        assert!(
            f.read(&[other], 0, &Utc)
                .unwrap_err()
                .contains("this agent")
        );
    }
}
