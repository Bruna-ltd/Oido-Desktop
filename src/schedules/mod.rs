//! Firing schedules while the app is open. A ticker asks [`App::fire_due`] every [`TICK`] (and once
//! at launch, which catches up on what was missed). A fire opens a new chat with the agent: a prompt
//! fire posts its prompt and ends when the agent stops ([`App::fire_settled`], called from
//! `settle`); a workflow fire runs the workflow and says how it went in that chat. A question the
//! agent asks during a fire is skipped, since nobody is there to answer it.
//!
//! An agent manages its own schedules with the `schedule` tool ([`App::run_schedule_calls`]): what
//! it creates or changes waits as a [`Proposal`] for you to approve; pausing, resuming and deleting
//! apply at once.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use chrono::{Local, TimeZone};
use serde_json::{Map, Value};

use crate::app::{App, now};
use crate::model::{Message, Outcome, chat, new_id};
use crate::providers::ModelStatus;
use crate::runs::Status;
use crate::schedule::timing::when;
use crate::schedule::tool::{self, Call, Fields};
use crate::schedule::{Fire, FireStatus, Proposal, Schedule, Target};
use crate::workflow::RunStatus;

pub mod form;

use form::{Form, Source};

/// The Schedules page: the open schedule and its newest fires, the form when one is being made or
/// changed, a delete waiting for confirmation, and what to tell you.
#[derive(Debug, Default)]
pub struct Page {
    pub open: Option<String>,
    pub fires: Vec<Fire>,
    pub form: Option<Form>,
    pub confirm: bool,
    pub notice: Option<String>,
    /// Why approving a draft in a chat did not go through, by the draft's tool call.
    pub card_notice: Option<(String, String)>,
}

impl Page {
    /// The form, for its text fields to edit. Only drawn while there is one; an empty one stands
    /// in otherwise.
    pub fn form_mut(&mut self) -> &mut Form {
        self.form.get_or_insert_with(|| Form::new(None))
    }
}

/// How many of a schedule's fires are kept (and shown); older ones go at launch, with their chats.
pub const KEEP_FIRES: usize = 20;

/// How often the ticker looks for due schedules: a fire can be this late.
pub const TICK: Duration = Duration::from_secs(30);

/// Calls `check` at launch and every [`TICK`] after, from a thread of its own that lives as long
/// as the app.
pub fn every_tick(check: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        loop {
            check();
            std::thread::sleep(TICK);
        }
    });
}

impl App {
    /// Reads the saved schedules. A fire still marked running was cut off by the app closing.
    pub(crate) fn load_schedules(&mut self) {
        let Some(db) = &self.store else {
            return;
        };
        if let Err(e) = db.interrupt_fires(now()) {
            eprintln!("oido: could not tidy interrupted fires: {e}");
        }
        match db.list_schedules() {
            Ok(list) => self.schedules = list,
            Err(e) => eprintln!("oido: could not load schedules: {e}"),
        }
        match db.list_proposals() {
            Ok(list) => self.proposals = list,
            Err(e) => eprintln!("oido: could not load schedule drafts: {e}"),
        }
        self.tidy_fires();
    }

    /// Keeps each schedule's newest [`KEEP_FIRES`] fires. An older fire goes, and its chat with it
    /// unless you wrote in it after the scheduled prompt: then it stays as an ordinary chat.
    fn tidy_fires(&mut self) {
        let ids: Vec<String> = self.schedules.iter().map(|s| s.id.clone()).collect();
        for id in ids {
            let old = match &self.store {
                Some(db) => db.old_fires(&id, KEEP_FIRES).unwrap_or_else(|e| {
                    eprintln!("oido: could not read old fires: {e}");
                    Vec::new()
                }),
                None => return,
            };
            for (fire, chat) in old {
                if let Some(sid) = chat.filter(|sid| !self.wrote_in(sid)) {
                    self.drop_chat(&sid);
                }
                self.save("the fire", |db| db.delete_fire(&fire));
            }
        }
    }

    /// You wrote in chat `sid` after its first message.
    fn wrote_in(&self, sid: &str) -> bool {
        self.roster
            .owner_of(sid)
            .and_then(|a| a.sessions.iter().find(|s| s.id == sid))
            .is_some_and(|c| {
                c.messages
                    .iter()
                    .filter(|m| matches!(m, Message::User(_)))
                    .count()
                    > 1
            })
    }

    /// Deletes chat `sid`, keeping the agent's open chat open. An agent's only chat stays.
    fn drop_chat(&mut self, sid: &str) {
        let Some(t) = self.roster.target_of(sid) else {
            return;
        };
        let Some(agent) = self.roster.agent_mut(&t.id) else {
            return;
        };
        let Some(i) = agent.sessions.iter().position(|s| s.id == sid) else {
            return;
        };
        if agent.sessions.len() < 2 {
            return;
        }
        agent.sessions.remove(i);
        if i < agent.active {
            agent.active -= 1;
        }
        agent.active = agent.active.min(agent.sessions.len() - 1);
        self.delete_saved_chat(&t, sid);
        self.save_agent_row(&t);
    }

    /// Fires every schedule that is due at `now`. Each fires once however many of its times were
    /// missed, and its next time is counted from `now`. One whose last fire is still going skips
    /// this one.
    pub fn fire_due(&mut self, now: i64) {
        if !self.run_schedules {
            return;
        }
        let due: Vec<(Schedule, i64)> = self
            .schedules
            .iter()
            .filter(|s| s.enabled)
            .filter_map(|s| s.next_at.filter(|t| *t <= now).map(|t| (s.clone(), t)))
            .collect();
        for (s, due_at) in due {
            self.advance(&s.id, now, &Local);
            match self.still_running(&s) {
                Some(why) => self.record_skip(&s, due_at, now, why),
                None => self.start_fire(&s, due_at, now),
            }
        }
    }

    /// Moves a schedule's next time past `now`. A "once" that has fired turns off.
    fn advance<Tz: TimeZone>(&mut self, id: &str, now: i64, tz: &Tz) {
        let Some(s) = self.schedules.iter_mut().find(|s| s.id == id) else {
            return;
        };
        s.next_at = s.timing.next_after(now, tz);
        s.enabled &= s.next_at.is_some();
        let (enabled, next_at) = (s.enabled, s.next_at);
        self.save("the schedule", |db| {
            db.set_schedule_state(id, enabled, next_at)
        });
    }

    /// Why the schedule cannot fire now, if it cannot: its last fire, or its workflow, still runs.
    fn still_running(&self, s: &Schedule) -> Option<&'static str> {
        if self.firing.contains_key(&s.id) {
            return Some("The previous fire was still running.");
        }
        match &s.target {
            Target::Workflow { id, .. } if self.wf_jobs.contains_key(id) => {
                Some("The workflow was already running.")
            }
            _ => None,
        }
    }

    fn record_skip(&mut self, s: &Schedule, due_at: i64, now: i64, why: &str) {
        let fire = Fire {
            id: new_id(),
            schedule_id: s.id.clone(),
            due_at,
            started_at: now,
            finished_at: Some(now),
            status: FireStatus::Skipped,
            note: why.to_owned(),
            session_id: None,
            workflow_run_id: None,
        };
        self.save("the fire", |db| db.insert_fire(&fire));
    }

    /// Opens the fire's chat with the agent and starts what the schedule does in it.
    fn start_fire(&mut self, s: &Schedule, due_at: i64, now: i64) {
        let title = format!("{} · {}", s.name, short_time(now, &Local));
        let session = chat(&title, Vec::new());
        let sid = session.id.clone();
        let Some(agent) = self.roster.agent_mut(&s.agent_id) else {
            return;
        };
        agent.sessions.push(session.clone());
        let position = agent.sessions.len() - 1;
        self.save("the chat", |db| {
            db.save_session(&s.agent_id, &session, position)
        });
        let fire = Fire {
            id: new_id(),
            schedule_id: s.id.clone(),
            due_at,
            started_at: now,
            finished_at: None,
            status: FireStatus::Running,
            note: String::new(),
            session_id: Some(sid.clone()),
            workflow_run_id: None,
        };
        self.save("the fire", |db| db.insert_fire(&fire));
        self.firing.insert(s.id.clone(), fire);
        match &s.target {
            Target::Prompt(text) => self.fire_prompt(s, &sid, text),
            Target::Workflow { id, inputs } => self.fire_workflow(s, &sid, id, inputs),
        }
    }

    fn fire_prompt(&mut self, s: &Schedule, sid: &str, text: &str) {
        let status = self
            .roster
            .agent_ref(&s.agent_id)
            .map(|a| self.providers.status(a.model.as_ref()));
        let why = match status {
            Some(ModelStatus::Ready(_)) => {
                self.post(sid, text.into());
                return;
            }
            Some(ModelStatus::Unavailable { why, .. }) => format!("The agent cannot answer: {why}"),
            _ => "The agent has no model to answer with.".to_owned(),
        };
        self.append(sid, Message::User(text.into()));
        self.append(sid, Message::reply("", None, Outcome::Failed(why.clone())));
        self.end_fire(&s.id, FireStatus::Failed, &why, None);
    }

    fn fire_workflow(&mut self, s: &Schedule, sid: &str, wf_id: &str, inputs: &Map<String, Value>) {
        let Some(wf) = self.workflows.iter().find(|w| w.id == wf_id).cloned() else {
            let why = "The workflow is gone.";
            self.append(
                sid,
                Message::reply("", None, Outcome::Failed(why.to_owned())),
            );
            self.end_fire(&s.id, FireStatus::Failed, why, None);
            return;
        };
        self.append(
            sid,
            Message::User(format!("Run the workflow {:?}.", wf.name).into()),
        );
        let cancel = Arc::new(AtomicBool::new(false));
        self.wf_jobs.insert(wf.id.clone(), Arc::clone(&cancel));
        let (schedule_id, chat_id, job) = (s.id.clone(), sid.to_owned(), wf.id.clone());
        let started_by = format!("schedule: {}", s.name);
        let started = self.launch_flow(&wf, &s.agent_id, inputs, &started_by, cancel, {
            move |app, record, finish| {
                app.wf_jobs.remove(&job);
                let (crate::command::Finish::Done(text) | crate::command::Finish::Stopped(text)) =
                    finish;
                let (status, outcome) = match record.status {
                    RunStatus::Succeeded => (FireStatus::Succeeded, Outcome::Done),
                    RunStatus::Stopped => (FireStatus::Failed, Outcome::Stopped),
                    _ => (FireStatus::Failed, Outcome::Failed(record.error.clone())),
                };
                let note = if status == FireStatus::Succeeded {
                    ""
                } else {
                    text.as_str()
                };
                app.append(&chat_id, Message::reply(text.clone(), None, outcome));
                app.end_fire(&schedule_id, status, note, Some(record.id.clone()));
            }
        });
        if let Err(why) = started {
            self.wf_jobs.remove(&wf.id);
            self.append(sid, Message::reply("", None, Outcome::Failed(why.clone())));
            self.end_fire(&s.id, FireStatus::Failed, &why, None);
        }
    }

    /// A run in chat `sid` ended with `status`. If the chat is a fire's: a question the agent asked
    /// is skipped (it carries on), and once nothing in the chat is left going, the fire ends.
    pub(crate) fn fire_settled(&mut self, sid: &str, status: Status, failure: Option<&str>) {
        if self.firing_in(sid).is_none() {
            return;
        }
        if status == Status::Ok
            && let Some(call) = self.pending_call_in(sid, crate::tools::ASK)
        {
            self.resolve(sid, &call, crate::tools::SKIPPED.to_owned(), true);
        }
        // Skipping starts a new run, which may already have ended the fire.
        let Some(schedule_id) = self.firing_in(sid) else {
            return;
        };
        if self.busy_in(sid) {
            return;
        }
        let (fire_status, note) = match status {
            Status::Ok => (FireStatus::Succeeded, ""),
            Status::Cancelled => (FireStatus::Failed, "Stopped."),
            Status::Error | Status::Running => (FireStatus::Failed, failure.unwrap_or("Failed.")),
        };
        self.end_fire(&schedule_id, fire_status, note, None);
    }

    /// The schedule whose running fire has chat `sid`.
    pub(crate) fn firing_in(&self, sid: &str) -> Option<String> {
        self.firing
            .iter()
            .find(|(_, f)| f.session_id.as_deref() == Some(sid))
            .map(|(id, _)| id.clone())
    }

    /// Records how the schedule's running fire ended, and says so in a desktop notification.
    fn end_fire(&mut self, schedule_id: &str, status: FireStatus, note: &str, run: Option<String>) {
        let Some(mut fire) = self.firing.remove(schedule_id) else {
            return;
        };
        fire.status = status;
        fire.note = note.to_owned();
        fire.finished_at = Some(now());
        fire.workflow_run_id = run;
        self.save("the fire", |db| db.finish_fire(&fire));
        let name = self
            .schedules
            .iter()
            .find(|s| s.id == schedule_id)
            .map_or("A schedule", |s| s.name.as_str());
        let body = match status {
            FireStatus::Succeeded => "Done.",
            _ => note,
        };
        crate::notify::post(crate::notify::Note {
            title: name.to_owned(),
            body: body.to_owned(),
            chat: fire.session_id.clone(),
        });
    }

    /// Adds a message to the end of chat `sid` and saves it.
    fn append(&mut self, sid: &str, m: Message) {
        let Some(chat) = self
            .roster
            .owner_of_mut(sid)
            .and_then(|a| a.sessions.iter_mut().find(|s| s.id == sid))
        else {
            return;
        };
        chat.messages.push(m);
        let seq = chat.messages.len() - 1;
        let saved = chat.messages[seq].clone();
        self.save("the message", |db| db.put_message(sid, seq, &saved));
    }

    // ---- the `schedule` tool

    /// Answers the `schedule` calls waiting in chat `sid`'s last reply. Always answers each, even
    /// to refuse it, so the agent carries on.
    pub(crate) fn run_schedule_calls(&mut self, sid: &str) {
        let calls: Vec<(String, String)> = match self
            .roster
            .owner_of(sid)
            .and_then(|a| a.sessions.iter().find(|s| s.id == sid))
            .and_then(|s| s.messages.last())
        {
            Some(Message::Assistant { tools, .. }) => tools
                .iter()
                .filter(|t| t.name == tool::NAME && t.output.is_none())
                .map(|t| (t.id.clone(), t.args.clone()))
                .collect(),
            _ => return,
        };
        for (id, args) in calls {
            let told = self
                .schedule_call(sid, &id, &args)
                .unwrap_or_else(|why| format!("Error: {why}."));
            self.resolve(sid, &id, told, true);
        }
    }

    fn schedule_call(&mut self, sid: &str, call_id: &str, args: &str) -> Result<String, String> {
        let agent_id = self
            .roster
            .owner_of(sid)
            .map(|a| a.id.clone())
            .ok_or("this chat has no agent")?;
        let now = now();
        match tool::parse(args, now, &Local)? {
            Call::List => Ok(self.list_for(&agent_id)),
            Call::Create(f) => {
                let target = self.target_of(&agent_id, &f, None)?;
                let (name, timing) = (f.name.unwrap_or_default(), f.timing);
                let timing = timing.ok_or("say when it fires")?;
                let s = Schedule::new(&agent_id, &name, target, timing, now, &Local)?;
                Ok(self.propose(sid, call_id, s, false))
            }
            Call::Update { id, fields } => {
                let mut s = self.own(&agent_id, &id)?.clone();
                s.target = self.target_of(&agent_id, &fields, Some(&s.target))?;
                if let Some(t) = fields.timing {
                    s.timing = t;
                }
                let name = fields.name.unwrap_or_else(|| s.name.clone());
                s.change(&name, now, &Local)?;
                Ok(self.propose(sid, call_id, s, true))
            }
            Call::Pause(id) => {
                let name = self.own(&agent_id, &id)?.name.clone();
                self.set_enabled(&id, false, now)?;
                Ok(format!("Paused {name:?}. It will not fire until resumed."))
            }
            Call::Resume(id) => {
                let name = self.own(&agent_id, &id)?.name.clone();
                self.set_enabled(&id, true, now)?;
                Ok(format!("Resumed {name:?}. {}", self.next_words(&id)))
            }
            Call::Delete(id) => {
                let name = self.own(&agent_id, &id)?.name.clone();
                self.delete_schedule(&id);
                Ok(format!("Deleted {name:?}."))
            }
        }
    }

    /// The agent's schedule with this id. Another agent's reads as not found.
    fn own(&self, agent_id: &str, id: &str) -> Result<&Schedule, String> {
        self.schedules
            .iter()
            .find(|s| s.id == id && s.agent_id == agent_id)
            .ok_or_else(|| format!("you have no schedule with id {id:?}; `list` shows yours"))
    }

    /// What the schedule does, from a call's fields: a prompt, one of the agent's own workflows by
    /// name, or (for an update that names neither) `current`, with new inputs if given.
    fn target_of(
        &self,
        agent_id: &str,
        f: &Fields,
        current: Option<&Target>,
    ) -> Result<Target, String> {
        if let Some(p) = &f.prompt {
            return Ok(Target::Prompt(p.clone()));
        }
        let (wf_id, inputs) = match (&f.workflow, current) {
            (Some(name), _) => {
                let wf = self
                    .workflows
                    .iter()
                    .find(|w| w.agent_id == agent_id && w.name.eq_ignore_ascii_case(name))
                    .ok_or_else(|| format!("you have no saved workflow named {name:?}"))?;
                (wf.id.clone(), f.inputs.clone().unwrap_or_default())
            }
            (None, Some(Target::Workflow { id, inputs })) => (
                id.clone(),
                f.inputs.clone().unwrap_or_else(|| inputs.clone()),
            ),
            (None, Some(t @ Target::Prompt(_))) => return Ok(t.clone()),
            (None, None) => return Err("give a `prompt` or a `workflow` to run".to_owned()),
        };
        if let Some(wf) = self.workflows.iter().find(|w| w.id == wf_id) {
            crate::workflow::engine::bind_inputs(&wf.definition, &inputs)?;
        }
        Ok(Target::Workflow { id: wf_id, inputs })
    }

    /// Shows the schedule in chat `sid` as a draft to approve, and says so to the agent.
    fn propose(&mut self, sid: &str, call_id: &str, schedule: Schedule, updates: bool) -> String {
        let p = Proposal {
            call_id: call_id.to_owned(),
            session_id: sid.to_owned(),
            updates,
            schedule,
        };
        self.save("the schedule draft", |db| db.put_proposal(&p));
        let told = format!(
            "The person now sees a draft of {:?} ({}) to approve. It does nothing until they approve it; tell them it is waiting for them.",
            p.schedule.name,
            p.schedule.timing.describe(&Local),
        );
        self.proposals.retain(|q| q.call_id != call_id);
        self.proposals.push(p);
        told
    }

    /// Your schedules and the drafts waiting for you, for the agent: one line each.
    fn list_for(&self, agent_id: &str) -> String {
        let mine: Vec<String> = self
            .schedules
            .iter()
            .filter(|s| s.agent_id == agent_id)
            .map(|s| {
                let state = match (s.enabled, s.next_at) {
                    (false, _) => "paused".to_owned(),
                    (true, Some(t)) => format!("next {}", when(t, &Local)),
                    (true, None) => "finished".to_owned(),
                };
                format!(
                    "- id {} · {:?} · {} · {state} · {}",
                    s.id,
                    s.name,
                    s.timing.describe(&Local),
                    self.target_words(&s.target)
                )
            })
            .collect();
        let waiting: Vec<String> = self
            .proposals
            .iter()
            .filter(|p| p.schedule.agent_id == agent_id)
            .map(|p| {
                format!(
                    "- {:?} · {}",
                    p.schedule.name,
                    p.schedule.timing.describe(&Local)
                )
            })
            .collect();
        let mut out = if mine.is_empty() {
            "You have no schedules.".to_owned()
        } else {
            format!("Your schedules:\n{}", mine.join("\n"))
        };
        if !waiting.is_empty() {
            out.push_str(&format!(
                "\nWaiting for the person's approval:\n{}",
                waiting.join("\n")
            ));
        }
        out
    }

    fn target_words(&self, t: &Target) -> String {
        match t {
            Target::Prompt(p) => format!("sends {p:?}"),
            Target::Workflow { id, .. } => {
                let name = self
                    .workflows
                    .iter()
                    .find(|w| &w.id == id)
                    .map_or("?", |w| w.name.as_str());
                format!("runs the workflow {name:?}")
            }
        }
    }

    fn next_words(&self, id: &str) -> String {
        match self
            .schedules
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.next_at)
        {
            Some(t) => format!("Next: {}.", when(t, &Local)),
            None => String::new(),
        }
    }

    /// Pauses or resumes a schedule. Resuming counts its next time from `now`; errs when it has
    /// none left (a "once" that has passed).
    fn set_enabled(&mut self, id: &str, on: bool, now: i64) -> Result<(), String> {
        let s = self
            .schedules
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or("that schedule is gone")?;
        if on {
            s.next_at = Some(
                s.timing
                    .next_after(now, &Local)
                    .ok_or("its time has passed; give it a new one with `update`")?,
            );
        }
        s.enabled = on;
        let (enabled, next_at) = (s.enabled, s.next_at);
        self.save("the schedule", |db| {
            db.set_schedule_state(id, enabled, next_at)
        });
        Ok(())
    }

    fn delete_schedule(&mut self, id: &str) {
        self.schedules.retain(|s| s.id != id);
        self.proposals
            .retain(|p| !(p.updates && p.schedule.id == id));
        self.save("the schedule", |db| db.delete_schedule(id));
    }

    // ---- approving drafts

    /// Saves the draft from tool call `call_id` as a schedule (or as the change to one), counting
    /// its next time from now. Errs, in words, if it can no longer be saved; the draft then stays.
    pub fn approve_proposal(&mut self, call_id: &str) -> Result<(), String> {
        let Some(i) = self.proposals.iter().position(|p| p.call_id == call_id) else {
            return Err("That draft is gone.".to_owned());
        };
        let mut s = self.proposals[i].schedule.clone();
        if self.proposals[i].updates {
            let Some(saved) = self.schedules.iter().find(|x| x.id == s.id) else {
                self.reject_proposal(call_id);
                return Err("The schedule it changes was deleted.".to_owned());
            };
            s.enabled = saved.enabled;
        }
        let name = s.name.clone();
        s.change(&name, now(), &Local)?;
        self.save("the schedule", |db| db.put_schedule(&s));
        match self.schedules.iter_mut().find(|x| x.id == s.id) {
            Some(slot) => *slot = s,
            None => self.schedules.push(s),
        }
        self.reject_proposal(call_id);
        Ok(())
    }

    /// Drops the draft from tool call `call_id`.
    pub fn reject_proposal(&mut self, call_id: &str) {
        self.proposals.retain(|p| p.call_id != call_id);
        self.save("the schedule draft", |db| db.delete_proposal(call_id));
    }

    // ---- the Schedules page

    /// The page opens on the schedule it was on, or the first.
    pub(crate) fn open_schedules(&mut self) {
        let keep = self
            .schedule_page
            .open
            .clone()
            .filter(|id| self.schedules.iter().any(|s| &s.id == id));
        match keep.or_else(|| self.schedules.first().map(|s| s.id.clone())) {
            Some(id) if self.schedule_page.form.is_none() => self.select_schedule(&id),
            _ => {}
        }
    }

    pub fn select_schedule(&mut self, id: &str) {
        let page = &mut self.schedule_page;
        page.open = Some(id.to_owned());
        page.form = None;
        page.confirm = false;
        page.notice = None;
        self.reload_fires();
    }

    fn reload_fires(&mut self) {
        let fires = match (&self.store, &self.schedule_page.open) {
            (Some(db), Some(id)) => db.list_fires(id, KEEP_FIRES).unwrap_or_else(|e| {
                eprintln!("oido: could not read fires: {e}");
                Vec::new()
            }),
            _ => Vec::new(),
        };
        self.schedule_page.fires = fires;
    }

    /// A blank form, for the open agent.
    pub fn new_schedule(&mut self) {
        let agent = self.current().map(|a| a.id.clone());
        self.schedule_page.form = Some(Form::new(agent));
        self.schedule_page.notice = None;
    }

    /// The open schedule, in the form.
    pub fn edit_schedule(&mut self) {
        let Some(s) = self.open_schedule().cloned() else {
            return;
        };
        let source = Source::Edit(s.id.clone());
        self.schedule_page.form = Some(Form::of(&s, source, &self.workflows, &Local));
    }

    /// An agent's draft, in the form on the Schedules page: saving it approves it as changed.
    pub fn edit_proposal(&mut self, call_id: &str) {
        let Some(p) = self.proposals.iter().find(|p| p.call_id == call_id) else {
            return;
        };
        let form = Form::of(
            &p.schedule,
            Source::Draft(call_id.to_owned()),
            &self.workflows,
            &Local,
        );
        self.go(crate::app::Route::Schedules);
        self.schedule_page.form = Some(form);
        self.schedule_page.notice = None;
    }

    pub fn cancel_schedule_form(&mut self) {
        self.schedule_page.form = None;
    }

    /// Saves what the form says: a new schedule, the change to one, or an agent's draft (which
    /// approves it). The form stays, with why, if it cannot be saved.
    pub fn save_schedule_form(&mut self) {
        let Some(form) = &self.schedule_page.form else {
            return;
        };
        let now = now();
        let saved =
            form.read(&self.workflows, now, &Local)
                .and_then(|(agent, name, target, timing)| {
                    let mut s = match &form.source {
                        Source::New => {
                            return Schedule::new(&agent, &name, target, timing, now, &Local);
                        }
                        Source::Edit(id) => self.schedules.iter().find(|s| &s.id == id).cloned(),
                        Source::Draft(call) => {
                            let p = self.proposals.iter().find(|p| &p.call_id == call);
                            let updates = p.is_some_and(|p| p.updates);
                            p.map(|p| p.schedule.clone()).map(|mut s| {
                                // A draft that changes a schedule keeps it paused if it is.
                                if updates
                                    && let Some(old) = self.schedules.iter().find(|o| o.id == s.id)
                                {
                                    s.enabled = old.enabled;
                                }
                                s
                            })
                        }
                    }
                    .ok_or("It is gone.")?;
                    s.agent_id = agent;
                    s.target = target;
                    s.timing = timing;
                    s.change(&name, now, &Local)?;
                    Ok(s)
                });
        let s = match saved {
            Ok(s) => s,
            Err(why) => {
                self.schedule_page.form_mut().notice = Some(why);
                return;
            }
        };
        if let Some(Source::Draft(call)) =
            self.schedule_page.form.as_ref().map(|f| f.source.clone())
        {
            self.reject_proposal(&call);
        }
        self.save("the schedule", |db| db.put_schedule(&s));
        let id = s.id.clone();
        match self.schedules.iter_mut().find(|x| x.id == s.id) {
            Some(slot) => *slot = s,
            None => self.schedules.push(s),
        }
        self.select_schedule(&id);
    }

    fn open_schedule(&self) -> Option<&Schedule> {
        let id = self.schedule_page.open.as_ref()?;
        self.schedules.iter().find(|s| &s.id == id)
    }

    /// Pauses the open schedule, or resumes it.
    pub fn toggle_open_schedule(&mut self) {
        let Some((id, on)) = self.open_schedule().map(|s| (s.id.clone(), !s.enabled)) else {
            return;
        };
        self.schedule_page.notice = self.set_enabled(&id, on, now()).err();
    }

    /// Fires the open schedule now, as if it were due; its timing is left as it was.
    pub fn run_open_schedule(&mut self) {
        let Some(s) = self.open_schedule().cloned() else {
            return;
        };
        if let Some(why) = self.still_running(&s) {
            self.schedule_page.notice = Some(why.to_owned());
            return;
        }
        let now = now();
        self.start_fire(&s, now, now);
        self.reload_fires();
    }

    /// Asks first, then deletes the open schedule. Its fires' chats stay.
    pub fn delete_open_schedule(&mut self) {
        if !self.schedule_page.confirm {
            self.schedule_page.confirm = true;
            return;
        }
        if let Some(id) = self.schedule_page.open.take() {
            self.delete_schedule(&id);
        }
        self.schedule_page.confirm = false;
        self.schedule_page.fires.clear();
        self.open_schedules();
    }

    pub fn cancel_delete_schedule(&mut self) {
        self.schedule_page.confirm = false;
    }

    /// Settings: schedules run, or none do.
    pub fn set_run_schedules(&mut self, on: bool) {
        self.run_schedules = on;
        self.save("the schedules setting", |db| {
            db.set_setting(crate::persist::RUN_SCHEDULES, &on)
        });
        if on {
            self.fire_due(now());
        }
    }

    /// The draft card's Approve: saves it, or keeps it with why.
    pub fn approve_from_card(&mut self, call_id: &str) {
        self.schedule_page.card_notice = self
            .approve_proposal(call_id)
            .err()
            .map(|why| (call_id.to_owned(), why));
    }

    /// The next few times a schedule would fire after now, for a draft card.
    pub fn upcoming(&self, s: &Schedule, n: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut t = now();
        while out.len() < n {
            let Some(next) = s.timing.next_after(t, &Local) else {
                break;
            };
            out.push(when(next, &Local));
            t = next;
        }
        out
    }
}

/// A time short enough for a chat's title: "10 Nov 09:00".
fn short_time<Tz: TimeZone>(t: i64, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    tz.timestamp_opt(t, 0)
        .earliest()
        .map_or_else(|| t.to_string(), |d| d.format("%-d %b %H:%M").to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;
    use crate::schedule::Timing;
    use crate::testing::{EVENTS, TempDb, app_with_server, call_stream, text_stream};

    const START: i64 = 1_800_000_000;

    /// A schedule for the open agent, every minute from [`START`], saved and loaded.
    fn every_minute(app: &mut App, target: Target) -> Schedule {
        let agent = app.current().unwrap().id.clone();
        let timing = Timing::every(START, 60).unwrap();
        let s = Schedule::new(&agent, "Check", target, timing, START - 1, &Utc).unwrap();
        app.save("s", |db| db.put_schedule(&s));
        app.schedules.push(s.clone());
        s
    }

    fn fires(app: &App, s: &Schedule) -> Vec<Fire> {
        app.store.as_ref().unwrap().list_fires(&s.id, 50).unwrap()
    }

    fn chat_of<'a>(app: &'a App, fire: &Fire) -> &'a crate::model::Session {
        let sid = fire.session_id.as_deref().unwrap();
        let agent = app.roster.owner_of(sid).unwrap();
        agent.sessions.iter().find(|c| c.id == sid).unwrap()
    }

    #[test]
    fn a_due_prompt_opens_a_chat_the_agent_answers_and_the_fire_succeeds() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(&db, vec![(200, EVENTS, text_stream("All quiet."))]);
        let open_before = app.current().unwrap().chat().id.clone();
        let s = every_minute(&mut app, Target::Prompt("Check the logs".into()));

        app.fire_due(START);
        assert!(seen.recv().unwrap().contains("check the logs"));
        let f = &fires(&app, &s)[0];
        assert_eq!(f.status, FireStatus::Succeeded, "{f:?}");
        assert_eq!(f.due_at, START);
        let c = chat_of(&app, f);
        assert!(c.title.starts_with("Check · "), "{}", c.title);
        assert!(
            matches!(c.messages.last(), Some(Message::Assistant { text, .. }) if text == "All quiet.")
        );
        assert_eq!(
            app.current().unwrap().chat().id,
            open_before,
            "the open chat stays open"
        );
        assert!(app.firing.is_empty());
        assert_eq!(app.schedules[0].next_at, Some(START + 60));

        app.fire_due(START + 30);
        assert_eq!(fires(&app, &s).len(), 1, "not due again yet");
    }

    #[test]
    fn a_question_during_a_fire_is_skipped_and_the_agent_carries_on() {
        let db = TempDb::new();
        let ask = call_stream("ask_user_question", json!({"question": "Which log?"}));
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (200, EVENTS, ask),
                (200, EVENTS, text_stream("Used the main log.")),
            ],
        );
        app.roster.current_mut().unwrap().tools = vec![crate::tools::ASK.into()];
        let s = every_minute(&mut app, Target::Prompt("Check the logs".into()));

        app.fire_due(START);
        let _ = seen.recv().unwrap();
        let second = seen.recv().unwrap();
        assert!(second.contains("skipped this question"), "{second}");
        let f = &fires(&app, &s)[0];
        assert_eq!(f.status, FireStatus::Succeeded);
        assert!(!app.awaiting_in(f.session_id.as_deref().unwrap()));
    }

    #[test]
    fn missed_times_fire_once_and_the_next_is_counted_from_now() {
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![(200, EVENTS, text_stream("ok"))]);
        let s = every_minute(&mut app, Target::Prompt("Check".into()));
        let later = START + 10 * 60 + 5;
        app.fire_due(later);
        assert_eq!(fires(&app, &s).len(), 1);
        assert_eq!(app.schedules[0].next_at, Some(START + 11 * 60));
    }

    #[test]
    fn a_fire_due_while_the_last_one_runs_is_skipped() {
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![]);
        let s = every_minute(&mut app, Target::Prompt("Check".into()));
        let chats = app.current().unwrap().sessions.len();
        let busy = Fire {
            id: "f0".into(),
            schedule_id: s.id.clone(),
            due_at: 0,
            started_at: 0,
            finished_at: None,
            status: FireStatus::Running,
            note: String::new(),
            session_id: None,
            workflow_run_id: None,
        };
        app.firing.insert(s.id.clone(), busy);

        app.fire_due(START);
        let f = &fires(&app, &s)[0];
        assert_eq!(
            (f.status, f.session_id.as_deref()),
            (FireStatus::Skipped, None)
        );
        assert!(f.note.contains("still running"));
        assert_eq!(
            app.current().unwrap().sessions.len(),
            chats,
            "no chat opened"
        );
        assert_eq!(app.schedules[0].next_at, Some(START + 60));
    }

    #[test]
    fn stopping_a_fire_while_its_command_runs_ends_it_so_the_next_can_fire() {
        use crate::model::ToolCall;
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![]);
        let s = every_minute(&mut app, Target::Prompt("Wait".into()));
        // The fire's chat, with the agent's command running.
        let sid = app.current().unwrap().chat().id.clone();
        let mut asked = Message::reply("", None, Outcome::Done);
        if let Message::Assistant { tools, .. } = &mut asked {
            tools.push(ToolCall {
                name: crate::tools::SHELL.into(),
                id: "c1".into(),
                args: r#"{"command":"sleep 60"}"#.into(),
                ..Default::default()
            });
        }
        app.append(&sid, asked);
        let fire = Fire {
            id: "f1".into(),
            schedule_id: s.id.clone(),
            due_at: START,
            started_at: START,
            finished_at: None,
            status: FireStatus::Running,
            note: String::new(),
            session_id: Some(sid.clone()),
            workflow_run_id: None,
        };
        app.save("f", |db| db.insert_fire(&fire));
        app.firing.insert(s.id.clone(), fire);
        app.commands
            .insert(sid.clone(), Arc::new(AtomicBool::new(true)));

        app.command_done(
            &sid,
            "c1",
            crate::command::Finish::Stopped("stopped".into()),
        );
        let f = &fires(&app, &s)[0];
        assert_eq!(
            (f.status, f.note.as_str()),
            (FireStatus::Failed, "Stopped."),
            "{f:?}"
        );
        assert!(app.firing.is_empty(), "the schedule is free to fire again");
    }

    #[test]
    fn an_agent_that_cannot_answer_fails_the_fire_at_once_and_says_why_in_its_chat() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        app.roster.current_mut().unwrap().model = None;
        app.providers.default_model = None;
        let s = every_minute(&mut app, Target::Prompt("Check".into()));
        app.fire_due(START);
        let f = &fires(&app, &s)[0];
        assert_eq!(f.status, FireStatus::Failed);
        assert!(f.note.contains("no model"), "{}", f.note);
        assert!(matches!(
            chat_of(&app, f).messages.last(),
            Some(Message::Assistant {
                outcome: Outcome::Failed(_),
                ..
            })
        ));
        assert!(app.firing.is_empty());
    }

    #[test]
    fn a_once_schedule_turns_off_after_it_fires() {
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![(200, EVENTS, text_stream("ok"))]);
        let mut s = every_minute(&mut app, Target::Prompt("Check".into()));
        s.timing = Timing::Once(START);
        app.schedules[0] = s.clone();
        app.fire_due(START);
        assert_eq!(
            (app.schedules[0].enabled, app.schedules[0].next_at),
            (false, None)
        );
        drop(app);
        let app = db.open();
        assert_eq!(
            (app.schedules[0].enabled, app.schedules[0].next_at),
            (false, None)
        );
    }

    #[test]
    fn nothing_fires_while_schedules_are_turned_off() {
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![]);
        let s = every_minute(&mut app, Target::Prompt("Check".into()));
        app.run_schedules = false;
        app.fire_due(START + 3600);
        assert!(fires(&app, &s).is_empty());
        assert_eq!(app.schedules[0].next_at, Some(START));
    }

    #[cfg(unix)]
    #[test]
    fn a_workflow_fire_runs_it_links_the_run_and_says_how_it_went() {
        use crate::workflow::Saved;
        use crate::workflow::definition::{Definition, Step};
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![]);
        app.roster.current_mut().unwrap().tools = vec![crate::tools::SHELL.into()];
        let wf = Saved {
            id: new_id(),
            agent_id: app.current().unwrap().id.clone(),
            name: "Echo".into(),
            goal: String::new(),
            definition: Definition {
                inputs: serde_json::from_value(json!({"word": {"type": "string"}})).unwrap(),
                steps: vec![Step {
                    id: "say".into(),
                    kind: "tool".into(),
                    tool: Some("shell".into()),
                    args: json!({"command": "echo {{inputs.word}}"})
                        .as_object()
                        .cloned(),
                    bind: Some("out".into()),
                    ..Step::default()
                }],
                success: String::new(),
            },
            created_at: now(),
        };
        app.save("w", |db| db.insert_workflow(&wf));
        app.workflows.push(wf.clone());
        let inputs = json!({"word": "pong"}).as_object().unwrap().clone();
        let s = every_minute(
            &mut app,
            Target::Workflow {
                id: wf.id.clone(),
                inputs,
            },
        );

        app.fire_due(START);
        let f = &fires(&app, &s)[0];
        assert_eq!(f.status, FireStatus::Succeeded, "{f:?}");
        let runs = app
            .store
            .as_ref()
            .unwrap()
            .list_workflow_runs(&wf.id, 5)
            .unwrap();
        assert_eq!(f.workflow_run_id.as_deref(), Some(runs[0].id.as_str()));
        assert_eq!(runs[0].started_by, "schedule: Check");
        let Some(Message::Assistant { text, .. }) = chat_of(&app, f).messages.last() else {
            panic!("a reply");
        };
        assert!(text.contains("succeeded"), "{text}");
        assert!(app.wf_jobs.is_empty() && app.firing.is_empty());
    }
    // ---- the tool

    /// The open agent's chat and id, and another agent's id.
    fn two_agents(app: &App) -> (String, String, String) {
        let me = app.current().unwrap();
        let other = app.roster.list.iter().find(|a| a.id != me.id).unwrap();
        (me.chat().id.clone(), me.id.clone(), other.id.clone())
    }

    fn call(app: &mut App, sid: &str, args: Value) -> String {
        app.schedule_call(sid, "c1", &args.to_string())
            .unwrap_or_else(|e| format!("Error: {e}"))
    }

    #[test]
    fn an_agent_schedules_itself_and_nothing_runs_until_you_approve() {
        let db = TempDb::new();
        let create = call_stream(
            "schedule",
            json!({"action": "create", "name": "Standup", "prompt": "Plan my day", "at": "09:00", "days": ["mon", "tue", "wed", "thu", "fri"]}),
        );
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (200, EVENTS, create),
                (200, EVENTS, text_stream("It waits for you.")),
            ],
        );
        app.roster.current_mut().unwrap().tools = vec![tool::NAME.into()];
        app.composer.edit = rune::TextEdit::new("remind me every weekday at 9");
        app.send();
        assert!(
            seen.recv().unwrap().contains("\"schedule\""),
            "offered the tool"
        );
        let told = seen.recv().unwrap();
        assert!(
            told.contains("draft of") && told.contains("weekdays at 09:00"),
            "{told}"
        );
        assert!(app.schedules.is_empty(), "nothing saved yet");
        assert_eq!(app.proposals.len(), 1);
        let sid = app.current().unwrap().chat().id.clone();
        assert_eq!(app.proposals[0].session_id, sid);

        drop(app);
        let mut app = db.open();
        assert_eq!(app.proposals.len(), 1, "the draft waits across a restart");
        app.approve_proposal("c1").unwrap();
        assert!(app.proposals.is_empty());
        let s = &app.schedules[0];
        assert_eq!((s.name.as_str(), s.enabled), ("Standup", true));
        assert!(s.next_at.unwrap() > now());
        drop(app);
        let app = db.open();
        assert_eq!(app.schedules.len(), 1);
        assert!(app.proposals.is_empty());
    }

    #[test]
    fn a_rejected_draft_leaves_nothing_behind() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        let (sid, _, _) = two_agents(&app);
        call(
            &mut app,
            &sid,
            json!({"action": "create", "name": "n", "prompt": "x", "every_minutes": 30}),
        );
        app.reject_proposal("c1");
        assert!(app.proposals.is_empty() && app.schedules.is_empty());
        drop(app);
        assert!(db.open().proposals.is_empty());
    }

    #[test]
    fn an_agent_sees_and_changes_only_its_own_schedules() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        let (sid, _, other) = two_agents(&app);
        let theirs = Schedule::new(
            &other,
            "Theirs",
            Target::Prompt("x".into()),
            Timing::every(0, 60).unwrap(),
            now(),
            &Local,
        )
        .unwrap();
        app.save("s", |db| db.put_schedule(&theirs));
        app.schedules.push(theirs.clone());

        assert_eq!(
            call(&mut app, &sid, json!({"action": "list"})),
            "You have no schedules."
        );
        for action in ["pause", "resume", "delete", "update"] {
            let told = call(
                &mut app,
                &sid,
                json!({"action": action, "id": theirs.id, "name": "Mine now"}),
            );
            assert!(
                told.starts_with("Error: you have no schedule"),
                "{action}: {told}"
            );
        }
        assert_eq!(app.schedules, vec![theirs], "untouched");
        assert!(app.proposals.is_empty());

        let wf = crate::workflow::Saved {
            id: "w-other".into(),
            agent_id: other.clone(),
            name: "Their flow".into(),
            goal: String::new(),
            definition: Default::default(),
            created_at: 0,
        };
        app.workflows.push(wf);
        let told = call(
            &mut app,
            &sid,
            json!({"action": "create", "name": "n", "workflow": "Their flow", "at": "08:00"}),
        );
        assert!(told.contains("no saved workflow"), "{told}");
    }

    #[test]
    fn pause_resume_and_delete_apply_at_once() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        let (sid, me, _) = two_agents(&app);
        let mine = Schedule::new(
            &me,
            "Mine",
            Target::Prompt("x".into()),
            Timing::every(0, 3600).unwrap(),
            now(),
            &Local,
        )
        .unwrap();
        app.save("s", |db| db.put_schedule(&mine));
        app.schedules.push(mine.clone());

        let listed = call(&mut app, &sid, json!({"action": "list"}));
        assert!(
            listed.contains(&mine.id)
                && listed.contains("every hour")
                && listed.contains("sends \"x\""),
            "{listed}"
        );
        assert!(
            call(&mut app, &sid, json!({"action": "pause", "id": mine.id})).starts_with("Paused")
        );
        assert!(!app.schedules[0].enabled);
        assert!(call(&mut app, &sid, json!({"action": "list"})).contains("paused"));
        assert!(call(&mut app, &sid, json!({"action": "resume", "id": mine.id})).contains("Next:"));
        assert!(app.schedules[0].enabled && app.schedules[0].next_at.unwrap() > now());
        assert!(
            call(&mut app, &sid, json!({"action": "delete", "id": mine.id})).starts_with("Deleted")
        );
        assert!(app.schedules.is_empty());
        drop(app);
        assert!(db.open().schedules.is_empty());
    }

    #[test]
    fn an_update_is_a_draft_that_changes_the_schedule_once_approved() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        let (sid, me, _) = two_agents(&app);
        let mine = Schedule::new(
            &me,
            "Mine",
            Target::Prompt("x".into()),
            Timing::every(0, 3600).unwrap(),
            now(),
            &Local,
        )
        .unwrap();
        app.save("s", |db| db.put_schedule(&mine));
        app.schedules.push(mine.clone());
        app.set_enabled(&mine.id, false, now()).unwrap();

        let told = call(
            &mut app,
            &sid,
            json!({"action": "update", "id": mine.id, "cron": "30 7 * * *"}),
        );
        assert!(told.contains("draft of \"Mine\""), "{told}");
        assert_eq!(
            app.schedules[0].timing, mine.timing,
            "unchanged until approved"
        );
        app.approve_proposal("c1").unwrap();
        let s = &app.schedules[0];
        assert_eq!(app.schedules.len(), 1);
        assert_eq!(
            (s.name.as_str(), s.timing.describe(&Utc).as_str()),
            ("Mine", "every day at 07:30")
        );
        assert!(!s.enabled, "still paused, as you left it");
        assert!(
            matches!(&s.target, Target::Prompt(p) if p == "x"),
            "target kept"
        );
    }

    #[test]
    fn approving_a_draft_whose_schedule_was_deleted_says_so_and_drops_it() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        let (sid, me, _) = two_agents(&app);
        let mine = Schedule::new(
            &me,
            "Mine",
            Target::Prompt("x".into()),
            Timing::every(0, 3600).unwrap(),
            now(),
            &Local,
        )
        .unwrap();
        app.save("s", |db| db.put_schedule(&mine));
        app.schedules.push(mine.clone());
        call(
            &mut app,
            &sid,
            json!({"action": "update", "id": mine.id, "name": "Renamed"}),
        );
        app.schedules.clear();
        assert!(app.approve_proposal("c1").unwrap_err().contains("deleted"));
        assert!(app.proposals.is_empty());
    }
    // ---- the page

    #[test]
    fn a_schedule_made_in_the_form_is_saved_and_opened_and_a_bad_one_says_why() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        app.go(crate::app::Route::Schedules);
        app.new_schedule();
        app.save_schedule_form();
        assert_eq!(
            app.schedule_page.form.as_ref().unwrap().notice.as_deref(),
            Some("Give it a name.")
        );
        let f = app.schedule_page.form_mut();
        f.name = rune::TextEdit::new("Morning");
        f.prompt = rune::TextEdit::new("Plan my day");
        app.save_schedule_form();
        assert!(app.schedule_page.form.is_none());
        let s = &app.schedules[0];
        assert_eq!(
            (s.name.as_str(), s.timing.describe(&Local).as_str()),
            ("Morning", "every day at 09:00")
        );
        assert_eq!(app.schedule_page.open.as_deref(), Some(s.id.as_str()));
        drop(app);
        assert_eq!(db.open().schedules.len(), 1);
    }

    #[test]
    fn editing_keeps_the_schedule_and_changes_what_was_typed() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        let s = every_minute(&mut app, Target::Prompt("Check".into()));
        app.select_schedule(&s.id);
        app.edit_schedule();
        let f = app.schedule_page.form_mut();
        assert_eq!(f.every.text, "1");
        f.every = rune::TextEdit::new("30");
        app.save_schedule_form();
        let after = &app.schedules[0];
        assert_eq!(
            (after.id.as_str(), after.created_at),
            (s.id.as_str(), s.created_at)
        );
        assert_eq!(
            after.timing,
            Timing::every(START, 1800).unwrap(),
            "same grid, new gap"
        );
        assert_eq!(app.schedules.len(), 1);
    }

    #[test]
    fn a_draft_changed_in_the_form_is_approved_by_saving_it() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        let (sid, _, _) = two_agents(&app);
        call(
            &mut app,
            &sid,
            json!({"action": "create", "name": "n", "prompt": "x", "every_minutes": 30}),
        );
        app.edit_proposal("c1");
        assert_eq!(app.route, crate::app::Route::Schedules);
        app.schedule_page.form_mut().name = rune::TextEdit::new("Renamed by you");
        app.save_schedule_form();
        assert!(app.proposals.is_empty());
        assert_eq!(app.schedules[0].name, "Renamed by you");
    }

    #[test]
    fn run_now_fires_without_moving_the_next_time_and_not_twice() {
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![(200, EVENTS, text_stream("ok"))]);
        let s = every_minute(&mut app, Target::Prompt("Check".into()));
        app.select_schedule(&s.id);
        app.run_open_schedule();
        assert_eq!(app.schedule_page.fires.len(), 1);
        assert_eq!(app.schedules[0].next_at, s.next_at);

        app.firing
            .insert(s.id.clone(), app.schedule_page.fires[0].clone());
        app.run_open_schedule();
        assert!(
            app.schedule_page
                .notice
                .as_deref()
                .unwrap()
                .contains("still running")
        );
    }

    #[test]
    fn deleting_asks_first_and_the_fire_chats_stay() {
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![(200, EVENTS, text_stream("ok"))]);
        let s = every_minute(&mut app, Target::Prompt("Check".into()));
        app.fire_due(START);
        let chat = fires(&app, &s)[0].session_id.clone().unwrap();
        app.select_schedule(&s.id);
        app.delete_open_schedule();
        assert_eq!(app.schedules.len(), 1, "asked first");
        app.delete_open_schedule();
        assert!(app.schedules.is_empty() && app.schedule_page.open.is_none());
        assert!(app.roster.owner_of(&chat).is_some());
    }

    #[test]
    fn turning_schedules_off_is_remembered() {
        let db = TempDb::new();
        let mut app = db.open();
        assert!(app.run_schedules, "on by default");
        app.set_run_schedules(false);
        drop(app);
        assert!(!db.open().run_schedules);
    }

    #[test]
    fn a_fire_opens_its_chat() {
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![(200, EVENTS, text_stream("ok"))]);
        let s = every_minute(&mut app, Target::Prompt("Check".into()));
        app.fire_due(START);
        let chat = fires(&app, &s)[0].session_id.clone().unwrap();
        app.go(crate::app::Route::Schedules);
        app.open_chat(&chat);
        assert_eq!(app.route, crate::app::Route::Chat);
        assert_eq!(app.current().unwrap().chat().id, chat);
    }
    #[test]
    fn at_launch_only_the_newest_fires_stay_and_old_chats_go_unless_you_wrote_in_them() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        let s = every_minute(&mut app, Target::Prompt("Check".into()));
        let agent = app.current().unwrap().id.clone();
        let open = app.current().unwrap().chat().id.clone();
        let total = KEEP_FIRES + 5;
        let mut chats = Vec::new();
        for i in 0..total {
            let mut c = chat(&format!("Check · {i}"), vec![Message::User("Check".into())]);
            if i == 0 {
                c.messages.push(Message::reply("ok", None, Outcome::Done));
                c.messages
                    .push(Message::User("thanks, now also check disk".into()));
            }
            let a = app.roster.agent_mut(&agent).unwrap();
            a.sessions.push(c.clone());
            let position = a.sessions.len() - 1;
            app.save("c", |db| db.save_session(&agent, &c, position));
            for (seq, m) in c.messages.iter().enumerate() {
                app.save("m", |db| db.put_message(&c.id, seq, m));
            }
            let fire = Fire {
                id: format!("f{i}"),
                schedule_id: s.id.clone(),
                due_at: i as i64,
                started_at: i as i64,
                finished_at: Some(i as i64),
                status: FireStatus::Succeeded,
                note: String::new(),
                session_id: Some(c.id.clone()),
                workflow_run_id: None,
            };
            app.save("f", |db| db.insert_fire(&fire));
            chats.push(c.id);
        }
        drop(app);

        let app = db.open();
        assert_eq!(fires(&app, &s).len(), KEEP_FIRES);
        let has = |sid: &str| app.roster.owner_of(sid).is_some();
        assert!(has(&chats[0]), "you wrote in the oldest: it stays");
        assert!(chats[1..5].iter().all(|c| !has(c)), "the other old ones go");
        assert!(chats[5..].iter().all(|c| has(c)), "the newest stay");
        assert_eq!(
            app.current().unwrap().chat().id,
            open,
            "the open chat is still open"
        );
        drop(app);
        let app = db.open();
        assert!(
            app.roster.owner_of(&chats[1]).is_none(),
            "gone from the file too"
        );
        assert!(app.roster.owner_of(&chats[0]).is_some());
    }
}
