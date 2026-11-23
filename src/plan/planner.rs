//! What turns a request into steps. A model will sit behind this in phase 2; until then the only
//! implementation is a test double, so the gate and the executor run with no model at all.

use super::{PlanError, StepSpec};

pub trait Planner {
    /// Steps for `request`. [`super::Plan::new`] checks the list, so an implementation need not.
    fn plan(&self, request: &str) -> Result<Vec<StepSpec>, PlanError>;
}

/// A planner that answers the same every time and counts how often it was asked.
#[cfg(test)]
pub struct Mock {
    answer: Result<Vec<StepSpec>, String>,
    calls: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl Mock {
    pub fn returning(steps: Vec<StepSpec>) -> Mock {
        Mock {
            answer: Ok(steps),
            calls: Default::default(),
        }
    }

    pub fn failing(why: &str) -> Mock {
        Mock {
            answer: Err(why.into()),
            calls: Default::default(),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
impl Planner for Mock {
    fn plan(&self, _request: &str) -> Result<Vec<StepSpec>, PlanError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.answer.clone().map_err(PlanError::Planner)
    }
}
