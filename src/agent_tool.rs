//! The `agents` tool: an agent lists, creates, changes and deletes agents and team members.
//! `list` and `models` answer at once. `create`, `update` and `delete` become a draft shown in the
//! chat and change nothing until the person approves it there: an agent can be given tools that
//! run commands or change files, so none is made or widened without them. Drafts are kept while
//! the app runs; one still waiting at a restart is gone, and the agent can be asked again.

use serde_json::{Value, json};

use crate::agents::{Agent, Target};
use crate::app::{App, Route};
use crate::completion::ToolSpec;
use crate::model::Message;
use crate::models::ModelRef;

pub const NAME: &str = "agents";

/// The most models `models` lists at once.
const MODELS_SHOWN: usize = 40;
/// How much of an agent's instructions `list` shows.
const BRIEF_SHOWN: usize = 160;

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: NAME.into(),
        description: "Manage the agents in this app, yourself included, and their team members. `list` shows every agent with its model, tools, skills and folder, and the tools and skills that can be given. `models` lists the models that can be chosen, filtered by `query`. `create`, `update` and `delete` show the person a draft and take effect only once they approve it; say so, and stop there. An agent is named by `agent`: its name or id from `list`. A team member is created with `lead`, the agent whose team it joins; a team member has no team of its own, and its `description` tells its lead when to hand it a task. For update, only the fields given change. You cannot delete yourself or your lead.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "models", "create", "update", "delete"]},
                "agent": {"type": "string", "description": "For update and delete: the agent's name or id."},
                "name": {"type": "string", "description": "Its name; for update, a new name."},
                "instructions": {"type": "string", "description": "What it should do, in full: its system prompt."},
                "description": {"type": "string", "description": "For a team member: when its lead should hand it a task."},
                "lead": {"type": "string", "description": "For create: the agent whose team it joins. Leave out for an agent of its own."},
                "model": {"type": "string", "description": "A model id from `models`, or \"default\" for the app's default model."},
                "provider": {"type": "string", "description": "With model: the provider, when more than one offers that id."},
                "tools": {"type": "array", "items": {"type": "string"}, "description": "MCP and TypeSafe tools it has, by name, from `list`. Every agent has the app's own tools already. The whole list: for update, it replaces the old one."},
                "skills": {"type": "array", "items": {"type": "string"}, "description": "The skills it has, by name, from `list`. The whole list."},
                "folder": {"type": ["string", "null"], "description": "Its working folder, an absolute path; null or \"\" for the home folder."},
                "query": {"type": "string", "description": "For models: words the model's id or name contains."}
            },
            "required": ["action"]
        }),
    }
}

/// A line saying what a call does, for lists: "create: researcher".
pub fn summary(args: &str) -> String {
    let v: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let part = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    match (part("action"), part("agent").or_else(|| part("name"))) {
        (Some(action), Some(who)) => format!("{action}: {who}"),
        (Some(action), None) => action.to_owned(),
        _ => args.to_owned(),
    }
}

/// What a call asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    List,
    Models(Option<String>),
    Create {
        fields: Fields,
        lead: Option<String>,
    },
    Update {
        agent: String,
        fields: Fields,
    },
    Delete(String),
}

/// The parts of an agent a call gives; `None` is "not given" (for an update, "keep it").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fields {
    pub name: Option<String>,
    pub instructions: Option<String>,
    pub description: Option<String>,
    pub model: Option<Pick>,
    pub tools: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    /// `Some(None)` is the home folder.
    pub folder: Option<Option<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    Default,
    Model {
        id: String,
        provider: Option<String>,
    },
}

/// Reads a call's arguments. Errs, in words the model can act on.
pub fn parse(args: &str) -> Result<Call, String> {
    let v: Value = serde_json::from_str(args).map_err(|_| "the arguments are not JSON")?;
    let text = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let names = |key: &str| -> Result<Option<Vec<String>>, String> {
        match v.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(items)) => items
                .iter()
                .map(|i| {
                    i.as_str()
                        .map(|s| s.trim().to_owned())
                        .ok_or_else(|| format!("every item of `{key}` must be a name"))
                })
                .collect::<Result<_, _>>()
                .map(Some),
            Some(_) => Err(format!("`{key}` must be a list of names")),
        }
    };
    let fields = || -> Result<Fields, String> {
        let model = match text("model") {
            None => None,
            Some(m) if m.eq_ignore_ascii_case("default") => Some(Pick::Default),
            Some(id) => Some(Pick::Model {
                id,
                provider: text("provider"),
            }),
        };
        let folder = match v.get("folder") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(s)) if s.trim().is_empty() => Some(None),
            Some(Value::String(s)) => Some(Some(s.trim().to_owned())),
            Some(_) => return Err("`folder` must be a path".into()),
        };
        Ok(Fields {
            name: text("name"),
            instructions: v
                .get("instructions")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_owned()),
            description: v
                .get("description")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_owned()),
            model,
            tools: names("tools")?,
            skills: names("skills")?,
            folder,
        })
    };
    let agent = || text("agent").ok_or_else(|| "give the `agent`: its name or id".to_owned());
    match text("action").as_deref() {
        Some("list") => Ok(Call::List),
        Some("models") => Ok(Call::Models(text("query"))),
        Some("create") => {
            let f = fields()?;
            if f.name.is_none() {
                return Err("give the new agent a `name`".into());
            }
            Ok(Call::Create {
                fields: f,
                lead: text("lead"),
            })
        }
        Some("update") => {
            let f = fields()?;
            if f == Fields::default() {
                return Err(
                    "say what to change: name, instructions, description, model, tools, skills or folder"
                        .into(),
                );
            }
            Ok(Call::Update {
                agent: agent()?,
                fields: f,
            })
        }
        Some("delete") => Ok(Call::Delete(agent()?)),
        Some(other) => Err(format!("there is no action {other:?}")),
        None => Err("give an `action`".into()),
    }
}

/// A change an agent asked for, waiting for the person in the chat it was asked in.
#[derive(Clone, Debug)]
pub struct Draft {
    /// The tool call that asked for it.
    pub call_id: String,
    pub session_id: String,
    pub proposal: Proposal,
}

#[derive(Clone, Debug)]
pub enum Proposal {
    /// A new agent, or with `lead` a team member of that agent.
    Create {
        agent: Agent,
        lead: Option<String>,
    },
    /// The agent as it would be, and which of its parts change, in words.
    Update {
        target: Target,
        agent: Agent,
        changed: Vec<&'static str>,
    },
    Delete {
        target: Target,
        name: String,
    },
}

impl App {
    /// Answers the chat's waiting `agents` calls.
    pub(crate) fn run_agent_calls(&mut self, sid: &str) {
        let calls: Vec<(String, String)> = match self
            .roster
            .owner_of(sid)
            .and_then(|a| a.sessions.iter().find(|s| s.id == sid))
            .and_then(|s| s.messages.last())
        {
            Some(Message::Assistant { tools, .. }) => tools
                .iter()
                .filter(|t| t.name == NAME && t.output.is_none())
                .map(|t| (t.id.clone(), t.args.clone()))
                .collect(),
            _ => return,
        };
        for (id, args) in calls {
            let told = self
                .agent_call(sid, &id, &args)
                .unwrap_or_else(|why| format!("Error: {why}."));
            self.resolve(sid, &id, told, true);
        }
    }

    fn agent_call(&mut self, sid: &str, call_id: &str, args: &str) -> Result<String, String> {
        let proposal = match parse(args)? {
            Call::List => return Ok(self.agents_for_agent()),
            Call::Models(query) => return Ok(self.models_for_agent(query.as_deref())),
            Call::Create { fields, lead } => self.propose_create(fields, lead.as_deref())?,
            Call::Update { agent, fields } => self.propose_update(&agent, fields)?,
            Call::Delete(agent) => {
                let target = self.agent_named(&agent)?;
                let own = self.roster.target_of(sid);
                let mine = own.as_ref().is_some_and(|o| {
                    o.id == target.id || o.parent.as_deref() == Some(target.id.as_str())
                });
                if mine {
                    return Err("you cannot delete yourself or your lead".into());
                }
                let name = self
                    .roster
                    .find(&target)
                    .map(|a| a.name.clone())
                    .unwrap_or(agent);
                Proposal::Delete { target, name }
            }
        };
        // What the agent will have, stated, so it is not described as having tools it lacks.
        let with = |a: &Agent| match a.tools.len() {
            0 => " with only the app's own tools".to_owned(),
            _ => format!(" with the app's own tools and {}", a.tools.join(", ")),
        };
        let what = match &proposal {
            Proposal::Create { agent, lead: None } => {
                format!("create the agent {:?}{}", agent.name, with(agent))
            }
            Proposal::Create {
                agent,
                lead: Some(lead),
            } => format!(
                "add {:?} to the team of {:?}{}",
                agent.name,
                self.roster.agent_ref(lead).map_or("?", |a| a.name.as_str()),
                with(agent)
            ),
            Proposal::Update { agent, changed, .. } => {
                let tools = if changed.contains(&"tools") {
                    with(agent)
                } else {
                    String::new()
                };
                format!("change {:?} ({}){tools}", agent.name, changed.join(", "))
            }
            Proposal::Delete { name, .. } => format!("delete {name:?} and its chats"),
        };
        self.agent_drafts.retain(|d| d.call_id != call_id);
        self.agent_drafts.push(Draft {
            call_id: call_id.to_owned(),
            session_id: sid.to_owned(),
            proposal,
        });
        Ok(format!(
            "The person now sees a draft to {what} in this chat. Nothing changes until they approve it; tell them it is waiting for them."
        ))
    }

    /// The agent or team member named `who`, by id, or by name in any case.
    fn agent_named(&self, who: &str) -> Result<Target, String> {
        let all = self.roster.list.iter().flat_map(|a| {
            std::iter::once((a, None))
                .chain(a.subagents.iter().map(move |m| (m, Some(a.id.clone()))))
        });
        let found: Vec<Target> = all
            .filter(|(a, _)| a.id == who || a.name.trim().eq_ignore_ascii_case(who.trim()))
            .map(|(a, parent)| Target {
                id: a.id.clone(),
                parent,
            })
            .collect();
        match &found[..] {
            [one] => Ok(one.clone()),
            [] => Err(format!(
                "there is no agent called {who:?}; `list` shows them"
            )),
            _ => Err(format!(
                "several agents are called {who:?}; give its id from `list`"
            )),
        }
    }

    fn propose_create(&self, fields: Fields, lead: Option<&str>) -> Result<Proposal, String> {
        let name = fields.name.clone().unwrap_or_default();
        if self.agent_named(&name).is_ok() {
            return Err(format!("there is already an agent called {name:?}"));
        }
        let lead = match lead {
            None => None,
            Some(who) => {
                let t = self.agent_named(who)?;
                if t.is_member() {
                    return Err(format!("{who} is a team member, and cannot have a team"));
                }
                Some(t.id)
            }
        };
        let mut agent = Agent::blank(crate::model::new_id());
        self.apply_fields(&mut agent, fields)?;
        Ok(Proposal::Create { agent, lead })
    }

    fn propose_update(&self, who: &str, fields: Fields) -> Result<Proposal, String> {
        let target = self.agent_named(who)?;
        let old = self.roster.find(&target).ok_or("that agent is gone")?;
        if let Some(name) = &fields.name
            && let Ok(other) = self.agent_named(name)
            && other.id != target.id
        {
            return Err(format!("there is already an agent called {name:?}"));
        }
        let mut agent = old.clone();
        self.apply_fields(&mut agent, fields)?;
        let changed: Vec<&'static str> = [
            ("name", old.name != agent.name),
            ("instructions", old.prompt != agent.prompt),
            ("description", old.description != agent.description),
            ("model", old.model != agent.model),
            ("tools", old.tools != agent.tools),
            ("skills", old.skills != agent.skills),
            ("folder", old.workdir != agent.workdir),
        ]
        .into_iter()
        .filter_map(|(what, differs)| differs.then_some(what))
        .collect();
        if changed.is_empty() {
            return Err(format!("that is how {} is already set", old.name));
        }
        Ok(Proposal::Update {
            target,
            agent,
            changed,
        })
    }

    /// Checks what the call gives against what exists, and sets it on `agent`.
    fn apply_fields(&self, agent: &mut Agent, f: Fields) -> Result<(), String> {
        if let Some(name) = f.name {
            agent.name = name;
        }
        if let Some(p) = f.instructions {
            agent.prompt = p;
        }
        if let Some(d) = f.description {
            agent.description = d;
        }
        if let Some(pick) = f.model {
            agent.model = match pick {
                Pick::Default => None,
                Pick::Model { id, provider } => Some(self.model_named(&id, provider.as_deref())?),
            };
        }
        if let Some(tools) = f.tools {
            let (mut found, mut wrong) = (Vec::new(), Vec::new());
            for t in &tools {
                match self.tool_named(t) {
                    Ok(name) => found.push(name),
                    Err(why) => wrong.push(why),
                }
            }
            if !wrong.is_empty() {
                return Err(format!("{}; `list` shows the tools", wrong.join("; ")));
            }
            // The app's own tools are every agent's, so only the others are kept.
            found.retain(|t| !crate::tools::is_builtin(t));
            agent.tools = dedup(found);
        }
        if let Some(skills) = f.skills {
            let known: Vec<String> = self.skill_names();
            let unknown: Vec<&str> = skills
                .iter()
                .filter(|s| !known.contains(s))
                .map(String::as_str)
                .collect();
            if !unknown.is_empty() {
                return Err(format!(
                    "there is no skill called {}; `list` shows the skills",
                    unknown.join(", ")
                ));
            }
            agent.skills = dedup(skills);
        }
        if let Some(folder) = f.folder {
            agent.workdir = match folder {
                None => None,
                Some(path) => {
                    let p = std::path::Path::new(&path);
                    if !p.is_absolute() {
                        return Err(format!("{path:?} is not an absolute path"));
                    }
                    if !p.is_dir() {
                        return Err(format!("there is no folder {path}"));
                    }
                    Some(path)
                }
            };
        }
        Ok(())
    }

    /// The name a tool is given to an agent by. An MCP server's tool is known by its full name
    /// (`mcp__server__tool`), and also by the server's own name for it, as `mcp_servers list`
    /// shows it, when only one server has a tool of that name. Case and punctuation do not
    /// matter. Errs, in words, naming what was meant if it is unclear.
    fn tool_named(&self, given: &str) -> Result<String, String> {
        let norm = |s: &str| -> String {
            s.chars()
                .filter(char::is_ascii_alphanumeric)
                .map(|c| c.to_ascii_lowercase())
                .collect()
        };
        let all: Vec<String> = self.all_tools().into_iter().map(|t| t.name).collect();
        if all.iter().any(|t| t == given) {
            return Ok(given.to_owned());
        }
        let wanted = norm(given);
        let matches: Vec<&String> = all
            .iter()
            .filter(|name| {
                let mut aliases = vec![norm(name)];
                if let Some((server, tool)) = crate::mcp::resolve(&self.mcp.servers, name) {
                    aliases.push(norm(&tool.name));
                    aliases.push(norm(&format!("{}{}", server.name, tool.name)));
                }
                aliases.contains(&wanted)
            })
            .collect();
        match &matches[..] {
            [one] => Ok((*one).clone()),
            [] => Err(format!("there is no tool called {given}")),
            many => Err(format!(
                "{given} could be {}; give one of these names",
                many.iter()
                    .map(|m| m.as_str())
                    .collect::<Vec<_>>()
                    .join(" or ")
            )),
        }
    }

    /// The model with this id among the connected providers'.
    fn model_named(&self, id: &str, provider: Option<&str>) -> Result<ModelRef, String> {
        let offers: Vec<String> = self
            .providers
            .groups()
            .into_iter()
            .filter(|g| provider.is_none_or(|p| g.provider == p))
            .filter(|g| g.models.iter().any(|m| m.id == id))
            .map(|g| g.provider)
            .collect();
        match &offers[..] {
            [one] => Ok(ModelRef::new(one, id)),
            [] => Err(format!(
                "no connected provider offers the model {id:?}; `models` lists them"
            )),
            _ => Err(format!(
                "{} all offer {id:?}; say which with `provider`",
                offers.join(", ")
            )),
        }
    }

    fn skill_names(&self) -> Vec<String> {
        self.skills_dir
            .as_deref()
            .map(crate::skills::discover)
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.name)
            .collect()
    }

    /// Every agent, then what can be given, for the agent.
    fn agents_for_agent(&self) -> String {
        let model = |m: &Option<ModelRef>| match m {
            Some(m) => format!("{} ({})", m.id, m.provider),
            None => "the default model".to_owned(),
        };
        let line = |a: &Agent, indent: &str| {
            let mut brief: String = a.prompt.split_whitespace().collect::<Vec<_>>().join(" ");
            if brief.chars().count() > BRIEF_SHOWN {
                brief = brief.chars().take(BRIEF_SHOWN).collect::<String>() + "…";
            }
            let mut l = format!("{indent}- {} (id {}): {}", a.name, a.id, model(&a.model));
            if !a.description.is_empty() {
                l.push_str(&format!("; for: {}", a.description));
            }
            l.push_str(&format!(
                "; folder: {}",
                a.workdir.as_deref().unwrap_or("home")
            ));
            if !a.tools.is_empty() {
                l.push_str(&format!("; tools: {}", a.tools.join(", ")));
            }
            if !a.skills.is_empty() {
                l.push_str(&format!("; skills: {}", a.skills.join(", ")));
            }
            if !brief.is_empty() {
                l.push_str(&format!("; instructions: {brief}"));
            }
            l
        };
        let mut out = vec!["Agents (team members indented under their lead):".to_owned()];
        for a in &self.roster.list {
            out.push(line(a, ""));
            out.extend(a.subagents.iter().map(|m| line(m, "  ")));
        }
        out.push(format!(
            "Default model: {}",
            model(&self.providers.default_model)
        ));
        // The app's own tools with what each does; an MCP server's by name only, since a server
        // can have a hundred.
        let own: Vec<String> = crate::tools::registered()
            .into_iter()
            .filter(|t| crate::tools::is_builtin(&t.name))
            .map(|t| {
                let shown = crate::tools::shown(&t.name, &self.mcp.servers);
                format!("{} ({})", t.name, shown.about)
            })
            .collect();
        out.push(format!(
            "Every agent has these tools, without choosing them: {}",
            own.join("; ")
        ));
        if self.typesafe_connected() {
            let typesafe: Vec<String> = crate::typesafe::specs()
                .into_iter()
                .map(|t| t.name)
                .collect();
            out.push(format!(
                "TypeSafe tools that can be given: {}",
                typesafe.join(", ")
            ));
        }
        for s in self.mcp.servers.iter().filter(|s| s.offers()) {
            let names: Vec<String> = s.tools.iter().map(|t| s.qualified(&t.name)).collect();
            out.push(format!(
                "Tools from the MCP server {}: {}",
                s.name,
                names.join(", ")
            ));
        }
        let skills = self.skill_names();
        out.push(if skills.is_empty() {
            "No skills are installed.".to_owned()
        } else {
            format!("Skills that can be given: {}", skills.join(", "))
        });
        out.join("\n")
    }

    /// The models that can be chosen, those matching `query` first few.
    fn models_for_agent(&self, query: Option<&str>) -> String {
        let words: Vec<String> = query
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let all: Vec<String> = self
            .providers
            .groups()
            .into_iter()
            .flat_map(|g| g.models.into_iter().map(move |m| (g.provider.clone(), m)))
            .filter(|(_, m)| {
                let hay = format!("{} {}", m.id, m.name).to_lowercase();
                words.iter().all(|w| hay.contains(w))
            })
            .map(|(p, m)| format!("- {} (provider {p}): {}", m.id, m.name))
            .collect();
        if all.is_empty() {
            return "No connected model matches.".to_owned();
        }
        let more = all.len().saturating_sub(MODELS_SHOWN);
        let mut out: Vec<String> = all.into_iter().take(MODELS_SHOWN).collect();
        if more > 0 {
            out.push(format!("…and {more} more; narrow it with `query`."));
        }
        out.join("\n")
    }

    // ---- the draft card ----

    fn take_agent_draft(&mut self, call_id: &str) -> Option<Draft> {
        let i = self
            .agent_drafts
            .iter()
            .position(|d| d.call_id == call_id)?;
        self.agent_draft_notice = None;
        Some(self.agent_drafts.remove(i))
    }

    /// The card's Approve: makes the change. One that can no longer be made (its lead or the
    /// agent is gone, the name was taken) keeps the draft and says why on the card.
    pub fn approve_agent_draft(&mut self, call_id: &str) {
        let Some(draft) = self.take_agent_draft(call_id) else {
            return;
        };
        let failed = match draft.proposal.clone() {
            Proposal::Create { agent, lead } => self.create_from_draft(agent, lead).err(),
            Proposal::Update { target, agent, .. } => match self.roster.find_mut(&target) {
                Some(a) => {
                    a.name = agent.name;
                    a.prompt = agent.prompt;
                    a.description = agent.description;
                    a.model = agent.model;
                    a.tools = agent.tools;
                    a.skills = agent.skills;
                    a.workdir = agent.workdir;
                    self.save_agent_row(&target);
                    None
                }
                None => Some("That agent is gone.".to_owned()),
            },
            Proposal::Delete { target, .. } => {
                self.delete_agent_now(&target);
                None
            }
        };
        if let Some(why) = failed {
            self.agent_draft_notice = Some((draft.call_id.clone(), why));
            self.agent_drafts.push(draft);
        }
    }

    fn create_from_draft(&mut self, agent: Agent, lead: Option<String>) -> Result<(), String> {
        if self.agent_named(&agent.name).is_ok() {
            return Err(format!("There is already an agent called {}.", agent.name));
        }
        let target = match lead {
            None => {
                let t = Target::agent(agent.id.clone());
                self.roster.list.push(agent);
                t
            }
            Some(lead) => {
                let t = Target::member(lead.clone(), agent.id.clone());
                let parent = self
                    .roster
                    .list
                    .iter_mut()
                    .find(|a| a.id == lead)
                    .ok_or("Its lead is gone.")?;
                parent.subagents.push(agent);
                self.roster.collapsed.retain(|c| c != &lead);
                t
            }
        };
        self.save_agent_tree(&target);
        Ok(())
    }

    /// The card's Edit: the agent form, filled in with the draft, to change and save from there.
    pub fn edit_agent_draft(&mut self, call_id: &str) {
        let Some(draft) = self.take_agent_draft(call_id) else {
            return;
        };
        let agent = match draft.proposal {
            Proposal::Create { agent, lead: None } => {
                self.new_agent();
                agent
            }
            Proposal::Create {
                agent,
                lead: Some(lead),
            } => {
                self.new_member(&lead);
                agent
            }
            Proposal::Update { target, agent, .. } => {
                self.edit_agent(&target);
                agent
            }
            Proposal::Delete { .. } => return,
        };
        if self.route != Route::AgentForm {
            return;
        }
        let f = &mut self.roster.form;
        f.name = rune::TextEdit::new(agent.name);
        f.prompt = rune::TextEdit::new(agent.prompt);
        f.description = rune::TextEdit::new(agent.description);
        f.model = agent.model;
        f.advanced =
            !(agent.skills.is_empty() && agent.tools.is_empty() && agent.workdir.is_none());
        f.tools = agent.tools;
        f.skills = agent.skills;
        f.workdir = agent.workdir;
    }

    pub fn reject_agent_draft(&mut self, call_id: &str) {
        self.take_agent_draft(call_id);
    }
}

/// The names in their first order, each once.
fn dedup(names: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(names.len());
    for n in names {
        if !n.is_empty() && !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::TempDb;

    fn call(app: &mut App, sid: &str, args: Value) -> Result<String, String> {
        app.agent_call(sid, "c1", &args.to_string())
    }

    fn chat_of(app: &App, name: &str) -> String {
        let t = app.agent_named(name).unwrap();
        app.roster.find(&t).unwrap().chat().id.clone()
    }

    #[test]
    fn arguments_are_read_and_mistakes_refused_in_words() {
        let Call::Update { agent, fields } = parse(
            r#"{"action":"update","agent":"Atlas","tools":["shell","shell"],"folder":null,"model":"default"}"#,
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(agent, "Atlas");
        assert_eq!(fields.folder, Some(None));
        assert_eq!(fields.model, Some(Pick::Default));
        assert_eq!(fields.name, None, "what is not given is kept");
        for (args, said) in [
            (r#"{"action":"create"}"#, "`name`"),
            (r#"{"action":"update","agent":"x"}"#, "say what to change"),
            (r#"{"action":"delete"}"#, "`agent`"),
            (
                r#"{"action":"create","name":"x","tools":"shell"}"#,
                "list of names",
            ),
            (r#"{"action":"fly"}"#, "no action"),
        ] {
            assert!(parse(args).unwrap_err().contains(said), "{args}");
        }
        assert_eq!(
            summary(r#"{"action":"delete","agent":"Forge"}"#),
            "delete: Forge"
        );
    }

    #[test]
    fn a_created_agent_waits_for_approval_and_is_saved_with_what_was_checked() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        let sid = app.roster.list[0].chat().id.clone();
        let before = app.roster.list.len();
        let dir = std::env::temp_dir();

        let unknown = call(
            &mut app,
            &sid,
            json!({"action": "create", "name": "x", "tools": ["fly"]}),
        );
        assert!(unknown.unwrap_err().contains("no tool called fly"));
        let rel = call(
            &mut app,
            &sid,
            json!({"action": "create", "name": "x", "folder": "rel/dir"}),
        );
        assert!(rel.unwrap_err().contains("absolute"));

        let told = call(
            &mut app,
            &sid,
            json!({"action": "create", "name": "Scribe", "instructions": "Write.", "tools": ["read_file"], "folder": dir}),
        )
        .unwrap();
        assert!(
            told.contains("draft to create the agent \"Scribe\""),
            "{told}"
        );
        assert_eq!(app.roster.list.len(), before, "nothing before approval");

        app.approve_agent_draft("c1");
        assert!(app.agent_drafts.is_empty());
        let t = app.agent_named("scribe").unwrap();
        let a = app.roster.find(&t).unwrap();
        assert_eq!(
            (a.prompt.as_str(), a.tools.clone()),
            ("Write.", Vec::<String>::new()),
            "the app's own tools are every agent's, so none is stored"
        );
        assert_eq!(a.workdir.as_deref(), dir.to_str());
        let saved = app.store.as_ref().unwrap().load_agents().unwrap();
        assert!(saved.iter().any(|s| s.name == "Scribe"), "saved");

        let again = call(
            &mut app,
            &sid,
            json!({"action": "create", "name": "SCRIBE"}),
        );
        assert!(again.unwrap_err().contains("already an agent"));
        assert!(
            call(&mut app, &sid, json!({"action": "list"}))
                .unwrap()
                .contains("Scribe (id")
        );
    }

    #[test]
    fn a_team_member_joins_its_lead_and_an_agent_cannot_delete_itself_or_its_lead() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        let lead = app.roster.list[0].name.clone();
        let lead_chat = app.roster.list[0].chat().id.clone();
        call(
            &mut app,
            &lead_chat,
            json!({"action": "create", "name": "Helper", "lead": lead, "description": "Small jobs."}),
        )
        .unwrap();
        app.approve_agent_draft("c1");
        let t = app.agent_named("Helper").unwrap();
        assert_eq!(t.parent.as_deref(), Some(app.roster.list[0].id.as_str()));

        let helper_chat = chat_of(&app, "Helper");
        let me = call(
            &mut app,
            &helper_chat,
            json!({"action": "delete", "agent": "Helper"}),
        );
        assert!(me.unwrap_err().contains("cannot delete yourself"));
        let my_lead = call(
            &mut app,
            &helper_chat,
            json!({"action": "delete", "agent": lead}),
        );
        assert!(my_lead.unwrap_err().contains("your lead"));
        let no_team = call(
            &mut app,
            &lead_chat,
            json!({"action": "create", "name": "Sub", "lead": "Helper"}),
        );
        assert!(no_team.unwrap_err().contains("cannot have a team"));

        call(
            &mut app,
            &lead_chat,
            json!({"action": "update", "agent": "Helper", "instructions": "Big jobs."}),
        )
        .unwrap();
        app.reject_agent_draft("c1");
        let a = app.roster.find(&t).unwrap();
        assert_eq!(a.prompt, "", "a rejected change changes nothing");
        call(
            &mut app,
            &lead_chat,
            json!({"action": "update", "agent": "Helper", "instructions": "Big jobs."}),
        )
        .unwrap();
        app.approve_agent_draft("c1");
        assert_eq!(app.roster.find(&t).unwrap().prompt, "Big jobs.");

        call(
            &mut app,
            &lead_chat,
            json!({"action": "delete", "agent": "Helper"}),
        )
        .unwrap();
        app.approve_agent_draft("c1");
        assert!(app.agent_named("Helper").is_err(), "deleted");
    }

    #[test]
    fn an_mcp_tool_is_found_by_the_servers_own_name_for_it_when_that_is_clear() {
        let db = TempDb::new();
        let mut app = db.open_with_samples();
        let server = |name: &str, tools: &[&str]| crate::mcp::Server {
            id: name.into(),
            name: name.into(),
            transport: crate::mcp::Transport::Http {
                url: "https://x".into(),
            },
            auth: crate::mcp::Auth::None,
            enabled: true,
            status: crate::mcp::Status::Reachable,
            detail: String::new(),
            tools: tools
                .iter()
                .map(|t| crate::mcp::Tool {
                    name: (*t).into(),
                    description: String::new(),
                    schema: json!({"type": "object"}),
                })
                .collect(),
        };
        app.mcp.servers = vec![
            server("Resend", &["list-inboxes", "search"]),
            server("Docs", &["search"]),
        ];
        for given in [
            "mcp__resend__list-inboxes",
            "list-inboxes",
            "list_inboxes",
            "Resend/list-inboxes",
        ] {
            assert_eq!(
                app.tool_named(given).unwrap(),
                "mcp__resend__list-inboxes",
                "{given}"
            );
        }
        assert_eq!(app.tool_named("read-file").unwrap(), "read_file");
        let unclear = app.tool_named("search").unwrap_err();
        assert!(
            unclear.contains("mcp__resend__search or mcp__docs__search"),
            "{unclear}"
        );
        assert!(
            app.tool_named("fly")
                .unwrap_err()
                .contains("no tool called fly")
        );

        let sid = app.roster.list[0].chat().id.clone();
        let told = call(
            &mut app,
            &sid,
            json!({"action": "create", "name": "Mail", "tools": ["list-inboxes"]}),
        )
        .unwrap();
        assert!(
            told.contains("with the app's own tools and mcp__resend__list-inboxes"),
            "{told}"
        );
        let list = call(&mut app, &sid, json!({"action": "list"})).unwrap();
        assert!(
            list.contains("Tools from the MCP server Resend: mcp__resend__list-inboxes"),
            "{list}"
        );
    }
}
