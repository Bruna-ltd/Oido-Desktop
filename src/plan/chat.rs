//! Plans in a chat, behind Settings → "Plan multi-step requests".
//!
//! With it on, and in an agent that has a working folder, each message you send is looked at before
//! the agent answers it, on a worker thread:
//!
//! - **No plan open in the folder.** TypeSafe is asked whether the message takes several steps. If
//!   so, the agent's model writes a plan, it is saved under `.oido/` as a draft and shown in the
//!   chat, and nothing runs until you approve it.
//! - **A plan open** (a draft waiting for your OK, or one that paused). The
//!   [router](super::router) places the message against it. *Continue* approves a draft and goes
//!   on with the steps still pending, one after another, each done by the agent itself as an
//!   unattended task. *Modify* has the model revise the steps not yet done and puts the plan back
//!   to a draft, so what will run is always what you last approved. A change that reaches a step
//!   already done is refused. *Cancel* ends the plan; *clarify* asks which you meant. A small
//!   aside or new work is answered as an ordinary message and the plan stays as it was.
//!
//! Whenever planning cannot help (off, no folder, TypeSafe or the planner failing, a short
//! request) the message goes the ordinary way, as if this were off.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::exec::{Outcome, StepRunner, Why, run};
use super::planner::Planner;
use super::router::{self, Handler, Turn};
use super::store::Store;
use super::{Gate, Plan, PlanError, PlanStatus, State, Step, StepSpec, StepStatus};
use crate::app::App;
use crate::delegate::{Crew, Taken};
use crate::model::{Message, Outcome as Finish};
use crate::typesafe;

/// How sure TypeSafe must be that a message takes several steps before it is planned. Anything
/// less is answered as an ordinary message, which costs nothing.
const MULTI_STEP: f64 = 0.7;
const TIMEOUT: Duration = Duration::from_secs(30);
const MULTI_STEP_QUESTION: &str = "Does this ask for work that takes several separate steps to finish, rather than a single quick answer or a small change?";
/// How much of an earlier step's result is handed on to the next.
const HANDED_ON: usize = 600;

/// The shape a plan is asked for in, shared by the first plan and a re-plan.
const STEP_FORMAT: &str = "Reply with only a JSON array and nothing else: no prose, no code fence. Each item is {\"action\": \"what to do, in one or two sentences\", \"side_effect\": true or false}. Set side_effect to true only when the step sends, posts, pays, deletes or deploys something outside this folder; writing or editing files in the folder is false.";

impl App {
    /// Settings: messages are planned, or are not.
    pub fn set_plan_on(&mut self, on: bool) {
        self.plan_on = on;
        self.save("the planning setting", |db| {
            db.set_setting(crate::persist::PLAN, &on)
        });
    }

    /// Looks at a message before the agent answers it. True when planning took it over, so the
    /// caller does not start a reply: one is started when planning decides it should be answered
    /// the ordinary way.
    pub(crate) fn plan_begin(&mut self, sid: &str, message: &str) -> bool {
        if !self.plan_on
            || message.trim().is_empty()
            || self.busy_in(sid)
            || !self.typesafe_connected()
        {
            return false;
        }
        let Some(agent) = self.roster.owner_of(sid) else {
            return false;
        };
        if agent.workdir.as_deref().is_none_or(|w| w.trim().is_empty()) {
            return false;
        }
        let turn = agent
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .map_or(0, |s| u32::try_from(s.messages.len()).unwrap_or(u32::MAX));
        let key = self
            .store
            .as_ref()
            .and_then(|db| db.provider_key(crate::providers::TYPESAFE).ok().flatten());
        let (Some(key), Ok(crew)) = (key, self.crew_of(sid)) else {
            return false;
        };
        let folder = self.workdir_of(sid);
        let cancel = Arc::new(AtomicBool::new(false));
        self.planning.insert(sid.to_owned(), Arc::clone(&cancel));
        self.refresh_status_of(sid);
        let (sid, message) = (sid.to_owned(), message.to_owned());
        let stopped = Arc::clone(&cancel);
        let worker_sid = sid.clone();
        self.spawn(
            move || {
                let taken = RefCell::new(Vec::new());
                let ordinary = decide(
                    &Setup {
                        sid: &worker_sid,
                        key: &key,
                        message: &message,
                        turn,
                        crew: &crew,
                        cancel: &cancel,
                        taken: &taken,
                    },
                    &Store::open(&folder),
                );
                (taken.into_inner(), ordinary)
            },
            move |app, (taken, ordinary)| {
                for t in taken {
                    app.record_taken(t);
                }
                app.planning.remove(&sid);
                app.refresh_status_of(&sid);
                // Stopped meanwhile: no reply is started either.
                if ordinary && !stopped.load(Ordering::Relaxed) {
                    app.begin_in(&sid);
                }
            },
        );
        true
    }

    /// Puts a line from the plan in the chat, as the agent's.
    fn plan_say(&mut self, sid: &str, text: String) {
        let Some(chat) = self
            .roster
            .owner_of_mut(sid)
            .and_then(|a| a.sessions.iter_mut().find(|s| s.id == sid))
        else {
            return;
        };
        let message = Message::reply(text, None, Finish::Done);
        let seq = chat.messages.len();
        chat.messages.push(message.clone());
        self.save("the plan", |db| db.put_message(sid, seq, &message));
    }
}

/// What a worker needs to look at one message.
struct Setup<'a> {
    sid: &'a str,
    key: &'a str,
    message: &'a str,
    turn: u32,
    crew: &'a Crew,
    cancel: &'a Arc<AtomicBool>,
    /// Every task done for the plan, to be filed as runs on the UI thread.
    taken: &'a RefCell<Vec<Taken>>,
}

fn say(sid: &str, text: String) {
    let sid = sid.to_owned();
    crate::app::on_ui_thread(move |app| app.plan_say(&sid, text));
}

fn warn(what: &str, why: impl std::fmt::Display) {
    eprintln!("oido: plans: {what}: {why}");
}

/// Places the message and does what that calls for. True when the message should now be answered
/// the ordinary way.
fn decide(s: &Setup, store: &Store) -> bool {
    let mut state = match store.state() {
        Ok(state) => state,
        Err(e) => {
            warn("could not read .oido", e);
            return true;
        }
    };
    let open = state
        .active_plan_id
        .as_deref()
        .and_then(|id| match store.plan(id) {
            Ok(plan) => plan,
            Err(e) => {
                warn("could not read the open plan", e);
                None
            }
        })
        .filter(|p| matches!(p.status, PlanStatus::Active | PlanStatus::Draft));
    if let Some(mut plan) = open {
        let turn = Turn {
            text: s.message,
            plan: &plan,
            parked: &[],
        };
        return match router::classify(typesafe::BASE, s.key, &turn) {
            Err(e) => {
                warn("the router failed", e);
                true
            }
            Ok(Handler::Continue) => {
                plan.status = PlanStatus::Active;
                go(s, store, &mut state, &mut plan);
                false
            }
            Ok(Handler::Modify) => {
                revise(s, store, &mut plan);
                false
            }
            Ok(Handler::Conflict) => {
                say(
                    s.sid,
                    "That would change a step that is already done, and I will not redo or rewrite a done step. Tell me what to do instead: add a new step after it, or cancel the plan.".to_owned(),
                );
                false
            }
            Ok(Handler::Cancel) => {
                plan.status = PlanStatus::Aborted;
                end(store, &mut state, &plan);
                say(
                    s.sid,
                    format!(
                        "Plan stopped: {}\n\nSteps already done stay done.",
                        plan.goal
                    ),
                );
                false
            }
            Ok(Handler::Clarify) => {
                let waiting = plan.status == PlanStatus::Draft;
                say(
                    s.sid,
                    format!(
                        "There is a plan open (\"{}\"){}. Do you want to {} it, change it, or cancel it?",
                        plan.goal,
                        if waiting { ", waiting for your OK" } else { "" },
                        if waiting { "approve" } else { "carry on with" },
                    ),
                );
                false
            }
            // An aside, a sub-plan, new work: answered as an ordinary message, plan untouched.
            Ok(_) => true,
        };
    }
    match typesafe::probability(
        typesafe::BASE,
        s.key,
        s.message,
        MULTI_STEP_QUESTION,
        TIMEOUT,
    ) {
        Ok(p) if p >= MULTI_STEP => {}
        Ok(_) => return true,
        Err(e) => {
            warn("could not tell if the request needs a plan", e);
            return true;
        }
    }
    let asker = s.crew.without_tools();
    let planner = ModelPlanner {
        crew: &asker,
        cancel: s.cancel,
        taken: s.taken,
    };
    let mut plan = match Gate::default().plan(&planner, s.message, s.turn) {
        Ok(plan) => plan,
        Err(e) => {
            warn("no plan", e);
            return true;
        }
    };
    plan.status = PlanStatus::Draft;
    state.active_plan_id = Some(plan.id.clone());
    if let Err(e) = store
        .save_plan(&plan)
        .and_then(|()| store.save_state(&state))
    {
        warn("could not save the plan", e);
        return true;
    }
    say(s.sid, proposal(&plan));
    false
}

/// The model rewrites the steps not yet done as the person asked. The plan goes back to a draft,
/// so what runs next is always what they last approved.
fn revise(s: &Setup, store: &Store, plan: &mut Plan) {
    let asker = s.crew.without_tools();
    let planner = ModelPlanner {
        crew: &asker,
        cancel: s.cancel,
        taken: s.taken,
    };
    let revised = planner
        .revise(plan, s.message)
        .and_then(|specs| plan.replace_pending(specs));
    if let Err(e) = revised {
        say(
            s.sid,
            format!(
                "I could not change the plan: {e}. It is as it was; say what you want changed, or approve it."
            ),
        );
        return;
    }
    plan.status = PlanStatus::Draft;
    if let Err(e) = store.save_plan(plan) {
        warn("could not save the plan", e);
    }
    say(s.sid, proposal(plan));
}

/// The plan as the person is asked to approve it.
fn proposal(plan: &Plan) -> String {
    let outside = plan.steps.iter().any(|st| st.side_effect);
    format!(
        "**Plan** — nothing has run yet.\n\n{}\n\n{}Reply **go** to run it, tell me what to change, or **cancel**.",
        listing(plan),
        if outside {
            "⚠ marks a step that changes something outside this folder.\n\n"
        } else {
            ""
        },
    )
}

/// Runs what is pending of `plan`, saving it and telling the chat as each step ends.
fn go(s: &Setup, store: &Store, state: &mut State, plan: &mut Plan) {
    let asker = s.crew.without_tools();
    let planner = ModelPlanner {
        crew: &asker,
        cancel: s.cancel,
        taken: s.taken,
    };
    let runner = ModelRunner {
        crew: s.crew,
        cancel: s.cancel,
        taken: s.taken,
        goal: plan.goal.clone(),
        done: RefCell::new(
            plan.steps
                .iter()
                .filter(|st| st.status == StepStatus::Done)
                .map(|st| (st.action.clone(), st.result.clone().unwrap_or_default()))
                .collect(),
        ),
    };
    let told: RefCell<HashSet<u32>> = RefCell::new(
        plan.steps
            .iter()
            .filter(|st| st.status != StepStatus::Pending)
            .map(|st| st.id)
            .collect(),
    );
    let replans = Cell::new(plan.replans);
    let outcome = run(plan, &runner, &planner, s.cancel, &mut |p| {
        if let Err(e) = store.save_plan(p) {
            warn("could not save the plan", e);
        }
        progress(s.sid, p, &mut told.borrow_mut(), &replans);
    });
    match outcome {
        Outcome::Done => {
            end(store, state, plan);
            let last = plan
                .steps
                .iter()
                .rev()
                .find(|st| st.status == StepStatus::Done)
                .and_then(|st| st.result.as_deref())
                .unwrap_or_default();
            say(s.sid, format!("**Plan finished.**\n\n{last}"));
        }
        Outcome::Failed { step, error, why } => {
            end(store, state, plan);
            say(s.sid, failure(step, &error, &why));
        }
        Outcome::Stopped => say(
            s.sid,
            "Plan paused. Tell me to carry on and it goes on with the steps left.".to_owned(),
        ),
        Outcome::NotActive => {}
    }
}

/// Tells the chat about steps that have ended since last time, and a changed plan.
fn progress(sid: &str, plan: &Plan, told: &mut HashSet<u32>, replans: &Cell<u32>) {
    for st in &plan.steps {
        if st.status == StepStatus::Pending || !told.insert(st.id) {
            continue;
        }
        let mark = if st.status == StepStatus::Done {
            "✓"
        } else {
            "✗"
        };
        say(sid, format!("{mark} Step {}: {}", st.id, st.action));
    }
    if plan.replans != replans.replace(plan.replans) && plan.status == super::PlanStatus::Active {
        say(
            sid,
            format!("**Plan changed** after a step failed:\n\n{}", listing(plan)),
        );
    }
}

/// The plan is over: nothing is open in the folder any more.
fn end(store: &Store, state: &mut State, plan: &Plan) {
    state.active_plan_id = None;
    if let Err(e) = store.save_plan(plan).and_then(|()| store.save_state(state)) {
        warn("could not close the plan", e);
    }
}

fn listing(plan: &Plan) -> String {
    plan.steps
        .iter()
        .map(|s| {
            let done = if s.status == StepStatus::Done {
                " ✓"
            } else {
                ""
            };
            let outside = if s.side_effect { "⚠ " } else { "" };
            format!("{}. {outside}{}{done}", s.id, s.action)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn failure(step: u32, error: &str, why: &Why) -> String {
    let because = match why {
        Why::CapReached => "the plan had already been changed twice".to_owned(),
        Why::SideEffect => "it changes something outside this folder and may have partly happened, so I did not retry or plan around it".to_owned(),
        Why::Replan(e) => format!("the plan could not be changed: {e}"),
    };
    format!("**Plan stopped.** Step {step} failed: {error}\n\nI did not go on because {because}.")
}

fn brief(text: &str, most: usize) -> String {
    let mut out: String = text.chars().take(most).collect();
    if text.chars().count() > most {
        out.push('…');
    }
    out
}

/// Steps from the model's reply: the first JSON array in it.
fn parse_steps(reply: &str) -> Result<Vec<StepSpec>, PlanError> {
    let not_a_list = || PlanError::Planner("the reply was not a JSON list of steps".into());
    let (Some(from), Some(to)) = (reply.find('['), reply.rfind(']')) else {
        return Err(not_a_list());
    };
    if to < from {
        return Err(not_a_list());
    }
    serde_json::from_str(&reply[from..=to]).map_err(|_| not_a_list())
}

fn plan_prompt(request: &str) -> String {
    format!(
        "You are planning, not doing. Break this request into 2 to 8 concrete steps, to be done in order by an assistant with file, command and web tools and no one to ask.\n{STEP_FORMAT}\n\nRequest: {request}"
    )
}

fn replan_prompt(plan: &Plan, failed: &Step) -> String {
    let done: String = plan
        .steps
        .iter()
        .filter(|s| s.status == StepStatus::Done)
        .map(|s| {
            format!(
                "- {}: {}\n",
                s.action,
                brief(s.result.as_deref().unwrap_or_default(), HANDED_ON)
            )
        })
        .collect();
    format!(
        "A plan is part way through and a step failed. You are planning, not doing.\n\nRequest: {}\n\nDone so far:\n{}\nFailed step: {}\nWhy it failed: {}\n\nGive the steps still needed to finish the request, replacing every step not yet done. Do not repeat what is done.\n{STEP_FORMAT}",
        plan.goal,
        if done.is_empty() {
            "nothing yet\n"
        } else {
            &done
        },
        failed.action,
        brief(failed.result.as_deref().unwrap_or_default(), HANDED_ON),
    )
}

fn revise_prompt(plan: &Plan, change: &str) -> String {
    let current: String = plan
        .steps
        .iter()
        .map(|s| {
            let done = if s.status == StepStatus::Done {
                " (done)"
            } else {
                ""
            };
            format!("{}. {}{done}\n", s.id, s.action)
        })
        .collect();
    format!(
        "You are revising a plan, not doing it.\n\nRequest: {}\n\nCurrent plan:\n{current}\nThe person asks for this change: {change}\n\nGive the steps still to do, replacing every step not yet done: keep what should stay, and change, add or drop the rest as asked. Do not repeat steps already done.\n{STEP_FORMAT}",
        plan.goal,
    )
}

fn step_prompt(goal: &str, done: &[(String, String)], action: &str) -> String {
    let before: String = done
        .iter()
        .map(|(a, r)| format!("- {a}: {}\n", brief(r, HANDED_ON)))
        .collect();
    format!(
        "You are doing one step of a larger job, with no one to ask.\n\nJob: {goal}\n\nDone so far:\n{}\nDo this step now, then reply with what you did and anything the later steps need to know:\n{action}",
        if before.is_empty() {
            "nothing yet\n"
        } else {
            &before
        },
    )
}

/// The agent's model, asked for steps with no tools in hand.
struct ModelPlanner<'a> {
    crew: &'a Crew,
    cancel: &'a Arc<AtomicBool>,
    taken: &'a RefCell<Vec<Taken>>,
}

impl ModelPlanner<'_> {
    fn ask(&self, task: &str) -> Result<Vec<StepSpec>, PlanError> {
        let taken = self.crew.take(task, None, self.cancel);
        let reply = taken.answer();
        self.taken.borrow_mut().push(taken);
        parse_steps(&reply.map_err(PlanError::Planner)?)
    }
}

impl Planner for ModelPlanner<'_> {
    fn plan(&self, request: &str) -> Result<Vec<StepSpec>, PlanError> {
        self.ask(&plan_prompt(request))
    }

    fn replan(&self, plan: &Plan, failed: &Step) -> Result<Vec<StepSpec>, PlanError> {
        self.ask(&replan_prompt(plan, failed))
    }

    fn revise(&self, plan: &Plan, change: &str) -> Result<Vec<StepSpec>, PlanError> {
        self.ask(&revise_prompt(plan, change))
    }
}

/// Each step done by the agent itself, as a task of its own, with what came before in hand.
struct ModelRunner<'a> {
    crew: &'a Crew,
    cancel: &'a Arc<AtomicBool>,
    taken: &'a RefCell<Vec<Taken>>,
    goal: String,
    /// (action, result) of each step done so far.
    done: RefCell<Vec<(String, String)>>,
}

impl StepRunner for ModelRunner<'_> {
    fn run(&self, step: &Step) -> Result<String, String> {
        let task = step_prompt(&self.goal, &self.done.borrow(), &step.action);
        let taken = self.crew.take(&task, None, self.cancel);
        let answer = taken.answer();
        self.taken.borrow_mut().push(taken);
        if let Ok(text) = &answer {
            self.done
                .borrow_mut()
                .push((step.action.clone(), text.clone()));
        }
        answer
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::tests::spec;
    use super::*;

    #[test]
    fn the_first_json_array_in_a_reply_is_the_plan() {
        let reply = "Here you go:\n```json\n[{\"action\":\"write the page\",\"side_effect\":false},{\"action\":\"deploy\",\"side_effect\":true}]\n```";
        let steps = parse_steps(reply).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].action, "write the page");
        assert!(!steps[0].side_effect && steps[1].side_effect);
    }

    #[test]
    fn a_reply_that_is_not_a_list_of_steps_is_an_error() {
        for reply in ["", "no list here", "] [", "[1, 2]", "[{\"nope\": 1}]"] {
            assert!(parse_steps(reply).is_err(), "{reply:?} was accepted");
        }
    }

    #[test]
    fn a_step_carries_the_goal_and_what_came_before() {
        let done = [("make the folder".to_owned(), "made it".to_owned())];
        let p = step_prompt("a bakery site", &done, "write index.html");
        assert!(p.contains("a bakery site") && p.contains("make the folder: made it"));
        assert!(p.ends_with("write index.html"));
        assert!(step_prompt("g", &[], "a").contains("nothing yet"));
    }

    #[test]
    fn a_replan_names_what_is_done_and_what_failed_and_forbids_repeats() {
        let mut plan = Plan::new("g", vec![spec("a", false), spec("b", false)], 0).unwrap();
        plan.steps[0].status = StepStatus::Done;
        plan.steps[0].result = Some("did a".into());
        plan.steps[1].status = StepStatus::Failed;
        plan.steps[1].result = Some("b broke".into());
        let p = replan_prompt(&plan, &plan.steps[1].clone());
        assert!(p.contains("a: did a") && p.contains("Failed step: b"));
        assert!(p.contains("b broke") && p.contains("Do not repeat"));
    }

    #[test]
    fn a_proposal_marks_outside_steps_and_asks_for_approval() {
        let plan = Plan::new("g", vec![spec("write", false), spec("deploy", true)], 0).unwrap();
        let p = proposal(&plan);
        assert!(p.contains("nothing has run yet") && p.contains("2. ⚠ deploy"));
        assert!(p.contains("**go**") && p.contains("outside this folder"));
        let quiet = Plan::new("g", vec![spec("write", false)], 0).unwrap();
        assert!(!proposal(&quiet).contains("outside this folder"));
    }

    #[test]
    fn a_revision_names_the_change_and_marks_done_steps() {
        let mut plan = Plan::new("g", vec![spec("a", false), spec("deploy", true)], 0).unwrap();
        plan.steps[0].status = StepStatus::Done;
        let p = revise_prompt(&plan, "drop the deploy");
        assert!(p.contains("1. a (done)") && p.contains("2. deploy\n"));
        assert!(p.contains("drop the deploy") && p.contains("Do not repeat"));
    }

    #[test]
    fn long_results_are_cut_when_handed_on() {
        assert_eq!(brief("abcdef", 3), "abc…");
        assert_eq!(brief("abc", 3), "abc");
    }

    #[test]
    fn planning_stays_out_of_the_way_unless_it_can_work() {
        let db = crate::testing::TempDb::new();
        let (mut app, _seen) = crate::testing::app_with_server(&db, vec![]);
        let sid = app.roster.current().unwrap().chat().id.clone();
        app.roster.current_mut().unwrap().workdir = Some("/tmp".into());
        // Off: nothing is taken over.
        assert!(!app.plan_begin(&sid, "build me a website"));
        // On, but TypeSafe is not connected: still nothing.
        app.plan_on = true;
        assert!(!app.plan_begin(&sid, "build me a website"));
        assert!(app.planning.is_empty());
    }
}
