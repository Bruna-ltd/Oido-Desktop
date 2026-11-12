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
use crate::completion::{self, Call, Event, Request, Tokens, ToolSpec, Turn};
use crate::flows::Prep;
use crate::guard::TripState;
use crate::guard::ask::Where;
use crate::model::{AgentStatus, Message, ToolCall, new_id};
use crate::models::{self, ModelRef};
use crate::runs::{Reply, Run, Setup, Status, Step, Trace};
use crate::tools::{self, DELEGATE};
use crate::window;

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
pub fn tools_for(agent: &Agent, typesafe: bool, mcp: &[ToolSpec]) -> Vec<ToolSpec> {
    let mut all = tools::offered(&agent.tools, typesafe, mcp);
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
        name: DELEGATE.into(),
        description: "Hand a task to one of your team members and get its answer. It starts with nothing but the task, so say everything it needs, and what you want back. It works on its own and cannot ask the person anything. To run several at once, call this several times in one turn.".into(),
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
    /// The member's run as it began: its steps are recorded against it.
    run: Run,
    brief: String,
    skills: Vec<(String, String)>,
    system: Option<String>,
    context: String,
    think: Option<completion::Think>,
    tools: Vec<ToolSpec>,
    task: String,
    /// Outside text its lead had in view when it handed the task over: the task may carry it.
    outside: Option<String>,
    /// Whether what it reads and runs in its folder counts as the person's own.
    trust_folder: bool,
    /// The model's context limit, when known.
    limit: Option<u32>,
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
    /// Outside text it had in view when it wrote its answer, which the answer may carry.
    outside: Option<String>,
    steps: Vec<Step>,
    /// What all its model calls used together.
    tokens: Tokens,
    ms: u32,
}

/// Tools that work without a person there: commands and files.
fn unattended(name: &str) -> bool {
    name == tools::SHELL || name == crate::web::NAME || crate::files::is_file_tool(name)
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
    if call.name == crate::web::NAME {
        return crate::web::fetch(&call.args);
    }
    if crate::files::is_file_tool(&call.name) {
        return match &job.prep.workdir {
            Some(root) => crate::files::run(root, &call.name, &call.args),
            None => "Error: this agent has no working folder, so there is nowhere to read or write files.".to_owned(),
        };
    }
    format!("Error: there is no tool called {} for you.", call.name)
}

/// Works on the task until the model answers without asking for a tool. Blocks.
fn work(job: &Job, tell: &dyn Fn(Note)) -> Outcome {
    let began = Instant::now();
    let mut trace = Trace::new(job.run.clone());
    let mut steps = trace.setup(&Setup {
        brief: &job.brief,
        skills: &job.skills,
        context: &job.context,
        tools: &job.tools,
        user: &job.task,
        user_name: None,
    });
    let mut turns = vec![Turn::user(job.task.clone())];
    let key = job.prep.key.as_deref().map(String::as_str);
    let guard = &job.prep.guard;
    // Its secrets, numbered as first seen, hidden from its model and put back for its tools.
    let mut hidden = crate::guard::Hidden::default();
    let place = crate::guard::Place {
        root: job.prep.shell_dir.clone(),
        home: dirs::home_dir(),
    };
    // Its calls whose results are outside text, and what each is; and what it has in view now.
    let mut came_in: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut in_view = job.outside.clone();
    // The last call's count, and whether the next goes again after a too-long refusal.
    let (mut measured, mut retry) = (None, false);
    let end = 'rounds: {
        for _ in 0..MAX_ROUNDS {
            if job.cancel.load(Ordering::Relaxed) {
                break 'rounds End::Stopped;
            }
            let mut request = Request {
                model: job.prep.model.clone(),
                system: job.system.clone(),
                context: Some(job.context.clone()),
                think: job.think,
                turns: turns.clone(),
                tools: job.tools.clone(),
            };
            let trimmed = window::fit(&mut request, job.limit, measured, retry).trimmed;
            in_view = job.outside.clone().or_else(|| {
                request.turns.iter().find_map(|t| {
                    let id = t
                        .call_id
                        .as_deref()
                        .filter(|_| !window::was_trimmed(&t.text))?;
                    came_in.get(id).cloned()
                })
            });
            let request = guard.request(request, &mut hidden);
            tell(Note::Model);
            let round = Instant::now();
            let (mut text, mut reasoning, mut calls) = (String::new(), String::new(), Vec::new());
            let (mut used, mut first) = (Tokens::default(), None);
            let (mut shown, mut last) = (String::new(), Instant::now());
            let result =
                completion::run(&job.prep.endpoint, key, &request, &job.cancel, &mut |ev| {
                    if first.is_none() && !matches!(ev, Event::Usage(_)) {
                        first = Some(round.elapsed());
                    }
                    match ev {
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
                        Event::Usage(t) => used.update(t),
                    }
                });
            if !shown.is_empty() {
                tell(Note::Text(shown));
            }
            tell(Note::ModelDone {
                input: used.input.unwrap_or(0),
                output: used.output.unwrap_or(0),
            });

            // Refused as too long: once, it goes again with all it can trim trimmed.
            let again =
                result == Err(completion::Error::TooLong) && !retry && window::trimmable(&turns);
            let (status, failure) = match &result {
                Ok(()) => (Status::Ok, None),
                Err(completion::Error::Cancelled) => (Status::Cancelled, None),
                Err(_) if again => (
                    Status::Error,
                    Some("Too long for the model. Sent again with the older tool results and pictures trimmed.".to_owned()),
                ),
                Err(e) => (Status::Error, Some(e.describe(&job.prep.who))),
            };
            let called: Vec<ToolCall> = calls
                .iter()
                .map(|c: &Call| ToolCall {
                    name: c.name.clone(),
                    summary: String::new(),
                    output: None,
                    id: c.id.clone(),
                    args: c.args.clone(),
                    trip: None,
                    outside: None,
                })
                .collect();
            steps.extend(trace.reply(&Reply {
                sent: &crate::run::transcript(&request.request().turns),
                text: &text,
                reasoning: &reasoning,
                calls: &called,
                failure: failure.as_deref(),
                status,
                tokens: used,
                began: round,
                first,
            }));
            if again {
                retry = true;
                continue;
            }
            retry = false;
            match (status, failure) {
                (Status::Cancelled, _) => break 'rounds End::Stopped,
                (_, Some(why)) => break 'rounds End::Failed(why),
                _ => {}
            }
            measured = used.input.map(|input| window::Measured {
                context: input.saturating_add(used.output.unwrap_or(0)),
                trimmed,
            });
            // What it wrote, with the hidden values back: it stays on this computer.
            let text = hidden.reveal(&text);
            if calls.is_empty() {
                break 'rounds if text.trim().is_empty() {
                    End::Failed(format!("{} sent back an empty reply.", job.prep.who))
                } else {
                    End::Done(text)
                };
            }
            // Each call checked before it runs; nobody is here to give a go-ahead, so a call a
            // rule questions does not run.
            let reviewed: Vec<(Call, Option<crate::guard::actions::Finding>)> = calls
                .into_iter()
                .map(|c| {
                    let (args, found) =
                        guard.review(&hidden, &c.name, &c.args, &place, in_view.as_deref());
                    (Call { args, ..c }, found)
                })
                .collect();
            let calls: Vec<Call> = reviewed.iter().map(|(c, _)| c.clone()).collect();
            turns.push(Turn::assistant(text, calls));
            for (call, found) in &reviewed {
                tell(Note::Tool {
                    name: call.name.clone(),
                    detail: tools::summary(&call.name, &call.args),
                });
                // A call a rule questions waits for the person on a card in the lead's chat.
                let refused = found.as_ref().and_then(|f| {
                    let (state, told) = crate::guard::unattended(
                        &job.prep.asker,
                        &call.name,
                        &call.args,
                        f,
                        &job.cancel,
                    );
                    let ended = match state {
                        TripState::Allowed => "you allowed it",
                        TripState::TimedOut => "nobody answered in time",
                        TripState::Denied => "you said no",
                        _ => "refused",
                    };
                    steps.push(
                        trace.guard(&format!("{}: {ended}", crate::guard::title(f.rule)), &f.why),
                    );
                    told
                });
                let ran = refused.is_none();
                let result = match refused {
                    Some(told) => told,
                    None => {
                        let result = run_tool(job, call);
                        let source =
                            crate::guard::taint::result_source(&call.name, job.trust_folder);
                        let what = if guard.on(crate::guard::Pack::Injection) {
                            crate::guard::injection::describe(source, &result)
                        } else {
                            source.map(str::to_owned)
                        };
                        if let Some(what) = what {
                            guard.learn_outside(&mut hidden, &result);
                            came_in.insert(call.id.clone(), what);
                        }
                        result
                    }
                };
                tell(Note::ToolDone {
                    ok: !result.starts_with("Error") && ran,
                });
                steps.extend(trace.tool_done(&call.id, &result));
                turns.push(Turn::tool(call.id.clone(), result));
            }
        }
        End::Failed(format!(
            "It was still working after {MAX_ROUNDS} model calls, so it was cut off."
        ))
    };
    Outcome {
        end,
        outside: in_view,
        steps,
        tokens: Tokens {
            input: trace.run.input_tokens,
            output: trace.run.output_tokens,
            cache_read: trace.run.cache_read_tokens,
            cache_write: trace.run.cache_write_tokens,
        },
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
        let mut run = Run::new(&new_id(), &member.id, &member.name, sid, model);
        run.parent = parent;
        run.title = crate::runs::title_of(&task);
        let (brief, skills) = self.brief_of(member);
        let job = Job {
            run: run.clone(),
            system: Some(crate::skills::system(&brief, &skills)).filter(|p| !p.trim().is_empty()),
            brief,
            skills,
            context: crate::context::now_text(&prep.shell_dir),
            think: crate::run::think_of(self.effort).filter(|_| can_think),
            tools: tools::offered(&member.tools, false, &[])
                .into_iter()
                .filter(|t| unattended(&t.name))
                .collect(),
            limit: self.context_limit(&ModelRef::new(&prep.provider, &prep.model)),
            prep: crate::flows::Prep {
                asker: self.asker(Where::Chat(sid.to_owned()), &member.name),
                ..prep
            },
            task,
            outside: self.tainted.get(sid).cloned(),
            trust_folder: member.trust_workdir,
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
        let t = out.tokens;
        (run.input_tokens, run.output_tokens) = (t.input, t.output);
        (run.cache_read_tokens, run.cache_write_tokens) = (t.cache_read, t.cache_write);
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
            (d.input, d.output) = (t.input.unwrap_or(0), t.output.unwrap_or(0));
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
        self.save_run(&run, &out.steps);
        if self.route == crate::app::Route::Runs {
            self.reload_runs();
        }
        self.release_delegation(sid);
        // Its answer carries what outside text it had in view.
        if let Some(what) = &out.outside {
            self.mark_outside(
                sid,
                call_id,
                format!("{name}'s answer, written with {what} in view"),
            );
        }
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
