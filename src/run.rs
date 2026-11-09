//! Running an agent: a message you send becomes a run. The agent's brief and the chat since the
//! last "Start fresh" go to the agent's model, and the reply is written into the chat as it
//! streams in. Every run is recorded (see [`crate::runs`]) however it ends.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::app::{App, Effort, Route, now};
use crate::completion::{self, Call, Endpoint, Error, Event, Request, Role, Think, Turn};
use crate::model::{AgentStatus, Message, Outcome, Stats, ToolCall, new_id};
use crate::providers::ModelStatus;
use crate::runs::{Record, Run, Status};

/// How often streamed text reaches the screen: often enough to read as typing, rarely enough that
/// a fast model does not redraw on every token.
const FLUSH: Duration = Duration::from_millis(40);
/// Chats are named after their first message, up to this many characters.
const TITLE: usize = 40;

/// A reply being written: what the run needs to be recorded when it ends.
pub struct Live {
    run: Run,
    cancel: Arc<AtomicBool>,
    began: Instant,
    system: Option<String>,
    user: String,
    /// Set when `user` is a tool's result rather than your message.
    user_name: Option<String>,
    /// What the model was given, as readable text.
    sent: String,
    reasoning: String,
    /// How long it thought: until its first word, when it thought out loud.
    thought: Option<Duration>,
}

/// The messages the model sees: everything since the last divider that was answered in full, with
/// the tools it called and what came back. Replies that were stopped or failed are left out, so two
/// messages of yours in a row become one. Calls with no id are sample data and are not sent.
pub fn build_turns(messages: &[Message]) -> Vec<Turn> {
    let start = messages
        .iter()
        .rposition(|m| matches!(m, Message::Divider(_)))
        .map_or(0, |i| i + 1);
    let mut turns: Vec<Turn> = Vec::new();
    for m in &messages[start..] {
        match m {
            Message::User(t) => match turns.last_mut().filter(|l| l.role == Role::User) {
                Some(last) => {
                    last.text.push_str("\n\n");
                    last.text.push_str(t);
                }
                None => turns.push(Turn::user(t.clone())),
            },
            Message::Assistant {
                text,
                tools,
                outcome: Outcome::Done,
                ..
            } => {
                let called: Vec<&ToolCall> = tools.iter().filter(|t| !t.id.is_empty()).collect();
                // A chat cannot start with the assistant.
                if turns.is_empty() || (text.is_empty() && called.is_empty()) {
                    continue;
                }
                let calls = called
                    .iter()
                    .map(|t| Call {
                        id: t.id.clone(),
                        name: t.name.clone(),
                        args: t.args.clone(),
                    })
                    .collect();
                turns.push(Turn::assistant(text.clone(), calls));
                for t in called {
                    let result = t.output.clone().unwrap_or_else(|| "(no answer)".to_owned());
                    turns.push(Turn::tool(t.id.clone(), result));
                }
            }
            _ => {}
        }
    }
    turns
}

/// What was sent, as text for the run's record.
pub(crate) fn transcript(system: Option<&str>, context: Option<&str>, turns: &[Turn]) -> String {
    let mut parts = Vec::new();
    if let Some(s) = system.filter(|s| !s.is_empty()) {
        parts.push(format!("[system]\n{s}"));
    }
    if let Some(c) = context.filter(|c| !c.is_empty()) {
        parts.push(format!("[context]\n{c}"));
    }
    for t in turns {
        let who = match t.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool result",
        };
        parts.push(format!("[{who}]\n{}", t.text));
        for c in &t.calls {
            parts.push(format!("[tool call] {} {}", c.name, c.args));
        }
    }
    parts.join("\n\n")
}

fn title_of(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    let mut title: String = line.chars().take(TITLE).collect();
    if line.chars().count() > TITLE {
        title.push('…');
    }
    title
}

pub(crate) fn think_of(effort: Effort) -> Option<Think> {
    match effort {
        Effort::Off => None,
        Effort::Low => Some(Think::Low),
        Effort::Medium => Some(Think::Medium),
        Effort::High => Some(Think::High),
    }
}

impl Live {
    /// A reply in progress, for screenshots.
    #[cfg(debug_assertions)]
    pub fn demo() -> Live {
        Live {
            run: Run {
                id: "r1".into(),
                agent_id: String::new(),
                agent_name: String::new(),
                session_id: String::new(),
                model: crate::models::ModelRef::new("openai", "x"),
                status: Status::Running,
                error: None,
                input_tokens: None,
                output_tokens: None,
                duration_ms: None,
                started_at: 0,
                ended_at: None,
                parent: None,
            },
            cancel: Arc::new(AtomicBool::new(false)),
            began: Instant::now(),
            system: None,
            user: String::new(),
            user_name: None,
            sent: String::new(),
            reasoning: String::new(),
            thought: None,
        }
    }
}

impl App {
    /// Whether the agent is writing a reply in any chat.
    pub(crate) fn writing(&self, agent_id: &str) -> bool {
        self.live.values().any(|l| l.run.agent_id == agent_id)
    }

    /// Seconds the open chat's reply has been in progress.
    pub fn writing_secs(&self) -> Option<u64> {
        let id = &self.current()?.chat().id;
        self.live.get(id).map(|l| l.began.elapsed().as_secs())
    }

    /// Whether the open chat is waiting for a reply.
    pub fn busy(&self) -> bool {
        self.current().is_some_and(|a| {
            let id = &a.chat().id;
            self.live.contains_key(id)
                || self.commands.contains_key(id)
                || self.delegating.contains_key(id)
        })
    }

    /// Sends the composer's text: it joins the chat and the agent starts answering.
    pub fn send(&mut self) {
        let text = self.composer.text.trim().to_owned();
        if text.is_empty() || !self.can_answer() || self.busy() || self.awaiting_answer() {
            return;
        }
        let Some(agent) = self.roster.current_mut() else {
            return;
        };
        let chat = agent.chat_mut();
        let naming = chat.title == "New chat" && chat.messages.is_empty();
        if naming {
            chat.title = title_of(&text);
        }
        chat.messages.push(Message::User(text));
        let seq = chat.messages.len() - 1;
        self.composer = rune::TextEdit::new("");
        self.composer.focused = true;
        self.save_message(seq);
        if naming {
            self.save_chat();
        }
        self.begin();
    }

    /// Asks again after a reply that was stopped or failed: that reply goes, and a new run
    /// answers the same message.
    pub fn retry(&mut self) {
        if self.busy() || !self.can_answer() {
            return;
        }
        let Some(agent) = self.roster.current_mut() else {
            return;
        };
        let chat = agent.chat_mut();
        let redo = matches!(
            chat.messages.last(),
            Some(Message::Assistant {
                outcome: Outcome::Stopped | Outcome::Failed(_),
                ..
            })
        );
        if !redo {
            return;
        }
        chat.messages.pop();
        let (sid, seq) = (chat.id.clone(), chat.messages.len());
        self.save("the chat", |db| db.delete_messages_from(&sid, seq));
        self.begin();
    }

    /// Stops the open chat's reply; what was written stays.
    pub fn stop(&mut self) {
        if let Some(id) = self.current().map(|a| a.chat().id.clone()) {
            self.cancel_session(&id);
        }
    }

    /// Starts a run that answers the open chat's last message.
    fn begin(&mut self) {
        if let Some(id) = self.current().map(|a| a.chat().id.clone()) {
            self.begin_in(&id);
        }
    }

    /// Starts a run that answers the last message of a chat, which need not be the open one (a
    /// command can finish after you have gone to another chat).
    fn begin_in(&mut self, sid: &str) {
        let Some(agent) = self.roster.owner_of(sid) else {
            return;
        };
        let Some(chat) = agent.sessions.iter().find(|s| s.id == sid) else {
            return;
        };
        if self.live.contains_key(sid)
            || self.commands.contains_key(sid)
            || self.delegating.contains_key(sid)
        {
            return;
        }
        let ModelStatus::Ready(model) = self.providers.status(agent.model.as_ref()) else {
            return;
        };
        let sid = sid.to_owned();
        let turns = build_turns(&chat.messages);
        if turns.last().is_none_or(|t| t.role == Role::Assistant) {
            return;
        }
        // What this run answers: your message, or the tool results that came back.
        let trailing: Vec<&Turn> = turns
            .iter()
            .rev()
            .take_while(|t| t.role != Role::Assistant)
            .collect();
        let user: String = trailing
            .iter()
            .rev()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        // Named for what came back: an answer of yours, or a command's output.
        let names: Vec<&str> = trailing
            .iter()
            .filter(|t| t.role == Role::Tool)
            .filter_map(|t| {
                turns
                    .iter()
                    .flat_map(|a| &a.calls)
                    .find(|c| Some(&c.id) == t.call_id.as_ref())
                    .map(|c| c.name.as_str())
            })
            .collect();
        let user_name = (!names.is_empty()).then(|| {
            if names.iter().all(|n| *n == crate::tools::ASK) {
                "Answer to your question".to_owned()
            } else {
                "Tool result".to_owned()
            }
        });
        let system = self.system_for(agent);
        let can_think = self
            .providers
            .model_info(&model)
            .is_none_or(|i| crate::models::can_think(&i.caps));
        let context = crate::context::now_text();
        let request = Request {
            model: model.id.clone(),
            system: system.clone(),
            context: Some(context.clone()),
            think: think_of(self.effort).filter(|_| can_think),
            turns: turns.clone(),
            tools: crate::delegate::tools_for(agent, self.typesafe_connected()),
        };
        let run = Run {
            id: new_id(),
            agent_id: agent.id.clone(),
            agent_name: agent.name.clone(),
            session_id: sid.clone(),
            model: model.clone(),
            status: Status::Running,
            error: None,
            input_tokens: None,
            output_tokens: None,
            duration_ms: None,
            started_at: now(),
            ended_at: None,
            parent: None,
        };
        let live = Live {
            run: run.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            began: Instant::now(),
            system: system.clone(),
            user,
            user_name,
            sent: transcript(system.as_deref(), Some(&context), &turns),
            reasoning: String::new(),
            thought: None,
        };
        let cancel = Arc::clone(&live.cancel);
        let target = self.target_for(&model.provider);
        self.save("the run", |db| db.start_run(&run));
        if let Some(agent) = self.roster.agent_mut(&run.agent_id) {
            agent.status = AgentStatus::Running;
            if let Some(chat) = agent.sessions.iter_mut().find(|s| s.id == sid) {
                chat.messages
                    .push(Message::reply("", Some(run.id.clone()), Outcome::Working));
            }
        }
        self.live.insert(sid.clone(), live);
        self.beat();
        match target {
            Ok((endpoint, key)) => self.stream(sid, run.id, endpoint, key, request, cancel),
            Err(why) => self.settle(&sid, &run.id, Err(Error::Setup(why))),
        }
    }

    /// The brief an agent is given: its instructions with the skills it has.
    pub(crate) fn system_for(&self, agent: &crate::agents::Agent) -> Option<String> {
        Some(crate::skills::with_skills(
            &agent.prompt,
            self.skills_dir.as_deref(),
            &agent.skills,
        ))
        .filter(|p| !p.trim().is_empty())
    }

    /// Where a provider's chat is, and the key to use, or why there is no way to ask it.
    pub(crate) fn target_for(
        &self,
        provider: &str,
    ) -> Result<(Endpoint, Option<Zeroizing<String>>), String> {
        let label = self.providers.label_of(provider);
        let p = self.providers.get(provider).filter(|p| p.connected);
        let endpoint = p
            .and_then(|p| completion::endpoint(&p.id, p.custom, p.base_url.as_deref()))
            .ok_or_else(|| format!("{label} has no chat address to send to."))?;
        let key = match &self.store {
            Some(db) => db.provider_key(provider).map_err(|e| e.to_string())?,
            None => None,
        };
        if key.is_none() && !p.is_some_and(|p| p.custom) {
            return Err(format!(
                "{label} has no saved key. Connect it again on the Providers page."
            ));
        }
        Ok((endpoint, key))
    }

    /// Asks the model on a worker thread; the reply arrives through [`App::apply`] and the end
    /// through [`App::settle`]. With no running app (tests) it all happens on the spot.
    fn stream(
        &mut self,
        sid: String,
        rid: String,
        endpoint: Endpoint,
        key: Option<Zeroizing<String>>,
        request: Request,
        cancel: Arc<AtomicBool>,
    ) {
        let ask = move |emit: &mut dyn FnMut(Event)| {
            completion::run(
                &endpoint,
                key.as_deref().map(String::as_str),
                &request,
                &cancel,
                emit,
            )
        };
        match crate::app::handle() {
            Some(handle) => {
                let handle = handle.clone();
                std::thread::spawn(move || {
                    let (mut pending, mut last) = (Vec::new(), Instant::now());
                    let send = |events: Vec<Event>| {
                        let (sid, rid) = (sid.clone(), rid.clone());
                        handle.update(move |app, _| app.apply(&sid, &rid, events));
                    };
                    let result = ask(&mut |ev| {
                        pending.push(ev);
                        if last.elapsed() >= FLUSH {
                            send(std::mem::take(&mut pending));
                            last = Instant::now();
                        }
                    });
                    if !pending.is_empty() {
                        send(pending);
                    }
                    handle.update(move |app, _| app.settle(&sid, &rid, result));
                });
            }
            None => {
                let mut events = Vec::new();
                let result = ask(&mut |ev| events.push(ev));
                self.apply(&sid, &rid, events);
                self.settle(&sid, &rid, result);
            }
        }
    }

    fn is_live(&self, sid: &str, rid: &str) -> bool {
        self.live.get(sid).is_some_and(|l| l.run.id == rid)
    }

    /// What the model said so far goes into the chat. Ignored once its run has ended.
    pub fn apply(&mut self, sid: &str, rid: &str, events: Vec<Event>) {
        if !self.is_live(sid, rid) {
            return;
        }
        let (mut text, mut calls) = (String::new(), Vec::new());
        if let Some(live) = self.live.get_mut(sid) {
            for ev in events {
                match ev {
                    Event::Text(t) => text.push_str(&t),
                    Event::Reasoning(t) => live.reasoning.push_str(&t),
                    Event::ToolCall(call) => calls.push(call),
                    Event::Usage { input, output } => {
                        live.run.input_tokens = input.or(live.run.input_tokens);
                        live.run.output_tokens = output.or(live.run.output_tokens);
                    }
                }
            }
            if !text.is_empty() && live.thought.is_none() && !live.reasoning.is_empty() {
                live.thought = Some(live.began.elapsed());
            }
        }
        if let Some(Message::Assistant {
            text: body, tools, ..
        }) = self
            .roster
            .owner_of_mut(sid)
            .and_then(|a| a.sessions.iter_mut().find(|s| s.id == sid))
            .and_then(|s| s.messages.last_mut())
        {
            body.push_str(&text);
            for call in calls {
                // The only tool there is asks you; a tool the model made up is answered at once.
                let known = crate::tools::knows(&call.name);
                tools.push(ToolCall {
                    summary: crate::tools::summary(&call.name, &call.args),
                    output: (!known)
                        .then(|| format!("Error: there is no tool called {}.", call.name)),
                    name: call.name,
                    id: call.id,
                    args: call.args,
                });
            }
        }
    }

    /// The run ends, however it ended: the reply is marked and saved and the run recorded. Does
    /// nothing if the run already ended (stopped, say, before the network gave up).
    pub fn settle(&mut self, sid: &str, rid: &str, result: Result<(), Error>) {
        if !self.is_live(sid, rid) {
            return;
        }
        let Some(live) = self.live.remove(sid) else {
            return;
        };
        let who = self.providers.label_of(&live.run.model.provider);
        let model_name = self.providers.model_name(&live.run.model);
        let mut run = live.run;
        let elapsed = live.began.elapsed();
        let (mut reply, mut called) = (String::new(), Vec::new());
        if let Some(Message::Assistant { text, tools, .. }) = self
            .roster
            .owner_of_mut(sid)
            .and_then(|a| a.sessions.iter_mut().find(|s| s.id == sid))
            .and_then(|s| s.messages.last_mut())
        {
            reply = text.clone();
            called = tools.clone();
        }
        let result = match result {
            Ok(()) if reply.is_empty() && called.is_empty() => {
                Err(Error::Setup(format!("{who} sent back an empty reply.")))
            }
            other => other,
        };
        let (outcome, status, failure) = match &result {
            Ok(()) => (Outcome::Done, Status::Ok, None),
            Err(Error::Cancelled) => (Outcome::Stopped, Status::Cancelled, None),
            Err(e) => {
                let why = e.describe(&who);
                (Outcome::Failed(why.clone()), Status::Error, Some(why))
            }
        };
        run.status = status;
        run.error = failure.clone();
        run.duration_ms = Some(elapsed.as_millis().min(u128::from(u32::MAX)) as u32);
        run.ended_at = Some(now());
        let stats = (status == Status::Ok).then(|| Stats {
            model: model_name,
            tokens: run.output_tokens.unwrap_or(0),
            seconds: elapsed.as_secs_f32(),
        });
        let thought = live.thought.map(|d| d.as_secs_f32());

        // The reply in the chat, saved with how it ended.
        let mut saved = None;
        if let Some(chat) = self
            .roster
            .owner_of_mut(sid)
            .and_then(|a| a.sessions.iter_mut().find(|s| s.id == sid))
            && let Some(seq) = chat.messages.len().checked_sub(1)
            && let Some(Message::Assistant {
                thought_secs,
                stats: slot,
                outcome: slot_outcome,
                ..
            }) = chat.messages.last_mut()
        {
            *thought_secs = thought;
            *slot = stats;
            *slot_outcome = outcome;
            saved = Some((chat.id.clone(), seq, chat.messages[seq].clone()));
        }
        if let Some((id, seq, m)) = saved {
            self.save("the reply", |db| db.put_message(&id, seq, &m));
        }

        let steps = Record {
            system: live.system.as_deref(),
            user: &live.user,
            user_name: live.user_name.as_deref(),
            calls: &called,
            sent: &live.sent,
            reply: &reply,
            reasoning: &live.reasoning,
            failure: failure.as_deref(),
            status,
            input_tokens: run.input_tokens,
            output_tokens: run.output_tokens,
            duration_ms: run.duration_ms.unwrap_or(0),
            started_at: run.started_at,
        }
        .steps();
        self.save("the run", |db| db.finish_run(&run, &steps));

        let others = self.writing(&run.agent_id);
        if let Some(agent) = self.roster.agent_mut(&run.agent_id)
            && !others
        {
            agent.status = AgentStatus::Idle;
        }
        if self.route == Route::Runs {
            self.reload_runs();
        }
        // The agent asked to run commands: they run now, with no go-ahead needed.
        if status == Status::Ok {
            self.run_file_tools(sid);
            self.run_teach_calls(sid);
            self.start_delegations(sid);
            self.start_next_command(sid);
        }
    }

    /// Ends the chat's reply as stopped, if it has one being written.
    pub(crate) fn cancel_session(&mut self, sid: &str) {
        if let Some(flag) = self.commands.get(sid) {
            flag.store(true, Ordering::Relaxed);
        }
        self.halt_delegations(sid);
        if let Some(live) = self.live.get(sid) {
            live.cancel.store(true, Ordering::Relaxed);
            let rid = live.run.id.clone();
            self.settle(sid, &rid, Err(Error::Cancelled));
        }
    }

    // ---- questions the agent asked

    /// The open chat ends with something the agent asked of you (a question, a command to allow)
    /// that is not settled yet.
    pub fn awaiting_answer(&self) -> bool {
        self.current().is_some_and(|a| {
            matches!(
                a.chat().messages.last(),
                Some(Message::Assistant { tools, .. })
                    if tools.iter().any(|t| crate::tools::needs_you(&t.name) && t.output.is_none())
            )
        })
    }

    /// The first call of `name` in the open chat that has no result yet.
    fn pending_call(&self, name: &str) -> Option<String> {
        match self.current()?.chat().messages.last()? {
            Message::Assistant { tools, .. } => tools
                .iter()
                .find(|t| t.name == name && t.output.is_none())
                .map(|t| t.id.clone()),
            _ => None,
        }
    }

    /// Gives a waiting call its result, in the chat that holds it. When nothing in that message is
    /// left waiting and `carry_on`, the agent continues with a new run.
    pub(crate) fn resolve(&mut self, sid: &str, call_id: &str, output: String, carry_on: bool) {
        let Some(chat) = self
            .roster
            .owner_of_mut(sid)
            .and_then(|a| a.sessions.iter_mut().find(|s| s.id == sid))
        else {
            return;
        };
        // Only the newest assistant message can be waiting.
        let Some((seq, Message::Assistant { tools, .. })) = chat
            .messages
            .iter_mut()
            .enumerate()
            .rev()
            .find(|(_, m)| matches!(m, Message::Assistant { .. }))
        else {
            return;
        };
        let Some(call) = tools
            .iter_mut()
            .find(|t| t.id == call_id && t.output.is_none())
        else {
            return;
        };
        call.output = Some(output);
        let waiting = tools.iter().any(|t| t.output.is_none());
        let saved = (chat.id.clone(), seq, chat.messages[seq].clone());
        self.save("the reply", |db| {
            db.put_message(&saved.0, saved.1, &saved.2)
        });
        if carry_on && !waiting {
            self.begin_in(sid);
        }
    }

    /// Answers the question being asked with `text`.
    pub fn answer(&mut self, text: &str) {
        if text.trim().is_empty() || self.busy() {
            return;
        }
        let (Some(sid), Some(call)) = (
            self.current().map(|a| a.chat().id.clone()),
            self.pending_call(crate::tools::ASK),
        ) else {
            return;
        };
        self.answer_text = rune::TextEdit::new("");
        self.picks.clear();
        self.resolve(&sid, &call, text.trim().to_owned(), true);
    }

    /// Starts the first command the agent asked for that has not run, if none is running. Commands
    /// run as soon as they are asked for, one after another, each in the agent's working folder.
    /// Returns whether one started.
    pub(crate) fn start_next_command(&mut self, sid: &str) -> bool {
        if self.commands.contains_key(sid) {
            return false;
        }
        let Some(agent) = self.roster.owner_of(sid) else {
            return false;
        };
        let Some(Message::Assistant { tools, .. }) = agent
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .and_then(|s| s.messages.last())
        else {
            return false;
        };
        let Some(call) = tools
            .iter()
            .find(|t| crate::tools::is_job(&t.name) && t.output.is_none())
        else {
            return false;
        };
        let (id, args, name) = (call.id.clone(), call.args.clone(), call.name.clone());
        let sid = sid.to_owned();
        if name == crate::tools::RUN_WORKFLOW {
            self.start_workflow_call(&sid, &id, &args);
            return true;
        }
        if crate::tools::needs_typesafe(&name) {
            self.start_typesafe_call(&sid, &id, &name, &args);
            return true;
        }
        let Some(spec) = crate::command::parse(&args) else {
            self.resolve(
                &sid,
                &id,
                "No command was given, so nothing ran.".to_owned(),
                true,
            );
            self.start_next_command(&sid);
            return true;
        };
        let dir = self.workdir_of(&sid);
        if !dir.is_dir() {
            let why = format!(
                "The working folder {} does not exist, so nothing ran. The person can choose another in the agent's settings.",
                dir.display()
            );
            self.resolve(&sid, &id, why, true);
            self.start_next_command(&sid);
            return true;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.commands.insert(sid.clone(), Arc::clone(&cancel));
        self.spawn(
            move || crate::command::run(&spec, &dir, &cancel),
            move |app, finish| app.command_done(&sid, &id, finish),
        );
        true
    }

    /// Whether TypeSafe is connected: its tools exist only then.
    pub fn typesafe_connected(&self) -> bool {
        self.providers
            .get(crate::providers::TYPESAFE)
            .is_some_and(|p| p.connected)
    }

    /// The agent put a question to TypeSafe: it is asked in the background like a command, so the
    /// answer goes back to the agent when it arrives. Always handles the call, even to refuse it.
    fn start_typesafe_call(&mut self, sid: &str, call_id: &str, name: &str, args: &str) {
        let key = match (&self.store, self.typesafe_connected()) {
            (Some(db), true) => db
                .provider_key(crate::providers::TYPESAFE)
                .map_err(|e| e.to_string())
                .and_then(|k| k.ok_or_else(|| "TypeSafe has no saved key.".to_owned())),
            _ => Err("TypeSafe is not connected.".to_owned()),
        };
        let key = match key {
            Ok(key) => key,
            Err(why) => {
                let told = format!("Error: {why} The person can connect it on the Providers page.");
                self.resolve(sid, call_id, told, true);
                self.start_next_command(sid);
                return;
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.commands.insert(sid.to_owned(), Arc::clone(&cancel));
        let (sid, id) = (sid.to_owned(), call_id.to_owned());
        let (name, args) = (name.to_owned(), args.to_owned());
        let root = self
            .roster
            .owner_of(&sid)
            .and_then(|a| a.workdir.clone())
            .map(std::path::PathBuf::from);
        self.spawn(
            move || {
                let told = crate::typesafe::run(
                    crate::typesafe::BASE,
                    &key,
                    root.as_deref(),
                    &name,
                    &args,
                );
                if cancel.load(Ordering::Relaxed) {
                    crate::command::Finish::Stopped("Stopped before TypeSafe answered.".into())
                } else {
                    crate::command::Finish::Done(told)
                }
            },
            move |app, finish| app.command_done(&sid, &id, finish),
        );
    }

    /// Runs the file tools the agent just asked for, at once: they are quick and stay inside the
    /// agent's working folder. Without a working folder they have nowhere to work, and say so.
    fn run_file_tools(&mut self, sid: &str) {
        let calls: Vec<(String, String, String)> = match self
            .roster
            .owner_of(sid)
            .and_then(|a| a.sessions.iter().find(|s| s.id == sid))
            .and_then(|s| s.messages.last())
        {
            Some(Message::Assistant { tools, .. }) => tools
                .iter()
                .filter(|t| crate::files::is_file_tool(&t.name) && t.output.is_none())
                .map(|t| (t.id.clone(), t.name.clone(), t.args.clone()))
                .collect(),
            _ => return,
        };
        let root = self
            .roster
            .owner_of(sid)
            .and_then(|a| a.workdir.clone())
            .map(std::path::PathBuf::from);
        for (id, name, args) in calls {
            let result = match &root {
                Some(root) => crate::files::run(root, &name, &args),
                None => "Error: this agent has no working folder, so there is nowhere to read or write files. The person can choose one in the agent's settings.".to_owned(),
            };
            self.resolve(sid, &id, result, true);
        }
    }

    /// Where the chat's agent runs commands: the folder chosen for it, else the home folder.
    pub fn workdir_of(&self, sid: &str) -> std::path::PathBuf {
        self.roster
            .owner_of(sid)
            .and_then(|a| a.workdir.clone())
            .map(std::path::PathBuf::from)
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| ".".into())
    }

    /// A command ended. Its output goes to the agent, and the next command starts; when none is
    /// left the agent carries on. If you stopped it, the rest do not run and the chat waits for
    /// you.
    pub(crate) fn command_done(&mut self, sid: &str, call: &str, finish: crate::command::Finish) {
        self.commands.remove(sid);
        match finish {
            crate::command::Finish::Done(text) => {
                self.resolve(sid, call, text, true);
                self.start_next_command(sid);
            }
            crate::command::Finish::Stopped(text) => {
                self.resolve(sid, call, text, false);
                self.close_commands(sid, "Not run: the person stopped the command before it.");
            }
        }
    }

    /// Gives every command still waiting in the chat's last message `why` as its result.
    fn close_commands(&mut self, sid: &str, why: &str) {
        let ids: Vec<String> = match self
            .roster
            .owner_of(sid)
            .and_then(|a| a.sessions.iter().find(|s| s.id == sid))
            .and_then(|s| s.messages.last())
        {
            Some(Message::Assistant { tools, .. }) => tools
                .iter()
                .filter(|t| {
                    (crate::tools::is_job(&t.name) || t.name == crate::tools::DELEGATE)
                        && t.output.is_none()
                })
                .map(|t| t.id.clone())
                .collect(),
            _ => Vec::new(),
        };
        for id in ids {
            self.resolve(sid, &id, why.to_owned(), false);
        }
    }

    /// On start: a command asked for before the app closed never ran, and must not start now.
    /// It is marked not run, so the chat is not left waiting for it.
    pub(crate) fn settle_interrupted_commands(&mut self) {
        let sessions: Vec<String> = self
            .roster
            .list
            .iter()
            .flat_map(|a| std::iter::once(a).chain(a.subagents.iter()))
            .flat_map(|a| a.sessions.iter().map(|s| s.id.clone()))
            .collect();
        for sid in sessions {
            self.close_commands(
                &sid,
                "Not run: the app was closed before this command started.",
            );
        }
    }

    /// Answers with what was chosen and typed.
    pub fn send_answer(&mut self) {
        let text = crate::tools::answer(&self.picks, &self.answer_text.text);
        self.answer(&text);
    }

    pub fn skip_question(&mut self) {
        self.answer(crate::tools::SKIPPED);
    }

    pub fn toggle_pick(&mut self, option: &str) {
        match self.picks.iter().position(|p| p == option) {
            Some(i) => {
                self.picks.remove(i);
            }
            None => self.picks.push(option.to_owned()),
        }
    }

    // ---- the Runs screen

    /// Goes to a screen. Runs is read fresh each time it opens.
    pub fn go(&mut self, route: Route) {
        self.route = route;
        if route == Route::Runs {
            self.reload_runs();
        }
        if route == Route::Workflows {
            self.reload_workflows();
        }
    }

    /// Reads the newest runs again, keeping the open one open if it is still there.
    pub fn reload_runs(&mut self) {
        let open = self.runs.get(self.run).map(|r| r.id.clone());
        let mut runs = Vec::new();
        if let Some(db) = &self.store {
            match db.list_runs(RUNS_SHOWN) {
                Ok(list) => runs = list,
                Err(e) => eprintln!("oido: could not read runs: {e}"),
            }
        }
        self.runs = crate::runs::nest(runs);
        let at = open
            .and_then(|id| self.runs.iter().position(|r| r.id == id))
            .unwrap_or(0);
        self.select_run(at);
    }

    pub fn select_run(&mut self, i: usize) {
        self.run = i;
        self.record = 0;
        self.steps = match (&self.store, self.runs.get(i)) {
            (Some(db), Some(r)) => db.list_steps(&r.id).unwrap_or_else(|e| {
                eprintln!("oido: could not read the run: {e}");
                Vec::new()
            }),
            _ => Vec::new(),
        };
    }

    /// Opens a run from the chat.
    pub fn open_run(&mut self, id: &str) {
        self.go(Route::Runs);
        if let Some(i) = self.runs.iter().position(|r| r.id == id) {
            self.select_run(i);
        }
    }
}

/// The Runs screen lists this many, newest first.
const RUNS_SHOWN: usize = 200;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::models::ModelRef;
    use crate::runs::{Run, Status, StepKind};
    use crate::testing::TempDb;
    use rune::TextEdit;

    fn user(t: &str) -> Message {
        Message::User(t.into())
    }

    fn reply(t: &str, outcome: Outcome) -> Message {
        Message::reply(t, None, outcome)
    }

    #[test]
    fn the_model_sees_the_chat_since_the_last_divider_with_unanswered_messages_merged() {
        let turns = build_turns(&[
            user("old"),
            reply("old answer", Outcome::Done),
            Message::Divider("Context cleared".into()),
            user("a"),
            reply("A", Outcome::Done),
            user("b"),
            reply("half", Outcome::Stopped),
            user("c"),
            reply("", Outcome::Working),
        ]);
        let shown: Vec<_> = turns.iter().map(|t| (t.role, t.text.as_str())).collect();
        assert_eq!(
            shown,
            vec![
                (Role::User, "a"),
                (Role::Assistant, "A"),
                (Role::User, "b\n\nc"),
            ]
        );
        assert!(build_turns(&[Message::Divider("x".into())]).is_empty());
        assert_eq!(
            build_turns(&[reply("orphan", Outcome::Done), user("q")]).len(),
            1
        );
    }

    #[test]
    fn a_chat_is_named_after_its_first_message() {
        assert_eq!(title_of("  Plan the launch\nmore"), "Plan the launch");
        let long = "x".repeat(60);
        assert_eq!(title_of(&long).chars().count(), TITLE + 1);
    }

    const STREAM: &str = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hmm\"}}]}\n\n\
        data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n\
        data: {\"choices\":[{\"delta\":{\"content\":\"lo!\"},\"finish_reason\":\"stop\"}]}\n\n\
        data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":4}}\n\n\
        data: [DONE]\n\n";

    use crate::testing::app_with_server;

    fn last(app: &App) -> &Message {
        app.current().unwrap().chat().messages.last().unwrap()
    }

    #[test]
    fn a_message_starts_a_run_and_the_streamed_reply_is_written_saved_and_recorded() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(&db, vec![(200, "text/event-stream", STREAM)]);
        app.effort = Effort::Off;
        app.composer = TextEdit::new("hi there");
        // An agent with a team is always offered `delegate`; this one has none.
        app.roster.current_mut().unwrap().subagents.clear();
        app.send();

        let request = seen.recv().unwrap();
        assert!(
            request.starts_with("post /v1/chat/completions "),
            "{request}"
        );
        assert!(request.contains("authorization: bearer sk-box"));
        assert!(request.contains("\"stream\":true") && request.contains("hi there"));
        assert!(
            !request.contains("\"tools\""),
            "an agent with no tools is offered none"
        );

        let Message::Assistant {
            text,
            outcome,
            run,
            stats,
            thought_secs,
            ..
        } = last(&app)
        else {
            panic!("a reply");
        };
        assert_eq!((text.as_str(), outcome), ("Hello!", &Outcome::Done));
        assert!(run.is_some() && thought_secs.is_some());
        assert_eq!(stats.as_ref().unwrap().tokens, 4);
        assert!(!app.busy() && app.live.is_empty());
        assert_eq!(app.current().unwrap().status, AgentStatus::Idle);

        app.go(Route::Runs);
        assert_eq!(app.runs.len(), 1);
        let r = &app.runs[0];
        assert_eq!(
            (r.status, r.input_tokens, r.output_tokens),
            (Status::Ok, Some(9), Some(4))
        );
        assert_eq!(r.model, ModelRef::new("llama-server:box", "llama-3.3-70b"));
        let kinds: Vec<_> = app.steps.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, vec![StepKind::System, StepKind::User, StepKind::Llm]);
        let call = &app.steps[2];
        assert_eq!(call.output.as_deref(), Some("Hello!"));
        assert_eq!(call.reasoning.as_deref(), Some("hmm"));
        assert!(call.input.as_deref().unwrap().contains("[user]\nhi there"));
        assert!(
            call.input
                .as_deref()
                .unwrap()
                .contains("[context]\n## Context"),
            "the record shows it"
        );
        drop(app);

        // After a restart the chat has the reply and the run is still listed.
        let mut app = db.open();
        assert!(
            matches!(last(&app), Message::Assistant { text, outcome: Outcome::Done, .. } if text == "Hello!")
        );
        app.go(Route::Runs);
        assert_eq!(app.runs.len(), 1);
    }

    #[test]
    fn a_key_the_provider_rejects_fails_the_reply_with_a_reason_and_retry_asks_again() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (401, "application/json", "{}"),
                (200, "text/event-stream", STREAM),
            ],
        );
        app.composer = TextEdit::new("hello");
        app.send();
        let _ = seen.recv().unwrap();
        let Message::Assistant {
            outcome: Outcome::Failed(why),
            ..
        } = last(&app)
        else {
            panic!("failed");
        };
        assert!(why.contains("Box did not accept the key"), "{why}");
        app.go(Route::Runs);
        assert_eq!(app.runs[0].status, Status::Error);
        assert_eq!(app.steps.last().unwrap().kind, StepKind::Error);

        let before = app.current().unwrap().chat().messages.len();
        app.retry();
        assert!(matches!(
            last(&app),
            Message::Assistant {
                outcome: Outcome::Done,
                ..
            }
        ));
        assert_eq!(
            app.current().unwrap().chat().messages.len(),
            before,
            "the failed reply was replaced, not kept beside the new one"
        );
        drop(app);
        let app = db.open();
        assert!(matches!(
            last(&app),
            Message::Assistant {
                outcome: Outcome::Done,
                ..
            }
        ));
    }

    #[test]
    fn a_provider_with_no_saved_key_says_so_without_asking() {
        let mut app = crate::testing::samples();
        crate::testing::ready(&mut app);
        app.composer = TextEdit::new("hello");
        app.send();
        let Message::Assistant {
            outcome: Outcome::Failed(why),
            ..
        } = last(&app)
        else {
            panic!("failed");
        };
        assert!(why.contains("no saved key"), "{why}");
    }

    #[test]
    fn stopping_keeps_what_was_written_and_a_late_ending_changes_nothing() {
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![]);
        let agent = app.current().unwrap();
        let (sid, agent_id) = (agent.chat().id.clone(), agent.id.clone());
        let model = ModelRef::new("llama-server:box", "llama-3.3-70b");
        let run = Run {
            id: "run-1".into(),
            agent_id,
            agent_name: "Atlas".into(),
            session_id: sid.clone(),
            model,
            status: Status::Running,
            error: None,
            input_tokens: None,
            output_tokens: None,
            duration_ms: None,
            started_at: now(),
            ended_at: None,
            parent: None,
        };
        app.save("the run", |db| db.start_run(&run));
        app.roster
            .current_mut()
            .unwrap()
            .chat_mut()
            .messages
            .push(Message::reply("", Some("run-1".into()), Outcome::Working));
        app.live.insert(
            sid.clone(),
            Live {
                run,
                cancel: Arc::new(AtomicBool::new(false)),
                began: Instant::now(),
                system: None,
                user: "q".into(),
                user_name: None,
                sent: "[user]\nq".into(),
                reasoning: String::new(),
                thought: None,
            },
        );
        assert!(app.busy());
        app.apply(&sid, "run-1", vec![Event::Text("Part".into())]);
        app.apply(&sid, "other-run", vec![Event::Text("IGNORED".into())]);
        app.stop();
        assert!(!app.busy());
        assert!(
            matches!(last(&app), Message::Assistant { text, outcome: Outcome::Stopped, .. } if text == "Part")
        );

        app.settle(&sid, "run-1", Ok(()));
        app.apply(&sid, "run-1", vec![Event::Text("late".into())]);
        assert!(
            matches!(last(&app), Message::Assistant { text, outcome: Outcome::Stopped, .. } if text == "Part")
        );
        app.go(Route::Runs);
        assert_eq!(app.runs[0].status, Status::Cancelled);
    }

    #[test]
    fn nothing_is_sent_while_a_reply_is_still_being_written() {
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![]);
        let sid = app.current().unwrap().chat().id.clone();
        let run = Run {
            id: "r".into(),
            agent_id: app.current().unwrap().id.clone(),
            agent_name: "A".into(),
            session_id: sid.clone(),
            model: ModelRef::new("llama-server:box", "llama-3.3-70b"),
            status: Status::Running,
            error: None,
            input_tokens: None,
            output_tokens: None,
            duration_ms: None,
            started_at: 0,
            ended_at: None,
            parent: None,
        };
        app.live.insert(
            sid,
            Live {
                run,
                cancel: Arc::new(AtomicBool::new(false)),
                began: Instant::now(),
                system: None,
                user: String::new(),
                user_name: None,
                sent: String::new(),
                reasoning: String::new(),
                thought: None,
            },
        );
        let before = app.current().unwrap().chat().messages.len();
        app.composer = TextEdit::new("another");
        app.send();
        assert_eq!(app.current().unwrap().chat().messages.len(), before);
        assert_eq!(app.composer.text, "another", "and what was typed is kept");
    }

    #[test]
    fn tool_calls_and_their_answers_go_back_to_the_model_and_sample_calls_do_not() {
        let asked = Message::Assistant {
            text: "Let me check.".into(),
            thought_secs: None,
            tools: vec![
                ToolCall {
                    name: "ask_user_question".into(),
                    summary: "Q?".into(),
                    output: Some("Blue".into()),
                    id: "c1".into(),
                    args: "{}".into(),
                },
                ToolCall {
                    name: "storage.read".into(),
                    summary: "sample".into(),
                    ..Default::default()
                },
            ],
            approval: None,
            stats: None,
            run: None,
            outcome: Outcome::Done,
        };
        let turns = build_turns(&[user("hi"), asked, user("thanks")]);
        let roles: Vec<_> = turns.iter().map(|t| t.role).collect();
        assert_eq!(roles, [Role::User, Role::Assistant, Role::Tool, Role::User]);
        assert_eq!(
            turns[1].calls.len(),
            1,
            "the sample call has no id and is left out"
        );
        assert_eq!(turns[2].call_id.as_deref(), Some("c1"));
        assert_eq!(turns[2].text, "Blue");
    }

    const ASK_STREAM: &str = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"ask_user_question","arguments":"{\"question\":\"Which colour?\","}}]}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"options\":[\"Red\",\"Blue\"]}"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;

    #[test]
    fn the_agent_can_ask_a_question_wait_for_the_answer_and_carry_on_in_a_new_run() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (200, "text/event-stream", ASK_STREAM),
                (200, "text/event-stream", STREAM),
            ],
        );
        app.roster.current_mut().unwrap().tools = vec![crate::tools::ASK.into()];
        app.composer = TextEdit::new("paint it");
        app.send();
        let first = seen.recv().unwrap();
        assert!(
            first.contains("ask_user_question") && first.contains("\"tools\""),
            "offered the tool"
        );

        let Message::Assistant { tools, outcome, .. } = last(&app) else {
            panic!("a reply");
        };
        assert_eq!(outcome, &Outcome::Done);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].summary, "Which colour?");
        assert!(tools[0].output.is_none());
        assert!(app.awaiting_answer() && !app.busy());

        app.composer = TextEdit::new("anything");
        app.send();
        assert_eq!(
            app.composer.text, "anything",
            "nothing is sent until the question is answered"
        );

        app.answer("Blue");
        let second = seen.recv().unwrap();
        assert!(
            second.contains("\"role\":\"tool\"")
                && second.contains("call_1")
                && second.contains("blue"),
            "{second}"
        );
        assert!(!app.awaiting_answer());
        assert!(matches!(last(&app), Message::Assistant { text, .. } if text == "Hello!"));

        app.go(Route::Runs);
        assert_eq!(
            app.runs.len(),
            2,
            "the question and the carrying on are two runs"
        );
        let carried = &app.steps;
        let answer = carried.iter().find(|s| s.kind == StepKind::User).unwrap();
        assert_eq!(answer.name, "Answer to your question");
        assert_eq!(answer.output.as_deref(), Some("Blue"));
        app.select_run(1);
        let call = app.steps.iter().find(|s| s.kind == StepKind::Llm).unwrap();
        assert!(
            call.output
                .as_deref()
                .unwrap()
                .contains("[tool call] ask_user_question")
        );
        drop(app);

        let app = db.open();
        assert!(matches!(last(&app), Message::Assistant { text, .. } if text == "Hello!"));
    }

    #[test]
    fn a_skipped_question_tells_the_model_so_and_a_question_survives_a_restart() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (200, "text/event-stream", ASK_STREAM),
                (200, "text/event-stream", STREAM),
            ],
        );
        app.roster.current_mut().unwrap().tools = vec![crate::tools::ASK.into()];
        app.composer = TextEdit::new("paint it");
        app.send();
        let _ = seen.recv().unwrap();
        drop(app);

        let mut app = db.open();
        assert!(
            app.awaiting_answer(),
            "the question is still waiting after a restart"
        );
        app.skip_question();
        let second = seen.recv().unwrap();
        assert!(second.contains("skipped this question"), "{second}");
        assert!(!app.awaiting_answer());
    }

    /// A stream in which the model asks to run `command`.
    fn shell_stream(command: &str) -> &'static str {
        let args = serde_json::json!({"command": command, "reason": "test"}).to_string();
        let chunk = serde_json::json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "sh1", "function": {"name": "shell", "arguments": args}}]}, "finish_reason": "tool_calls"}]});
        Box::leak(format!("data: {chunk}\n\ndata: [DONE]\n\n").into_boxed_str())
    }

    fn marker() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("oido-shell-test-{}", uuid::Uuid::new_v4()))
    }

    #[cfg(unix)]
    #[test]
    fn a_command_runs_as_soon_as_it_is_asked_for_and_its_output_goes_back_to_the_model() {
        let file = marker();
        let command = format!("echo ran-it > {}; echo done-out", file.display());
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (200, "text/event-stream", shell_stream(&command)),
                (200, "text/event-stream", STREAM),
            ],
        );
        app.roster.current_mut().unwrap().tools = vec![crate::tools::SHELL.into()];
        app.composer = TextEdit::new("do it");
        app.send();
        let first = seen.recv().unwrap();
        assert!(first.contains("\"name\":\"shell\""), "offered the tool");

        assert_eq!(
            std::fs::read_to_string(&file).unwrap().trim(),
            "ran-it",
            "it ran without being asked twice"
        );
        let second = seen.recv().unwrap();
        assert!(
            second.contains("sh1")
                && second.contains("exit code: 0")
                && second.contains("done-out"),
            "{second}"
        );
        assert!(!app.awaiting_answer() && app.commands.is_empty() && !app.busy());
        assert!(matches!(last(&app), Message::Assistant { text, .. } if text == "Hello!"));

        app.go(Route::Runs);
        let result = app.steps.iter().find(|s| s.kind == StepKind::User).unwrap();
        assert_eq!(result.name, "Tool result");
        assert!(result.output.as_deref().unwrap().contains("done-out"));
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn a_call_with_no_command_runs_nothing_and_says_so() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (200, "text/event-stream", shell_stream("   ")),
                (200, "text/event-stream", STREAM),
            ],
        );
        app.roster.current_mut().unwrap().tools = vec![crate::tools::SHELL.into()];
        app.composer = TextEdit::new("do it");
        app.send();
        let _ = seen.recv().unwrap();
        let second = seen.recv().unwrap();
        assert!(second.contains("no command was given"), "{second}");
    }

    #[test]
    fn a_command_still_waiting_when_the_app_closed_is_marked_not_run_and_never_started() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        let pending = Message::Assistant {
            text: "Running it.".into(),
            thought_secs: None,
            tools: vec![ToolCall {
                name: "shell".into(),
                summary: "touch x".into(),
                output: None,
                id: "c1".into(),
                args: r#"{"command":"touch x"}"#.into(),
            }],
            approval: None,
            stats: None,
            run: None,
            outcome: Outcome::Done,
        };
        let sid = {
            let agent = app.roster.current_mut().unwrap();
            agent.chat_mut().messages.push(pending.clone());
            agent.chat().id.clone()
        };
        let seq = app.current().unwrap().chat().messages.len() - 1;
        app.save("the reply", |db| db.put_message(&sid, seq, &pending));
        drop(app);

        let app = db.open();
        assert!(!app.awaiting_answer(), "the chat is not left waiting");
        let Message::Assistant { tools, .. } = last(&app) else {
            panic!("the reply");
        };
        assert!(
            tools[0]
                .output
                .as_deref()
                .unwrap()
                .contains("app was closed")
        );
        assert!(app.commands.is_empty());
    }

    #[test]
    fn an_agent_without_the_shell_tool_is_not_offered_it() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(&db, vec![(200, "text/event-stream", STREAM)]);
        app.roster.current_mut().unwrap().tools = vec![crate::tools::ASK.into()];
        app.composer = TextEdit::new("hi");
        app.send();
        let request = seen.recv().unwrap();
        assert!(request.contains("ask_user_question") && !request.contains("\"shell\""));
    }

    #[cfg(unix)]
    #[test]
    fn a_command_runs_in_the_agents_chosen_folder_and_says_so_if_it_is_gone() {
        let folder = std::env::temp_dir().join(format!("oido-wd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder).unwrap();
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (200, "text/event-stream", shell_stream("pwd")),
                (200, "text/event-stream", STREAM),
                (200, "text/event-stream", shell_stream("pwd")),
                (200, "text/event-stream", STREAM),
            ],
        );
        let agent = app.roster.current_mut().unwrap();
        agent.tools = vec![crate::tools::SHELL.into()];
        agent.workdir = Some(folder.to_string_lossy().into_owned());
        app.composer = TextEdit::new("where am i");
        app.send();
        let _ = seen.recv().unwrap();
        let second = seen.recv().unwrap();
        let name = folder.file_name().unwrap().to_string_lossy().to_lowercase();
        assert!(second.contains(&name), "ran in the chosen folder: {second}");

        std::fs::remove_dir_all(&folder).unwrap();
        app.composer = TextEdit::new("again");
        app.send();
        let _ = seen.recv().unwrap();
        let third = seen.recv().unwrap();
        assert!(third.contains("does not exist, so nothing ran"), "{third}");
    }

    /// A stream in which the model calls the tool `name` with `args`.
    fn call_stream(name: &str, args: serde_json::Value) -> &'static str {
        let chunk = serde_json::json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "f1", "function": {"name": name, "arguments": args.to_string()}}]}, "finish_reason": "tool_calls"}]});
        Box::leak(format!("data: {chunk}\n\ndata: [DONE]\n\n").into_boxed_str())
    }

    fn temp_folder() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("oido-ft-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    #[test]
    fn file_tools_run_at_once_in_the_working_folder_and_the_result_goes_back() {
        let folder = temp_folder();
        std::fs::write(folder.join("notes.txt"), "alpha beta").unwrap();
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (
                    200,
                    "text/event-stream",
                    call_stream(
                        "edit_file",
                        serde_json::json!({"path": "notes.txt", "old_text": "beta", "new_text": "gamma"}),
                    ),
                ),
                (200, "text/event-stream", STREAM),
            ],
        );
        let agent = app.roster.current_mut().unwrap();
        agent.tools = vec![crate::files::EDIT.into()];
        agent.workdir = Some(folder.to_string_lossy().into_owned());
        app.composer = TextEdit::new("change it");
        app.send();
        let first = seen.recv().unwrap();
        assert!(first.contains("\"edit_file\""), "offered the tool");
        assert_eq!(
            std::fs::read_to_string(folder.join("notes.txt")).unwrap(),
            "alpha gamma"
        );
        let second = seen.recv().unwrap();
        assert!(
            second.contains("replaced 1 occurrence in notes.txt"),
            "{second}"
        );
        assert!(
            !app.awaiting_answer() && !app.busy(),
            "no go-ahead was needed"
        );
        assert!(matches!(last(&app), Message::Assistant { text, .. } if text == "Hello!"));
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_file_tool_cannot_reach_outside_and_needs_a_working_folder() {
        let folder = temp_folder();
        let outside = folder
            .parent()
            .unwrap()
            .join(format!("oido-escape-{}", uuid::Uuid::new_v4()));
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (
                    200,
                    "text/event-stream",
                    call_stream(
                        "write_file",
                        serde_json::json!({"path": format!("../{}", outside.file_name().unwrap().to_string_lossy()), "content": "x"}),
                    ),
                ),
                (200, "text/event-stream", STREAM),
                (
                    200,
                    "text/event-stream",
                    call_stream("read_file", serde_json::json!({"path": "a.txt"})),
                ),
                (200, "text/event-stream", STREAM),
            ],
        );
        let agent = app.roster.current_mut().unwrap();
        agent.tools = vec![crate::files::WRITE.into(), crate::files::READ.into()];
        agent.workdir = Some(folder.to_string_lossy().into_owned());
        app.composer = TextEdit::new("write it");
        app.send();
        let _ = seen.recv().unwrap();
        let second = seen.recv().unwrap();
        assert!(second.contains("outside the working folder"), "{second}");
        assert!(!outside.exists(), "nothing was written outside");

        app.roster.current_mut().unwrap().workdir = None;
        app.composer = TextEdit::new("read it");
        app.send();
        let _ = seen.recv().unwrap();
        let fourth = seen.recv().unwrap();
        assert!(fourth.contains("no working folder"), "{fourth}");
        std::fs::remove_dir_all(folder).unwrap();
    }

    /// A stream in which the model hands `task` to `agent`.
    fn delegate_stream(agent: &str, task: &str) -> &'static str {
        let args = serde_json::json!({"agent": agent, "task": task}).to_string();
        let chunk = serde_json::json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "dg1", "function": {"name": "delegate", "arguments": args}}]}, "finish_reason": "tool_calls"}]});
        Box::leak(format!("data: {chunk}\n\ndata: [DONE]\n\n").into_boxed_str())
    }

    fn with_team(app: &mut App) {
        let lead = app.roster.current_mut().unwrap();
        let model = lead.model.clone();
        lead.subagents.push(crate::agents::Agent {
            model,
            name: "checker".into(),
            prompt: "You check sums.".into(),
            description: "Checks arithmetic.".into(),
            ..crate::agents::Agent::blank("m1".into())
        });
    }

    #[test]
    fn a_lead_hands_a_task_to_a_team_member_that_starts_fresh_and_its_answer_comes_back() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (
                    200,
                    "text/event-stream",
                    delegate_stream("Checker", "verify 2+2"),
                ),
                (200, "text/event-stream", STREAM),
                (200, "text/event-stream", STREAM),
            ],
        );
        with_team(&mut app);
        app.composer = TextEdit::new("do the sums");
        app.send();
        let lead = seen.recv().unwrap();
        assert!(
            lead.contains("\"name\":\"delegate\"") && lead.contains("checks arithmetic."),
            "the lead is offered the team: {lead}"
        );
        let member = seen.recv().unwrap();
        assert!(
            member.contains("verify 2+2") && member.contains("you check sums."),
            "{member}"
        );
        assert!(
            !member.contains("do the sums") && !member.contains("\"name\":\"delegate\""),
            "it sees only its task, and cannot hand work on"
        );
        let back = seen.recv().unwrap();
        assert!(back.contains("dg1") && back.contains("hello!"), "{back}");
        assert!(!app.busy() && app.delegating.is_empty());
        let card = app.delegated.get("dg1").unwrap();
        assert!(card.ended.is_some() && card.input + card.output == 13);
        assert!(
            app.roster.agent_ref("m1").unwrap().status == AgentStatus::Idle,
            "the member is idle again"
        );
        assert!(matches!(last(&app), Message::Assistant { text, .. } if text == "Hello!"));

        let runs = db.open().store.as_ref().unwrap().list_runs(10).unwrap();
        let child = runs.iter().find(|r| r.agent_name == "checker").unwrap();
        let lead_run = runs
            .iter()
            .find(|r| Some(&r.id) == child.parent.as_ref())
            .unwrap();
        assert_eq!(child.status, Status::Ok);
        let own = |r: &Run| r.input_tokens.unwrap_or(0) + r.output_tokens.unwrap_or(0);
        assert!(
            own(child) == 13 && own(lead_run) == 13,
            "the lead's run (which reported none of its own) counts its team member's tokens"
        );
        let lead_id = lead_run.id.clone();
        let shown = crate::runs::nest(runs);
        let at = |f: &dyn Fn(&Run) -> bool| shown.iter().position(f).unwrap();
        assert_eq!(
            at(&|r| r.agent_name == "checker"),
            at(&|r| r.id == lead_id) + 1,
            "a handed-over run is listed right under the run that handed it over"
        );
    }

    #[test]
    fn a_task_for_someone_not_on_the_team_comes_back_as_an_error_the_lead_can_act_on() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (200, "text/event-stream", delegate_stream("nobody", "x")),
                (200, "text/event-stream", STREAM),
            ],
        );
        with_team(&mut app);
        app.composer = TextEdit::new("go");
        app.send();
        seen.recv().unwrap();
        let back = seen.recv().unwrap();
        assert!(
            back.contains("no team member called") && back.contains("checker"),
            "{back}"
        );
        assert!(!app.busy());
    }
}
