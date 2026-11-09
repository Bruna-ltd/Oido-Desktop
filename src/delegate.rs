//! Handing a task to a team member. An agent with a team is given the `delegate` tool: it names a
//! team member and a task, and gets that member's final answer back as the tool's result.
//!
//! The member starts fresh, with its own brief, model, tools and working folder and only the task
//! to go on, and works unattended on a worker thread, so it cannot ask you anything, save a
//! workflow or hand work on. Several tasks handed over in one turn run side by side. Each is
//! recorded as a run of its own, linked to the run that handed it over.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::agents::Agent;
use crate::app::{App, now};
use crate::command::{self, Finish};
use crate::completion::{self, Call, Event, Request, ToolSpec, Turn};
use crate::flows::Prep;
use crate::model::{AgentStatus, Message, new_id};
use crate::models::{self, ModelRef};
use crate::runs::{INPUT_LIMIT, OUTPUT_LIMIT, Run, Status, Step, StepKind, clip};
use crate::tools::{self, DELEGATE};

/// The most times a team member may call the model for one task before it is cut off.
const MAX_ROUNDS: usize = 20;

/// How often streamed text reaches the screen.
const FLUSH: Duration = Duration::from_millis(40);

/// Tasks a chat's agent handed over that have not come back.
pub struct Delegating {
    pending: usize,
    /// The person pressed Stop on the chat: the lead waits for them instead of carrying on.
    halted: bool,
}

/// One handed-over task as the chat shows it while it works, and after: kept until the app closes.
pub struct Delegated {
    pub sid: String,
    pub member_id: String,
    pub began: Instant,
    pub ended: Option<Instant>,
    /// What the member is writing in its current model call.
    pub text: String,
    pub stages: Vec<Stage>,
    pub input: u32,
    pub output: u32,
    /// Stops this one.
    pub cancel: Arc<AtomicBool>,
}

/// One thing the member did: a model call or a tool.
pub struct Stage {
    pub label: String,
    pub detail: String,
    pub began: Instant,
    pub ended: Option<Instant>,
    pub ok: bool,
}

/// What a worker tells the screen as it goes.
enum Note {
    /// A model call starts.
    Model,
    Text(String),
    /// The model call ended, with what it used.
    ModelDone {
        input: u32,
        output: u32,
    },
    Tool {
        name: String,
        detail: String,
    },
    ToolDone {
        ok: bool,
    },
}

fn notifier(sid: String, id: String) -> impl Fn(Note) {
    move |note| {
        if let Some(handle) = crate::app::handle() {
            let (sid, id) = (sid.clone(), id.clone());
            handle.update(move |app, _| app.delegation_note(&sid, &id, note));
        }
    }
}

/// The tools a lead agent is given: the ones it chose, and `delegate` while it has a team.
pub fn tools_for(agent: &Agent, typesafe: bool) -> Vec<ToolSpec> {
    let mut all = tools::offered(&agent.tools, typesafe);
    if !agent.subagents.is_empty() {
        all.push(spec(&agent.subagents));
    }
    all
}

fn spec(team: &[Agent]) -> ToolSpec {
    let roster: Vec<String> = team
        .iter()
        .map(|m| match m.description.trim() {
            "" => format!("- {}", m.name),
            d => format!("- {}: {d}", m.name),
        })
        .collect();
    ToolSpec {
        name: DELEGATE,
        description: "Hand a task to one of your team members and get its answer. It starts with nothing but the task, so say everything it needs, and what you want back. It works on its own and cannot ask the person anything. To run several at once, call this several times in one turn.",
        schema: json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "enum": team.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
                    "description": format!("Who to hand it to:\n{}", roster.join("\n")),
                },
                "task": {"type": "string", "description": "The task, complete in itself."}
            },
            "required": ["agent", "task"]
        }),
    }
}

/// Who a call is for and what it asks of them. A call that is not that shape gets empty strings.
fn parse(args: &str) -> (String, String) {
    let v: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let get = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    (get("agent"), get("task"))
}

/// A line saying what a call is, for lists: who, and the first line of the task.
pub fn summary(args: &str) -> String {
    let (agent, task) = parse(args);
    if agent.is_empty() {
        return args.to_owned();
    }
    format!("{agent}: {}", task.lines().next().unwrap_or_default())
}

/// Everything a team member's worker needs, worked out before leaving the UI thread.
struct Job {
    prep: Prep,
    system: Option<String>,
    context: String,
    think: Option<completion::Think>,
    tools: Vec<ToolSpec>,
    task: String,
    /// Stops this task.
    cancel: Arc<AtomicBool>,
}

enum End {
    Done(String),
    Failed(String),
    Stopped,
}

struct Outcome {
    end: End,
    steps: Vec<Step>,
    input: u32,
    output: u32,
    ms: u32,
}

/// Tools that work without a person there: commands and files.
fn unattended(name: &str) -> bool {
    name == tools::SHELL || crate::files::is_file_tool(name)
}

fn run_tool(job: &Job, call: &Call) -> String {
    if call.name == tools::SHELL {
        let Some(spec) = command::parse(&call.args) else {
            return "No command was given, so nothing ran.".to_owned();
        };
        if !job.prep.shell_dir.is_dir() {
            return format!(
                "The working folder {} does not exist, so nothing ran.",
                job.prep.shell_dir.display()
            );
        }
        let (Finish::Done(text) | Finish::Stopped(text)) =
            command::run(&spec, &job.prep.shell_dir, &job.cancel);
        return text;
    }
    if crate::files::is_file_tool(&call.name) {
        return match &job.prep.workdir {
            Some(root) => crate::files::run(root, &call.name, &call.args),
            None => "Error: this agent has no working folder, so there is nowhere to read or write files.".to_owned(),
        };
    }
    format!("Error: there is no tool called {} for you.", call.name)
}

/// A step that is just text: the brief, the task, or what the tools gave back.
fn note(steps: &mut Vec<Step>, kind: StepKind, name: &str, text: &str) {
    let mut s = Step::new(steps.len() as u32 + 1, kind, now());
    s.name = name.to_owned();
    let (text, cut) = clip(text, INPUT_LIMIT);
    (s.output, s.truncated) = (Some(text), cut);
    steps.push(s);
}

/// Works on the task until the model answers without asking for a tool. Blocks.
fn work(job: &Job, tell: &dyn Fn(Note)) -> Outcome {
    let began = Instant::now();
    let (mut steps, mut input, mut output) = (Vec::new(), 0u32, 0u32);
    if let Some(system) = &job.system {
        note(&mut steps, StepKind::System, "", system);
    }
    note(&mut steps, StepKind::User, "", &job.task);
    let mut turns = vec![Turn::user(job.task.clone())];
    let key = job.prep.key.as_deref().map(String::as_str);
    let end = 'rounds: {
        for _ in 0..MAX_ROUNDS {
            if job.cancel.load(Ordering::Relaxed) {
                break 'rounds End::Stopped;
            }
            let request = Request {
                model: job.prep.model.clone(),
                system: job.system.clone(),
                context: Some(job.context.clone()),
                think: job.think,
                turns: turns.clone(),
                tools: job.tools.clone(),
            };
            tell(Note::Model);
            let (at, round) = (now(), Instant::now());
            let (mut text, mut reasoning, mut calls) = (String::new(), String::new(), Vec::new());
            let (mut used_in, mut used_out) = (None, None);
            let (mut shown, mut last) = (String::new(), Instant::now());
            let result = completion::run(
                &job.prep.endpoint,
                key,
                &request,
                &job.cancel,
                &mut |ev| match ev {
                    Event::Text(t) => {
                        text.push_str(&t);
                        shown.push_str(&t);
                        if last.elapsed() >= FLUSH {
                            tell(Note::Text(std::mem::take(&mut shown)));
                            last = Instant::now();
                        }
                    }
                    Event::Reasoning(t) => reasoning.push_str(&t),
                    Event::ToolCall(c) => calls.push(c),
                    Event::Usage { input, output } => {
                        used_in = input.or(used_in);
                        used_out = output.or(used_out);
                    }
                },
            );
            if !shown.is_empty() {
                tell(Note::Text(shown));
            }
            input += used_in.unwrap_or(0);
            output += used_out.unwrap_or(0);
            tell(Note::ModelDone {
                input: used_in.unwrap_or(0),
                output: used_out.unwrap_or(0),
            });

            let mut said = text.clone();
            for c in &calls {
                if !said.is_empty() {
                    said.push_str("\n\n");
                }
                said.push_str(&format!("[tool call] {} {}", c.name, c.args));
            }
            let sent = crate::run::transcript(job.system.as_deref(), Some(&job.context), &turns);
            let mut s = Step::new(steps.len() as u32 + 1, StepKind::Llm, at);
            s.status = match &result {
                Ok(()) => Status::Ok,
                Err(completion::Error::Cancelled) => Status::Cancelled,
                Err(_) => Status::Error,
            };
            let ((sent, cut_in), (said, cut_out)) =
                (clip(&sent, INPUT_LIMIT), clip(&said, OUTPUT_LIMIT));
            (s.input, s.output, s.truncated) = (Some(sent), Some(said), cut_in || cut_out);
            s.reasoning = Some(reasoning).filter(|r| !r.is_empty());
            (s.input_tokens, s.output_tokens) = (used_in, used_out);
            s.duration_ms = Some(round.elapsed().as_millis().min(u128::from(u32::MAX)) as u32);
            steps.push(s);

            match result {
                Ok(()) => {}
                Err(completion::Error::Cancelled) => break 'rounds End::Stopped,
                Err(e) => {
                    let why = e.describe(&job.prep.who);
                    let mut s = Step::new(steps.len() as u32 + 1, StepKind::Error, now());
                    s.status = Status::Error;
                    s.output = Some(why.clone());
                    steps.push(s);
                    break 'rounds End::Failed(why);
                }
            }
            if calls.is_empty() {
                break 'rounds if text.trim().is_empty() {
                    End::Failed(format!("{} sent back an empty reply.", job.prep.who))
                } else {
                    End::Done(text)
                };
            }
            turns.push(Turn::assistant(text, calls.clone()));
            let mut results = Vec::new();
            for call in &calls {
                tell(Note::Tool {
                    name: call.name.clone(),
                    detail: tools::summary(&call.name, &call.args),
                });
                let result = run_tool(job, call);
                tell(Note::ToolDone {
                    ok: !result.starts_with("Error"),
                });
                results.push(format!("[{}]\n{result}", call.name));
                turns.push(Turn::tool(call.id.clone(), result));
            }
            note(
                &mut steps,
                StepKind::User,
                "Tool result",
                &results.join("\n\n"),
            );
        }
        End::Failed(format!(
            "It was still working after {MAX_ROUNDS} model calls, so it was cut off."
        ))
    };
    Outcome {
        end,
        steps,
        input,
        output,
        ms: began.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
    }
}

impl App {
    /// Starts every task the agent just handed over, all at once. Each result goes back to the
    /// agent as its call's result; when the last is in, the agent carries on.
    pub(crate) fn start_delegations(&mut self, sid: &str) {
        let Some(Message::Assistant { tools, run, .. }) = self
            .roster
            .owner_of(sid)
            .and_then(|a| a.sessions.iter().find(|s| s.id == sid))
            .and_then(|s| s.messages.last())
        else {
            return;
        };
        let calls: Vec<(String, String)> = tools
            .iter()
            .filter(|t| t.name == DELEGATE && t.output.is_none() && !t.id.is_empty())
            .map(|t| (t.id.clone(), t.args.clone()))
            .collect();
        let parent = run.clone();
        if calls.is_empty() {
            return;
        }
        self.delegating.insert(
            sid.to_owned(),
            Delegating {
                pending: calls.len(),
                halted: false,
            },
        );
        if let Some(lead) = self.roster.owner_of_mut(sid) {
            lead.status = AgentStatus::Running;
        }
        self.beat();
        for (id, args) in calls {
            match self.prepare_delegation(sid, &args, parent.clone()) {
                Ok((run, job, name)) => {
                    self.save("the run", |db| db.start_run(&run));
                    if self.route == crate::app::Route::Runs {
                        self.reload_runs();
                    }
                    self.delegated.insert(
                        id.clone(),
                        Delegated {
                            sid: sid.to_owned(),
                            member_id: run.agent_id.clone(),
                            began: Instant::now(),
                            ended: None,
                            text: String::new(),
                            stages: Vec::new(),
                            input: 0,
                            output: 0,
                            cancel: Arc::clone(&job.cancel),
                        },
                    );
                    if let Some(member) = self.roster.agent_mut(&run.agent_id) {
                        member.status = AgentStatus::Running;
                    }
                    let tell = notifier(sid.to_owned(), id.clone());
                    let (sid, id) = (sid.to_owned(), id);
                    self.spawn(
                        move || work(&job, &tell),
                        move |app, out| app.delegation_done(&sid, &id, &name, run, out),
                    );
                }
                Err(why) => {
                    self.release_delegation(sid);
                    self.resolve(sid, &id, format!("Error: {why}"), true);
                }
            }
        }
    }

    /// The team member a call names, with its run record and what its worker needs, or why it
    /// cannot start.
    fn prepare_delegation(
        &self,
        sid: &str,
        args: &str,
        parent: Option<String>,
    ) -> Result<(Run, Job, String), String> {
        let (who, task) = parse(args);
        let lead = self.roster.owner_of(sid).ok_or("That agent is gone.")?;
        let Some(member) = lead
            .subagents
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(&who))
        else {
            let team: Vec<&str> = lead.subagents.iter().map(|m| m.name.as_str()).collect();
            return Err(if team.is_empty() {
                format!("you have no team member called {who:?}. You have no team.")
            } else {
                format!(
                    "you have no team member called {who:?}. Yours are: {}.",
                    team.join(", ")
                )
            });
        };
        if task.is_empty() {
            return Err(format!("no task was given for {}.", member.name));
        }
        let prep = self.prepare_model(&member.id)?;
        let model = ModelRef::new(&prep.provider, &prep.model);
        let can_think = self
            .providers
            .model_info(&model)
            .is_none_or(|i| models::can_think(&i.caps));
        let run = Run {
            id: new_id(),
            agent_id: member.id.clone(),
            agent_name: member.name.clone(),
            session_id: sid.to_owned(),
            model,
            status: Status::Running,
            error: None,
            input_tokens: None,
            output_tokens: None,
            duration_ms: None,
            started_at: now(),
            ended_at: None,
            parent,
        };
        let job = Job {
            system: self.system_for(member),
            context: crate::context::now_text(),
            think: crate::run::think_of(self.effort).filter(|_| can_think),
            tools: tools::offered(&member.tools, false)
                .into_iter()
                .filter(|t| unattended(t.name))
                .collect(),
            prep,
            task,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        Ok((run, job, member.name.clone()))
    }

    /// A worker reported progress: it goes on the task's card.
    fn delegation_note(&mut self, sid: &str, call_id: &str, note: Note) {
        let Some(d) = self
            .delegated
            .get_mut(call_id)
            .filter(|d| d.sid == sid && d.ended.is_none())
        else {
            return;
        };
        let end_last = |d: &mut Delegated, ok: bool| {
            if let Some(stage) = d.stages.last_mut().filter(|s| s.ended.is_none()) {
                (stage.ended, stage.ok) = (Some(Instant::now()), ok);
            }
        };
        match note {
            Note::Model => {
                d.text.clear();
                d.stages.push(Stage {
                    label: "Thinking".to_owned(),
                    detail: String::new(),
                    began: Instant::now(),
                    ended: None,
                    ok: true,
                });
            }
            Note::Text(t) => d.text.push_str(&t),
            Note::ModelDone { input, output } => {
                (d.input, d.output) = (d.input + input, d.output + output);
                end_last(d, true);
            }
            Note::Tool { name, detail } => d.stages.push(Stage {
                label: name,
                detail,
                began: Instant::now(),
                ended: None,
                ok: true,
            }),
            Note::ToolDone { ok } => end_last(d, ok),
        }
    }

    /// A team member finished: its run is recorded, and its answer goes back to the agent.
    fn delegation_done(
        &mut self,
        sid: &str,
        call_id: &str,
        name: &str,
        mut run: Run,
        out: Outcome,
    ) {
        run.input_tokens = Some(out.input);
        run.output_tokens = Some(out.output);
        run.duration_ms = Some(out.ms);
        run.ended_at = Some(now());
        // Stop on the chat holds the lead for you; stopping one member lets the lead go on without it.
        let halted = self.delegating.get(sid).is_some_and(|d| d.halted);
        let (told, carry_on) = match &out.end {
            End::Done(text) => {
                run.status = Status::Ok;
                (text.clone(), true)
            }
            End::Failed(why) => {
                (run.status, run.error) = (Status::Error, Some(why.clone()));
                (format!("Error: {name} could not finish. {why}"), true)
            }
            End::Stopped => {
                run.status = Status::Cancelled;
                (
                    format!("The person stopped {name} before it finished."),
                    !halted,
                )
            }
        };
        if let Some(d) = self.delegated.get_mut(call_id) {
            d.ended = Some(Instant::now());
            d.text.clear();
            (d.input, d.output) = (out.input, out.output);
            for stage in d.stages.iter_mut().filter(|s| s.ended.is_none()) {
                (stage.ended, stage.ok) = (d.ended, false);
            }
            let member_id = d.member_id.clone();
            let busy = self
                .delegated
                .values()
                .any(|o| o.member_id == member_id && o.ended.is_none());
            if let Some(member) = self.roster.agent_mut(&member_id).filter(|_| !busy) {
                member.status = AgentStatus::Idle;
            }
        }
        self.save("the run", |db| db.finish_run(&run, &out.steps));
        if self.route == crate::app::Route::Runs {
            self.reload_runs();
        }
        self.release_delegation(sid);
        self.resolve(sid, call_id, told, carry_on);
    }

    /// One handed-over task is no longer out; the chat is free again when none are.
    fn release_delegation(&mut self, sid: &str) {
        let Some(d) = self.delegating.get_mut(sid) else {
            return;
        };
        d.pending = d.pending.saturating_sub(1);
        if d.pending > 0 {
            return;
        }
        self.delegating.remove(sid);
        let lead = self.roster.owner_of(sid).map(|a| a.id.clone());
        let working = lead.as_deref().is_some_and(|id| self.writing(id));
        if let Some(lead) = lead
            .and_then(|id| self.roster.agent_mut(&id))
            .filter(|_| !working)
        {
            lead.status = AgentStatus::Idle;
        }
    }

    /// Stops one handed-over task. Its member's answer becomes "stopped", and the agent goes on.
    pub fn stop_delegation(&mut self, call_id: &str) {
        if let Some(d) = self.delegated.get(call_id).filter(|d| d.ended.is_none()) {
            d.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The person stopped the chat: every task it handed over stops, and the agent waits.
    pub(crate) fn halt_delegations(&mut self, sid: &str) {
        if let Some(d) = self.delegating.get_mut(sid) {
            d.halted = true;
        }
        for d in self
            .delegated
            .values()
            .filter(|d| d.sid == sid && d.ended.is_none())
        {
            d.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Redraws about once a second while something is working, so the clocks on screen run.
    pub(crate) fn beat(&self) {
        static BEATING: AtomicBool = AtomicBool::new(false);
        let Some(handle) = crate::app::handle() else {
            return;
        };
        if BEATING.swap(true, Ordering::Relaxed) {
            return;
        }
        let handle = handle.clone();
        std::thread::spawn(move || {
            while BEATING.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(1));
                handle.update(|app, _| {
                    app.tick = app.tick.wrapping_add(1);
                    let working =
                        !app.live.is_empty() || app.delegated.values().any(|d| d.ended.is_none());
                    if !working {
                        BEATING.store(false, Ordering::Relaxed);
                    }
                });
            }
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn card(sid: &str) -> Delegated {
        Delegated {
            sid: sid.into(),
            member_id: "m1".into(),
            began: Instant::now(),
            ended: None,
            text: String::new(),
            stages: Vec::new(),
            input: 0,
            output: 0,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn progress_builds_the_card_step_by_step_and_a_finished_card_ignores_late_news() {
        let mut app = crate::testing::samples();
        app.delegated.insert("c1".into(), card("s1"));
        let note = |app: &mut App, n| app.delegation_note("s1", "c1", n);
        note(&mut app, Note::Model);
        note(&mut app, Note::Text("let me ".into()));
        note(&mut app, Note::Text("look".into()));
        note(
            &mut app,
            Note::ModelDone {
                input: 10,
                output: 3,
            },
        );
        note(
            &mut app,
            Note::Tool {
                name: "shell".into(),
                detail: "ls".into(),
            },
        );
        let d = &app.delegated["c1"];
        assert_eq!(d.text, "let me look");
        assert_eq!((d.input, d.output), (10, 3));
        assert!(d.stages[0].ended.is_some() && d.stages[1].ended.is_none());

        note(&mut app, Note::ToolDone { ok: false });
        note(&mut app, Note::Model);
        let d = &app.delegated["c1"];
        assert!(d.text.is_empty(), "a new model call starts a fresh line");
        assert!(!d.stages[1].ok && d.stages[1].ended.is_some());

        app.delegated.get_mut("c1").unwrap().ended = Some(Instant::now());
        note(&mut app, Note::Text("late".into()));
        assert!(app.delegated["c1"].text.is_empty());
        app.delegation_note("other-chat", "c1", Note::Model);
        assert_eq!(app.delegated["c1"].stages.len(), 3);
    }

    #[test]
    fn stopping_one_task_flags_only_that_task_and_stopping_the_chat_flags_all() {
        let mut app = crate::testing::samples();
        app.delegated.insert("a".into(), card("s1"));
        app.delegated.insert("b".into(), card("s1"));
        app.delegated.insert("c".into(), card("s2"));
        app.stop_delegation("a");
        let flagged = |app: &App, id: &str| app.delegated[id].cancel.load(Ordering::Relaxed);
        assert!(flagged(&app, "a") && !flagged(&app, "b"));
        app.delegating.insert(
            "s1".into(),
            Delegating {
                pending: 2,
                halted: false,
            },
        );
        app.halt_delegations("s1");
        assert!(flagged(&app, "b") && !flagged(&app, "c"));
        assert!(app.delegating["s1"].halted);
    }
}
