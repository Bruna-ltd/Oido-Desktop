//! Where the app's state meets the local store: what it starts with, and what each change saves.
//!
//! A failed save never stops the app: it is reported on stderr and the app carries on, since the
//! screen is already right and the next change saves again.

use crate::agents::{Roster, RowDrag, Target};
use crate::app::App;
use crate::models::ModelRef;
use crate::providers::{self, Providers};
use crate::store::{self, Store, Verified};
use crate::theme::Appearance;
use rune::{Drag, Phase, Tx};
use rune_kit::shell::PaneState;
use serde::{Deserialize, Serialize};

const SEEDED: &str = "seeded";
const SELECTED: &str = "agents.selected";
const DEFAULT_MODEL: &str = "models.default";
const APPEARANCE: &str = "appearance";
const AUTO_UPDATE: &str = "updates.auto";
pub const RUN_SCHEDULES: &str = "schedules.run";
pub const ROUTE_BULK: &str = "router.bulk";
/// Whether TypeSafe judges each finished run for a skill (docs/skills-from-runs.md, ADR-6).
pub const SCORE_RUNS: &str = "runs.score";
/// Agents' own choices about that, by agent id (ADR-8).
pub const SCORE_AGENTS: &str = "runs.score.agents";
pub const PLAN: &str = "plan.on";
const PANES: &str = "panes";
const SIDEBAR_FOLDS: &str = "sidebar.folds";
const ONBOARDED: &str = "onboarding.done";

/// What is folded away in the agents sidebar: its two sections, and the agents whose team is.
#[derive(Default, Serialize, Deserialize)]
struct Folds {
    pinned: bool,
    others: bool,
    teams: Vec<String>,
}

/// What the app starts with: what the store holds, or on the very first launch the built-in
/// providers (written once, so deleting all of it stays deleted).
///
/// When the saved data cannot be read, the store is handed back as `None`: the app runs without
/// saving, since every save from the empty state it starts with would overwrite what is there.
pub fn load(store: Option<Store>) -> (Option<Store>, Roster, Providers) {
    let Some(db) = store else {
        return (None, Roster::new(), providers::init());
    };
    match try_load(&db) {
        Ok((roster, providers)) => (Some(db), roster, providers),
        Err(e) => {
            eprintln!("oido: could not read saved data, running without saving so it is kept: {e}");
            (None, Roster::new(), providers::init())
        }
    }
}

/// The appearance chosen on the Settings page; Oido's own when none was, or it cannot be read.
pub fn appearance(store: Option<&Store>) -> Appearance {
    let Some(db) = store else {
        return Appearance::default();
    };
    db.setting::<Appearance>(APPEARANCE)
        .unwrap_or_else(|e| {
            eprintln!("oido: could not read the appearance: {e}");
            None
        })
        .unwrap_or_default()
}

/// Whether the first-run setup shows: until it is finished or skipped, and only while nothing
/// is connected. Someone who already has a provider has no need of it, so that settles it.
pub fn needs_onboarding(store: Option<&Store>, providers: &Providers) -> bool {
    let Some(db) = store else {
        return false;
    };
    let done = db.setting::<bool>(ONBOARDED).unwrap_or_else(|e| {
        eprintln!("oido: could not read the setup state: {e}");
        None
    });
    if done == Some(true) {
        return false;
    }
    if !providers.connected().is_empty() {
        if let Err(e) = db.set_setting(ONBOARDED, &true) {
            eprintln!("oido: could not save the setup state: {e}");
        }
        return false;
    }
    true
}

/// Whether the app updates itself: on unless turned off on the Settings page.
pub fn auto_update(store: Option<&Store>) -> bool {
    let Some(db) = store else {
        return true;
    };
    db.setting::<bool>(AUTO_UPDATE)
        .unwrap_or_else(|e| {
            eprintln!("oido: could not read the update setting: {e}");
            None
        })
        .unwrap_or(true)
}

/// Whether schedules run (Settings → Schedules). On unless turned off.
pub fn run_schedules(store: Option<&Store>) -> bool {
    let Some(db) = store else {
        return true;
    };
    db.setting::<bool>(RUN_SCHEDULES)
        .unwrap_or_else(|e| {
            eprintln!("oido: could not read the schedules setting: {e}");
            None
        })
        .unwrap_or(true)
}

/// Whether TypeSafe judges each finished run for a skill (Settings → Context). Off unless turned on.
pub fn score_runs(store: Option<&Store>) -> bool {
    let Some(db) = store else {
        return false;
    };
    db.setting::<bool>(SCORE_RUNS)
        .unwrap_or_else(|e| {
            eprintln!("oido: could not read the run scoring setting: {e}");
            None
        })
        .unwrap_or(false)
}

/// Whether a message that takes several steps is planned first (Settings → Context). Off unless
/// turned on.
pub fn plan_on(store: Option<&Store>) -> bool {
    let Some(db) = store else {
        return false;
    };
    db.setting::<bool>(PLAN)
        .unwrap_or_else(|e| {
            eprintln!("oido: could not read the planning setting: {e}");
            None
        })
        .unwrap_or(false)
}

/// Each agent's own choice about judging its runs; agents not in it follow Settings.
pub fn score_overrides(store: Option<&Store>) -> std::collections::HashMap<String, bool> {
    let Some(db) = store else {
        return std::collections::HashMap::new();
    };
    db.setting(SCORE_AGENTS)
        .unwrap_or_else(|e| {
            eprintln!("oido: could not read the agents' run scoring choices: {e}");
            None
        })
        .unwrap_or_default()
}

/// The runs whose skill offer is still open.
pub fn skill_offers(store: Option<&Store>) -> Vec<String> {
    store
        .map(|db| {
            db.skill_offers().unwrap_or_else(|e| {
                eprintln!("oido: could not read the skill offers: {e}");
                Vec::new()
            })
        })
        .unwrap_or_default()
}

/// Whether TypeSafe sorts each message first (Settings → Context). Off unless turned on.
pub fn route_bulk(store: Option<&Store>) -> bool {
    let Some(db) = store else {
        return false;
    };
    db.setting::<bool>(ROUTE_BULK)
        .unwrap_or_else(|e| {
            eprintln!("oido: could not read the router setting: {e}");
            None
        })
        .unwrap_or(false)
}

/// Whether each side pane shows, and how wide: what the window was left with.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct Panes {
    sidebar: (bool, f32),
    inspector: (bool, f32),
}

impl Panes {
    fn of(app: &App) -> Panes {
        let w = |p: &PaneState| (p.is_open(), p.width());
        Panes {
            sidebar: w(&app.sidebar),
            inspector: w(&app.inspector),
        }
    }
}

fn wear(pane: &mut PaneState, (open, width): (bool, f32)) {
    pane.set_width(width);
    if !open {
        *pane = pane.collapsed();
    }
}

impl App {
    /// Saves the side panes when they differ from what was last seen. The panes change inside
    /// the toolkit (a button, a drag, a key, a peek), so the view checks once a frame.
    pub fn sync_panes(&self) {
        let now = Panes::of(self);
        if self
            .panes_seen
            .replace(Some(now))
            .is_some_and(|was| was != now)
        {
            self.save("the panes", |db| db.set_setting(PANES, &now));
        }
    }
}

/// Folds the agents sidebar back the way it was left.
pub(crate) fn restore_folds(app: &mut App) {
    let Some(db) = &app.store else { return };
    let folds = db
        .setting::<Folds>(SIDEBAR_FOLDS)
        .unwrap_or_else(|e| {
            eprintln!("oido: could not read the sidebar: {e}");
            None
        })
        .unwrap_or_default();
    app.pinned_closed = folds.pinned;
    app.others_closed = folds.others;
    app.roster.collapsed = folds.teams;
    let saved = db.setting::<Panes>(PANES).unwrap_or_else(|e| {
        eprintln!("oido: could not read the panes: {e}");
        None
    });
    if let Some(p) = saved {
        wear(&mut app.sidebar, p.sidebar);
        wear(&mut app.inspector, p.inspector);
    }
    app.panes_seen.set(Some(Panes::of(app)));
}

fn try_load(db: &Store) -> store::Result<(Roster, Providers)> {
    if db.setting::<bool>(SEEDED)?.unwrap_or(false) {
        let roster = Roster::from_saved(db.load_agents()?, db.setting::<Target>(SELECTED)?);
        let mut providers = Providers::from_saved(
            db.load_providers()?,
            db.load_models()?,
            db.setting::<ModelRef>(DEFAULT_MODEL)?,
        );
        // A provider can only be connected with a key. One that is not was cut short mid-connect.
        let cut_short = db.connected_without_key()?;
        for id in &cut_short {
            providers.disconnect(id);
        }
        if !cut_short.is_empty() {
            db.replace_providers(&providers.list, None)?;
        }
        return Ok((roster, providers));
    }
    let (roster, providers) = (Roster::new(), providers::init());
    db.save_roster(&roster.list)?;
    db.save_providers(&providers.list)?;
    if let Some(t) = &roster.selected {
        db.set_setting(SELECTED, t)?;
    }
    db.set_setting(SEEDED, &true)?;
    Ok((roster, providers))
}

impl App {
    /// Ends the first-run setup, finished or skipped, and goes to the chat.
    pub fn finish_onboarding(&mut self) {
        self.save("the setup state", |db| db.set_setting(ONBOARDED, &true));
        self.route = crate::app::Route::Chat;
    }

    pub(crate) fn save(&self, what: &str, f: impl FnOnce(&Store) -> store::Result<()>) {
        if let Some(db) = &self.store
            && let Err(e) = f(db)
        {
            eprintln!("oido: could not save {what}: {e}");
        }
    }

    /// Where an agent sits: itself, its parent, and its place among its siblings.
    fn place(&self, t: &Target) -> Option<(&crate::agents::Agent, Option<String>, usize)> {
        let agent = self.roster.find(t)?;
        match &t.parent {
            None => Some((
                agent,
                None,
                self.roster.list.iter().position(|a| a.id == t.id)?,
            )),
            Some(p) => {
                let parent = self.roster.list.iter().find(|a| &a.id == p)?;
                Some((
                    agent,
                    Some(p.clone()),
                    parent.subagents.iter().position(|a| a.id == t.id)?,
                ))
            }
        }
    }

    /// The agent's own row: its name, brief, model, pin, skills and tools, and which chat is open.
    pub(crate) fn save_agent_row(&self, t: &Target) {
        if let Some((a, parent, pos)) = self.place(t) {
            self.save("the agent", |db| db.save_agent(a, parent.as_deref(), pos));
        }
    }

    /// A new agent with its chats and team.
    pub(crate) fn save_agent_tree(&self, t: &Target) {
        if let Some((a, parent, pos)) = self.place(t) {
            self.save("the agent", |db| {
                db.save_agent_tree(a, parent.as_deref(), pos)
            });
        }
    }

    pub(crate) fn save_selection(&self) {
        match &self.roster.selected {
            Some(t) => self.save("the selection", |db| db.set_setting(SELECTED, t)),
            None => self.save("the selection", |db| db.delete_setting(SELECTED)),
        }
    }

    pub(crate) fn delete_saved_agent(&self, id: &str) {
        self.save("the deletion", |db| db.delete_agent(id));
    }

    /// The current chat's message at `seq`: added, or replaced after a change.
    pub(crate) fn save_message(&self, seq: usize) {
        let Some(chat) = self.current().map(|a| a.chat()) else {
            return;
        };
        if let Some(m) = chat.messages.get(seq) {
            self.save("the message", |db| db.put_message(&chat.id, seq, m));
        }
    }

    /// The current agent's open chat row, and the agent row (which chat is open).
    pub(crate) fn save_chat(&self) {
        let (Some(t), Some(a)) = (self.roster.selected.clone(), self.current()) else {
            return;
        };
        let (active, chat) = (a.active, a.chat());
        self.save("the chat", |db| db.save_session(&a.id, chat, active));
        self.save_agent_row(&t);
    }

    /// Chat `id`, already removed from `t`'s chats: deleted, with the chats left renumbered.
    pub(crate) fn delete_saved_chat(&self, t: &Target, id: &str) {
        if let Some(a) = self.roster.find(t) {
            self.save("the chat", |db| db.remove_session(id, &a.id, &a.sessions));
        }
        self.connector.forget(id);
    }

    pub(crate) fn save_providers(&self) {
        if let Err(why) = self.write_providers(None) {
            eprintln!("oido: could not save providers: {why}");
        }
    }

    /// Saves the provider list, and with it a verified key and its models if there is one, in
    /// one step.
    pub(crate) fn write_providers(&self, verified: Option<Verified>) -> Result<(), String> {
        let Some(db) = &self.store else {
            return Ok(());
        };
        db.replace_providers(&self.providers.list, verified)
            .map_err(|e| e.to_string())
    }

    /// Remembers (or forgets) the model agents use when they have none of their own.
    pub(crate) fn save_default_model(&self) {
        match &self.providers.default_model {
            Some(m) => self.save("the default model", |db| db.set_setting(DEFAULT_MODEL, m)),
            None => self.save("the default model", |db| db.delete_setting(DEFAULT_MODEL)),
        }
    }

    // ---- changes that also save: the screens call these, not the state directly

    pub fn toggle_pin(&mut self, id: &str) {
        self.roster.toggle_pin(id);
        self.save_agent_row(&Target::agent(id));
    }

    /// Moves an agent to place `to` in its sidebar section, or a member within its team, and
    /// saves the new order.
    pub fn move_agent(&mut self, t: &Target, to: usize) {
        if !self.roster.move_within(t, to) {
            return;
        }
        // Each row saves its place among its siblings, and every sibling's may have changed.
        let siblings: Vec<Target> = match &t.parent {
            None => self
                .roster
                .list
                .iter()
                .map(|a| Target::agent(a.id.clone()))
                .collect(),
            Some(p) => self
                .roster
                .list
                .iter()
                .filter(|a| &a.id == p)
                .flat_map(|a| &a.subagents)
                .map(|m| Target::member(p.clone(), m.id.clone()))
                .collect(),
        };
        for s in &siblings {
            self.save_agent_row(s);
        }
    }

    /// A drag on sidebar row `t`, sibling `index` of siblings `extents` tall (see
    /// [`RowDrag`]): it follows the pointer, and a drop moves it there.
    pub fn drag_row(&mut self, t: &Target, index: usize, extents: &[f32], d: &Drag, tx: &mut Tx) {
        match d.phase {
            Phase::Down => {}
            Phase::Move => {
                // The siblings it passes spring aside; it follows the pointer exactly.
                tx.animated();
                RowDrag::follow(&mut self.roster.drag, t, index, extents, d.dy);
            }
            Phase::Up => {
                let drop = self.roster.drag.take().and_then(|r| r.end(d.cancelled()));
                if let Some((t, to)) = drop {
                    self.move_agent(&t, to);
                }
            }
        }
    }

    /// Remembers which sections and teams of the agents sidebar are folded.
    pub(crate) fn save_folds(&self) {
        let folds = Folds {
            pinned: self.pinned_closed,
            others: self.others_closed,
            teams: self.roster.collapsed.clone(),
        };
        self.save("the sidebar", |db| db.set_setting(SIDEBAR_FOLDS, &folds));
    }

    pub fn toggle_team(&mut self, id: &str) {
        self.roster.toggle_team(id);
        self.save_folds();
    }

    pub fn toggle_pinned_section(&mut self) {
        self.pinned_closed ^= true;
        self.save_folds();
    }

    pub fn toggle_others_section(&mut self) {
        self.others_closed ^= true;
        self.save_folds();
    }

    /// Wears `appearance` and remembers it.
    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.save("the appearance", |db| {
            db.set_setting(APPEARANCE, &appearance)
        });
        self.look.appearance = appearance;
    }

    pub(crate) fn save_auto_update(&self) {
        let on = self.updates.auto;
        self.save("the update setting", |db| db.set_setting(AUTO_UPDATE, &on));
    }

    /// Picks the model for the current agent (the composer's picker, and the agent form's).
    pub fn set_model(&mut self, model: Option<ModelRef>) {
        let Some(t) = self.roster.selected.clone() else {
            return;
        };
        if let Some(a) = self.roster.current_mut() {
            a.model = model;
        }
        self.save_agent_row(&t);
    }

    /// Picks the current agent's working folder from the composer; `None` leaves it with none.
    pub fn set_workdir(&mut self, folder: Option<String>) {
        let Some(t) = self.roster.selected.clone() else {
            return;
        };
        if let Some(a) = self.roster.current_mut() {
            a.workdir = folder;
        }
        self.save_agent_row(&t);
    }
}

#[cfg(test)]
mod onboarding_tests {
    use super::*;
    use crate::vault::Vault;

    fn db() -> Store {
        Store::in_memory(Vault::fixed([1; 32])).unwrap()
    }

    #[test]
    fn first_run_shows_setup_until_skipped() {
        let db = db();
        let providers = providers::init();
        assert!(needs_onboarding(Some(&db), &providers));
        db.set_setting(ONBOARDED, &true).unwrap();
        assert!(!needs_onboarding(Some(&db), &providers));
    }

    #[test]
    fn a_connected_provider_settles_it_for_good() {
        let db = db();
        let mut providers = providers::init();
        let id = providers.list[0].id.clone();
        providers.connect(&id);
        assert!(!needs_onboarding(Some(&db), &providers));
        assert_eq!(db.setting::<bool>(ONBOARDED).unwrap(), Some(true));
        // Disconnecting later does not bring setup back.
        assert!(!needs_onboarding(Some(&db), &providers::init()));
    }

    #[test]
    fn no_store_means_no_setup() {
        assert!(!needs_onboarding(None, &providers::init()));
    }
}
