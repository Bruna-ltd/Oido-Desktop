//! Workflows in the app: learning one from a chat (a draft you confirm or change), saving it, and
//! running it, either from the Workflows page or when an agent calls `run_workflow`. The steps and
//! the engine are in [`crate::workflow`]; this is where they meet the model, the tools and the
//! screens.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::{Map, Value, json};

use crate::app::{App, Route, now};
use crate::command::{self, Finish};
use crate::completion::{self, Completion, Endpoint, Request, Turn};
use crate::credentials::Credential;
use crate::model::{Message, new_id};
use crate::models::ModelRef;
use crate::providers::ModelStatus;
use crate::store::Counted;
use crate::usage::{Source, Totals};
use crate::workflow::compile::{self, Trace};
use crate::workflow::definition::{Definition, STEP_TOOLS};
use crate::workflow::engine::{self, Ending, Env, Outcome, ToolOutcome, Usage};
use crate::workflow::{Draft, RunRecord, RunStatus, Saved};

/// What is happening to a chat's draft, when something is.
#[derive(Clone, PartialEq, Debug)]
pub enum Teach {
    Working(&'static str),
    Failed(String),
}

/// The request that led to the work in a chat, and the tool calls that did it, since the last
/// divider. Calls still waiting for a result, and the workflow tools themselves, are left out.
pub fn trace_of(messages: &[Message]) -> (String, Vec<Trace>) {
    let start = messages
        .iter()
        .rposition(|m| matches!(m, Message::Divider(_)))
        .map_or(0, |i| i + 1);
    let mut request = Vec::new();
    let mut trace = Vec::new();
    for m in &messages[start..] {
        match m {
            Message::User(p) => request.push(p.text.as_str()),
            Message::Assistant { tools, .. } => {
                for t in tools {
                    let useful =
                        STEP_TOOLS.contains(&t.name.as_str()) || t.name == crate::tools::ASK;
                    if let (true, Some(result), false) = (useful, &t.output, t.id.is_empty()) {
                        trace.push(Trace {
                            tool: t.name.clone(),
                            args: t.args.clone(),
                            result: result.clone(),
                        });
                    }
                }
            }
            Message::Divider(_) | Message::Summary { .. } => {}
        }
    }
    (request.join("\n\n"), trace)
}

/// What a model call or a run needs, worked out before leaving the UI thread.
pub(crate) struct Prep {
    pub endpoint: Endpoint,
    pub credential: Option<Credential>,
    pub model: String,
    pub provider: String,
    /// The provider's name, for saying who failed.
    pub who: String,
    /// The agent's working folder, where file tools work.
    pub workdir: Option<PathBuf>,
    /// Where commands run: that folder, or home.
    pub shell_dir: PathBuf,
    /// The guardrails on when it started.
    pub guard: crate::guard::Guard,
    /// Who a call a rule questions is put to, since nobody watches the work.
    pub asker: crate::guard::ask::Asker,
}

fn ask(p: &Prep, cancel: &AtomicBool, system: &str, message: &str) -> Result<Completion, String> {
    let request = Request {
        model: p.model.clone(),
        system: Some(system.to_owned()),
        context: None,
        turns: vec![Turn::user(message)],
        think: None,
        tools: Vec::new(),
    };
    let mut hidden = crate::guard::Hidden::default();
    let request = p.guard.request(request, &mut hidden);
    completion::complete(&p.endpoint, p.credential.as_ref(), &request, cancel)
        .map(|mut c| {
            // The step's output stays on this computer, so it carries the real values.
            c.text = hidden.reveal(&c.text);
            c
        })
        .map_err(|e| e.describe(&p.who))
}

/// Runs the workflow's steps with real tools and a real model. Blocks.
fn work(def: &Definition, inputs: &Value, p: &Prep, stop: &Arc<AtomicBool>) -> Outcome {
    let cancel: &AtomicBool = stop;
    let place = crate::guard::Place {
        root: p.shell_dir.clone(),
        home: dirs::home_dir(),
    };
    let tool = |name: &str, args: &Value| -> ToolOutcome {
        // A step is one you saved, not one a model chose, so outside text does not hold it.
        let (_, found) = p.guard.review(
            &crate::guard::Hidden::default(),
            name,
            &args.to_string(),
            &place,
            None,
        );
        if let Some(f) = found {
            let (_, refused) =
                crate::guard::unattended(&p.asker, name, &args.to_string(), &f, stop);
            if let Some(told) = refused {
                return ToolOutcome {
                    value: Value::Null,
                    error: Some(told),
                };
            }
        }
        if name == crate::tools::SHELL {
            let Some(spec) = command::parse(&args.to_string()) else {
                return ToolOutcome {
                    value: Value::Null,
                    error: Some("No command was given.".to_owned()),
                };
            };
            let raw = command::run_raw(&spec, &p.shell_dir, cancel);
            return ToolOutcome {
                value: json!({
                    "exit_code": match raw.end { command::Ended::Exit(Some(c)) => json!(c), _ => Value::Null },
                    "stdout": raw.stdout,
                    "stderr": raw.stderr,
                }),
                error: (!raw.succeeded()).then(|| raw.text()),
            };
        }
        if name == crate::web::NAME {
            let text = crate::web::fetch(&args.to_string());
            let error = text.starts_with("Error:").then(|| text.clone());
            return ToolOutcome {
                value: Value::String(text),
                error,
            };
        }
        let Some(root) = &p.workdir else {
            return ToolOutcome {
                value: Value::Null,
                error: Some("This agent has no working folder for files.".to_owned()),
            };
        };
        let text = crate::files::run(root, name, &args.to_string());
        let error = text.starts_with("Error:").then(|| text.clone());
        ToolOutcome {
            value: Value::String(text),
            error,
        }
    };
    let model = |system: &str, message: &str| -> Result<(String, Usage), String> {
        ask(p, cancel, system, message).map(|c| {
            (
                c.text,
                Usage {
                    calls: 1,
                    input: c.tokens.input.unwrap_or(0),
                    output: c.tokens.output.unwrap_or(0),
                    cache_read: c.tokens.cache_read.unwrap_or(0),
                    cache_write: c.tokens.cache_write.unwrap_or(0),
                },
            )
        })
    };
    engine::execute(
        def,
        inputs,
        &Env {
            tool: &tool,
            model: &model,
            cancel: stop,
        },
    )
}

impl App {
    // ---- loading

    /// Reads the saved workflows and drafts from the database.
    pub(crate) fn load_workflows(&mut self) {
        let Some(db) = &self.store else {
            return;
        };
        match db.list_workflows() {
            Ok(list) => self.workflows = list,
            Err(e) => eprintln!("oido: could not read workflows: {e}"),
        }
        match db.list_drafts() {
            Ok(list) => {
                self.drafts = list
                    .into_iter()
                    .map(|d| (d.session_id.clone(), d))
                    .collect();
            }
            Err(e) => eprintln!("oido: could not read workflow drafts: {e}"),
        }
    }

    // ---- what a model call or a run needs

    pub(crate) fn prepare_model(&self, agent_id: &str) -> Result<Prep, String> {
        let agent = self
            .roster
            .agent_ref(agent_id)
            .ok_or("That agent is gone.")?;
        let ModelStatus::Ready(model) = self.providers.status(agent.model.as_ref()) else {
            return Err(format!(
                "{} has no model that can answer. Choose one first.",
                agent.name
            ));
        };
        let (endpoint, credential) = self.target_for(&model.provider)?;
        let workdir = agent.workdir.clone().map(PathBuf::from);
        Ok(Prep {
            endpoint,
            credential,
            who: self.providers.label_of(&model.provider),
            provider: model.provider,
            model: model.id,
            shell_dir: workdir
                .clone()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| ".".into()),
            workdir,
            guard: self.guard.clone(),
            asker: crate::guard::ask::Asker::nobody(),
        })
    }

    /// Everything a run of `def` by `agent_id` needs, or why it cannot run. A step uses only
    /// built-in tools, which every agent has, so a workflow does not widen what an agent may do.
    fn prepare_flow(&self, agent_id: &str, def: &Definition) -> Result<Prep, String> {
        let agent = self
            .roster
            .agent_ref(agent_id)
            .ok_or("That agent is gone.")?;
        let prep = self.prepare_model(agent_id)?;
        let files = crate::workflow::definition::tools_used(def)
            .iter()
            .any(|t| crate::files::is_file_tool(t));
        if files && prep.workdir.is_none() {
            return Err(format!(
                "{} has no working folder, which this workflow needs for files. Choose one in the agent's settings.",
                agent.name
            ));
        }
        Ok(prep)
    }

    // ---- learning

    /// Whether the open chat has done something a workflow could be made from.
    pub fn can_teach(&self) -> bool {
        self.current().is_some_and(|a| {
            let sid = &a.chat().id;
            !matches!(self.teaching.get(sid), Some(Teach::Working(_)))
                && !trace_of(&a.chat().messages).1.is_empty()
        })
    }

    /// Learns a workflow from the open chat. The draft appears in the chat when it is ready.
    pub fn teach_open(&mut self) {
        if let Some(sid) = self.current().map(|a| a.chat().id.clone()) {
            self.teach(&sid);
        }
    }

    pub(crate) fn teach(&mut self, sid: &str) {
        if matches!(self.teaching.get(sid), Some(Teach::Working(_))) {
            return;
        }
        let Some(agent) = self.roster.owner_of(sid) else {
            return;
        };
        let Some(chat) = agent.sessions.iter().find(|s| s.id == sid) else {
            return;
        };
        let (request, trace) = trace_of(&chat.messages);
        let into = Draft {
            session_id: sid.to_owned(),
            agent_id: agent.id.clone(),
            name: String::new(),
            goal: String::new(),
            definition: Definition::default(),
        };
        let prep = match self.prepare_model(&into.agent_id) {
            Ok(p) => p,
            Err(why) => {
                self.teaching.insert(sid.to_owned(), Teach::Failed(why));
                return;
            }
        };
        self.teaching.insert(
            sid.to_owned(),
            Teach::Working("Learning this from the chat…"),
        );
        let sid = sid.to_owned();
        self.spawn(
            move || {
                let cancel = AtomicBool::new(false);
                let model = |system: &str, message: &str| {
                    ask(&prep, &cancel, system, message).map(|c| c.text)
                };
                compile::compile(&model, &request, &trace, &into)
            },
            move |app, result| app.draft_ready(&sid, result),
        );
    }

    /// Changes the open chat's draft as typed in its card.
    pub fn refine_open(&mut self) {
        let Some(sid) = self.current().map(|a| a.chat().id.clone()) else {
            return;
        };
        let instruction = self.refine.text.trim().to_owned();
        let Some(draft) = self.drafts.get(&sid).cloned() else {
            return;
        };
        if instruction.is_empty() || matches!(self.teaching.get(&sid), Some(Teach::Working(_))) {
            return;
        }
        let prep = match self.prepare_model(&draft.agent_id) {
            Ok(p) => p,
            Err(why) => {
                self.teaching.insert(sid, Teach::Failed(why));
                return;
            }
        };
        self.refine = rune::TextEdit::new("");
        self.teaching
            .insert(sid.clone(), Teach::Working("Changing it…"));
        self.spawn(
            move || {
                let cancel = AtomicBool::new(false);
                let model = |system: &str, message: &str| {
                    ask(&prep, &cancel, system, message).map(|c| c.text)
                };
                compile::revise(&model, &draft, &instruction)
            },
            move |app, result| app.draft_ready(&sid, result),
        );
    }

    fn draft_ready(&mut self, sid: &str, result: Result<Draft, String>) {
        match result {
            Ok(draft) => {
                self.teaching.remove(sid);
                self.save("the workflow draft", |db| db.put_draft(&draft));
                self.drafts.insert(sid.to_owned(), draft);
                self.channel_workflow_ready(sid, None);
            }
            Err(why) => {
                self.channel_workflow_ready(sid, Some(&why));
                self.teaching.insert(sid.to_owned(), Teach::Failed(why));
            }
        }
    }

    /// Saves the open chat's draft as a workflow of its agent.
    pub fn save_draft(&mut self) {
        if let Some(sid) = self.current().map(|a| a.chat().id.clone()) {
            self.save_draft_in(&sid);
        }
    }

    /// Saves chat `sid`'s draft as a workflow of its agent, under its name or, if the agent has
    /// one called that, a numbered one. Returns the name it was saved under.
    pub(crate) fn save_draft_in(&mut self, sid: &str) -> Option<String> {
        let sid = sid.to_owned();
        let draft = self.drafts.remove(&sid)?;
        let mut name = draft.name.clone();
        let mut n = 1;
        while self
            .workflows
            .iter()
            .any(|w| w.agent_id == draft.agent_id && w.name.eq_ignore_ascii_case(&name))
        {
            n += 1;
            name = format!("{} {n}", draft.name);
        }
        let saved = Saved {
            id: new_id(),
            agent_id: draft.agent_id.clone(),
            name,
            goal: draft.goal.clone(),
            definition: draft.definition,
            created_at: now(),
        };
        self.save("the workflow", |db| {
            db.insert_workflow(&saved)?;
            db.delete_draft(&sid)
        });
        self.teaching.remove(&sid);
        let name = saved.name.clone();
        self.workflows.push(saved);
        self.workflows
            .sort_by_key(|w| (w.name.to_lowercase(), w.id.clone()));
        Some(name)
    }

    pub fn discard_draft(&mut self) {
        if let Some(sid) = self.current().map(|a| a.chat().id.clone()) {
            self.discard_draft_in(&sid);
        }
    }

    pub(crate) fn discard_draft_in(&mut self, sid: &str) {
        self.drafts.remove(sid);
        self.teaching.remove(sid);
        self.save("the workflow draft", |db| db.delete_draft(sid));
    }

    /// The agent called `teach_workflow`: learning starts, and the agent is told it has.
    pub(crate) fn run_teach_calls(&mut self, sid: &str) {
        let ids: Vec<String> = match self
            .roster
            .owner_of(sid)
            .and_then(|a| a.sessions.iter().find(|s| s.id == sid))
            .and_then(|s| s.messages.last())
        {
            Some(Message::Assistant { tools, .. }) => tools
                .iter()
                .filter(|t| t.name == crate::tools::TEACH && t.output.is_none())
                .map(|t| t.id.clone())
                .collect(),
            _ => return,
        };
        for id in ids {
            self.teach(sid);
            let told = match self.teaching.get(sid) {
                Some(Teach::Failed(why)) => format!("Error: the workflow could not be learned: {why}"),
                _ => "Learning the workflow from this chat now. The person will see a draft to confirm or change; tell them it is on its way.".to_owned(),
            };
            self.resolve(sid, &id, told, true);
        }
    }

    // ---- running

    /// Starts a run of `wf` as `agent_id` on a worker. `done` hears how it ended, on the UI thread,
    /// with the run as recorded. Errs, in words, if it cannot start.
    /// Runs workflow `wf` in the background. A step a rule questions waits for your go-ahead on a
    /// card in `chat`, the chat whose agent ran it, or on the workflow's page.
    #[expect(
        clippy::too_many_arguments,
        reason = "each caller says who started it, where it asks and what follows"
    )]
    pub(crate) fn launch_flow<D>(
        &mut self,
        wf: &Saved,
        agent_id: &str,
        given: &Map<String, Value>,
        started_by: &str,
        chat: Option<&str>,
        cancel: Arc<AtomicBool>,
        done: D,
    ) -> Result<(), String>
    where
        D: FnOnce(&mut App, &RunRecord, Finish) + Send + 'static,
    {
        let inputs = engine::bind_inputs(&wf.definition, given)?;
        let at = match chat {
            Some(sid) => crate::guard::ask::Where::Chat(sid.to_owned()),
            None => crate::guard::ask::Where::Workflow(wf.id.clone()),
        };
        let prep = Prep {
            asker: self.asker(at, &wf.name),
            ..self.prepare_flow(agent_id, &wf.definition)?
        };
        let record = RunRecord {
            id: new_id(),
            workflow_id: wf.id.clone(),
            started_by: started_by.to_owned(),
            status: RunStatus::Running,
            inputs: inputs.to_string(),
            output: String::new(),
            failed_step: String::new(),
            error: String::new(),
            steps: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            started_at: now(),
            finished_at: None,
        };
        self.save("the workflow run", |db| db.start_workflow_run(&record));
        let (def, name) = (wf.definition.clone(), wf.name.clone());
        let model = ModelRef::new(&prep.provider, &prep.model);
        let agent_name = self
            .roster
            .agent_ref(agent_id)
            .map(|a| a.name.clone())
            .unwrap_or_default();
        let agent_id = agent_id.to_owned();
        self.spawn(
            move || work(&def, &inputs, &prep, &cancel),
            move |app, outcome| {
                let mut record = record;
                let who = Counted {
                    model: &model,
                    agent_id: &agent_id,
                    agent_name: &agent_name,
                    source: Source::Workflow,
                };
                let finish = app.finish_flow(&mut record, &name, &outcome, &who);
                done(app, &record, finish);
            },
        );
        Ok(())
    }

    /// Records how a run ended, counts its model calls as `who`'s, and says it in words for an
    /// agent that started it.
    fn finish_flow(
        &mut self,
        record: &mut RunRecord,
        name: &str,
        o: &Outcome,
        who: &Counted,
    ) -> Finish {
        record.steps = o.steps.clone();
        record.output = o.output.clone();
        record.input_tokens = o.usage.input;
        record.output_tokens = o.usage.output;
        record.finished_at = Some(now());
        let finish = match &o.ending {
            Ending::Succeeded => {
                record.status = RunStatus::Succeeded;
                Finish::Done(format!(
                    "The workflow {name:?} succeeded after {} steps.\nOutput:\n{}",
                    o.steps.len(),
                    o.output
                ))
            }
            Ending::Failed { step, error } => {
                record.status = RunStatus::Failed;
                (record.failed_step, record.error) = (step.clone(), error.clone());
                Finish::Done(format!(
                    "The workflow {name:?} failed at step {step}: {error}"
                ))
            }
            Ending::Stopped => {
                record.status = RunStatus::Stopped;
                Finish::Stopped("The person stopped the workflow before it finished.".to_owned())
            }
        };
        self.save("the workflow run", |db| db.finish_workflow_run(record));
        let u = o.usage;
        if u.calls > 0 {
            let t = Totals {
                calls: u.calls.into(),
                input: u.input.into(),
                output: u.output.into(),
                cache_read: u.cache_read.into(),
                cache_write: u.cache_write.into(),
                ..Totals::default()
            };
            self.save("the usage", |db| db.add_usage(record.started_at, who, &t));
        }
        if self.route == Route::Workflows {
            self.reload_wf_runs();
        }
        finish
    }

    /// The agent called `run_workflow`: starts it as a background job like a command, so the
    /// result goes back to the agent when it ends. Always handles the call, even to refuse it.
    pub(crate) fn start_workflow_call(&mut self, sid: &str, call_id: &str, args: &str) {
        let Some(agent_id) = self.roster.owner_of(sid).map(|a| a.id.clone()) else {
            return;
        };
        let v: Value = serde_json::from_str(args).unwrap_or(Value::Null);
        let name = v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        let given = v
            .get("inputs")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let found = self
            .workflows
            .iter()
            .find(|w| w.agent_id == agent_id && w.name.eq_ignore_ascii_case(&name))
            .cloned();
        let Some(wf) = found else {
            let yours: Vec<&str> = self
                .workflows
                .iter()
                .filter(|w| w.agent_id == agent_id)
                .map(|w| w.name.as_str())
                .collect();
            let list = if yours.is_empty() {
                "You have none saved.".to_owned()
            } else {
                format!("Yours are: {}.", yours.join(", "))
            };
            self.resolve(
                sid,
                call_id,
                format!("Error: you have no workflow called {name:?}. {list}"),
                true,
            );
            self.start_next_command(sid);
            return;
        };
        let cancel = self.track(sid, call_id);
        let (s, c) = (sid.to_owned(), call_id.to_owned());
        let started = self.launch_flow(
            &wf,
            &agent_id,
            &given,
            "agent",
            Some(sid),
            cancel,
            move |app, _, finish| {
                app.command_done(&s, &c, finish);
            },
        );
        if let Err(why) = started {
            self.commands.remove(sid);
            self.resolve(sid, call_id, format!("Error: {why}"), true);
            self.start_next_command(sid);
        }
    }

    // ---- the Workflows page

    fn open_workflow(&self) -> Option<&Saved> {
        let id = self.wf_open.as_ref()?;
        self.workflows.iter().find(|w| &w.id == id)
    }

    /// Reads workflows again when the page opens, keeping the open one if it is still there.
    pub(crate) fn reload_workflows(&mut self) {
        self.load_workflows();
        let keep = self
            .wf_open
            .clone()
            .filter(|id| self.workflows.iter().any(|w| &w.id == id))
            .or_else(|| self.workflows.first().map(|w| w.id.clone()));
        match keep {
            Some(id) => self.select_workflow(&id),
            None => {
                self.wf_open = None;
                self.wf_runs.clear();
            }
        }
    }

    pub fn select_workflow(&mut self, id: &str) {
        self.wf_open = Some(id.to_owned());
        self.wf_notice = None;
        self.wf_confirm = false;
        self.wf_inputs = self
            .open_workflow()
            .map(|w| {
                w.definition
                    .inputs
                    .iter()
                    .map(|(name, input)| {
                        let start = input
                            .default
                            .as_ref()
                            .map(crate::workflow::template::as_text)
                            .unwrap_or_default();
                        (name.clone(), rune::TextEdit::new(start))
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.reload_wf_runs();
    }

    pub(crate) fn reload_wf_runs(&mut self) {
        let (Some(id), Some(db)) = (self.wf_open.clone(), &self.store) else {
            return;
        };
        match db.list_workflow_runs(&id, 50) {
            Ok(runs) => {
                self.wf_runs = runs;
                self.wf_run = self.wf_run.min(self.wf_runs.len().saturating_sub(1));
            }
            Err(e) => eprintln!("oido: could not read workflow runs: {e}"),
        }
    }

    pub fn select_wf_run(&mut self, i: usize) {
        self.wf_run = i;
    }

    /// Runs the open workflow with the inputs typed on the page.
    pub fn run_open_workflow(&mut self) {
        let Some(wf) = self.open_workflow().cloned() else {
            return;
        };
        if self.wf_jobs.contains_key(&wf.id) {
            return;
        }
        let given: Map<String, Value> = self
            .wf_inputs
            .iter()
            .map(|(name, edit)| (name.clone(), Value::String(edit.text.clone())))
            .collect();
        let cancel = Arc::new(AtomicBool::new(false));
        self.wf_jobs.insert(wf.id.clone(), Arc::clone(&cancel));
        self.wf_notice = None;
        let id = wf.id.clone();
        let started = self.launch_flow(
            &wf,
            &wf.agent_id,
            &given,
            "you",
            None,
            cancel,
            move |app, _, _| {
                app.wf_jobs.remove(&id);
                app.reload_wf_runs();
            },
        );
        if let Err(why) = started {
            self.wf_jobs.remove(&wf.id);
            self.wf_notice = Some(why);
        } else {
            self.reload_wf_runs();
            self.wf_run = 0;
        }
    }

    pub fn stop_open_workflow(&mut self) {
        if let Some(flag) = self.wf_open.as_ref().and_then(|id| self.wf_jobs.get(id)) {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Deletes the open workflow, after a second press.
    pub fn delete_open_workflow(&mut self) {
        if !self.wf_confirm {
            self.wf_confirm = true;
            return;
        }
        let Some(id) = self.wf_open.take() else {
            return;
        };
        self.save("the workflow", |db| db.delete_workflow(&id));
        self.wf_confirm = false;
        self.reload_workflows();
    }

    pub fn cancel_delete_workflow(&mut self) {
        self.wf_confirm = false;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use rune::TextEdit;

    use super::*;
    use crate::model::{Outcome, ToolCall};
    use crate::testing::{EVENTS, TempDb, app_with_server, call_stream, text_stream};
    use crate::workflow::definition::Step;

    fn tool_step(id: &str, command: &str, bind: &str) -> Step {
        Step {
            id: id.into(),
            kind: "tool".into(),
            tool: Some("shell".into()),
            args: json!({"command": command}).as_object().cloned(),
            bind: Some(bind.into()),
            ..Step::default()
        }
    }

    fn save_workflow(app: &mut App, steps: Vec<Step>, inputs: Value) -> Saved {
        let w = Saved {
            id: new_id(),
            agent_id: app.current().unwrap().id.clone(),
            name: "Say it".into(),
            goal: "Says it.".into(),
            definition: Definition {
                inputs: serde_json::from_value(inputs).unwrap(),
                steps,
                success: String::new(),
            },
            created_at: now(),
        };
        app.save("w", |db| db.insert_workflow(&w));
        app.workflows.push(w.clone());
        w
    }

    #[test]
    fn what_a_chat_did_is_the_trace_without_unfinished_calls_or_the_workflow_tools_themselves() {
        let call = |name: &str, id: &str, out: Option<&str>| ToolCall {
            name: name.into(),
            summary: String::new(),
            output: out.map(str::to_owned),
            id: id.into(),
            args: "{}".into(),
            trip: None,
            outside: None,
        };
        let reply = |tools| Message::Assistant {
            text: String::new(),
            thinking: String::new(),
            thought_secs: None,
            tools,
            approval: None,
            stats: None,
            run: None,
            outcome: Outcome::Done,
        };
        let (request, trace) = trace_of(&[
            Message::User("old".into()),
            Message::Divider("Context cleared".into()),
            Message::User("first".into()),
            reply(vec![
                call("shell", "a", Some("exit code: 0")),
                call("shell", "b", None),
                call("teach_workflow", "c", Some("ok")),
            ]),
            reply(vec![
                call("ask_user_question", "d", Some("Blue")),
                call("storage.read", "", Some("sample")),
            ]),
            Message::User("second".into()),
        ]);
        assert_eq!(request, "first\n\nsecond");
        assert_eq!(
            trace.iter().map(|t| t.tool.as_str()).collect::<Vec<_>>(),
            ["shell", "ask_user_question"]
        );
    }

    #[test]
    fn a_workflow_is_taught_from_a_chat_changed_in_words_saved_and_still_there_after_a_restart() {
        let compiled = r#"{"name":"Say hi","goal_summary":"Says hi.","definition":{"steps":[{"id":"say","kind":"tool","tool":"shell","args":{"command":"echo hi-from-chat"},"bind":"out"}]}}"#;
        let revised = r#"{"name":"Say hi and bye","goal_summary":"Says both.","definition":{"steps":[{"id":"say","kind":"tool","tool":"shell","args":{"command":"echo hi; echo bye"},"bind":"out"}]}}"#;
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (
                    200,
                    EVENTS,
                    call_stream("shell", json!({"command": "echo hi-from-chat"})),
                ),
                (200, EVENTS, text_stream("Done.")),
                (200, EVENTS, text_stream(compiled)),
                (200, EVENTS, text_stream(revised)),
            ],
        );
        app.roster.current_mut().unwrap().tools = vec![crate::tools::SHELL.into()];
        assert!(!app.can_teach(), "nothing to learn from yet");
        app.composer.edit = TextEdit::new("show me hi");
        app.send();
        let _ = (seen.recv().unwrap(), seen.recv().unwrap());
        assert!(app.can_teach());

        app.teach_open();
        let asked = seen.recv().unwrap();
        assert!(
            asked.contains("the user asked")
                && asked.contains("show me hi")
                && asked.contains("echo hi-from-chat"),
            "{asked}"
        );
        let sid = app.current().unwrap().chat().id.clone();
        assert!(app.teaching.is_empty());
        assert_eq!(app.drafts[&sid].name, "Say hi");

        app.refine = TextEdit::new("also say bye");
        app.refine_open();
        assert!(seen.recv().unwrap().contains("also say bye"));
        assert_eq!(app.drafts[&sid].name, "Say hi and bye");
        assert!(app.refine.text.is_empty());

        app.save_draft();
        assert!(app.drafts.is_empty() && app.workflows.len() == 1);
        assert_eq!(app.workflows[0].name, "Say hi and bye");
        drop(app);

        let app = db.open();
        assert_eq!(app.workflows.len(), 1);
        assert!(app.drafts.is_empty(), "a saved draft is not left behind");
    }

    #[cfg(unix)]
    #[test]
    fn a_workflow_runs_from_its_page_with_inputs_and_the_model_only_writes_what_it_is_asked_to() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(&db, vec![(200, EVENTS, text_stream("HELLO!"))]);
        app.roster.current_mut().unwrap().tools = vec![crate::tools::SHELL.into()];
        let generate = Step {
            id: "shout".into(),
            kind: "generate".into(),
            instruction: Some("Shout {{said.stdout}}".into()),
            ..Step::default()
        };
        save_workflow(
            &mut app,
            vec![tool_step("echo", "echo {{inputs.word}}", "said"), generate],
            json!({"word": {"type": "string", "default": "hey"}}),
        );
        app.go(Route::Workflows);
        assert_eq!(app.wf_inputs[0].1.text, "hey", "the default is filled in");
        app.wf_inputs[0].1 = TextEdit::new("hello");
        app.run_open_workflow();

        let prompt = seen.recv().unwrap();
        assert!(
            prompt.contains("«said.stdout»") && prompt.contains("hello"),
            "{prompt}"
        );
        assert!(app.wf_jobs.is_empty() && app.wf_notice.is_none());
        let run = &app.wf_runs[0];
        assert_eq!(
            (run.status, run.started_by.as_str(), run.output.as_str()),
            (RunStatus::Succeeded, "you", "HELLO!")
        );
        assert_eq!(run.steps.len(), 2);
        assert!(run.steps[0].output.contains("hello"));
        assert!(run.inputs.contains("hello"));
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_step_ends_the_run_and_says_where_and_why_without_asking_the_model() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(&db, vec![]);
        app.roster.current_mut().unwrap().tools = vec![crate::tools::SHELL.into()];
        let after = Step {
            id: "after".into(),
            kind: "generate".into(),
            instruction: Some("never".into()),
            ..Step::default()
        };
        save_workflow(
            &mut app,
            vec![tool_step("boom", "echo oops >&2; exit 3", "b"), after],
            json!({}),
        );
        app.go(Route::Workflows);
        app.run_open_workflow();
        let run = &app.wf_runs[0];
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.failed_step, "boom");
        assert!(
            run.error.contains("exit code: 3") && run.error.contains("oops"),
            "{}",
            run.error
        );
        assert_eq!(run.steps.len(), 1, "the next step never ran");
        assert!(seen.try_recv().is_err(), "the model was never asked");
    }

    #[cfg(unix)]
    #[test]
    fn an_agent_can_run_its_saved_workflow_by_name_and_gets_the_result_back() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (
                    200,
                    EVENTS,
                    call_stream("run_workflow", json!({"name": "say it"})),
                ),
                (200, EVENTS, text_stream("Done.")),
            ],
        );
        app.roster.current_mut().unwrap().tools = vec![
            crate::tools::RUN_WORKFLOW.into(),
            crate::tools::SHELL.into(),
        ];
        let w = save_workflow(
            &mut app,
            vec![tool_step("s", "echo ok-from-flow", "r")],
            json!({}),
        );
        app.composer.edit = TextEdit::new("do the thing");
        app.send();
        assert!(seen.recv().unwrap().contains("run_workflow"));
        let second = seen.recv().unwrap();
        assert!(
            second.contains("succeeded") && second.contains("ok-from-flow"),
            "{second}"
        );
        assert!(app.commands.is_empty() && !app.busy());
        assert!(
            matches!(app.current().unwrap().chat().messages.last(), Some(Message::Assistant { text, .. }) if text == "Done.")
        );
        let runs = app
            .store
            .as_ref()
            .unwrap()
            .list_workflow_runs(&w.id, 10)
            .unwrap();
        assert_eq!((runs.len(), runs[0].started_by.as_str()), (1, "agent"));
    }

    #[test]
    fn asking_for_a_workflow_that_does_not_exist_lists_the_ones_there_are() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (
                    200,
                    EVENTS,
                    call_stream("run_workflow", json!({"name": "nope"})),
                ),
                (200, EVENTS, text_stream("Sorry.")),
            ],
        );
        app.roster.current_mut().unwrap().tools = vec![crate::tools::RUN_WORKFLOW.into()];
        save_workflow(&mut app, vec![tool_step("s", "echo x", "r")], json!({}));
        app.composer.edit = TextEdit::new("go");
        app.send();
        let _ = seen.recv().unwrap();
        let second = seen.recv().unwrap();
        assert!(
            second.contains("no workflow called") && second.contains("say it"),
            "{second}"
        );
    }

    #[test]
    fn a_typesafe_call_without_typesafe_connected_is_refused_in_words() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (
                    200,
                    EVENTS,
                    call_stream(
                        crate::typesafe::YES_NO,
                        json!({"content": "hi", "question": "Is it spam?"}),
                    ),
                ),
                (200, EVENTS, text_stream("Ok.")),
            ],
        );
        assert!(!app.typesafe_connected());
        app.composer.edit = TextEdit::new("check it");
        app.send();
        let _ = seen.recv().unwrap();
        let second = seen.recv().unwrap();
        assert!(second.contains("typesafe is not connected"), "{second}");
        assert!(app.commands.is_empty() && !app.busy());
    }
}
