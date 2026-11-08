//! App state and the small actions the views trigger.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use rune::AppHandle;
use rune::TextEdit;
use rune_kit::shell::PaneState;

use crate::agents::{Agent, FormKind, Roster, Target};
use crate::model::{self, AgentStatus, Approval, Decision, Message, Run, chat};
use crate::persist;
use crate::picker::PickerState;
use crate::providers::{ModelStatus, Providers};
use crate::store::Store;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Chat,
    Runs,
    Providers,
    /// The create, edit and team-member form (`Roster::editing` says which).
    AgentForm,
    /// A Studio section not built natively yet.
    Soon(&'static str),
}

/// How hard the agent thinks before answering (the composer's "Think" button).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    Off,
    Low,
    Medium,
    High,
}

impl Effort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Think",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Off,
        }
    }
}

pub struct App {
    pub sidebar: PaneState,
    pub inspector: PaneState,
    pub route: Route,
    pub roster: Roster,
    pub runs: Vec<Run>,
    pub run: usize,
    pub record: usize,
    pub composer: TextEdit,
    pub effort: Effort,
    /// The composer's model picker.
    pub model_picker: PickerState,
    pub pinned_closed: bool,
    pub others_closed: bool,
    /// The right rail shows labels beside its icons.
    pub rail_open: bool,
    pub providers: Providers,
    /// The local database. `None` when it could not be opened: the app then runs without saving.
    pub store: Option<Store>,
}

/// The running app, for worker threads to hand their results back through. Unset in tests and
/// headless runs, where [`App::spawn`] runs the work on the spot instead.
static HANDLE: OnceLock<AppHandle<App>> = OnceLock::new();

pub fn set_handle(handle: AppHandle<App>) {
    let _ = HANDLE.set(handle);
}

/// Seconds since the epoch.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub fn init() -> App {
    let store = Store::open_default()
        .map_err(|e| eprintln!("oido: running without saved data: {e}"))
        .ok();
    App::new(store)
}

impl App {
    /// Runs `job` off the UI thread and then `done` with its result on it, so a slow network call
    /// never freezes the window. With no running app (tests, headless) both run at once.
    pub fn spawn<R: Send + 'static>(
        &mut self,
        job: impl FnOnce() -> R + Send + 'static,
        done: impl FnOnce(&mut App, R) + Send + 'static,
    ) {
        match HANDLE.get() {
            Some(handle) => {
                let handle = handle.clone();
                std::thread::spawn(move || {
                    let result = job();
                    handle.update(move |app, _| done(app, result));
                });
            }
            None => {
                let result = job();
                done(self, result);
            }
        }
    }

    pub fn new(store: Option<Store>) -> App {
        let (roster, providers) = persist::load(store.as_ref());
        App {
            sidebar: PaneState::new(264.0).range(180.0, 420.0),
            inspector: PaneState::new(300.0).range(240.0, 460.0),
            route: Route::Chat,
            roster,
            runs: model::runs(),
            run: 0,
            record: 0,
            composer: TextEdit::new(""),
            effort: Effort::Off,
            model_picker: PickerState::default(),
            pinned_closed: false,
            others_closed: false,
            rail_open: false,
            providers,
            store,
        }
    }

    /// Whether the open agent has a model that can answer right now.
    pub fn can_answer(&self) -> bool {
        self.current().is_some_and(|a| {
            matches!(
                self.providers.status(a.model.as_ref()),
                ModelStatus::Ready(_)
            )
        })
    }

    pub fn current(&self) -> Option<&Agent> {
        self.roster.current()
    }

    pub fn select_agent(&mut self, t: Target) {
        self.roster.select(t);
        self.route = Route::Chat;
        self.save_selection();
    }

    pub fn new_agent(&mut self) {
        self.roster.open_new();
        self.route = Route::AgentForm;
    }

    pub fn new_member(&mut self, parent: &str) {
        self.roster.open_member(parent);
        self.route = if self.roster.editing.is_some() {
            Route::AgentForm
        } else {
            self.route
        };
    }

    pub fn edit_agent(&mut self, t: &Target) {
        self.roster.open_edit(t);
        self.route = if self.roster.editing.is_some() {
            Route::AgentForm
        } else {
            self.route
        };
    }

    /// Leaves the form without saving, back to the conversation.
    pub fn cancel_form(&mut self) {
        self.roster.close_form();
        self.route = Route::Chat;
    }

    pub fn submit_form(&mut self) {
        let editing = matches!(self.roster.editing, Some(FormKind::Edit(_)));
        if let Some(t) = self.roster.submit() {
            self.route = Route::Chat;
            // An edit changes the agent's row; a new agent is saved whole, with its first chat.
            if editing {
                self.save_agent_row(&t);
            } else {
                self.save_agent_tree(&t);
            }
            self.save_selection();
        }
    }

    /// The form's title and its submit button's label, by what it is for.
    pub fn form_labels(&self) -> (String, &'static str) {
        let name = self.roster.form.name.text.trim();
        let or = |fallback: &str| {
            if name.is_empty() {
                fallback.to_owned()
            } else {
                name.to_owned()
            }
        };
        match &self.roster.editing {
            Some(FormKind::NewAgent) | None => (or("New agent"), "Create"),
            Some(FormKind::NewMember(_)) => (or("New team member"), "Add"),
            Some(FormKind::Edit(_)) => (or("Edit agent"), "Save changes"),
        }
    }

    /// Deletes the confirmed agent. If its form is open, that closes too.
    pub fn confirm_delete(&mut self) {
        if let Some(t) = self.roster.confirm.clone() {
            self.roster.delete(&t);
            self.delete_saved_agent(&t.id);
            self.save_selection();
            if self.route == Route::AgentForm {
                self.cancel_form();
            }
        }
    }

    pub fn select_run(&mut self, i: usize) {
        self.run = i;
        self.record = 0;
    }

    /// Appends the composer's text as a user message. The agent's reply is the logic layer's job.
    pub fn send(&mut self) {
        let text = self.composer.text.trim().to_owned();
        if text.is_empty() || !self.can_answer() {
            return;
        }
        let Some(agent) = self.roster.current_mut() else {
            return;
        };
        let messages = &mut agent.chat_mut().messages;
        messages.push(Message::User(text));
        let seq = messages.len() - 1;
        self.composer = TextEdit::new("");
        self.composer.focused = true;
        self.save_message(seq);
    }

    /// Decides the open approval gate of the current agent, if any.
    pub fn decide(&mut self, decision: Decision) {
        let Some(agent) = self.roster.current_mut() else {
            return;
        };
        let mut decided = None;
        for (i, m) in agent.chat_mut().messages.iter_mut().enumerate().rev() {
            if let Message::Assistant {
                approval:
                    Some(Approval {
                        decision: d @ None, ..
                    }),
                ..
            } = m
            {
                *d = Some(decision);
                decided = Some(i);
                break;
            }
        }
        agent.status = AgentStatus::Idle;
        if let Some(i) = decided {
            self.save_message(i);
        }
    }

    /// Stops a running agent. The run itself is the logic layer's job.
    pub fn stop(&mut self) {
        if let Some(a) = self.roster.current_mut() {
            a.status = AgentStatus::Idle;
        }
    }

    pub fn new_tab(&mut self) {
        let Some(agent) = self.roster.current_mut() else {
            return;
        };
        agent.sessions.push(chat("New chat", Vec::new()));
        agent.active = agent.sessions.len() - 1;
        self.composer.focused = true;
        self.save_chat();
    }

    pub fn select_tab(&mut self, i: usize) {
        if let Some(a) = self.roster.current_mut() {
            a.active = i;
        }
        if let Some(t) = self.roster.selected.clone() {
            self.save_agent_row(&t);
        }
    }

    pub fn close_tab(&mut self, i: usize) {
        let Some(agent) = self.roster.current_mut() else {
            return;
        };
        if agent.sessions.len() > 1 {
            let gone = agent.sessions.remove(i);
            agent.active = agent.active.min(agent.sessions.len() - 1);
            self.delete_saved_chat(&gone.id);
            if let Some(t) = self.roster.selected.clone() {
                self.save_agent_row(&t);
            }
        }
    }

    /// The agent forgets the conversation; what was already read stays on screen under a divider.
    pub fn start_fresh(&mut self) {
        let Some(agent) = self.roster.current_mut() else {
            return;
        };
        let messages = &mut agent.chat_mut().messages;
        if !matches!(messages.last(), None | Some(Message::Divider(_))) {
            messages.push(Message::Divider("Context cleared".into()));
            let seq = messages.len() - 1;
            self.save_message(seq);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::models::ModelRef;
    use crate::testing::{TempDb, ready};

    #[test]
    fn send_appends_a_user_message_and_clears_the_composer() {
        let mut app = App::new(None);
        ready(&mut app);
        let before = app.current().unwrap().chat().messages.len();
        app.composer = TextEdit::new("  hello  ");
        app.send();
        assert_eq!(app.current().unwrap().chat().messages.len(), before + 1);
        assert!(
            matches!(app.current().unwrap().chat().messages.last(), Some(Message::User(t)) if t == "hello")
        );
        assert!(app.composer.text.is_empty());
        app.send();
        assert_eq!(
            app.current().unwrap().chat().messages.len(),
            before + 1,
            "blank input sends nothing"
        );
    }

    #[test]
    fn nothing_is_sent_until_the_agent_has_a_model_that_can_answer() {
        let mut app = App::new(None);
        let before = app.current().unwrap().chat().messages.len();
        app.composer = TextEdit::new("hello");
        app.send();
        assert_eq!(
            app.current().unwrap().chat().messages.len(),
            before,
            "no model and no default: choose one first"
        );
        assert_eq!(app.composer.text, "hello", "and what was typed is kept");

        ready(&mut app);
        app.set_model(Some(ModelRef::new("anthropic", "retired")));
        app.send();
        assert_eq!(
            app.current().unwrap().chat().messages.len(),
            before,
            "a model its provider no longer lists cannot answer"
        );

        app.set_model(None);
        app.send();
        assert_eq!(
            app.current().unwrap().chat().messages.len(),
            before + 1,
            "with the default it can"
        );
    }

    #[test]
    fn deciding_the_gate_resolves_it_and_frees_the_agent() {
        let mut app = App::new(None);
        assert!(app.current().unwrap().status == AgentStatus::Waiting);
        app.decide(Decision::Approve);
        let decided = app.current().unwrap().chat().messages.iter().any(|m| {
            matches!(
                m,
                Message::Assistant {
                    approval: Some(Approval {
                        decision: Some(Decision::Approve),
                        ..
                    }),
                    ..
                }
            )
        });
        assert!(decided);
        assert!(app.current().unwrap().status == AgentStatus::Idle);
    }

    #[test]
    fn tabs_open_close_and_keep_one() {
        let mut app = App::new(None);
        let before = app.current().unwrap().sessions.len();
        app.new_tab();
        assert_eq!(app.current().unwrap().sessions.len(), before + 1);
        assert_eq!(
            app.current().unwrap().active,
            before,
            "the new tab is selected"
        );
        app.close_tab(before);
        assert_eq!(app.current().unwrap().sessions.len(), before);
        app.roster.select(Target::agent("scout"));
        app.close_tab(0);
        assert_eq!(
            app.current().unwrap().sessions.len(),
            1,
            "the last tab stays"
        );
    }

    #[test]
    fn start_fresh_marks_the_cut_once_and_keeps_what_was_read() {
        let mut app = App::new(None);
        let before = app.current().unwrap().chat().messages.len();
        app.start_fresh();
        app.start_fresh();
        let m = &app.current().unwrap().chat().messages;
        assert_eq!(m.len(), before + 1);
        assert!(matches!(m.last(), Some(Message::Divider(_))));
    }

    #[test]
    fn the_first_launch_starts_with_the_samples_and_later_launches_with_what_was_saved() {
        let db = TempDb::new();
        let first = db.open();
        let names: Vec<String> = first.roster.list.iter().map(|a| a.name.clone()).collect();
        assert!(
            names.contains(&"Atlas".to_owned()),
            "sample agents on the very first launch"
        );
        assert!(
            first.providers.list.iter().all(|p| !p.connected),
            "no provider starts connected"
        );
        drop(first);
        assert_eq!(db.open().roster.list.len(), names.len());
    }

    #[test]
    fn what_you_change_survives_a_restart() {
        let db = TempDb::new();
        let mut app = db.open();
        ready(&mut app);
        app.composer = TextEdit::new("remember me");
        app.send();
        app.new_tab();
        app.toggle_pin("scout");
        app.set_model(Some(ModelRef::new("anthropic", "claude-test")));
        app.new_agent();
        app.roster.form.name = TextEdit::new("keeper");
        app.submit_form();
        let keeper = app.roster.selected.clone().unwrap();
        drop(app);

        let app = db.open();
        let atlas = app.roster.find(&Target::agent("atlas")).unwrap();
        assert!(
            atlas.sessions[0]
                .messages
                .contains(&Message::User("remember me".into()))
        );
        assert_eq!(
            atlas.sessions.len(),
            3,
            "the new chat came back beside the two"
        );
        assert_eq!(atlas.active, 2, "and the chat that was open is still open");
        assert!(app.roster.find(&Target::agent("scout")).unwrap().pinned);
        assert_eq!(
            atlas.model,
            Some(ModelRef::new("anthropic", "claude-test")),
            "a model is a provider and an id"
        );
        assert_eq!(app.roster.find(&keeper).unwrap().name, "keeper");
        assert_eq!(
            app.roster.selected,
            Some(keeper),
            "the selection is remembered"
        );
    }

    #[test]
    fn deleting_everything_stays_deleted_and_does_not_bring_the_samples_back() {
        let db = TempDb::new();
        let mut app = db.open();
        let ids: Vec<String> = app.roster.list.iter().map(|a| a.id.clone()).collect();
        for id in ids {
            app.roster.confirm = Some(Target::agent(id));
            app.confirm_delete();
        }
        drop(app);
        assert!(db.open().roster.list.is_empty());
    }

    #[test]
    fn an_approval_decided_in_place_is_saved_in_place() {
        let db = TempDb::new();
        let mut app = db.open();
        app.decide(Decision::Reject);
        drop(app);
        let app = db.open();
        let rejected = app.current().unwrap().chat().messages.iter().any(|m| {
            matches!(
                m,
                Message::Assistant {
                    approval: Some(Approval {
                        decision: Some(Decision::Reject),
                        ..
                    }),
                    ..
                }
            )
        });
        assert!(rejected);
    }

    #[test]
    fn closing_a_chat_removes_it_from_the_saved_data_too() {
        let db = TempDb::new();
        let mut app = db.open();
        app.close_tab(1);
        drop(app);
        assert_eq!(db.open().current().unwrap().sessions.len(), 1);
    }

    #[test]
    fn a_connect_that_was_cut_short_is_undone_on_the_next_launch() {
        let db = TempDb::new();
        {
            // what a build killed between "connected" and "key stored" left behind
            let first = db.open();
            let mut list = first.providers.list.clone();
            list.iter_mut()
                .find(|p| p.id == "openrouter")
                .unwrap()
                .connected = true;
            first
                .store
                .as_ref()
                .unwrap()
                .replace_providers(&list, None)
                .unwrap();
        }
        let app = db.open();
        assert!(
            !app.providers.get("openrouter").unwrap().connected,
            "a connection with no key is not shown"
        );
        assert!(
            app.store
                .as_ref()
                .unwrap()
                .connected_without_key()
                .unwrap()
                .is_empty(),
            "and the saved data is repaired too"
        );
    }
}
