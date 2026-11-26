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

/// For a run whose task did not succeed, how high "reusable" and "evidence" must both be for its
/// mistake to be worth a warning (ADR-10).
const WARNING_FLOOR: f64 = 0.75;

/// What TypeSafe's answers make of a run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Nothing to offer: not judged, low answers, or covered by a skill already.
    Nothing,
    /// Its task succeeded and it holds a method worth keeping.
    Method,
    /// Its task did not succeed, but it clearly shows a mistake to avoid next time. Only a warning
    /// may be made from it, never a way to do the task.
    Warning,
}

/// What answers to [`QUESTIONS`], in their order, make of a run. A method needs task success
/// first and then a high mean of the first three: a mean alone lets two high answers outvote a
/// failed task. A failed task can still give a warning, when its lesson is both clearly reusable
/// and clearly shown. Either way, nothing is offered when a skill covers it already (when that
/// was asked).
pub fn verdict(answers: &[f64]) -> Verdict {
    let (Some(&task), Some(first)) = (answers.first(), answers.get(..3)) else {
        return Verdict::Nothing;
    };
    if answers.get(3).is_some_and(|&c| c >= COVERED_CEILING) {
        return Verdict::Nothing;
    }
    let (reusable, evidence) = (first[1], first[2]);
    if task >= TASK_FLOOR {
        if first.iter().sum::<f64>() / 3.0 >= MEAN_FLOOR {
            return Verdict::Method;
        }
    } else if reusable >= WARNING_FLOOR && evidence >= WARNING_FLOOR {
        return Verdict::Warning;
    }
    Verdict::Nothing
}

/// The answers in words, for the Runs screen: "task 0.31 · reusable 0.82 · …".
pub fn scores_line(answers: &[f64]) -> String {
    let names = ["task", "reusable", "evidence", "covered"];
    answers
        .iter()
        .zip(names)
        .map(|(p, n)| format!("{n} {p:.2}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// What TypeSafe is shown of a run: the skills it could already be in, what came before it in
/// the chat (`earlier`, see `skill_draft::earlier_block`), then its transcript, with every secret
/// and personal detail found in any of them hidden.
pub fn judged_text(steps: &[Step], known: &[Known], earlier: &str) -> String {
    let mut text = transcript(steps);
    if !known.is_empty() || !earlier.is_empty() {
        let skills = if known.is_empty() {
            String::new()
        } else {
            format!("{}\n\n", known_block(known, false))
        };
        text = format!("{skills}{earlier}<run>\n{text}\n</run>");
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
        let earlier = self.earlier_text(&run);
        let text = judged_text(&steps, &known, &earlier);
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
                Ok(answers) => app.scored(&id, &answers),
                Err(why) => eprintln!("oido: TypeSafe could not judge the run: {why}"),
            },
        );
    }

    fn scored(&mut self, id: &str, answers: &[f64]) {
        let worth = verdict(answers) != Verdict::Nothing;
        self.save("the run's verdict", |db| {
            db.set_run_worth(id, worth, answers)
        });
        if let Some(r) = self.runs.iter_mut().find(|r| r.id == id) {
            r.worth = Some(worth);
            r.scores = answers.to_vec();
        }
        if worth && !self.skill_offers.iter().any(|o| o == id) {
            self.skill_offers.push(id.to_owned());
        }
    }

    /// What kind of offer run `id` has, by its saved answers; [`Verdict::Method`] if they are not
    /// at hand.
    pub fn offer_kind(&self, id: &str) -> Verdict {
        match self.runs.iter().find(|r| r.id == id) {
            Some(r) if !r.scores.is_empty() => verdict(&r.scores),
            _ => Verdict::Method,
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
    fn a_method_needs_task_success_and_a_failed_task_can_only_give_a_clear_warning() {
        use Verdict::*;
        assert_eq!(verdict(&[0.9, 0.8, 0.7]), Method);
        assert_eq!(
            verdict(&[0.5, 0.65, 0.65]),
            Method,
            "both floors are inclusive"
        );
        assert_eq!(verdict(&[0.9, 0.3, 0.4]), Nothing, "a mean of 0.53");
        assert_eq!(
            verdict(&[0.27, 0.86, 0.69]),
            Nothing,
            "a failed task is not averaged into a method (beacon #649), nor clear enough to warn"
        );
        assert_eq!(verdict(&[0.27, 0.86, 0.79]), Warning);
        assert_eq!(
            verdict(&[0.27, 0.86, 0.79, 0.7]),
            Nothing,
            "covered already"
        );
        assert_eq!(verdict(&[0.9, 0.8, 0.7, 0.2]), Method);
        assert_eq!(verdict(&[0.9, 0.8, 0.7, 0.8]), Nothing);
        assert_eq!(verdict(&[0.9, 0.8]), Nothing);
        assert_eq!(verdict(&[]), Nothing);
        assert_eq!(scores_line(&[0.312, 0.8]), "task 0.31 · reusable 0.80");
    }

    #[test]
    fn what_typesafe_sees_names_the_skills_it_may_be_in_with_secrets_hidden() {
        let mut s = Step::new(1, StepKind::User, 0);
        s.output = Some(
            "deploy with sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCD and mail ops@example.com"
                .into(),
        );
        let text = judged_text(std::slice::from_ref(&s), &[], "");
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
        let text = judged_text(&[s], &known, "<earlier>\nx\n</earlier>\n\n");
        assert!(
            text.starts_with(
                "<skills>\n- deploy: Deploy the app.\n</skills>\n\n<earlier>\nx\n</earlier>\n\n<run>\n[person]"
            ),
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

        app.scored("r1", &[0.9, 0.8, 0.7]);
        app.scored("r1", &[0.9, 0.8, 0.7]);
        app.scored("r2", &[0.2, 0.9, 0.9]);
        app.scored("r3", &[0.2, 0.3, 0.3]);
        assert_eq!(app.skill_offers, ["r1", "r2"], "a method, and a warning");
        app.close_skill_offer("r1");
        assert!(!app.offers_skill("r1"));
    }
}
