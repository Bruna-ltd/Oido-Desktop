//! The Turn Router: what a new message means for the plan that is running. Jev answers a few
//! questions about the message in one request; [`route`] turns the answers into one [`Handler`]
//! by a table written out here in code. Jev makes the judgments, the table makes the decision, and
//! nothing in a plan changes because of a judgment alone.
//!
//! Jev is asked only about what a person could tell from the message and the plan:
//!
//! | question       | read when                                  |
//! |----------------|--------------------------------------------|
//! | `fits`         | always: is the message clear enough to act on |
//! | `turn`         | always: continue, change, aside, cancel, new goal |
//! | `touches_done` | the message changes the plan and a step is already done |
//! | `needs_plan`   | the message is an aside                    |
//!
//! The last two are asked every time and read only when they matter, so they cost no extra
//! request. Confidence is not used: a probability is, against the thresholds below.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};

use super::{Plan, StepStatus};
use crate::typesafe;

/// A yes at or above this; a no at or below [`NO`]. Between them the answer is not settled.
const YES: f64 = 0.7;
const NO: f64 = 0.3;
/// The option a choice picks must have at least this probability to be acted on.
const PICKED: f64 = 0.5;
/// Cancelling is the one destructive answer, so it must be nearly certain.
const CANCEL: f64 = YES;
/// Parked plans a folder keeps under the active one. A deeper pile is asked about, not grown.
pub const MAX_STACK: usize = 3;
const TIMEOUT: Duration = Duration::from_secs(30);

/// What to do with a turn. Every one is a named handler; a turn the router cannot place is
/// [`Handler::Clarify`], never a guess.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handler {
    /// Go on with the plan as it is.
    Continue,
    /// Change pending steps only.
    Modify,
    /// The change reaches a step already done: ask before anything is touched.
    Conflict,
    /// A small ask: answer it and leave the plan alone.
    AsideInline,
    /// A real sub-goal: park the plan and plan it.
    AsidePlan,
    Cancel,
    /// Different work altogether: park the plan and start another.
    NewGoal,
    Clarify,
}

/// The four options of the `turn` question.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Continue,
    Modify,
    Aside,
    Cancel,
    NewGoal,
}

impl Kind {
    /// Names that say what they mean, because the model reads them.
    const OPTIONS: [(&'static str, Kind, &'static str); 5] = [
        (
            "go_on_with_plan",
            Kind::Continue,
            "The user agrees, approves, or asks to carry on with `plan` as it is.",
        ),
        (
            "change_plan",
            Kind::Modify,
            "The user wants a step of `plan` done differently, added, or dropped.",
        ),
        (
            "aside",
            Kind::Aside,
            "The user asks for something small in passing, after which `plan` would go on.",
        ),
        (
            "cancel_plan",
            Kind::Cancel,
            "The user wants to stop `plan` and not do the rest of it.",
        ),
        (
            "new_goal",
            Kind::NewGoal,
            "The user moves on to different work and leaves `plan` behind.",
        ),
    ];

    fn named(name: &str) -> Option<Kind> {
        Self::OPTIONS.iter().find(|o| o.0 == name).map(|o| o.1)
    }
}

/// What Jev said, as the table reads it.
#[derive(Clone, PartialEq, Debug)]
pub struct Answers {
    fits: f64,
    kind: Kind,
    /// Probability of the option picked.
    picked: f64,
    touches_done: f64,
    needs_plan: f64,
}

/// What the router needs to know about the moment.
pub struct Turn<'a> {
    pub text: &'a str,
    pub plan: &'a Plan,
    /// Goals of the plans parked under this one.
    pub parked: &'a [String],
}

/// The `/systemone` request for a turn. The state has named fields a person would look at: the
/// message, the plan with each step's status, and the parked goals when there are any. An empty
/// list is left out, since it could be read as a signal.
fn request(turn: &Turn) -> Value {
    let steps: Vec<Value> = turn
        .plan
        .steps
        .iter()
        .map(|s| json!({"action": s.action, "status": s.status}))
        .collect();
    let mut state = json!({
        "turn": turn.text,
        "plan": {"goal": turn.plan.goal, "steps": steps},
    });
    if turn.plan.status == super::PlanStatus::Draft {
        state["plan"]["waiting_for_approval"] = json!(true);
    }
    if !turn.parked.is_empty() {
        state["parked"] = json!(turn.parked);
    }
    let options: serde_json::Map<String, Value> = Kind::OPTIONS
        .iter()
        .map(|(name, _, about)| ((*name).to_owned(), json!(about)))
        .collect();
    json!({
        "model": typesafe::MODEL,
        "state": state,
        "questions": {
            "turn": {"type": "choice", "instructions": "What does the user's `turn` ask for, with respect to `plan`?", "criteria": options},
            "fits": {"type": "noul", "instructions": "Is `turn` a clear instruction, request, or reply, rather than noise or too vague to act on?"},
            "touches_done": {"type": "noul", "instructions": "Would doing what `turn` asks change a step of `plan` that is already done?"},
            "needs_plan": {"type": "noul", "instructions": "Does `turn` ask for something that takes several steps to do, rather than a quick answer?"},
        },
    })
}

/// The answers in a `/systemone` reply, or what was missing from it.
fn parse(body: &str) -> Result<Answers, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "the reply was not JSON.")?;
    let noul = |id: &str| {
        v.pointer(&format!("/answers/{id}/noul"))
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("the reply had no `{id}` answer."))
    };
    let turn = v
        .pointer("/answers/turn")
        .ok_or("the reply had no `turn` answer.")?;
    let name = turn["choice"]
        .as_str()
        .ok_or("the `turn` answer picked nothing.")?;
    let kind = Kind::named(name)
        .ok_or_else(|| format!("the `turn` answer picked {name:?}, which is not an option."))?;
    let probs: BTreeMap<String, f64> = turn["probabilities"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, p)| Some((k.clone(), p.as_f64()?)))
                .collect()
        })
        .unwrap_or_default();
    let picked = *probs
        .get(name)
        .ok_or("the `turn` answer had no probability for its pick.")?;
    Ok(Answers {
        fits: noul("fits")?,
        kind,
        picked,
        touches_done: noul("touches_done")?,
        needs_plan: noul("needs_plan")?,
    })
}

/// Asks Jev about a turn and places it. An error is a failed call, not an answer: the caller shows
/// it or retries, and does not treat it as "clarify". Blocks on the network, so call it from a
/// worker thread.
pub fn classify(base: &str, key: &str, turn: &Turn) -> Result<Handler, String> {
    let body = typesafe::systemone(base, key, &request(turn), false, TIMEOUT)?;
    Ok(route(turn, &parse(&body)?))
}

/// The table: every combination of answers has a handler.
///
/// | answers                                   | handler                         |
/// |-------------------------------------------|---------------------------------|
/// | `fits` not a yes, or the pick under 0.5   | Clarify                         |
/// | cancel, under 0.7                         | Clarify                         |
/// | cancel                                    | Cancel                          |
/// | continue                                  | Continue                        |
/// | change, no step done yet                  | Modify                          |
/// | change, `touches_done` a no               | Modify                          |
/// | change, `touches_done` yes or unsettled   | Conflict (a done step may have had a side effect, so ask) |
/// | aside, `needs_plan` a no or unsettled     | AsideInline (nothing is mutated, so a wrong answer costs little) |
/// | aside, `needs_plan` a yes                 | AsidePlan                       |
/// | new goal                                  | NewGoal                         |
/// | AsidePlan or NewGoal with the stack full  | Clarify                         |
pub fn route(turn: &Turn, a: &Answers) -> Handler {
    if a.fits < YES || a.picked < PICKED {
        return Handler::Clarify;
    }
    let stack_full = turn.parked.len() >= MAX_STACK;
    match a.kind {
        Kind::Cancel if a.picked < CANCEL => Handler::Clarify,
        Kind::Cancel => Handler::Cancel,
        Kind::Continue => Handler::Continue,
        Kind::Modify => {
            let any_done = turn.plan.steps.iter().any(|s| s.status == StepStatus::Done);
            if !any_done || a.touches_done <= NO {
                Handler::Modify
            } else {
                Handler::Conflict
            }
        }
        Kind::Aside if a.needs_plan >= YES => {
            if stack_full {
                Handler::Clarify
            } else {
                Handler::AsidePlan
            }
        }
        Kind::Aside => Handler::AsideInline,
        Kind::NewGoal if stack_full => Handler::Clarify,
        Kind::NewGoal => Handler::NewGoal,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::tests::spec;
    use super::*;
    use crate::testing::serve;

    fn plan(done: usize) -> Plan {
        let mut p = Plan::new("trip", vec![spec("flight", true), spec("hotel", false)], 1).unwrap();
        for s in p.steps.iter_mut().take(done) {
            s.status = StepStatus::Done;
        }
        p
    }

    fn answers(kind: Kind, picked: f64) -> Answers {
        Answers {
            fits: 0.95,
            kind,
            picked,
            touches_done: 0.5,
            needs_plan: 0.5,
        }
    }

    fn place(plan: &Plan, parked: &[String], a: &Answers) -> Handler {
        route(
            &Turn {
                text: "x",
                plan,
                parked,
            },
            a,
        )
    }

    #[test]
    fn the_request_holds_one_state_and_four_questions() {
        let p = plan(1);
        let r = request(&Turn {
            text: "make the hotel 5-star",
            plan: &p,
            parked: &[],
        });
        assert_eq!(r["state"]["turn"], "make the hotel 5-star");
        assert_eq!(r["state"]["plan"]["goal"], "trip");
        assert_eq!(
            r["state"]["plan"]["steps"][0],
            json!({"action": "flight", "status": "done"})
        );
        assert!(
            r["state"].get("parked").is_none(),
            "an empty list is left out"
        );
        let q = r["questions"].as_object().unwrap();
        let mut ids: Vec<_> = q.keys().map(String::as_str).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["fits", "needs_plan", "touches_done", "turn"]);
        assert_eq!(
            q["turn"]["criteria"].as_object().unwrap().len(),
            Kind::OPTIONS.len()
        );

        let mut draft = plan(0);
        draft.status = super::super::PlanStatus::Draft;
        let r = request(&Turn {
            text: "go",
            plan: &draft,
            parked: &[],
        });
        assert_eq!(r["state"]["plan"]["waiting_for_approval"], json!(true));
        let r = request(&Turn {
            text: "go",
            plan: &p,
            parked: &[],
        });
        assert!(r["state"]["plan"].get("waiting_for_approval").is_none());

        let parked = ["car in Rome".to_owned()];
        let r = request(&Turn {
            text: "x",
            plan: &p,
            parked: &parked,
        });
        assert_eq!(r["state"]["parked"], json!(["car in Rome"]));
    }

    #[test]
    fn the_table_places_each_kind_of_turn() {
        let fresh = plan(0);
        let started = plan(1);
        let at = |plan: &Plan, kind, picked, touches, needs| {
            let a = Answers {
                fits: 0.95,
                kind,
                picked,
                touches_done: touches,
                needs_plan: needs,
            };
            place(plan, &[], &a)
        };
        assert_eq!(
            at(&started, Kind::Continue, 0.9, 0.5, 0.5),
            Handler::Continue
        );
        assert_eq!(at(&started, Kind::NewGoal, 0.9, 0.5, 0.5), Handler::NewGoal);
        // Change: only a clear no on touching a done step is safe, and no done step is always safe.
        assert_eq!(at(&fresh, Kind::Modify, 0.9, 0.95, 0.5), Handler::Modify);
        assert_eq!(at(&started, Kind::Modify, 0.9, 0.1, 0.5), Handler::Modify);
        assert_eq!(at(&started, Kind::Modify, 0.9, 0.5, 0.5), Handler::Conflict);
        assert_eq!(at(&started, Kind::Modify, 0.9, 0.9, 0.5), Handler::Conflict);
        // Aside: a clear yes needs a plan; no or unsettled answers inline.
        assert_eq!(at(&started, Kind::Aside, 0.9, 0.5, 0.9), Handler::AsidePlan);
        assert_eq!(
            at(&started, Kind::Aside, 0.9, 0.5, 0.1),
            Handler::AsideInline
        );
        assert_eq!(
            at(&started, Kind::Aside, 0.9, 0.5, 0.5),
            Handler::AsideInline
        );
        // Cancel needs near certainty.
        assert_eq!(at(&started, Kind::Cancel, 0.9, 0.5, 0.5), Handler::Cancel);
        assert_eq!(at(&started, Kind::Cancel, 0.6, 0.5, 0.5), Handler::Clarify);
    }

    #[test]
    fn anything_unsettled_is_clarify_never_a_guess() {
        let p = plan(1);
        let mut a = answers(Kind::Continue, 0.9);
        a.fits = 0.5;
        assert_eq!(place(&p, &[], &a), Handler::Clarify, "unclear message");
        assert_eq!(
            place(&p, &[], &answers(Kind::Continue, 0.4)),
            Handler::Clarify,
            "weak pick"
        );
    }

    #[test]
    fn a_full_stack_is_asked_about_not_grown() {
        let p = plan(1);
        let full: Vec<String> = (0..MAX_STACK).map(|i| format!("plan {i}")).collect();
        let mut aside = answers(Kind::Aside, 0.9);
        aside.needs_plan = 0.9;
        assert_eq!(place(&p, &full, &aside), Handler::Clarify);
        assert_eq!(
            place(&p, &full, &answers(Kind::NewGoal, 0.9)),
            Handler::Clarify
        );
        // Turns that park nothing are unaffected.
        assert_eq!(
            place(&p, &full, &answers(Kind::Continue, 0.9)),
            Handler::Continue
        );
    }

    const REPLY: &str = r#"{"answers":{
        "turn":{"type":"choice","choice":"change_plan","confidence":0.9,"probabilities":{"change_plan":0.88,"aside":0.1}},
        "fits":{"type":"noul","noul":0.97},
        "touches_done":{"type":"noul","noul":0.05},
        "needs_plan":{"type":"noul","noul":0.2}}}"#;

    #[test]
    fn a_reply_is_read_into_answers() {
        let a = parse(REPLY).unwrap();
        assert_eq!((a.kind, a.picked, a.fits), (Kind::Modify, 0.88, 0.97));
        assert_eq!((a.touches_done, a.needs_plan), (0.05, 0.2));
    }

    #[test]
    fn a_reply_with_a_gap_is_an_error() {
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"answers":{}}"#).is_err());
        let unknown = REPLY.replace("change_plan", "teleport");
        assert!(parse(&unknown).is_err(), "a pick that is not an option");
        let no_fits = REPLY.replace(r#""fits":{"type":"noul","noul":0.97},"#, "");
        assert!(parse(&no_fits).is_err());
    }

    #[test]
    fn classify_asks_once_and_places_the_turn() {
        let (base, seen) = serve(vec![(200, REPLY)]);
        let p = plan(1);
        let turn = Turn {
            text: "make the hotel 5-star",
            plan: &p,
            parked: &[],
        };
        let h = classify(&format!("{base}/v1"), "k", &turn).unwrap();
        assert_eq!(h, Handler::Modify);
        let sent = seen.recv().unwrap();
        assert!(sent.contains("make the hotel 5-star") && sent.contains("touches_done"));
    }

    #[test]
    fn a_failed_call_is_an_error_not_clarify() {
        let (base, _) = serve(vec![(500, "{}")]);
        let p = plan(0);
        let turn = Turn {
            text: "x",
            plan: &p,
            parked: &[],
        };
        assert!(classify(&format!("{base}/v1"), "k", &turn).is_err());
    }
}
