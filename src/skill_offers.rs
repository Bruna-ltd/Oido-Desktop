//! Spotting runs worth a skill (see `docs/skills-from-runs.md`, ADR-6 to ADR-8): when a run ends
//! well, TypeSafe judges whether it holds something worth keeping that no existing skill covers.
//! If so, its chat offers to make one. Whether runs are judged is set for all agents in Settings,
//! and each agent can override that.

use std::time::Duration;

use crate::app::App;
use crate::runs::Step;
use crate::skill_draft::{Known, known_block, transcript};

/// What TypeSafe is asked about a finished run, task success first. The last is asked only when
/// there are skills it could already be in.
pub const QUESTIONS: [(&str, &str); 4] = [
    (
        "task_success",
        "Did the agent complete the person's task successfully?",
    ),
    (
        "reusable",
        "Does this run contain an approach, fix or debugging pattern that a future agent doing a similar task should reuse?",
    ),
    (
        "evidence",
        "Is that reusable lesson backed by concrete steps in the run, such as commands, results or errors?",
    ),
    (
        "covered",
        "Is that reusable lesson already fully covered by one of the skills listed under <skills>, so that nothing in it needs correcting or adding?",
    ),
];
/// Task success below this rules a run out, whatever the other answers: a mean alone lets two high
/// answers outvote a failed task (0.27, 0.86, 0.69 averages 0.61).
const TASK_FLOOR: f64 = 0.50;
/// The mean of the first three answers a run needs, once its task succeeded.
const MEAN_FLOOR: f64 = 0.60;
/// From this likelihood that an existing skill covers it, a run is not offered.
const COVERED_CEILING: f64 = 0.50;
const SCORE_WAIT: Duration = Duration::from_secs(30);

/// Whether answers to [`QUESTIONS`], in their order, mark a run as worth a skill: its task
/// succeeded, the first three are high on average, and no existing skill covers it (when that was
/// asked).
pub fn worth(answers: &[f64]) -> bool {
    let (Some(&task), Some(first)) = (answers.first(), answers.get(..3)) else {
        return false;
    };
    let covered = answers.get(3).copied().unwrap_or(0.0);
    task >= TASK_FLOOR && first.iter().sum::<f64>() / 3.0 >= MEAN_FLOOR && covered < COVERED_CEILING
}

/// What TypeSafe is shown of a run: the skills it could already be in, then its transcript, with
/// every secret and personal detail found in either hidden.
pub fn judged_text(steps: &[Step], known: &[Known]) -> String {
    let mut text = transcript(steps);
    if !known.is_empty() {
        text = format!("{}\n\n<run>\n{text}\n</run>", known_block(known, false));
    }
    let mut hidden = crate::guard::Hidden::default();
    hidden.learn(&text);
    hidden.learn_personal(&text);
    hidden.hide(&text)
}

impl App {
    /// Settings: TypeSafe judges finished runs for a skill, or does not, for agents that do not
    /// say otherwise.
    pub fn set_score_runs(&mut self, on: bool) {
        self.score_runs = on;
        self.save("the run scoring setting", |db| {
            db.set_setting(crate::persist::SCORE_RUNS, &on)
        });
    }

    /// An agent's own choice: judge its runs (`Some(true)`), never (`Some(false)`), or as Settings
    /// says (`None`).
    pub fn set_score_for(&mut self, agent_id: &str, choice: Option<bool>) {
        match choice {
            Some(on) => self.score_overrides.insert(agent_id.to_owned(), on),
            None => self.score_overrides.remove(agent_id),
        };
        let all = self.score_overrides.clone();
        self.save("the run scoring setting", |db| {
            db.set_setting(crate::persist::SCORE_AGENTS, &all)
        });
    }

    /// Whether agent `agent_id`'s runs are judged: its own choice, else Settings'.
    pub fn scores_for(&self, agent_id: &str) -> bool {
        self.score_overrides
            .get(agent_id)
            .copied()
            .unwrap_or(self.score_runs)
    }

    /// Chat `sid`'s run finished well: if its agent's runs are judged and TypeSafe is connected,
    /// TypeSafe judges in the background whether it is worth a skill. A team member's run is not
    /// judged.
    pub fn score_run(&mut self, sid: &str) {
        let Some(run) = self.traces.get(sid).map(|t| t.run.clone()) else {
            return;
        };
        if !self.scores_for(&run.agent_id)
            || !self.typesafe_connected()
            || run.parent.is_some()
            || run.calls == 0
        {
            return;
        }
        let key = self
            .store
            .as_ref()
            .and_then(|db| db.provider_key(crate::providers::TYPESAFE).ok().flatten());
        let steps = self
            .store
            .as_ref()
            .and_then(|db| db.list_steps(&run.id).ok())
            .unwrap_or_default();
        let (Some(key), false) = (key, steps.is_empty()) else {
            return;
        };
        let known = self.known_skills(&run.agent_id, &steps);
        let asked = if known.is_empty() { 3 } else { 4 };
        let text = judged_text(&steps, &known);
        let id = run.id;
        self.spawn(
            move || {
                crate::typesafe::probabilities(
                    crate::typesafe::BASE,
                    &key,
                    &text,
                    &QUESTIONS[..asked],
                    SCORE_WAIT,
                )
            },
            move |app, got| match got {
                Ok(answers) => app.scored(&id, worth(&answers)),
                Err(why) => eprintln!("oido: TypeSafe could not judge the run: {why}"),
            },
        );
    }

    fn scored(&mut self, id: &str, worth: bool) {
        self.save("the run's verdict", |db| db.set_run_worth(id, worth));
        if let Some(r) = self.runs.iter_mut().find(|r| r.id == id) {
            r.worth = Some(worth);
        }
        if worth && !self.skill_offers.iter().any(|o| o == id) {
            self.skill_offers.push(id.to_owned());
        }
    }

    /// Whether run `id`'s chat offers to make a skill of it.
    pub fn offers_skill(&self, id: &str) -> bool {
        self.skill_offers.iter().any(|o| o == id)
    }

    /// The offer's Review: the run opens with its draft being written.
    pub fn review_skill_offer(&mut self, id: &str) {
        self.open_run(id);
        if self.runs.get(self.run).is_some_and(|r| r.id == id) {
            self.make_skill();
        }
    }

    /// The offer is answered, by Not now or by saving a skill from the run: it is not made again.
    pub fn close_skill_offer(&mut self, id: &str) {
        if !self.offers_skill(id) {
            return;
        }
        self.skill_offers.retain(|o| o != id);
        self.save("the skill offer", |db| db.close_skill_offer(id));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::runs::StepKind;

    #[test]
    fn a_run_is_worth_a_skill_only_when_its_task_succeeded_the_answers_are_high_and_nothing_covers_it()
     {
        assert!(worth(&[0.9, 0.8, 0.7]));
        assert!(
            !worth(&[0.27, 0.86, 0.69]),
            "a failed task is not averaged in (beacon #649)"
        );
        assert!(!worth(&[0.9, 0.3, 0.4]), "a mean of 0.53");
        assert!(worth(&[0.5, 0.65, 0.65]), "both floors are inclusive");
        assert!(worth(&[0.9, 0.8, 0.7, 0.2]), "no skill covers it");
        assert!(!worth(&[0.9, 0.8, 0.7, 0.8]), "a skill covers it already");
        assert!(!worth(&[0.9, 0.8]));
        assert!(!worth(&[]));
    }

    #[test]
    fn what_typesafe_sees_names_the_skills_it_may_be_in_with_secrets_hidden() {
        let mut s = Step::new(1, StepKind::User, 0);
        s.output = Some(
            "deploy with sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCD and mail ops@example.com"
                .into(),
        );
        let text = judged_text(std::slice::from_ref(&s), &[]);
        assert!(
            !text.contains("sk-ant-api03") && !text.contains("ops@example.com"),
            "{text}"
        );
        assert!(text.starts_with("[person] deploy with "));
        let known = [Known {
            name: "deploy".into(),
            description: "Deploy the app.".into(),
            body: "Run make deploy.".into(),
        }];
        let text = judged_text(&[s], &known);
        assert!(
            text.starts_with("<skills>\n- deploy: Deploy the app.\n</skills>\n\n<run>\n[person]"),
            "{text}"
        );
        assert!(
            !text.contains("make deploy"),
            "TypeSafe is shown descriptions only"
        );
    }

    #[test]
    fn an_agent_s_own_choice_wins_over_settings_and_an_offer_is_made_once() {
        let mut app = crate::testing::samples();
        app.score_runs = false;
        assert!(!app.scores_for("atlas"));
        app.set_score_for("atlas", Some(true));
        assert!(app.scores_for("atlas") && !app.scores_for("scout"));
        app.score_runs = true;
        app.set_score_for("scout", Some(false));
        assert!(!app.scores_for("scout"));
        app.set_score_for("scout", None);
        assert!(app.scores_for("scout"), "back to what Settings says");

        app.scored("r1", true);
        app.scored("r1", true);
        app.scored("r2", false);
        assert_eq!(app.skill_offers, ["r1"]);
        app.close_skill_offer("r1");
        assert!(!app.offers_skill("r1"));
    }
}
