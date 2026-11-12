//! The agents: who they are, their team members, and the create, edit, pin and delete flows.
//! Mock data until `oido-core` serves `/api/agents`; the rules mirror Studio's sidebar and its
//! `create-form`, `edit-form` and `add-subagent-form`.

use crate::avatars::Avatar;
use crate::models::ModelRef;
use crate::picker::PickerState;
use rune::TextEdit;

use crate::model::{AgentStatus, Session, chat, new_id};

#[derive(Clone, PartialEq, Debug)]
/// An agent, or one of its team members. A team member is an agent too, but never has a team
/// of its own: the sidebar shows two levels at most.
pub struct Agent {
    pub id: String,
    pub name: String,
    /// What it should do: its system prompt.
    pub prompt: String,
    /// `None` uses the default model.
    pub model: Option<ModelRef>,
    /// The avatar's colour; `None` derives it from the name.
    pub hue: Option<f32>,
    /// A picture in place of the initial on that colour.
    pub avatar: Option<Avatar>,
    pub pinned: bool,
    pub status: AgentStatus,
    pub skills: Vec<String>,
    pub tools: Vec<String>,
    pub subagents: Vec<Agent>,
    /// The folder its commands run in; `None` is the home folder.
    pub workdir: Option<String>,
    /// Whether what it reads and runs there counts as yours: off, a file it read or a command's
    /// output is outside text (see `guard::taint`). Only you can change it, never an agent.
    pub trust_workdir: bool,
    /// What it answers about on channels; a channel message naming none of them is turned away
    /// (see `guard::intake`). Empty, it answers anything. Only you can change it.
    pub channel_topics: Vec<String>,
    /// For a team member: when its lead should hand it a task. The lead reads this to choose.
    pub description: String,
    pub active: usize,
    pub sessions: Vec<Session>,
}

impl Agent {
    pub(crate) fn blank(id: String) -> Agent {
        Agent {
            id,
            name: String::new(),
            prompt: String::new(),
            model: None,
            hue: None,
            avatar: None,
            pinned: false,
            status: AgentStatus::Idle,
            skills: Vec::new(),
            tools: Vec::new(),
            subagents: Vec::new(),
            workdir: None,
            trust_workdir: true,
            channel_topics: Vec::new(),
            description: String::new(),
            active: 0,
            sessions: vec![chat("New chat", Vec::new())],
        }
    }

    pub fn chat(&self) -> &Session {
        &self.sessions[self.active]
    }

    pub fn chat_mut(&mut self) -> &mut Session {
        &mut self.sessions[self.active]
    }

    pub fn hue(&self) -> f32 {
        self.hue.unwrap_or_else(|| hue_of(&self.name))
    }

    pub fn initial(&self) -> String {
        initial_of(&self.name)
    }

    /// The busiest state in the family: a team member waiting on you is the agent waiting on you.
    pub fn family_status(&self) -> AgentStatus {
        let all = std::iter::once(self.status).chain(self.subagents.iter().map(|s| s.status));
        let mut out = AgentStatus::Idle;
        for s in all {
            match s {
                AgentStatus::Waiting => return AgentStatus::Waiting,
                AgentStatus::Running => out = AgentStatus::Running,
                AgentStatus::Idle => {}
            }
        }
        out
    }
}

pub fn initial_of(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().collect())
}

/// A stable colour for a name, so an agent looks the same everywhere before anyone picks one.
pub fn hue_of(name: &str) -> f32 {
    let h = name
        .trim()
        .bytes()
        .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    (h % 360) as f32
}

/// Which agent is meant: a top-level one, or a team member of `parent`.
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Target {
    pub id: String,
    pub parent: Option<String>,
}

impl Target {
    pub fn agent(id: impl Into<String>) -> Target {
        Target {
            id: id.into(),
            parent: None,
        }
    }

    pub fn member(parent: impl Into<String>, id: impl Into<String>) -> Target {
        Target {
            id: id.into(),
            parent: Some(parent.into()),
        }
    }

    pub fn is_member(&self) -> bool {
        self.parent.is_some()
    }
}

/// The open row menu: anchored to its ⋯ button, or at a right-click.
#[derive(Clone)]
pub struct MenuOpen {
    pub target: Target,
    pub at: Option<(f32, f32)>,
}

#[derive(Clone, PartialEq, Eq)]
pub enum FormKind {
    NewAgent,
    NewMember(String),
    Edit(Target),
}

/// Starting points for the create page. They only fill in the name and the instructions;
/// everything stays editable, and nothing is made until Create.
pub struct Template {
    pub label: &'static str,
    pub hint: &'static str,
    pub name: &'static str,
    pub prompt: &'static str,
}

pub const TEMPLATES: [Template; 4] = [
    Template {
        label: "Customer support",
        hint: "Answers customer questions",
        name: "support-helper",
        prompt: "You answer questions from our customers about our product. Be friendly, brief and accurate.\n\nIf you are not sure about something, say so and offer to pass the question to a person. Never make up policies, prices or dates.",
    },
    Template {
        label: "Research assistant",
        hint: "Finds and compares sources",
        name: "research-assistant",
        prompt: "You research a topic thoroughly. Look for reliable sources, compare them, then give a short summary followed by the key points and where each one came from.\n\nSay clearly when sources disagree or when you are unsure.",
    },
    Template {
        label: "Email writer",
        hint: "Drafts emails in your voice",
        name: "email-writer",
        prompt: "You draft emails for me. If it is not clear who the email is for or what it should achieve, ask first.\n\nWrite in a warm, direct tone, keep it short, and always suggest a subject line.",
    },
    Template {
        label: "Meeting notes",
        hint: "Turns notes into action items",
        name: "meeting-notes",
        prompt: "You turn meeting notes or transcripts into a clear summary with three parts: decisions made, action items with an owner for each, and open questions.\n\nKeep it to one page.",
    },
];

pub const HUES: [f32; 8] = [25.0, 60.0, 150.0, 200.0, 250.0, 295.0, 330.0, 10.0];

/// The create, edit and team-member form. One set of fields, reused by all three.
pub struct Form {
    pub name: TextEdit,
    pub prompt: TextEdit,
    pub model: Option<ModelRef>,
    pub hue: Option<f32>,
    pub avatar: Option<Avatar>,
    /// Why the picture just dropped on the form could not be used.
    pub avatar_error: Option<String>,
    pub skills: Vec<String>,
    pub tools: Vec<String>,
    pub workdir: Option<String>,
    pub trust_workdir: bool,
    /// Its channel topics, as typed: split on commas when saved.
    pub topics: TextEdit,
    pub description: TextEdit,
    pub folders: crate::folders::Browser,
    pub template: Option<usize>,
    pub advanced: bool,
    pub picker: bool,
    pub model_picker: PickerState,
    pub skill_picker: crate::picker::MultiPicker,
    pub tool_picker: crate::picker::MultiPicker,
}

impl Form {
    fn empty() -> Form {
        Form {
            name: TextEdit::new(""),
            prompt: TextEdit::new(""),
            model: None,
            hue: None,
            avatar: None,
            avatar_error: None,
            skills: Vec::new(),
            tools: Vec::new(),
            workdir: None,
            trust_workdir: true,
            topics: TextEdit::new(""),
            description: TextEdit::new(""),
            folders: crate::folders::Browser::default(),
            template: None,
            advanced: false,
            picker: false,
            model_picker: PickerState::default(),
            skill_picker: crate::picker::MultiPicker::default(),
            tool_picker: crate::picker::MultiPicker::default(),
        }
    }

    /// A one-line summary of "More options", so what is set shows while it is closed.
    pub fn summary(&self) -> String {
        let plural = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
        match (self.skills.len(), self.tools.len()) {
            (0, 0) => "Skills and tools. Nothing added yet".to_owned(),
            (s, 0) => plural(s, "skill"),
            (0, t) => plural(t, "tool"),
            (s, t) => format!("{} · {}", plural(s, "skill"), plural(t, "tool")),
        }
    }
}

pub struct Roster {
    pub list: Vec<Agent>,
    pub selected: Option<Target>,
    /// Agents whose team is folded away in the sidebar.
    pub collapsed: Vec<String>,
    pub menu: Option<MenuOpen>,
    /// The agent waiting on a delete confirmation.
    pub confirm: Option<Target>,
    /// The form on screen, if any.
    pub editing: Option<FormKind>,
    pub form: Form,
}

impl Roster {
    pub fn new() -> Roster {
        Roster::from_saved(Vec::new(), None)
    }

    /// A roster of `list`, with `selected` open if it still exists, else the first agent.
    pub fn from_saved(mut list: Vec<Agent>, selected: Option<Target>) -> Roster {
        // Built-in tools are every agent's now; lists saved when they were chosen drop them.
        for a in &mut list {
            a.tools.retain(|t| !crate::tools::is_builtin(t));
            for m in &mut a.subagents {
                m.tools.retain(|t| !crate::tools::is_builtin(t));
            }
        }
        let first = list.first().map(|a| Target::agent(a.id.clone()));
        let mut r = Roster::bare(list);
        r.selected = selected.filter(|t| r.find(t).is_some()).or(first);
        r
    }

    fn bare(list: Vec<Agent>) -> Roster {
        let selected = None;
        Roster {
            list,
            selected,
            collapsed: Vec::new(),
            menu: None,
            confirm: None,
            editing: None,
            form: Form::empty(),
        }
    }

    pub fn find(&self, t: &Target) -> Option<&Agent> {
        match &t.parent {
            None => self.list.iter().find(|a| a.id == t.id),
            Some(p) => self
                .list
                .iter()
                .find(|a| &a.id == p)?
                .subagents
                .iter()
                .find(|a| a.id == t.id),
        }
    }

    pub fn find_mut(&mut self, t: &Target) -> Option<&mut Agent> {
        match &t.parent {
            None => self.list.iter_mut().find(|a| a.id == t.id),
            Some(p) => self
                .list
                .iter_mut()
                .find(|a| &a.id == p)?
                .subagents
                .iter_mut()
                .find(|a| a.id == t.id),
        }
    }

    pub fn current(&self) -> Option<&Agent> {
        self.selected.as_ref().and_then(|t| self.find(t))
    }

    pub fn current_mut(&mut self) -> Option<&mut Agent> {
        let t = self.selected.clone()?;
        self.find_mut(&t)
    }

    /// An agent or team member by id.
    pub fn agent_ref(&self, id: &str) -> Option<&Agent> {
        self.list.iter().find_map(|a| {
            if a.id == id {
                Some(a)
            } else {
                a.subagents.iter().find(|m| m.id == id)
            }
        })
    }

    /// An agent or team member by id.
    pub fn agent_mut(&mut self, id: &str) -> Option<&mut Agent> {
        self.list.iter_mut().find_map(|a| {
            if a.id == id {
                Some(a)
            } else {
                a.subagents.iter_mut().find(|m| m.id == id)
            }
        })
    }

    pub fn owner_of(&self, session_id: &str) -> Option<&Agent> {
        let holds = |a: &Agent| a.sessions.iter().any(|s| s.id == session_id);
        self.list.iter().find_map(|a| {
            if holds(a) {
                Some(a)
            } else {
                a.subagents.iter().find(|m| holds(m))
            }
        })
    }

    /// Which agent, or team member, holds the chat.
    pub fn target_of(&self, session_id: &str) -> Option<Target> {
        let holds = |a: &Agent| a.sessions.iter().any(|s| s.id == session_id);
        self.list.iter().find_map(|a| {
            if holds(a) {
                return Some(Target::agent(a.id.clone()));
            }
            a.subagents
                .iter()
                .find(|m| holds(m))
                .map(|m| Target::member(a.id.clone(), m.id.clone()))
        })
    }

    /// The agent, or team member, that holds the chat.
    pub fn owner_of_mut(&mut self, session_id: &str) -> Option<&mut Agent> {
        let holds = |a: &Agent| a.sessions.iter().any(|s| s.id == session_id);
        self.list.iter_mut().find_map(|a| {
            if holds(a) {
                Some(a)
            } else {
                a.subagents.iter_mut().find(|m| holds(m))
            }
        })
    }

    /// Every chat of an agent and its team.
    pub fn session_ids(a: &Agent) -> Vec<String> {
        a.sessions
            .iter()
            .chain(a.subagents.iter().flat_map(|m| m.sessions.iter()))
            .map(|s| s.id.clone())
            .collect()
    }

    pub fn select(&mut self, t: Target) {
        if self.find(&t).is_some() {
            self.selected = Some(t);
        }
    }

    pub fn pinned(&self) -> Vec<&Agent> {
        self.list.iter().filter(|a| a.pinned).collect()
    }

    pub fn unpinned(&self) -> Vec<&Agent> {
        self.list.iter().filter(|a| !a.pinned).collect()
    }

    pub fn team_open(&self, id: &str) -> bool {
        !self.collapsed.iter().any(|c| c == id)
    }

    pub fn toggle_team(&mut self, id: &str) {
        if let Some(i) = self.collapsed.iter().position(|c| c == id) {
            self.collapsed.remove(i);
        } else {
            self.collapsed.push(id.to_owned());
        }
    }

    /// Pins are for top-level agents only.
    pub fn toggle_pin(&mut self, id: &str) {
        if let Some(a) = self.list.iter_mut().find(|a| a.id == id) {
            a.pinned = !a.pinned;
        }
    }

    pub fn open_menu(&mut self, target: Target, at: Option<(f32, f32)>) {
        self.menu = Some(MenuOpen { target, at });
    }

    pub fn menu_open_for(&self, t: &Target) -> Option<&MenuOpen> {
        self.menu.as_ref().filter(|m| &m.target == t)
    }

    /// Removes an agent, or a team member. The selection falls back to the first agent left.
    pub fn delete(&mut self, t: &Target) {
        match &t.parent {
            None => self.list.retain(|a| a.id != t.id),
            Some(p) => {
                if let Some(parent) = self.list.iter_mut().find(|a| &a.id == p) {
                    parent.subagents.retain(|a| a.id != t.id);
                }
            }
        }
        self.confirm = None;
        self.menu = None;
        if self
            .selected
            .as_ref()
            .is_some_and(|s| self.find(s).is_none())
        {
            self.selected = self.list.first().map(|a| Target::agent(a.id.clone()));
        }
    }

    // ---- the form

    pub fn open_new(&mut self) {
        self.form = Form::empty();
        self.form.name.focused = true;
        self.editing = Some(FormKind::NewAgent);
    }

    /// Adds a team member to `parent`. A team member cannot have a team of its own.
    pub fn open_member(&mut self, parent: &str) {
        if !self.list.iter().any(|a| a.id == parent) {
            return;
        }
        self.form = Form::empty();
        self.form.name.focused = true;
        self.editing = Some(FormKind::NewMember(parent.to_owned()));
    }

    pub fn open_edit(&mut self, t: &Target) {
        let Some(a) = self.find(t) else { return };
        let mut form = Form::empty();
        form.name = TextEdit::new(a.name.clone());
        form.prompt = TextEdit::new(a.prompt.clone());
        form.model = a.model.clone();
        form.hue = a.hue;
        form.avatar = a.avatar.clone();
        form.skills = a.skills.clone();
        form.tools = a.tools.clone();
        form.workdir = a.workdir.clone();
        form.trust_workdir = a.trust_workdir;
        form.topics = TextEdit::new(a.channel_topics.join(", "));
        form.description = TextEdit::new(a.description.clone());
        form.advanced = !(a.skills.is_empty()
            && a.tools.is_empty()
            && a.workdir.is_none()
            && a.trust_workdir
            && a.channel_topics.is_empty());
        form.name.focused = true;
        self.form = form;
        self.editing = Some(FormKind::Edit(t.clone()));
    }

    pub fn close_form(&mut self) {
        self.editing = None;
        self.menu = None;
        self.form.skill_picker.close();
        self.form.tool_picker.close();
        self.form.folders.close();
    }

    /// Fills the name and the instructions from a template. Nothing else changes.
    pub fn pick_template(&mut self, i: usize) {
        let Some(t) = TEMPLATES.get(i) else { return };
        self.form.name = TextEdit::new(t.name);
        self.form.prompt = TextEdit::new(t.prompt);
        self.form.template = Some(i);
    }

    /// Gives the face a colour, which takes the picture off it: a face is one or the other.
    pub fn pick_colour(&mut self, hue: Option<f32>) {
        self.form.hue = hue;
        self.form.avatar = None;
        self.form.avatar_error = None;
    }

    /// Gives the face a picture. Its colour is left as it was.
    pub fn pick_avatar(&mut self, avatar: Avatar) {
        self.form.avatar = Some(avatar);
        self.form.avatar_error = None;
    }

    pub fn can_submit(&self) -> bool {
        self.editing.is_some() && !self.form.name.text.trim().is_empty()
    }

    pub fn toggle_skill(&mut self, name: &str) {
        toggle(&mut self.form.skills, name);
    }

    pub fn toggle_tool(&mut self, name: &str) {
        toggle(&mut self.form.tools, name);
    }

    /// Creates or saves, selects the result, and closes the form.
    pub fn submit(&mut self) -> Option<Target> {
        if !self.can_submit() {
            return None;
        }
        let kind = self.editing.take()?;
        let new_id = (!matches!(kind, FormKind::Edit(_))).then(|| self.fresh_id());
        // The form is spent once it is saved; the next open starts clean.
        let f = std::mem::replace(&mut self.form, Form::empty());
        let apply = |a: &mut Agent| {
            a.name = f.name.text.trim().to_owned();
            a.prompt = f.prompt.text.trim().to_owned();
            a.model = f.model.clone();
            a.hue = f.hue;
            a.avatar = f.avatar.clone();
            a.skills = f.skills.clone();
            a.tools = f.tools.clone();
            a.workdir = f.workdir.clone();
            a.trust_workdir = f.trust_workdir;
            a.channel_topics = crate::guard::topics(&f.topics.text);
            a.description = f.description.text.trim().to_owned();
        };
        let target = match kind {
            FormKind::NewAgent => {
                let id = new_id.unwrap_or_default();
                let mut a = Agent::blank(id.clone());
                apply(&mut a);
                self.list.push(a);
                Target::agent(id)
            }
            FormKind::NewMember(parent) => {
                let id = new_id.unwrap_or_default();
                let mut a = Agent::blank(id.clone());
                apply(&mut a);
                if let Some(p) = self.list.iter_mut().find(|a| a.id == parent) {
                    p.subagents.push(a);
                }
                self.collapsed.retain(|c| c != &parent);
                Target::member(parent, id)
            }
            FormKind::Edit(t) => {
                if let Some(a) = self.find_mut(&t) {
                    apply(a);
                }
                t
            }
        };
        self.selected = Some(target.clone());
        Some(target)
    }

    fn fresh_id(&mut self) -> String {
        new_id()
    }
}

fn toggle(list: &mut Vec<String>, name: &str) {
    if let Some(i) = list.iter().position(|x| x == name) {
        list.remove(i);
    } else {
        list.push(name.to_owned());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::demo::sample_agents;

    /// A roster with the sample agents: Atlas and Forge pinned, each with a team member.
    fn roster() -> Roster {
        Roster::from_saved(sample_agents(), None)
    }

    fn named(r: &mut Roster, name: &str) {
        r.form.name = TextEdit::new(name);
    }

    #[test]
    fn creating_needs_a_name_then_adds_and_selects() {
        let mut r = roster();
        let before = r.list.len();
        r.open_new();
        assert!(!r.can_submit(), "a name is required");
        assert!(r.submit().is_none());
        named(&mut r, "  helper  ");
        r.form.prompt = TextEdit::new("Help.");
        let t = r.submit().unwrap();
        assert_eq!(r.list.len(), before + 1);
        assert_eq!(r.find(&t).unwrap().name, "helper");
        assert_eq!(r.selected, Some(t));
        assert!(r.editing.is_none());
    }

    #[test]
    fn a_template_fills_only_the_name_and_instructions() {
        let mut r = roster();
        r.open_new();
        r.form.model = Some(ModelRef::new("anthropic", "claude-opus-5-5"));
        r.pick_template(2);
        assert_eq!(r.form.name.text, "email-writer");
        assert!(r.form.prompt.text.contains("draft emails"));
        assert_eq!(
            r.form.model,
            Some(ModelRef::new("anthropic", "claude-opus-5-5"))
        );
    }

    #[test]
    fn team_members_nest_one_level_and_expand_their_parent() {
        let mut r = roster();
        r.collapsed.push("scout".into());
        r.open_member("scout");
        named(&mut r, "triager");
        let t = r.submit().unwrap();
        assert!(t.is_member());
        assert_eq!(r.find(&Target::agent("scout")).unwrap().subagents.len(), 1);
        assert!(r.team_open("scout"), "a new member shows");
        r.open_member("nobody");
        assert!(r.editing.is_none(), "no parent, no form");
    }

    #[test]
    fn editing_prefills_and_saves_in_place() {
        let mut r = roster();
        let t = Target::member("forge", "reviewer");
        r.open_edit(&t);
        assert_eq!(r.form.name.text, "reviewer");
        named(&mut r, "careful-reviewer");
        r.toggle_skill("tests");
        r.submit().unwrap();
        let a = r.find(&t).unwrap();
        assert_eq!(a.name, "careful-reviewer");
        assert_eq!(a.skills, vec!["tests".to_owned()]);
        assert_eq!(r.list.len(), 4, "editing adds nobody");
    }

    #[test]
    fn a_face_is_a_colour_or_a_picture_and_is_kept_with_the_agent() {
        use crate::avatars::Avatar;
        let mut r = roster();
        let t = Target::agent("scout");
        r.open_edit(&t);
        r.pick_avatar(Avatar::Preset("chef"));
        r.pick_colour(Some(HUES[2]));
        assert!(r.form.avatar.is_none(), "a colour takes the picture off");
        r.pick_avatar(Avatar::Preset("chef"));
        r.submit().unwrap();
        assert_eq!(r.find(&t).unwrap().avatar, Some(Avatar::Preset("chef")));
        r.open_edit(&t);
        assert_eq!(r.form.avatar, Some(Avatar::Preset("chef")));
        r.form.avatar_error = Some("old trouble".into());
        r.pick_colour(None);
        assert!(
            r.form.avatar_error.is_none(),
            "a new choice forgets the last mistake"
        );
        r.submit().unwrap();
        assert_eq!(
            r.find(&t).unwrap().avatar,
            None,
            "and the colour comes back"
        );
    }

    #[test]
    fn deleting_the_selected_agent_falls_back_and_removing_a_member_keeps_the_parent() {
        let mut r = roster();
        r.select(Target::member("forge", "reviewer"));
        r.delete(&Target::member("forge", "reviewer"));
        assert!(
            r.find(&Target::agent("forge"))
                .unwrap()
                .subagents
                .is_empty()
        );
        assert_eq!(
            r.selected,
            Some(Target::agent("atlas")),
            "selection fell back"
        );
        r.delete(&Target::agent("atlas"));
        assert!(r.find(&Target::agent("atlas")).is_none());
        assert_eq!(r.selected, Some(Target::agent("forge")));
        for id in ["forge", "scout", "ledger"] {
            r.delete(&Target::agent(id));
        }
        assert!(r.selected.is_none() && r.current().is_none());
    }

    #[test]
    fn pins_apply_to_top_level_agents_and_group_them() {
        let mut r = roster();
        assert_eq!(r.pinned().len(), 2);
        r.toggle_pin("scout");
        assert_eq!(r.pinned().len(), 3);
        r.toggle_pin("atlas");
        assert_eq!(r.unpinned().len(), 2);
    }

    #[test]
    fn a_team_member_waiting_on_you_means_the_agent_is() {
        let mut r = roster();
        let forge = r.list.iter_mut().find(|a| a.id == "forge").unwrap();
        forge.status = AgentStatus::Idle;
        forge.subagents[0].status = AgentStatus::Waiting;
        assert_eq!(
            r.find(&Target::agent("forge")).unwrap().family_status(),
            AgentStatus::Waiting
        );
    }
}
