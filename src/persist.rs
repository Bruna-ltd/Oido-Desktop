//! Where the app's state meets the local store: what it starts with, and what each change saves.
//!
//! A failed save never stops the app: it is reported on stderr and the app carries on, since the
//! screen is already right and the next change saves again.

use crate::agents::{Roster, Target};
use crate::app::App;
use crate::models::ModelRef;
use crate::providers::{self, Providers};
use crate::store::{self, Store, Verified};
use crate::theme::Appearance;

const SEEDED: &str = "seeded";
const SELECTED: &str = "agents.selected";
const DEFAULT_MODEL: &str = "models.default";
const APPEARANCE: &str = "appearance";

/// What the app starts with: what the store holds, or on the very first launch the sample data
/// (written once, so deleting all of it stays deleted).
pub fn load(store: Option<&Store>) -> (Roster, Providers) {
    let Some(db) = store else {
        return (Roster::new(), providers::init());
    };
    try_load(db).unwrap_or_else(|e| {
        eprintln!("oido: could not read saved data, starting with samples: {e}");
        (Roster::new(), providers::init())
    })
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

    pub(crate) fn delete_saved_chat(&self, id: &str) {
        self.save("the chat", |db| db.delete_session(id));
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

    /// Wears `appearance` and remembers it.
    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.save("the appearance", |db| {
            db.set_setting(APPEARANCE, &appearance)
        });
        self.look.appearance = appearance;
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
}
