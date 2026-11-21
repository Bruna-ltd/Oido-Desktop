//! The hooks in the app: the calls a reply asks for go past the before-tool hooks once the
//! guardrails let them through, and each result goes past the after-tool hooks before the model
//! sees it. Hooks run off the UI thread; the chat is busy meanwhile.

use std::path::{Path, PathBuf};

use super::{Event, Fail, Hook, OFF, SAVED, Verdict, after_event, base_event, before_event, run};
use crate::app::App;
use crate::guard::{Action, Hidden, Trip, TripState, title};
use crate::model::{AgentStatus, Message};

/// What the before-tool hooks decided about one call.
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Before {
    /// The arguments to run it with, when a hook changed them.
    pub args: Option<String>,
    pub stop: Option<Stop>,
    /// What to say in the run: a failed hook, a change.
    pub notes: Vec<(String, String)>,
}

/// How a hook stopped a call.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    /// It does not run.
    Block(String),
    /// It waits for your go-ahead.
    Ask(String),
}

/// What the after-tool hooks made of one result.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct After {
    /// What the model gets: the result, changed or withheld by a hook, or as it was.
    pub result: String,
    /// What the call's result was before any hook saw it.
    pub original: String,
    pub notes: Vec<(String, String)>,
}

/// What the model is told in place of a result a hook withheld.
fn withheld(reason: &str) -> String {
    format!(
        "Result withheld: a hook the person set refused it. {reason} Tell the person, and do not try to get around it."
    )
}

fn failed(hook: &Hook, why: &str) -> (String, String) {
    let go = match hook.fail {
        Fail::Open => "the call went on",
        Fail::Closed => "the call was stopped",
    };
    (
        format!("Hook {} failed", hook.label()),
        format!("{why}; {go}."),
    )
}

/// Runs `hooks` before tool `tool` with `args`, in order. Off the UI thread.
pub(crate) fn before(
    hooks: &[Hook],
    (chat, agent): (&str, &str),
    dir: &Path,
    tool: &str,
    args: &str,
) -> Before {
    let mut out = Before::default();
    let mut now = args.to_owned();
    for h in hooks {
        let event = before_event(chat, agent, dir, tool, &now);
        let answer = run(h, &event, dir).and_then(|v| match v {
            Verdict::Replace(a) if serde_json::from_str::<serde_json::Value>(&a).is_err() => {
                Err("\"args\" is not JSON".to_owned())
            }
            v => Ok(v),
        });
        match answer {
            Ok(Verdict::Allow) => {}
            Ok(Verdict::Block(why)) => {
                out.stop = Some(Stop::Block(format!("{}: {why}", h.label())));
                break;
            }
            Ok(Verdict::Ask(why)) => {
                if out.stop.is_none() {
                    out.stop = Some(Stop::Ask(format!("{}: {why}", h.label())));
                }
            }
            Ok(Verdict::Replace(a)) => {
                out.notes.push((
                    format!("Hook {} changed the arguments", h.label()),
                    a.clone(),
                ));
                now = a;
            }
            Err(why) => {
                out.notes.push(failed(h, &why));
                if h.fail == Fail::Closed {
                    out.stop = Some(Stop::Block(format!("{} failed: {why}", h.label())));
                    break;
                }
            }
        }
    }
    out.args = (now != args).then_some(now);
    out
}

/// Runs `hooks` over `result` of tool `tool`, in order. Off the UI thread.
pub(crate) fn after(
    hooks: &[Hook],
    (chat, agent): (&str, &str),
    dir: &Path,
    (tool, args): (&str, &str),
    result: String,
) -> After {
    let original = result.clone();
    let mut now = result;
    let mut notes = Vec::new();
    for h in hooks {
        let event = after_event(chat, agent, dir, tool, args, &now);
        match run(h, &event, dir) {
            Ok(Verdict::Allow | Verdict::Ask(_)) => {}
            Ok(Verdict::Block(why)) => {
                notes.push((
                    format!("Hook {} withheld the result", h.label()),
                    why.clone(),
                ));
                now = withheld(&format!("{}: {why}", h.label()));
                break;
            }
            Ok(Verdict::Replace(text)) => {
                notes.push((
                    format!("Hook {} changed the result", h.label()),
                    text.clone(),
                ));
                now = text;
            }
            Err(why) => {
                notes.push(failed(h, &why));
                if h.fail == Fail::Closed {
                    now = withheld(&format!("{} failed: {why}", h.label()));
                    break;
                }
            }
        }
    }
    After {
        result: now,
        original,
        notes,
    }
}

/// The tool hooks for work nobody watches (a team member, a workflow step): worked out on the UI
/// thread, then used on the worker thread, where waiting for a hook blocks nothing.
#[derive(Clone, Default)]
pub(crate) struct Worker {
    before: Vec<Hook>,
    after: Vec<Hook>,
    agent: String,
    dir: PathBuf,
}

fn matching(list: &[Hook], tool: &str) -> Vec<Hook> {
    list.iter().filter(|h| h.is_for(tool)).cloned().collect()
}

impl Worker {
    /// The hooks that are on for agent `id`, working in `dir`.
    pub(crate) fn new(hooks: &super::Hooks, id: &str, agent: &str, dir: PathBuf) -> Worker {
        let (before, after) = if hooks.runs_for(id) {
            (
                hooks.for_event(Event::BeforeTool, ""),
                hooks.for_event(Event::AfterTool, ""),
            )
        } else {
            Default::default()
        };
        Worker {
            before,
            after,
            agent: agent.to_owned(),
            dir,
        }
    }

    /// Runs the before-tool hooks over a call the guardrails let through. They may change `args`,
    /// and what they change is looked at by the guardrails again. Returns what the model is told
    /// when the call must not run: a hook refused it, or held it and you said no (the card goes
    /// through `asker`). `notes` gets what to say in the run.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn before(
        &self,
        asker: &crate::guard::ask::Asker,
        (guard, place, outside): (&crate::guard::Guard, &crate::guard::Place, Option<&str>),
        tool: &str,
        args: &mut String,
        stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        notes: &mut Vec<(String, String)>,
    ) -> Option<String> {
        let hooks = matching(&self.before, tool);
        if hooks.is_empty() {
            return None;
        }
        let b = before(&hooks, ("", &self.agent), &self.dir, tool, args);
        notes.extend(b.notes);
        if let Some(changed) = b.args {
            *args = changed;
            // A hook may change what runs, never what the guardrails allow.
            let (_, found) =
                guard.review(&crate::guard::Hidden::default(), tool, args, place, outside);
            if let Some(f) = found {
                let (state, told) = crate::guard::unattended(asker, tool, args, &f, stop);
                notes.push((format!("{}: {state:?}", crate::guard::title(f.rule)), f.why));
                if told.is_some() {
                    return told;
                }
            }
        }
        let (rule, state, why) = match b.stop? {
            Stop::Block(why) => ("hook", TripState::Blocked, why),
            Stop::Ask(why) => {
                let f = crate::guard::actions::Finding {
                    rule: "hook",
                    why,
                    action: crate::guard::Action::Ask,
                };
                let (state, told) = crate::guard::unattended(asker, tool, args, &f, stop);
                notes.push((format!("Hook: {state:?}"), f.why));
                return told;
            }
        };
        notes.push(("Hook: refused".to_owned(), why.clone()));
        Some(
            Trip {
                rule: rule.to_owned(),
                why,
                state,
                asked_at: None,
            }
            .told(),
        )
    }

    /// Runs the after-tool hooks over `result` of a call that ran.
    pub(crate) fn after(
        &self,
        tool: &str,
        args: &str,
        result: String,
        notes: &mut Vec<(String, String)>,
    ) -> String {
        let hooks = matching(&self.after, tool);
        if hooks.is_empty() {
            return result;
        }
        let done = after(&hooks, ("", &self.agent), &self.dir, (tool, args), result);
        notes.extend(done.notes);
        done.result
    }
}

/// A call waiting in the chat's last reply, as a hook job needs it.
struct Pending {
    id: String,
    name: String,
    args: String,
    hooks: Vec<Hook>,
}

impl App {
    /// Sets the hooks, and saves them.
    pub fn set_hooks(&mut self, list: Vec<Hook>) {
        self.hooks.set(list);
        let saved = self.hooks.list().to_vec();
        self.save("the hooks", |db| db.set_setting(SAVED, &saved));
    }

    /// Saves the form as a hook: a new one at the end, or in place of the one it was filled from
    /// (keeping whether it is on). The form is cleared.
    pub fn save_hook_form(&mut self) {
        if !self.hook_form.ready() {
            return;
        }
        let mut list = self.hooks.list().to_vec();
        let at = self
            .hook_form
            .id
            .as_ref()
            .and_then(|id| list.iter().position(|h| &h.id == id));
        let hook = self.hook_form.hook(at.is_none_or(|i| list[i].on));
        match at {
            Some(i) => list[i] = hook,
            None => list.push(hook),
        }
        self.set_hooks(list);
        self.hook_form = super::Form::default();
    }

    /// Fills the form with hook `id`, to change it.
    pub fn edit_hook(&mut self, id: &str) {
        if let Some(h) = self.hooks.list().iter().find(|h| h.id == id) {
            self.hook_form = super::Form::of(h);
        }
    }

    /// Switches hook `id` on or off.
    pub fn toggle_hook(&mut self, id: &str) {
        let mut list = self.hooks.list().to_vec();
        if let Some(h) = list.iter_mut().find(|h| h.id == id) {
            h.on = !h.on;
            self.set_hooks(list);
        }
    }

    /// Removes hook `id`.
    pub fn delete_hook(&mut self, id: &str) {
        let list: Vec<Hook> = self
            .hooks
            .list()
            .iter()
            .filter(|h| h.id != id)
            .cloned()
            .collect();
        self.set_hooks(list);
        if self.hook_form.id.as_deref() == Some(id) {
            self.hook_form = super::Form::default();
        }
    }

    /// The hooks to run for `event` (and tool `tool`) in chat `sid`: none when its agent is switched
    /// out of them.
    fn hooks_for(&self, sid: &str, event: Event, tool: &str) -> Vec<Hook> {
        match self.roster.owner_of(sid) {
            Some(a) if self.hooks.runs_for(&a.id) => self.hooks.for_event(event, tool),
            _ => Vec::new(),
        }
    }

    /// The tool hooks for work `agent` does unattended, in `dir`.
    pub(crate) fn hooks_worker(&self, agent: &crate::agents::Agent, dir: PathBuf) -> Worker {
        Worker::new(&self.hooks, &agent.id, &agent.name, dir)
    }

    /// Switches agent `id` in or out of running hooks, and saves it.
    pub fn set_hooks_for(&mut self, id: &str, runs: bool) {
        self.hooks.set_runs_for(id, runs);
        let off = self.hooks.off().to_vec();
        self.save("the hooks", |db| db.set_setting(OFF, &off));
    }

    /// Tells the hooks for `event` in chat `sid` what happened (`data`, fields added to the common
    /// ones). They only watch: they run off the UI thread, nothing waits for them, and a failure
    /// is noted in the run.
    pub(crate) fn fire(&mut self, sid: &str, event: Event, data: serde_json::Value) {
        let hooks = self.hooks_for(sid, event, "");
        if hooks.is_empty() {
            return;
        }
        let (agent, dir) = self.hook_scene(sid);
        let mut payload = base_event(event, sid, &agent, &dir);
        if let (Some(into), serde_json::Value::Object(from)) = (payload.as_object_mut(), data) {
            into.extend(from);
        }
        let chat = sid.to_owned();
        self.spawn(
            move || {
                hooks
                    .iter()
                    .filter_map(|h| {
                        run(h, &payload, &dir)
                            .err()
                            .map(|why| (format!("Hook {} failed", h.label()), format!("{why}.")))
                    })
                    .collect::<Vec<_>>()
            },
            move |app, notes| app.hook_notes(&chat, &notes),
        );
    }

    /// Tells the before-model hooks what chat `sid`'s next model call is sent, as the model sees it.
    pub(crate) fn fire_before_model(&mut self, sid: &str, request: &crate::completion::Request) {
        if self.hooks_for(sid, Event::BeforeModel, "").is_empty() {
            return;
        }
        let turns: Vec<_> = request
            .turns
            .iter()
            .map(|t| serde_json::json!({"role": format!("{:?}", t.role).to_lowercase(), "text": t.text}))
            .collect();
        let tools: Vec<&str> = request.tools.iter().map(|t| t.name.as_str()).collect();
        self.fire(
            sid,
            Event::BeforeModel,
            serde_json::json!({"model": request.model, "system": request.system, "turns": turns, "tools": tools}),
        );
    }

    /// Who is asking and where, as a hook is told.
    fn hook_scene(&self, sid: &str) -> (String, PathBuf) {
        let agent = self
            .roster
            .owner_of(sid)
            .map_or_else(String::new, |a| a.name.clone());
        (agent, self.workdir_of(sid))
    }

    /// Notes in chat `sid`'s run what hooks did.
    fn hook_notes(&mut self, sid: &str, notes: &[(String, String)]) {
        let steps: Vec<_> = match self.traces.get_mut(sid) {
            Some(trace) => notes.iter().map(|(n, t)| trace.guard(n, t)).collect(),
            None => Vec::new(),
        };
        self.record(sid, &steps);
    }

    /// Notes that a hook job for chat `sid` is out, and says so on its status.
    fn hooking_begin(&mut self, sid: &str) {
        *self.hooking.entry(sid.to_owned()).or_default() += 1;
        self.refresh_status_of(sid);
    }

    /// Notes that a hook job for chat `sid` is back. False when the chat was stopped meanwhile: what
    /// the job found no longer matters.
    fn hooking_end(&mut self, sid: &str) -> bool {
        let Some(n) = self.hooking.get_mut(sid) else {
            return false;
        };
        *n -= 1;
        if *n == 0 {
            self.hooking.remove(sid);
        }
        true
    }

    /// The chat was stopped: calls still waiting for the hooks never run.
    pub(crate) fn stop_hooking(&mut self, sid: &str) {
        if self.hooking.remove(sid).is_none() {
            return;
        }
        let waiting: Vec<String> = match self
            .roster
            .owner_of(sid)
            .and_then(|a| a.sessions.iter().find(|s| s.id == sid))
            .and_then(|s| s.messages.last())
        {
            Some(Message::Assistant { tools, .. }) => tools
                .iter()
                .filter(|t| {
                    t.output.is_none()
                        && t.trip.as_ref().is_none_or(|t| t.state != TripState::Asking)
                })
                .map(|t| t.id.clone())
                .collect(),
            _ => Vec::new(),
        };
        for id in waiting {
            self.resolve(
                sid,
                &id,
                "Not run: the person stopped this before it began.".to_owned(),
                false,
            );
        }
    }

    /// Runs the calls chat `sid`'s last reply asked for that the guardrails let through. First the
    /// before-tool hooks look at the ones they are for, off the UI thread; then the calls run, minus
    /// any a hook refused, and held for your go-ahead if a hook asks for it.
    pub(crate) fn hook_then_dispatch(&mut self, sid: &str) {
        let pending = self.pending_for_hooks(sid);
        if pending.is_empty() {
            self.dispatch_calls(sid);
            return;
        }
        self.hooking_begin(sid);
        let (agent, dir) = self.hook_scene(sid);
        let chat = sid.to_owned();
        self.spawn(
            {
                let chat = chat.clone();
                move || {
                    pending
                        .into_iter()
                        .map(|p| {
                            let b = before(&p.hooks, (&chat, &agent), &dir, &p.name, &p.args);
                            (p.id, b)
                        })
                        .collect::<Vec<_>>()
                }
            },
            move |app, decided| app.hooks_decided(&chat, decided),
        );
    }

    /// The waiting calls in chat `sid`'s last reply some before-tool hook is for, each only once.
    fn pending_for_hooks(&mut self, sid: &str) -> Vec<Pending> {
        let Some(Message::Assistant { tools, .. }) = self
            .roster
            .owner_of(sid)
            .and_then(|a| a.sessions.iter().find(|s| s.id == sid))
            .and_then(|s| s.messages.last())
        else {
            return Vec::new();
        };
        let found: Vec<Pending> = tools
            .iter()
            .filter(|t| {
                t.output.is_none()
                    && !t.id.is_empty()
                    && !self.hooked.contains(&t.id)
                    && t.trip.as_ref().is_none_or(|t| !t.state.stopped())
            })
            .map(|t| Pending {
                id: t.id.clone(),
                name: t.name.clone(),
                args: t.args.clone(),
                hooks: self.hooks_for(sid, Event::BeforeTool, &t.name),
            })
            .filter(|p| !p.hooks.is_empty())
            .collect();
        self.hooked.extend(found.iter().map(|p| p.id.clone()));
        found
    }

    /// The before-tool hooks have looked at calls in chat `sid`: apply what they decided, and run
    /// what is left unless a call is held for your go-ahead.
    fn hooks_decided(&mut self, sid: &str, decided: Vec<(String, Before)>) {
        // Stopped meanwhile: the calls were answered then.
        if !self.hooking_end(sid) {
            return;
        }
        let place = self.place_of(sid);
        let mut stopped: Vec<(String, Trip)> = Vec::new();
        let mut notes = Vec::new();
        let mut saved = None;
        if let Some(chat) = self
            .roster
            .owner_of_mut(sid)
            .and_then(|a| a.sessions.iter_mut().find(|s| s.id == sid))
            && let Some(seq) = chat.messages.len().checked_sub(1)
            && let Some(Message::Assistant { tools, .. }) = chat.messages.last_mut()
        {
            for (id, b) in decided {
                let Some(call) = tools.iter_mut().find(|t| t.id == id && t.output.is_none()) else {
                    continue;
                };
                notes.extend(b.notes);
                let mut stop = b.stop;
                if let Some(args) = b.args {
                    call.summary = crate::tools::summary(&call.name, &args);
                    call.args = args;
                    // A hook may change what runs, never what the guardrails allow: the new
                    // arguments are looked at again, and a hook's own refusal or hold stands.
                    let outside = self.tainted.get(sid).map(String::as_str);
                    let (_, found) = self.guard.review(
                        &Hidden::default(),
                        &call.name,
                        &call.args,
                        &place,
                        outside,
                    );
                    if let Some(f) = found {
                        let blocked = matches!(stop, Some(Stop::Block(_)));
                        if !blocked {
                            notes.push((
                                format!("{}: after a hook changed the call", title(f.rule)),
                                f.why.clone(),
                            ));
                            let why = format!("{} ({})", f.why, title(f.rule));
                            stop = Some(match f.action {
                                Action::Block => Stop::Block(why),
                                Action::Ask => Stop::Ask(why),
                            });
                        }
                    }
                }
                let (state, why) = match stop {
                    Some(Stop::Block(why)) => (TripState::Blocked, why),
                    Some(Stop::Ask(why)) => (TripState::Asking, why),
                    None => continue,
                };
                let trip = Trip {
                    rule: "hook".to_owned(),
                    why,
                    state,
                    asked_at: (state == TripState::Asking).then(crate::app::now),
                };
                notes.push((
                    format!(
                        "Hook: {}",
                        if state == TripState::Blocked {
                            "refused"
                        } else {
                            "waiting for your go-ahead"
                        }
                    ),
                    format!("{}\n\n{}", trip.why, call.summary),
                ));
                call.trip = Some(trip.clone());
                stopped.push((id, trip));
            }
            saved = Some((chat.id.clone(), seq, chat.messages[seq].clone()));
        }
        if let Some((id, seq, m)) = saved {
            self.save("the reply", |db| db.put_message(&id, seq, &m));
        }
        self.hook_notes(sid, &notes);
        let held = stopped.iter().any(|(_, t)| t.state == TripState::Asking);
        if held && let Some(agent) = self.roster.owner_of_mut(sid) {
            agent.status = AgentStatus::Waiting;
        }
        for (id, trip) in stopped {
            if trip.state == TripState::Blocked {
                self.resolve(sid, &id, trip.told(), true);
            }
        }
        self.refresh_status_of(sid);
        if !held {
            self.dispatch_calls(sid);
        }
    }

    /// Gives call `call_id` in chat `sid` its result, once the after-tool hooks have seen it. A
    /// result nothing refused or ran (the person stopped it, a refusal) goes straight in.
    pub(crate) fn resolve(&mut self, sid: &str, call_id: &str, output: String, carry_on: bool) {
        let Some((name, args, hooks)) = carry_on
            .then(|| self.after_hooks_for(sid, call_id))
            .flatten()
        else {
            self.resolve_now(sid, call_id, output, carry_on);
            return;
        };
        self.hooking_begin(sid);
        let (agent, dir) = self.hook_scene(sid);
        let chat = sid.to_owned();
        let call = call_id.to_owned();
        self.spawn(
            {
                let chat = chat.clone();
                move || after(&hooks, (&chat, &agent), &dir, (&name, &args), output)
            },
            move |app, done| {
                // Stopped meanwhile: the result is dropped for what the call gave, and no run follows.
                let alive = app.hooking_end(&chat);
                if alive {
                    app.hook_notes(&chat, &done.notes);
                }
                let result = if alive { done.result } else { done.original };
                app.resolve_now(&chat, &call, result, alive);
            },
        );
    }

    /// The tool and the after-tool hooks for waiting call `call_id` in chat `sid`, if any are for it.
    /// A call that was refused never ran, so no result of it is looked at.
    fn after_hooks_for(&self, sid: &str, call_id: &str) -> Option<(String, String, Vec<Hook>)> {
        let Some(Message::Assistant { tools, .. }) = self
            .roster
            .owner_of(sid)?
            .sessions
            .iter()
            .find(|s| s.id == sid)?
            .messages
            .last()
        else {
            return None;
        };
        let call = tools
            .iter()
            .find(|t| t.id == call_id && t.output.is_none())?;
        if call.trip.as_ref().is_some_and(|t| t.state.stopped()) {
            return None;
        }
        let hooks = self.hooks_for(sid, Event::AfterTool, &call.name);
        (!hooks.is_empty()).then(|| (call.name.clone(), call.args.clone(), hooks))
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn hook(event: Event, command: &str) -> Hook {
        let mut h = Hook::new(event);
        h.name = "h".into();
        h.command = command.to_owned();
        h
    }

    fn tmp() -> PathBuf {
        PathBuf::from("/tmp")
    }

    fn ask(hooks: &[Hook], args: &str) -> Before {
        before(hooks, ("c", "a"), &tmp(), "shell", args)
    }

    #[test]
    fn a_refusal_stops_the_rest() {
        let b = ask(
            &[
                hook(Event::BeforeTool, "exit 2"),
                hook(
                    Event::BeforeTool,
                    r#"echo '{"action":"replace","args":{"x":1}}'"#,
                ),
            ],
            "{}",
        );
        assert!(matches!(b.stop, Some(Stop::Block(_))));
        assert_eq!(b.args, None);
    }

    #[test]
    fn a_change_reaches_the_next_hook() {
        let b = ask(
            &[
                hook(
                    Event::BeforeTool,
                    r#"echo '{"action":"replace","args":{"command":"ls"}}'"#,
                ),
                hook(Event::BeforeTool, r#"grep -q '"command":"ls"' || exit 2"#),
            ],
            r#"{"command":"rm -rf x"}"#,
        );
        assert_eq!(b.stop, None);
        assert_eq!(b.args.as_deref(), Some(r#"{"command":"ls"}"#));
    }

    #[test]
    fn a_failing_hook_lets_the_call_go_unless_it_fails_closed() {
        let open = hook(Event::BeforeTool, "exit 9");
        let b = ask(std::slice::from_ref(&open), "{}");
        assert_eq!(b.stop, None);
        assert_eq!(b.notes.len(), 1);
        let mut closed = open;
        closed.fail = Fail::Closed;
        assert!(matches!(ask(&[closed], "{}").stop, Some(Stop::Block(_))));
    }

    #[test]
    fn a_replacement_that_is_not_json_counts_as_a_failure() {
        let b = ask(
            &[hook(
                Event::BeforeTool,
                r#"echo '{"action":"replace","args":"nope"}'"#,
            )],
            "{}",
        );
        assert_eq!((b.stop, b.args), (None, None));
        assert_eq!(b.notes.len(), 1);
    }

    #[test]
    fn results_are_replaced_or_withheld() {
        let args = ("shell", "{}");
        let changed = after(
            &[hook(
                Event::AfterTool,
                r#"echo '{"action":"replace","result":"short"}'"#,
            )],
            ("c", "a"),
            &tmp(),
            args,
            "a very long result".into(),
        );
        assert_eq!(
            (changed.result.as_str(), changed.original.as_str()),
            ("short", "a very long result")
        );
        let held = after(
            &[hook(Event::AfterTool, "echo no >&2; exit 2")],
            ("c", "a"),
            &tmp(),
            args,
            "secret".into(),
        );
        assert!(held.result.starts_with("Result withheld") && !held.result.contains("secret"));
    }

    use rune::TextEdit;
    use serde_json::json;

    use crate::testing::{EVENTS, TempDb, app_with_server, text_stream};

    fn calling_shell(command: &str) -> &'static str {
        let args = json!({"command": command}).to_string();
        let chunk = json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "h1", "function": {"name": crate::tools::SHELL, "arguments": args}}]}, "finish_reason": "tool_calls"}]});
        Box::leak(format!("data: {chunk}\n\ndata: [DONE]\n\n").into_boxed_str())
    }

    /// An agent with the shell tool, hooks set, and a chat that asks it to run `command`; the
    /// requests the model was sent come on the receiver.
    fn chat_with(
        db: &TempDb,
        hooks: Vec<Hook>,
        command: &str,
        dir: &Path,
    ) -> (App, std::sync::mpsc::Receiver<String>) {
        let (mut app, seen) = app_with_server(
            db,
            vec![
                (200, EVENTS, calling_shell(command)),
                (200, EVENTS, text_stream("Done.")),
            ],
        );
        app.set_hooks(hooks);
        let agent = app.roster.current_mut().unwrap();
        agent.tools = vec![crate::tools::SHELL.into()];
        agent.workdir = Some(dir.to_string_lossy().into_owned());
        app.composer.edit = TextEdit::new("go");
        app.send();
        (app, seen)
    }

    fn folder() -> PathBuf {
        let d = std::env::temp_dir().join(format!("oido-hooks-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    #[test]
    fn a_before_tool_hook_can_stop_a_call_and_the_model_is_told() {
        let dir = folder();
        let db = TempDb::new();
        let (app, seen) = chat_with(
            &db,
            vec![hook(Event::BeforeTool, "echo 'not on Fridays' >&2; exit 2")],
            "touch ran",
            &dir,
        );
        let _first = seen.recv().unwrap();
        let second = seen.recv().unwrap();
        assert!(!dir.join("ran").exists(), "the command never ran");
        assert!(
            second.contains("a hook the person set stopped it"),
            "{second}"
        );
        assert!(second.contains("not on fridays"), "{second}");
        assert!(!app.busy());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_before_tool_hook_can_change_what_runs() {
        let dir = folder();
        let db = TempDb::new();
        let (_app, seen) = chat_with(
            &db,
            vec![hook(
                Event::BeforeTool,
                r#"echo '{"action":"replace","args":{"command":"touch swapped"}}'"#,
            )],
            "touch original",
            &dir,
        );
        let _ = seen.recv().unwrap();
        let _ = seen.recv().unwrap();
        assert!(dir.join("swapped").exists() && !dir.join("original").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_hook_can_hold_a_call_for_your_go_ahead() {
        let dir = folder();
        let db = TempDb::new();
        let (mut app, seen) = chat_with(
            &db,
            vec![hook(
                Event::BeforeTool,
                r#"echo '{"action":"ask","reason":"prod"}'"#,
            )],
            "touch ran",
            &dir,
        );
        let _ = seen.recv().unwrap();
        let sid = app.current().unwrap().chat().id.clone();
        assert!(app.holding_in(&sid) && !dir.join("ran").exists());
        app.decide_trip(&sid, "h1", true);
        assert!(dir.join("ran").exists(), "it ran once allowed");
        assert!(seen.recv().unwrap().contains("h1"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_after_tool_hook_changes_what_the_model_sees() {
        let dir = folder();
        let db = TempDb::new();
        let (_app, seen) = chat_with(
            &db,
            vec![hook(
                Event::AfterTool,
                r#"echo '{"action":"replace","result":"SHORT VERSION"}'"#,
            )],
            "echo a very long listing",
            &dir,
        );
        let _ = seen.recv().unwrap();
        let second = seen.recv().unwrap();
        assert!(second.contains("short version"), "{second}");
        assert!(
            !second.contains("exit code"),
            "the real result is not sent: {second}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A hook that appends its input to `log` (a file in `dir`), so a test can see what it was told.
    fn logger(event: Event, dir: &Path) -> Hook {
        hook(
            event,
            &format!(
                "cat >> {}/log; echo >> {}/log",
                dir.display(),
                dir.display()
            ),
        )
    }

    fn logged(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("log")).unwrap_or_default()
    }

    #[test]
    fn watching_hooks_are_told_each_step_of_the_work() {
        let dir = folder();
        let db = TempDb::new();
        let hooks = Event::WORK.map(|e| logger(e, &dir)).to_vec();
        let (app, seen) = chat_with(&db, hooks, "echo hi", &dir);
        let _ = seen.recv().unwrap();
        let _ = seen.recv().unwrap();
        let log = logged(&dir);
        for name in ["before_agent", "before_model", "after_model", "after_agent"] {
            assert!(
                log.contains(&format!("\"event\":\"{name}\"")),
                "{name}: {log}"
            );
        }
        assert!(!log.contains("on_error"), "nothing failed: {log}");
        // The model's own words, with what it was sent, as the model sees them.
        assert!(log.contains("\"tool_calls\":[\"shell\"]"), "{log}");
        assert!(log.contains("\"message\":\"go\""), "{log}");
        assert!(!app.busy());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_hook_that_only_watches_cannot_change_the_work() {
        let dir = folder();
        let db = TempDb::new();
        let refuse = hook(Event::BeforeModel, "echo '{\"action\":\"block\"}'; exit 2");
        let (app, seen) = chat_with(&db, vec![refuse], "touch ran", &dir);
        let _ = seen.recv().unwrap();
        let _ = seen.recv().unwrap();
        assert!(dir.join("ran").exists(), "the work went on");
        assert!(!app.busy());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_agent_switched_out_runs_no_hooks() {
        let dir = folder();
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(
            &db,
            vec![
                (200, EVENTS, calling_shell("touch ran")),
                (200, EVENTS, text_stream("Done.")),
            ],
        );
        let id = app.roster.current().unwrap().id.clone();
        app.set_hooks(vec![hook(Event::BeforeTool, "exit 2")]);
        app.set_hooks_for(&id, false);
        assert!(!app.hooks.runs_for(&id));
        let agent = app.roster.current_mut().unwrap();
        agent.tools = vec![crate::tools::SHELL.into()];
        agent.workdir = Some(dir.to_string_lossy().into_owned());
        app.composer.edit = TextEdit::new("go");
        app.send();
        let _ = seen.recv().unwrap();
        let _ = seen.recv().unwrap();
        assert!(dir.join("ran").exists(), "its hooks were off");
        std::fs::remove_dir_all(dir).unwrap();
    }

    const DISK: &str = r#"{"action":"replace","args":{"command":"mkfs.ext4 /dev/sdb1"}}"#;

    #[test]
    fn a_hook_cannot_change_a_call_into_one_the_guardrails_refuse() {
        let dir = folder();
        let db = TempDb::new();
        let (app, seen) = chat_with(
            &db,
            vec![hook(Event::BeforeTool, &format!("echo '{DISK}'"))],
            "touch ran",
            &dir,
        );
        let _ = seen.recv().unwrap();
        let second = seen.recv().unwrap();
        assert!(second.contains("not run"), "{second}");
        assert!(!dir.join("ran").exists());
        assert!(!app.busy());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn worker(hooks: Vec<Hook>) -> Worker {
        let mut all = super::super::Hooks::default();
        all.set(hooks);
        Worker::new(&all, "a", "Member", tmp())
    }

    fn scene() -> (crate::guard::Guard, crate::guard::Place) {
        (
            crate::guard::Guard::default(),
            crate::guard::Place {
                root: tmp(),
                home: None,
                blocked: Vec::new(),
            },
        )
    }

    fn gate(w: &Worker, args: &mut String) -> Option<String> {
        let (guard, place) = scene();
        w.before(
            &crate::guard::ask::Asker::nobody(),
            (&guard, &place, None),
            "shell",
            args,
            &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            &mut Vec::new(),
        )
    }

    #[test]
    fn unattended_work_runs_hooks_too() {
        let w = worker(vec![hook(Event::BeforeTool, "echo nope >&2; exit 2")]);
        let told = gate(&w, &mut "{}".to_owned()).unwrap();
        assert!(told.contains("a hook the person set stopped it") && told.contains("nope"));

        let w = worker(vec![hook(
            Event::BeforeTool,
            r#"echo '{"action":"replace","args":{"command":"ls"}}'"#,
        )]);
        let mut args = r#"{"command":"pwd"}"#.to_owned();
        assert_eq!(gate(&w, &mut args), None);
        assert_eq!(args, r#"{"command":"ls"}"#);

        let w = worker(vec![hook(Event::BeforeTool, &format!("echo '{DISK}'"))]);
        assert!(
            gate(&w, &mut "{}".to_owned()).is_some(),
            "the guardrails look again"
        );

        let w = worker(vec![hook(
            Event::AfterTool,
            r#"echo '{"action":"replace","result":"short"}'"#,
        )]);
        assert_eq!(
            w.after("shell", "{}", "long".into(), &mut Vec::new()),
            "short"
        );
    }
}
