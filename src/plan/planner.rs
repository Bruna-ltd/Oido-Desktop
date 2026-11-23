//! What turns a request into steps, and a failed plan into new ones. A model will sit behind this
//! later; until then the only implementation is a test double, so the gate and the executor run
//! with no model at all.

use super::{Plan, PlanError, Step, StepSpec};

pub trait Planner {
    /// Steps for `request`. [`Plan::new`] checks the list, so an implementation need not.
    fn plan(&self, request: &str) -> Result<Vec<StepSpec>, PlanError>;

    /// Steps to put in place of the pending ones, after `failed` failed. [`Plan::replace_pending`]
    /// checks the list, so an implementation need not.
    fn replan(&self, plan: &Plan, failed: &Step) -> Result<Vec<StepSpec>, PlanError>;
}

/// A planner with canned answers that counts how often it was asked. `plan` answers the same every
/// time; `replan` gives its answers in turn, and the last one again once they run out.
#[cfg(test)]
pub struct Mock {
    answer: Result<Vec<StepSpec>, String>,
    replans: std::sync::Mutex<std::collections::VecDeque<Result<Vec<StepSpec>, String>>>,
    calls: std::sync::atomic::AtomicUsize,
    replan_calls: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
impl Mock {
    fn with(answer: Result<Vec<StepSpec>, String>) -> Mock {
        Mock {
            answer,
            replans: Default::default(),
            calls: Default::default(),
            replan_calls: Default::default(),
        }
    }

    pub fn returning(steps: Vec<StepSpec>) -> Mock {
        Mock::with(Ok(steps))
    }

    pub fn failing(why: &str) -> Mock {
        Mock::with(Err(why.into()))
    }

    /// A planner that is never asked for a first plan and answers each re-plan from `answers`.
    pub fn replanning(answers: Vec<Result<Vec<StepSpec>, &str>>) -> Mock {
        let mock = Mock::failing("plan was not expected");
        *mock.replans.lock().unwrap() = answers
            .into_iter()
            .map(|a| a.map_err(str::to_owned))
            .collect();
        mock
    }

    pub fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn replan_calls(&self) -> usize {
        self.replan_calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
impl Planner for Mock {
    fn plan(&self, _request: &str) -> Result<Vec<StepSpec>, PlanError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.answer.clone().map_err(PlanError::Planner)
    }

    fn replan(&self, _plan: &Plan, _failed: &Step) -> Result<Vec<StepSpec>, PlanError> {
        self.replan_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut queue = self.replans.lock().unwrap();
        let answer = if queue.len() > 1 {
            queue.pop_front()
        } else {
            queue.front().cloned()
        };
        answer
            .unwrap_or_else(|| Err("no re-plan answer was set".into()))
            .map_err(PlanError::Planner)
    }
}
