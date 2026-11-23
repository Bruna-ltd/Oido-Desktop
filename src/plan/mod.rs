//! Plans that outlive one request: a goal broken into steps, each with a status, kept in the
//! working folder so any tab or model can pick it up (`docs/llm-kick-in-planning-flow.md`).
//!
//! Phase 1, the skeleton: the types, a rules [`Gate`] that answers a request from a template when
//! one fits, the [`planner::Planner`] a model will sit behind, an [`exec`] loop that works through
//! pending steps, and the [`store`] that saves them. Nothing here calls a model, and nothing is
//! wired into a chat yet, so every part can be tested on its own.
//!
//! The invariant everything else leans on: only a `Pending` step can change. A `Done` step is
//! never run again and never patched, because it may have had a side effect.

pub mod exec;
pub mod planner;
pub mod router;
pub mod store;

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::new_id;
use planner::Planner;

/// Most steps one plan may hold; a longer list from a planner is refused, not cut.
pub const MAX_STEPS: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Done,
    Failed,
}

/// A step as a template or planner proposes it, before it has an id or a status.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct StepSpec {
    pub action: String,
    #[serde(default)]
    pub args: Value,
    /// The step changes something outside the plan, so it cannot be redone or quietly changed.
    #[serde(default)]
    pub side_effect: bool,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Step {
    /// 1, 2, 3... in order; stable for the life of the plan.
    pub id: u32,
    pub action: String,
    #[serde(default)]
    pub args: Value,
    pub status: StepStatus,
    #[serde(default)]
    pub side_effect: bool,
    /// What the step gave back (or why it failed), once it has run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Active,
    Parked,
    Done,
    Failed,
    Aborted,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub goal: String,
    pub status: PlanStatus,
    pub steps: Vec<Step>,
    pub created_from_turn: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parked_at_turn: Option<u32>,
}

impl Plan {
    /// An active plan from checked specs, its steps numbered from 1.
    pub fn new(goal: &str, specs: Vec<StepSpec>, turn: u32) -> Result<Plan, PlanError> {
        check(&specs)?;
        let steps = specs
            .into_iter()
            .zip(1..)
            .map(|(s, id)| Step {
                id,
                action: s.action,
                args: s.args,
                status: StepStatus::Pending,
                side_effect: s.side_effect,
                result: None,
            })
            .collect();
        Ok(Plan {
            id: new_id(),
            goal: goal.into(),
            status: PlanStatus::Active,
            steps,
            created_from_turn: turn,
            parked_at_turn: None,
        })
    }

    /// Index of the first step still to run.
    pub fn next_pending(&self) -> Option<usize> {
        self.steps
            .iter()
            .position(|s| s.status == StepStatus::Pending)
    }
}

/// What a folder remembers across plans: which is running and which are waiting under it.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub active_plan_id: Option<String>,
    /// Parked plans, the one to resume last.
    pub plan_stack: Vec<String>,
}

impl Default for State {
    fn default() -> Self {
        State {
            version: store::VERSION,
            active_plan_id: None,
            plan_stack: Vec::new(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PlanError {
    Empty,
    TooMany(usize),
    BlankAction(usize),
    /// The planner could not produce a plan.
    Planner(String),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("a plan needs at least one step"),
            Self::TooMany(n) => write!(f, "a plan of {n} steps is over the {MAX_STEPS} step limit"),
            Self::BlankAction(i) => write!(f, "step {i} has no action"),
            Self::Planner(why) => write!(f, "planning failed: {why}"),
        }
    }
}

impl std::error::Error for PlanError {}

/// Whatever builds a step list is held to the same rules: some steps, not too many, each named.
fn check(specs: &[StepSpec]) -> Result<(), PlanError> {
    if specs.is_empty() {
        return Err(PlanError::Empty);
    }
    if specs.len() > MAX_STEPS {
        return Err(PlanError::TooMany(specs.len()));
    }
    match specs.iter().position(|s| s.action.trim().is_empty()) {
        Some(i) => Err(PlanError::BlankAction(i + 1)),
        None => Ok(()),
    }
}

/// A request a template answers with no model: it fits when the request holds every keyword.
#[derive(Clone, PartialEq, Debug)]
pub struct Template {
    pub keywords: Vec<String>,
    pub steps: Vec<StepSpec>,
}

impl Template {
    fn fits(&self, request: &str) -> bool {
        !self.keywords.is_empty()
            && self
                .keywords
                .iter()
                .all(|k| request.contains(&k.to_lowercase()))
    }
}

/// The rules in front of the planner, and a cost firewall: a request a template covers never
/// reaches a model.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Gate {
    templates: Vec<Template>,
}

impl Gate {
    pub fn new(templates: Vec<Template>) -> Gate {
        Gate { templates }
    }

    /// A plan for `request`: from the first template that fits, else from the planner.
    pub fn plan(&self, planner: &dyn Planner, request: &str, turn: u32) -> Result<Plan, PlanError> {
        let lower = request.to_lowercase();
        let specs = match self.templates.iter().find(|t| t.fits(&lower)) {
            Some(t) => t.steps.clone(),
            None => planner.plan(request)?,
        };
        Plan::new(request, specs, turn)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;
    use planner::Mock;
    use serde_json::json;

    pub fn spec(action: &str, side_effect: bool) -> StepSpec {
        StepSpec {
            action: action.into(),
            args: json!({}),
            side_effect,
        }
    }

    #[test]
    fn steps_are_numbered_from_one_and_start_pending() {
        let plan = Plan::new("trip", vec![spec("a", true), spec("b", false)], 3).unwrap();
        let ids: Vec<u32> = plan.steps.iter().map(|s| s.id).collect();
        assert_eq!(ids, [1, 2]);
        assert!(plan.steps.iter().all(|s| s.status == StepStatus::Pending));
        assert_eq!(plan.status, PlanStatus::Active);
        assert_eq!(plan.created_from_turn, 3);
    }

    #[test]
    fn bad_step_lists_are_refused() {
        assert_eq!(Plan::new("g", vec![], 0), Err(PlanError::Empty));
        let many = vec![spec("a", false); MAX_STEPS + 1];
        assert_eq!(
            Plan::new("g", many, 0),
            Err(PlanError::TooMany(MAX_STEPS + 1))
        );
        let blank = vec![spec("a", false), spec("  ", false)];
        assert_eq!(Plan::new("g", blank, 0), Err(PlanError::BlankAction(2)));
    }

    #[test]
    fn a_template_that_fits_never_reaches_the_planner() {
        let gate = Gate::new(vec![Template {
            keywords: vec!["Deploy".into(), "staging".into()],
            steps: vec![spec("build", false), spec("push", true)],
        }]);
        let planner = Mock::failing("must not be asked");
        let plan = gate
            .plan(&planner, "please deploy to STAGING now", 1)
            .unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(planner.calls(), 0);
    }

    #[test]
    fn a_request_no_template_fits_goes_to_the_planner() {
        let gate = Gate::new(vec![Template {
            keywords: vec!["deploy".into()],
            steps: vec![spec("build", false)],
        }]);
        let planner = Mock::returning(vec![spec("research", false)]);
        let plan = gate.plan(&planner, "plan a trip", 1).unwrap();
        assert_eq!(plan.steps[0].action, "research");
        assert_eq!(plan.goal, "plan a trip");
        assert_eq!(planner.calls(), 1);
    }

    #[test]
    fn a_template_with_no_keywords_never_fits() {
        let gate = Gate::new(vec![Template {
            keywords: vec![],
            steps: vec![spec("x", false)],
        }]);
        let planner = Mock::returning(vec![spec("y", false)]);
        let plan = gate.plan(&planner, "anything", 1).unwrap();
        assert_eq!(plan.steps[0].action, "y");
    }

    #[test]
    fn planner_failure_and_bad_output_are_errors_not_plans() {
        let gate = Gate::default();
        let err = gate.plan(&Mock::failing("down"), "x", 1).unwrap_err();
        assert_eq!(err, PlanError::Planner("down".into()));
        let err = gate.plan(&Mock::returning(vec![]), "x", 1).unwrap_err();
        assert_eq!(err, PlanError::Empty);
    }
}
