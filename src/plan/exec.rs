//! Working through a plan: run each pending step in order, record what it gave, and report to the
//! caller after every step so a crash loses at most the step in progress.
//!
//! Only `Pending` steps run. A plan that was stopped or that the app closed on carries its `Done`
//! steps with it, so running it again goes on where it left off and never repeats a step that may
//! have had a side effect. A failed step ends the plan; re-planning is a later phase.

use std::sync::atomic::{AtomicBool, Ordering};

use super::{Plan, PlanStatus, Step, StepStatus};

/// Whatever does one step's work: a tool call, a workflow, a delegate.
pub trait StepRunner {
    /// What the step gave back, or why it failed.
    fn run(&self, step: &Step) -> Result<String, String>;
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    Done,
    Failed {
        step: u32,
        error: String,
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
                let id = step.id;
                plan.status = PlanStatus::Failed;
                saved(plan);
                return Outcome::Failed { step: id, error };
            }
        }
    }
    plan.status = PlanStatus::Done;
    saved(plan);
    Outcome::Done
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::cell::RefCell;

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

    #[test]
    fn runs_every_step_in_order_and_saves_after_each() {
        let mut p = plan();
        let fake = Fake::new(None);
        let mut saves = 0;
        let out = run(&mut p, &fake, &AtomicBool::new(false), &mut |_| saves += 1);
        assert_eq!(out, Outcome::Done);
        assert_eq!(*fake.ran.borrow(), [1, 2, 3]);
        assert_eq!(p.status, PlanStatus::Done);
        assert_eq!(p.steps[0].result.as_deref(), Some("a ok"));
        assert_eq!(saves, 4);
    }

    #[test]
    fn a_failed_step_ends_the_plan_and_leaves_the_rest_pending() {
        let mut p = plan();
        let fake = Fake::new(Some("b"));
        let out = run(&mut p, &fake, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(
            out,
            Outcome::Failed {
                step: 2,
                error: "b broke".into()
            }
        );
        assert_eq!(*fake.ran.borrow(), [1, 2]);
        assert_eq!(p.status, PlanStatus::Failed);
        let status: Vec<_> = p.steps.iter().map(|s| s.status).collect();
        assert_eq!(
            status,
            [StepStatus::Done, StepStatus::Failed, StepStatus::Pending]
        );
    }

    #[test]
    fn stop_between_steps_then_resume_never_repeats_a_done_step() {
        let mut p = plan();
        let stop = AtomicBool::new(false);
        let first = Fake::new(None);
        // Stop once step 1 is saved.
        let out = run(&mut p, &first, &stop, &mut |_| {
            stop.store(true, Ordering::SeqCst)
        });
        assert_eq!(out, Outcome::Stopped);
        assert_eq!(*first.ran.borrow(), [1]);

        let second = Fake::new(None);
        let out = run(&mut p, &second, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(out, Outcome::Done);
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
            let out = run(&mut p, &fake, &AtomicBool::new(false), &mut |_| {});
            assert_eq!(out, Outcome::NotActive);
            assert!(fake.ran.borrow().is_empty());
        }
    }
}
