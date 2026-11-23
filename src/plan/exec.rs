//! Working through a plan: run each pending step in order, record what it gave, and report to the
//! caller after every step so a crash loses at most the step in progress.
//!
//! Only `Pending` steps run. A plan that was stopped or that the app closed on carries its `Done`
//! steps with it, so running it again goes on where it left off and never repeats a step that may
//! have had a side effect.
//!
//! A failed step asks the planner for new pending steps, at most [`MAX_REPLANS`] times for the
//! whole plan, and the answer is checked ([`Plan::replace_pending`]) before it replaces anything.
//! A step with a side effect that fails is never re-planned past: it may have half happened, so
//! the plan stops and the person is told.

use std::sync::atomic::{AtomicBool, Ordering};

use super::planner::Planner;
use super::{Plan, PlanError, PlanStatus, Step, StepStatus};

/// How many times one plan may have its pending steps replaced after failures.
pub const MAX_REPLANS: u32 = 2;

/// Whatever does one step's work: a tool call, a workflow, a delegate.
pub trait StepRunner {
    /// What the step gave back, or why it failed.
    fn run(&self, step: &Step) -> Result<String, String>;
}

/// Why a failed step ended the plan instead of being planned around.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Why {
    /// The plan already had [`MAX_REPLANS`] re-plans.
    CapReached,
    /// The step changes things outside the plan, and may have partly happened.
    SideEffect,
    /// The planner failed, or its answer was refused.
    Replan(PlanError),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    Done,
    /// The plan ended on a step that failed with `error`, and `why` it was not planned around.
    Failed {
        step: u32,
        error: String,
        why: Why,
    },
    /// The stop flag was set between steps; the rest stay pending.
    Stopped,
    /// The plan is parked, done, failed or aborted, so nothing ran.
    NotActive,
}

/// Run `plan`'s pending steps. `saved` is called after each step with the plan as it now stands,
/// and once more when the plan ends.
pub fn run(
    plan: &mut Plan,
    runner: &dyn StepRunner,
    planner: &dyn Planner,
    stop: &AtomicBool,
    saved: &mut dyn FnMut(&Plan),
) -> Outcome {
    if plan.status != PlanStatus::Active {
        return Outcome::NotActive;
    }
    while let Some(i) = plan.next_pending() {
        if stop.load(Ordering::SeqCst) {
            return Outcome::Stopped;
        }
        let result = runner.run(&plan.steps[i]);
        let step = &mut plan.steps[i];
        match result {
            Ok(out) => {
                step.status = StepStatus::Done;
                step.result = Some(out);
                saved(plan);
            }
            Err(error) => {
                step.status = StepStatus::Failed;
                step.result = Some(error.clone());
                let failed = step.clone();
                match recover(plan, planner, &failed) {
                    Ok(()) => saved(plan),
                    Err(why) => {
                        plan.status = PlanStatus::Failed;
                        saved(plan);
                        return Outcome::Failed {
                            step: failed.id,
                            error,
                            why,
                        };
                    }
                }
            }
        }
    }
    plan.status = PlanStatus::Done;
    saved(plan);
    Outcome::Done
}

/// Replace the pending steps after `failed`, or say why not. A re-plan is counted when it is
/// asked for, so a planner that keeps failing is stopped by the cap like one that keeps answering.
fn recover(plan: &mut Plan, planner: &dyn Planner, failed: &Step) -> Result<(), Why> {
    if failed.side_effect {
        return Err(Why::SideEffect);
    }
    if plan.replans >= MAX_REPLANS {
        return Err(Why::CapReached);
    }
    plan.replans += 1;
    let specs = planner.replan(plan, failed).map_err(Why::Replan)?;
    plan.replace_pending(specs).map_err(Why::Replan)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::cell::RefCell;

    use super::super::planner::Mock;
    use super::super::tests::spec;
    use super::*;

    /// Records the steps it was asked to run, and fails on one action if told to.
    struct Fake {
        ran: RefCell<Vec<u32>>,
        fail_on: Option<&'static str>,
    }

    impl Fake {
        fn new(fail_on: Option<&'static str>) -> Fake {
            Fake {
                ran: RefCell::new(Vec::new()),
                fail_on,
            }
        }
    }

    impl StepRunner for Fake {
        fn run(&self, step: &Step) -> Result<String, String> {
            self.ran.borrow_mut().push(step.id);
            if self.fail_on == Some(step.action.as_str()) {
                Err(format!("{} broke", step.action))
            } else {
                Ok(format!("{} ok", step.action))
            }
        }
    }

    fn plan() -> Plan {
        Plan::new(
            "g",
            vec![spec("a", true), spec("b", false), spec("c", false)],
            1,
        )
        .unwrap()
    }

    /// A planner that has nothing to offer, for runs that should never ask.
    fn silent() -> Mock {
        Mock::replanning(vec![])
    }

    fn go(p: &mut Plan, fake: &Fake, planner: &Mock) -> Outcome {
        run(p, fake, planner, &AtomicBool::new(false), &mut |_| {})
    }

    #[test]
    fn runs_every_step_in_order_and_saves_after_each() {
        let mut p = plan();
        let fake = Fake::new(None);
        let mut saves = 0;
        let out = run(
            &mut p,
            &fake,
            &silent(),
            &AtomicBool::new(false),
            &mut |_| saves += 1,
        );
        assert_eq!(out, Outcome::Done);
        assert_eq!(*fake.ran.borrow(), [1, 2, 3]);
        assert_eq!(p.status, PlanStatus::Done);
        assert_eq!(p.steps[0].result.as_deref(), Some("a ok"));
        assert_eq!(saves, 4);
    }

    #[test]
    fn stop_between_steps_then_resume_never_repeats_a_done_step() {
        let mut p = plan();
        let stop = AtomicBool::new(false);
        let first = Fake::new(None);
        // Stop once step 1 is saved.
        let out = run(&mut p, &first, &silent(), &stop, &mut |_| {
            stop.store(true, Ordering::SeqCst)
        });
        assert_eq!(out, Outcome::Stopped);
        assert_eq!(*first.ran.borrow(), [1]);

        let second = Fake::new(None);
        assert_eq!(go(&mut p, &second, &silent()), Outcome::Done);
        assert_eq!(*second.ran.borrow(), [2, 3]);
    }

    #[test]
    fn a_plan_that_is_not_active_does_not_run() {
        for status in [
            PlanStatus::Parked,
            PlanStatus::Done,
            PlanStatus::Failed,
            PlanStatus::Aborted,
        ] {
            let mut p = plan();
            p.status = status;
            let fake = Fake::new(None);
            assert_eq!(go(&mut p, &fake, &silent()), Outcome::NotActive);
            assert!(fake.ran.borrow().is_empty());
        }
    }

    #[test]
    fn a_failed_step_is_planned_around_and_the_plan_finishes() {
        let mut p = plan();
        let fake = Fake::new(Some("b"));
        let planner = Mock::replanning(vec![Ok(vec![spec("b2", false), spec("c2", false)])]);
        assert_eq!(go(&mut p, &fake, &planner), Outcome::Done);
        // The old `c` was dropped, the new steps are numbered on, and nothing done was rerun.
        assert_eq!(*fake.ran.borrow(), [1, 2, 3, 4]);
        let seen: Vec<_> = p
            .steps
            .iter()
            .map(|s| (s.id, s.action.as_str(), s.status))
            .collect();
        assert_eq!(
            seen,
            [
                (1, "a", StepStatus::Done),
                (2, "b", StepStatus::Failed),
                (3, "b2", StepStatus::Done),
                (4, "c2", StepStatus::Done),
            ]
        );
        assert_eq!((p.replans, p.status), (1, PlanStatus::Done));
    }

    #[test]
    fn re_planning_stops_at_the_cap() {
        let mut p = Plan::new("g", vec![spec("a", false), spec("bad", false)], 1).unwrap();
        let fake = Fake::new(Some("bad"));
        // Every re-plan offers the same failing step again.
        let planner = Mock::replanning(vec![Ok(vec![spec("bad", false)])]);
        let out = go(&mut p, &fake, &planner);
        assert_eq!(
            out,
            Outcome::Failed {
                step: 4,
                error: "bad broke".into(),
                why: Why::CapReached
            }
        );
        assert_eq!(planner.replan_calls() as u32, MAX_REPLANS);
        assert_eq!((p.replans, p.status), (MAX_REPLANS, PlanStatus::Failed));
    }

    #[test]
    fn a_failed_side_effect_is_never_planned_past() {
        let mut p = Plan::new("g", vec![spec("book", true), spec("pay", false)], 1).unwrap();
        let fake = Fake::new(Some("book"));
        let planner = Mock::replanning(vec![Ok(vec![spec("x", false)])]);
        let out = go(&mut p, &fake, &planner);
        assert_eq!(
            out,
            Outcome::Failed {
                step: 1,
                error: "book broke".into(),
                why: Why::SideEffect
            }
        );
        assert_eq!(planner.replan_calls(), 0);
        assert_eq!(
            p.steps[1].status,
            StepStatus::Pending,
            "the rest is left as it was"
        );
        assert_eq!(p.status, PlanStatus::Failed);
    }

    #[test]
    fn a_planner_that_fails_ends_the_plan_and_still_counts() {
        let mut p = plan();
        let fake = Fake::new(Some("b"));
        let planner = Mock::replanning(vec![Err("down")]);
        let out = go(&mut p, &fake, &planner);
        assert_eq!(
            out,
            Outcome::Failed {
                step: 2,
                error: "b broke".into(),
                why: Why::Replan(PlanError::Planner("down".into()))
            }
        );
        assert_eq!((p.replans, p.status), (1, PlanStatus::Failed));
    }

    #[test]
    fn a_refused_replacement_ends_the_plan_and_changes_no_steps() {
        let mut p = Plan::new("g", vec![spec("book", true), spec("x", false)], 1).unwrap();
        let fake = Fake::new(Some("x"));
        // Booking again is exactly what the check is there to refuse.
        let planner = Mock::replanning(vec![Ok(vec![spec("book", true)])]);
        let out = go(&mut p, &fake, &planner);
        assert_eq!(
            out,
            Outcome::Failed {
                step: 2,
                error: "x broke".into(),
                why: Why::Replan(PlanError::RepeatsDone(1))
            }
        );
        assert_eq!(p.steps.len(), 2);
        assert_eq!(*fake.ran.borrow(), [1, 2], "book was not run twice");
    }
}
