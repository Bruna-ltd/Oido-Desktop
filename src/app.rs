//! App state and the small actions the views trigger.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use rune::AppHandle;
use rune::TextEdit;
use rune_kit::shell::PaneState;

use crate::agents::{Agent, FormKind, Roster, Target};
use crate::model::{AgentStatus, Approval, Decision, Message, chat};
use crate::persist;
use crate::picker::PickerState;
use crate::providers::{ModelStatus, Providers};
use crate::runs::{Run, Step};
use crate::store::Store;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    Chat,
    Runs,
    Providers,
    /// Saved workflows and their runs.
    Workflows,
    /// Schedules: prompts and workflows fired on a timing, and how their fires went.
    Schedules,
    /// What agents used, in tokens, and its estimated cost.
    Usage,
    /// MCP servers: other programs that give agents tools.
    Mcp,
    /// Messaging apps (Telegram) through which paired people talk to agents.
    Channels,
    /// The checks made before anything reaches a model or an agent's action runs.
    Guardrails,
    /// The app's own settings: its appearance.
    Settings,
    /// The create, edit and team-member form (`Roster::editing` says which).
    AgentForm,
    /// A Studio section not built natively yet.
    Soon(&'static str),
}

/// How hard the agent thinks before answering (the composer's "Think" button).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

    /// Its name in words, as `/think` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
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
    /// The Runs screen: the newest runs from the database, the open one, and its steps.
    pub runs: Vec<Run>,
    pub run: usize,
    pub steps: Vec<Step>,
    pub record: usize,
    /// Replies being written, by chat.
    pub live: std::collections::HashMap<String, crate::run::Live>,
    /// Each chat's latest run, still being recorded while the agent works on what you said.
    pub traces: std::collections::HashMap<String, crate::runs::Trace>,
    /// Commands running for the shell tool, by chat, with the flag that stops each.
    pub commands: std::collections::HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Chats whose agent has handed tasks to team members that have not come back yet.
    pub delegating: std::collections::HashMap<String, crate::delegate::Delegating>,
    /// Chats whose older messages are being summarized to make room, with the flag that stops each.
    pub summarizing:
        std::collections::HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Chats that carry on in the same run once their summary is written.
    pub resuming: std::collections::HashSet<String>,
    /// Summaries opened in a chat, by chat and position.
    pub summaries_open: std::collections::HashSet<(String, usize)>,
    /// Replies whose thinking you opened or closed yourself, by chat and position: open or not.
    pub thinking_open: std::collections::HashMap<(String, usize), bool>,
    /// A reply's picture opened large, by its source and alt text.
    pub viewing: Option<(String, String)>,
    /// Every task handed over this launch, by the call that handed it over, with how it went.
    pub delegated: std::collections::HashMap<String, crate::delegate::Delegated>,
    /// Counts seconds while something works, so clocks on screen redraw.
    pub tick: u64,
    /// Workflows: the saved ones, drafts learned from chats (by chat), and what is being learned.
    pub workflows: Vec<crate::workflow::Saved>,
    pub drafts: std::collections::HashMap<String, crate::workflow::Draft>,
    pub teaching: std::collections::HashMap<String, crate::flows::Teach>,
    /// The change asked of the open draft, typed in its card.
    pub refine: TextEdit,
    /// The Workflows page: the open workflow, its runs, the open run, the inputs for the next run,
    /// workflows running from the page (with the flag that stops each), and what to tell you.
    pub wf_open: Option<String>,
    pub wf_runs: Vec<crate::workflow::RunRecord>,
    pub wf_run: usize,
    pub wf_inputs: Vec<(String, TextEdit)>,
    pub wf_jobs: std::collections::HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub wf_notice: Option<String>,
    pub wf_confirm: bool,
    pub composer: crate::composer::Composer,
    /// What was typed in the chats not open, by session id: each tab keeps its own draft.
    pub parked: std::collections::HashMap<String, crate::composer::Composer>,
    /// Messages sent while their chat's agent was still answering, by session id. Each goes when
    /// its chat's reply ends.
    pub queued: std::collections::HashMap<String, crate::prompt::Prompt>,
    /// The open chat's transcript is scrolled up away from its newest message.
    pub scrolled_up: bool,
    /// The working folder's files, for the composer's `@`.
    pub file_index: Option<crate::composer::FileIndex>,
    /// The answer being typed to the agent's question, and the options picked for it.
    pub answer_text: TextEdit,
    pub picks: Vec<String>,
    pub effort: Effort,
    /// The composer's model picker.
    pub model_picker: PickerState,
    /// The composer's working-folder browser.
    pub workdir_browser: crate::folders::Browser,
    pub pinned_closed: bool,
    pub others_closed: bool,
    /// The right rail shows labels beside its icons.
    pub rail_open: bool,
    pub providers: Providers,
    /// Skills found in the skills folder, and where it is.
    pub skills: Vec<crate::skills::Skill>,
    pub skills_dir: Option<std::path::PathBuf>,
    /// MCP servers and the page that manages them.
    pub mcp: crate::mcp::Mcp,
    /// The chosen appearance and the Omarchy themes it can pick from.
    pub look: crate::theme::Look,
    /// The Settings page's theme list is open.
    pub theme_open: bool,
    /// This app's own updates: on their own or not, and where they stand.
    pub updates: crate::update::Updates,
    /// Model prices, the catalog's and yours, for working out cost; and whether the catalog is
    /// being fetched.
    pub prices: crate::usage::Prices,
    pub prices_fetching: bool,
    /// The catalog's context limits, fetched with the prices.
    pub limits: crate::window::Limits,
    /// The Usage screen.
    pub usage: crate::usage::Page,
    /// Each chat's tokens by model, as saved, for the cost in its header.
    pub chat_usage:
        std::collections::HashMap<String, Vec<(crate::models::ModelRef, crate::usage::Totals)>>,
    /// The open agent form's agent's daily totals over the last 30 days.
    pub agent_spend: Vec<crate::usage::Row>,
    /// Saved schedules, the fire each running one is on (by schedule id), and whether schedules
    /// run at all (Settings).
    pub schedules: Vec<crate::schedule::Schedule>,
    pub firing: std::collections::HashMap<String, crate::schedule::Fire>,
    pub run_schedules: bool,
    /// Schedules agents proposed with their `schedule` tool, waiting for approval in their chats.
    pub proposals: Vec<crate::schedule::Proposal>,
    /// Agent changes agents asked for with their `agents` tool, waiting for approval in their chats.
    pub agent_drafts: Vec<crate::agent_tool::Draft>,
    /// Why approving an agent draft did not go through, by the draft's tool call.
    pub agent_draft_notice: Option<(String, String)>,
    /// The Schedules page.
    pub schedule_page: crate::schedules::Page,
    /// The channels (Telegram): each one's bot, the people paired with it and their chats.
    pub channels: crate::channels::Channels,
    /// The guardrails on: what is hidden from models and which tool calls wait for you.
    pub guard: crate::guard::Guard,
    /// Chats whose last model call had outside text in it, with what it was in words (see
    /// `guard::taint`): while it is there, calls that act wait for you.
    pub tainted: std::collections::HashMap<String, String>,
    /// Calls from work nobody is watching (team members, workflows) waiting for your go-ahead.
    pub waiting: Vec<crate::guard::ask::Waiting>,
    /// The local database. `None` when it could not be opened: the app then runs without saving.
    pub store: Option<Store>,
    /// The window has the keyboard focus, so the person is looking at it: no notifications then.
    pub focused: bool,
}

/// The running app, for worker threads to hand their results back through. Unset in tests and
/// headless runs, where [`App::spawn`] runs the work on the spot instead.
static HANDLE: OnceLock<AppHandle<App>> = OnceLock::new();

pub fn set_handle(handle: AppHandle<App>) {
    let _ = HANDLE.set(handle);
}

/// The running app, if there is one (not in tests or headless runs).
pub fn handle() -> Option<&'static AppHandle<App>> {
    HANDLE.get()
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
    if let Some(db) = &store
        && let Err(e) = db.tidy_runs(now(), crate::runs::KEEP_SECS)
    {
        eprintln!("oido: could not tidy old runs: {e}");
    }
    if let Some(db) = &store
        && let Err(e) = db.tidy_workflow_runs(now(), crate::runs::KEEP_SECS)
    {
        eprintln!("oido: could not tidy old workflow runs: {e}");
    }
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
        let (store, roster, providers) = persist::load(store);
        let look = crate::theme::Look::load(persist::appearance(store.as_ref()));
        let skills_dir = crate::skills::dir();
        let mut app = App {
            sidebar: PaneState::new(264.0).range(180.0, 420.0),
            inspector: PaneState::new(380.0).range(260.0, 720.0),
            route: Route::Chat,
            roster,
            runs: Vec::new(),
            run: 0,
            steps: Vec::new(),
            record: 0,
            live: std::collections::HashMap::new(),
            traces: std::collections::HashMap::new(),
            commands: std::collections::HashMap::new(),
            delegating: std::collections::HashMap::new(),
            summarizing: std::collections::HashMap::new(),
            resuming: std::collections::HashSet::new(),
            summaries_open: std::collections::HashSet::new(),
            thinking_open: std::collections::HashMap::new(),
            viewing: None,
            delegated: std::collections::HashMap::new(),
            tick: 0,
            workflows: Vec::new(),
            drafts: std::collections::HashMap::new(),
            teaching: std::collections::HashMap::new(),
            refine: TextEdit::new(""),
            wf_open: None,
            wf_runs: Vec::new(),
            wf_run: 0,
            wf_inputs: Vec::new(),
            wf_jobs: std::collections::HashMap::new(),
            wf_notice: None,
            wf_confirm: false,
            composer: crate::composer::Composer::new(),
            parked: std::collections::HashMap::new(),
            queued: std::collections::HashMap::new(),
            file_index: None,
            scrolled_up: false,
            answer_text: TextEdit::new(""),
            picks: Vec::new(),
            effort: Effort::Off,
            model_picker: PickerState::default(),
            workdir_browser: crate::folders::Browser::default(),
            pinned_closed: false,
            others_closed: false,
            rail_open: false,
            providers,
            skills: skills_dir
                .as_deref()
                .map(crate::skills::discover)
                .unwrap_or_default(),
            skills_dir,
            mcp: crate::mcp::Mcp::new(Vec::new()),
            look,
            theme_open: false,
            updates: crate::update::Updates::new(persist::auto_update(store.as_ref())),
            schedules: Vec::new(),
            firing: std::collections::HashMap::new(),
            run_schedules: persist::run_schedules(store.as_ref()),
            prices: crate::usage::Prices::default(),
            prices_fetching: false,
            limits: crate::window::Limits::default(),
            usage: crate::usage::Page::default(),
            chat_usage: std::collections::HashMap::new(),
            agent_spend: Vec::new(),
            proposals: Vec::new(),
            agent_drafts: Vec::new(),
            agent_draft_notice: None,
            schedule_page: crate::schedules::Page::default(),
            channels: crate::channels::Channels::default(),
            guard: crate::guard::Guard::load(store.as_ref()),
            tainted: std::collections::HashMap::new(),
            waiting: Vec::new(),
            store,
            focused: false,
        };
        app.settle_interrupted_commands();
        app.load_workflows();
        app.load_mcp();
        app.load_schedules();
        app.load_prices();
        app.load_chat_usage();
        app.load_channels();
        app
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

    /// Runs a change of which chat is open, parking the composer's text with the chat it was typed
    /// in and bringing back the new chat's own.
    fn switching_chat(&mut self, change: impl FnOnce(&mut Self)) {
        let open = |s: &Self| s.roster.current().map(|a| a.chat().id.clone());
        let before = open(self);
        change(self);
        let after = open(self);
        if before == after {
            return;
        }
        let draft = std::mem::take(&mut self.composer).parked();
        if let Some(id) = before.filter(|_| !draft.is_blank()) {
            self.parked.insert(id, draft);
        }
        if let Some(back) = after.and_then(|id| self.parked.remove(&id)) {
            self.composer = back;
        }
    }

    /// Opens chat `sid`, whichever agent or team member holds it.
    pub fn open_chat(&mut self, sid: &str) {
        let Some(t) = self.roster.target_of(sid) else {
            return;
        };
        self.switching_chat(|s| {
            s.roster.select(t.clone());
            if let Some(a) = s.roster.current_mut()
                && let Some(i) = a.sessions.iter().position(|c| c.id == sid)
            {
                a.active = i;
            }
        });
        self.route = Route::Chat;
        self.save_selection();
        self.save_agent_row(&t);
    }

    pub fn select_agent(&mut self, t: Target) {
        self.switching_chat(|s| s.roster.select(t));
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
        self.load_agent_spend(&t.id);
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

    /// Deletes the confirmed agent.
    pub fn confirm_delete(&mut self) {
        if let Some(t) = self.roster.confirm.clone() {
            self.delete_agent_now(&t);
        }
    }

    /// Deletes an agent or team member, stopping what runs in its chats. If its form is open,
    /// that closes too.
    pub(crate) fn delete_agent_now(&mut self, t: &Target) {
        if let Some(a) = self.roster.find(t) {
            for id in Roster::session_ids(a) {
                self.cancel_session(&id);
            }
        }
        self.roster.delete(t);
        self.delete_saved_agent(&t.id);
        self.load_workflows();
        self.save_selection();
        if self.route == Route::AgentForm {
            self.cancel_form();
        }
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

    pub fn new_tab(&mut self) {
        self.switching_chat(Self::open_new_tab);
        self.composer.edit.focused = true;
    }

    fn open_new_tab(&mut self) {
        let Some(agent) = self.roster.current_mut() else {
            return;
        };
        agent.sessions.push(chat("New chat", Vec::new()));
        agent.active = agent.sessions.len() - 1;
        self.save_chat();
    }

    pub fn select_tab(&mut self, i: usize) {
        self.switching_chat(|s| {
            if let Some(a) = s.roster.current_mut() {
                a.active = i;
            }
        });
        if let Some(t) = self.roster.selected.clone() {
            self.save_agent_row(&t);
        }
    }

    pub fn close_tab(&mut self, i: usize) {
        let gone = self
            .roster
            .current()
            .and_then(|a| a.sessions.get(i))
            .map(|c| c.id.clone());
        self.switching_chat(|s| s.remove_tab(i));
        if let Some(id) = gone {
            self.parked.remove(&id);
        }
    }

    fn remove_tab(&mut self, i: usize) {
        let Some(agent) = self.roster.current_mut() else {
            return;
        };
        if agent.sessions.len() > 1 {
            let gone = agent.sessions.remove(i);
            agent.active = agent.active.min(agent.sessions.len() - 1);
            self.cancel_session(&gone.id);
            if let Some(t) = self.roster.selected.clone() {
                self.delete_saved_chat(&t, &gone.id);
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
    use crate::testing::{TempDb, ready, samples};

    #[test]
    fn send_appends_a_user_message_and_clears_the_composer() {
        let mut app = samples();
        ready(&mut app);
        let before = app.current().unwrap().chat().messages.len();
        app.composer.edit = TextEdit::new("  hello  ");
        app.send();
        let messages = &app.current().unwrap().chat().messages;
        assert_eq!(
            messages.len(),
            before + 2,
            "your message and the reply to it"
        );
        assert!(matches!(&messages[before], Message::User(p) if p.text == "hello"));
        assert!(app.composer.edit.text.is_empty());
        app.send();
        assert_eq!(
            app.current().unwrap().chat().messages.len(),
            before + 2,
            "blank input sends nothing"
        );
    }

    #[test]
    fn nothing_is_sent_until_the_agent_has_a_model_that_can_answer() {
        let mut app = samples();
        let before = app.current().unwrap().chat().messages.len();
        app.composer.edit = TextEdit::new("hello");
        app.send();
        assert_eq!(
            app.current().unwrap().chat().messages.len(),
            before,
            "no model and no default: choose one first"
        );
        assert_eq!(
            app.composer.edit.text, "hello",
            "and what was typed is kept"
        );

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
            before + 2,
            "with the default it can"
        );
    }

    #[test]
    fn deciding_the_gate_resolves_it_and_frees_the_agent() {
        let mut app = samples();
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
    fn each_tab_keeps_its_own_draft() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        ready(&mut app);
        let old = app.current().unwrap().active;
        app.composer.edit = TextEdit::new("first");
        app.new_tab();
        let new = app.current().unwrap().active;
        assert!(app.composer.edit.text.is_empty(), "a new tab starts empty");
        app.composer.edit = TextEdit::new("second");
        app.select_tab(old);
        assert_eq!(app.composer.edit.text, "first");
        app.select_tab(new);
        assert_eq!(app.composer.edit.text, "second");
        app.select_agent(Target::agent("scout"));
        assert!(
            app.composer.edit.text.is_empty(),
            "another agent's chat is its own"
        );
        app.select_agent(Target::agent("atlas"));
        assert_eq!(
            app.composer.edit.text, "second",
            "and the open tab was kept"
        );
        app.select_tab(old);
        app.close_tab(new);
        assert_eq!(app.composer.edit.text, "first");
        app.new_tab();
        let doomed = app.current().unwrap().active;
        app.composer.edit = TextEdit::new("doomed");
        app.select_tab(old);
        app.close_tab(doomed);
        assert_eq!(app.composer.edit.text, "first");
        assert!(app.parked.is_empty(), "a closed tab leaves no draft behind");
    }

    #[test]
    fn tabs_open_close_and_keep_one() {
        let mut app = samples();
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
        let mut app = samples();
        let before = app.current().unwrap().chat().messages.len();
        app.start_fresh();
        app.start_fresh();
        let m = &app.current().unwrap().chat().messages;
        assert_eq!(m.len(), before + 1);
        assert!(matches!(m.last(), Some(Message::Divider(_))));
    }

    #[test]
    fn a_new_install_has_no_agents_and_no_connected_provider() {
        let db = TempDb::new();
        let app = db.open();
        assert!(app.roster.list.is_empty(), "no sample agents");
        assert!(app.current().is_none() && app.roster.selected.is_none());
        assert!(
            app.providers.list.iter().all(|p| !p.connected),
            "no provider starts connected"
        );
        assert!(
            !app.providers.list.is_empty(),
            "but the providers you can connect are listed"
        );
        drop(app);
        assert!(
            db.open().roster.list.is_empty(),
            "and it stays that way on the next launch"
        );
    }

    #[test]
    fn the_first_agent_is_made_from_the_form_selected_and_kept_across_a_restart() {
        let db = TempDb::new();
        let mut app = db.open();
        app.new_agent();
        assert!(
            app.route == Route::AgentForm && !app.roster.can_submit(),
            "a name is required"
        );
        app.roster.form.name = TextEdit::new("Billing helper");
        app.roster.form.prompt = TextEdit::new("You answer billing questions.");
        app.submit_form();
        assert!(app.route == Route::Chat);
        let made = app.roster.selected.clone().unwrap();
        assert_eq!(app.current().unwrap().name, "Billing helper");
        assert_eq!(
            app.current().unwrap().sessions.len(),
            1,
            "it starts with one chat"
        );
        drop(app);

        let app = db.open();
        assert_eq!(app.roster.list.len(), 1, "exactly the agent that was made");
        assert_eq!(
            app.roster.find(&made).unwrap().prompt,
            "You answer billing questions."
        );
        assert_eq!(app.roster.selected, Some(made), "and it is the open one");
    }

    #[test]
    fn a_team_member_can_be_added_to_the_first_agent_and_deleting_it_leaves_an_empty_app() {
        let db = TempDb::new();
        let mut app = db.open();
        app.new_agent();
        app.roster.form.name = TextEdit::new("Lead");
        app.submit_form();
        let lead = app.roster.selected.clone().unwrap();
        app.new_member(&lead.id);
        app.roster.form.name = TextEdit::new("Checker");
        app.submit_form();
        assert_eq!(app.roster.find(&lead).unwrap().subagents.len(), 1);
        app.roster.confirm = Some(lead);
        app.confirm_delete();
        assert!(app.roster.list.is_empty() && app.current().is_none());
        drop(app);
        assert!(
            db.open().roster.list.is_empty(),
            "the team went with its agent, saved"
        );
    }

    #[test]
    fn what_you_change_survives_a_restart() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        ready(&mut app);
        app.composer.edit = TextEdit::new("remember me");
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
    fn deleting_every_agent_leaves_none_and_nothing_comes_back() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
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
        let mut app = db.open_with_samples();
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
        let mut app = db.open_with_samples();
        app.close_tab(1);
        drop(app);
        assert_eq!(db.open().current().unwrap().sessions.len(), 1);
    }

    #[test]
    fn tabs_come_back_in_their_order_with_the_same_one_open_after_closing_some() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        for _ in 0..3 {
            app.new_tab();
        }
        for _ in 0..2 {
            app.select_tab(0);
            app.close_tab(0);
        }
        // Before, this was saved at a position an older tab still held, and came back before it.
        app.new_tab();
        let ids = |a: &Agent| a.sessions.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        let t = app.roster.selected.clone().unwrap();
        let (order, open) = {
            let a = app.current().unwrap();
            (ids(a), a.chat().id.clone())
        };
        drop(app);
        let back = db.open();
        let a = back.roster.find(&t).unwrap();
        assert_eq!(ids(a), order, "the tabs' order");
        assert_eq!(a.chat().id, open, "the open tab");
    }

    #[test]
    fn saved_data_that_cannot_be_read_is_left_alone_not_overwritten() {
        let db = TempDb::new();
        let app = db.open_with_samples();
        let agents = app.roster.list.len();
        let sid = app.current().unwrap().chat().id.clone();
        drop(app);
        let raw = || rusqlite::Connection::open(&db.0).unwrap();
        // A kind of message this build does not know, as a newer one could write.
        raw()
            .execute(
                "INSERT INTO messages (session_id, seq, body) VALUES (?1, 999, '{\"type\":\"later\"}')",
                [&sid],
            )
            .unwrap();
        let mut app = db.open();
        assert!(app.store.is_none(), "it runs without saving");
        assert!(app.roster.list.is_empty());
        app.save_providers();
        app.new_agent();
        app.roster.form.name = TextEdit::new("Stray");
        app.submit_form();
        drop(app);
        raw()
            .execute("DELETE FROM messages WHERE seq = 999", [])
            .unwrap();
        let back = db.open();
        assert_eq!(back.roster.list.len(), agents, "every agent is still there");
        assert!(back.roster.list.iter().all(|a| a.name != "Stray"));
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
