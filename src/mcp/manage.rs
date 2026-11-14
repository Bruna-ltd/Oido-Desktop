//! An agent manages MCP servers with the `mcp_servers` tool ([`App::run_mcp_server_calls`]).
//! `list` answers at once. `add`, `update` and `remove` become a draft shown in the chat, and
//! change nothing until the person approves it there: adding a local server runs a program on
//! their computer, and every server is shared by all agents. Drafts are kept while the app runs;
//! one still waiting at a restart is gone, and the agent can be asked again.

use super::client;
use super::tool::{self, Call, Change, New};
use super::{Auth, Secrets, Server, Status, Transport, slug};
use crate::app::App;
use crate::model::Message;

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
    Add {
        server: Server,
        secrets: Secrets,
    },
    /// The server as it would be (same id and name), and its secrets if they change.
    Update {
        server: Server,
        secrets: Option<Secrets>,
        /// The variables set or removed, by name, and whether the key changes: for the card,
        /// which never shows their values.
        touched: Vec<String>,
    },
    Remove {
        id: String,
        name: String,
    },
}

impl App {
    /// Answers the chat's waiting `mcp_servers` calls.
    pub(crate) fn run_mcp_server_calls(&mut self, sid: &str) {
        let calls: Vec<(String, String)> = match self
            .roster
            .owner_of(sid)
            .and_then(|a| a.sessions.iter().find(|s| s.id == sid))
            .and_then(|s| s.messages.last())
        {
            Some(Message::Assistant { tools, .. }) => tools
                .iter()
                .filter(|t| t.name == tool::NAME && t.output.is_none())
                .map(|t| (t.id.clone(), t.args.clone()))
                .collect(),
            _ => return,
        };
        for (id, args) in calls {
            let told = self
                .mcp_server_call(sid, &id, &args)
                .unwrap_or_else(|why| format!("Error: {why}."));
            self.resolve(sid, &id, told, true);
        }
    }

    fn mcp_server_call(&mut self, sid: &str, call_id: &str, args: &str) -> Result<String, String> {
        let proposal = match tool::parse(args)? {
            Call::List => return Ok(self.mcp_list_for_agent()),
            Call::Add(new) => self.mcp_propose_add(new)?,
            Call::Update { name, change } => self.mcp_propose_update(&name, change)?,
            Call::Remove(name) => {
                let s = self.mcp_named(&name)?;
                Proposal::Remove {
                    id: s.id.clone(),
                    name: s.name.clone(),
                }
            }
        };
        let what = match &proposal {
            Proposal::Add { server, .. } => format!("add {:?} ({})", server.name, server.target()),
            Proposal::Update { server, .. } => format!("change {:?}", server.name),
            Proposal::Remove { name, .. } => format!("remove {name:?}"),
        };
        self.mcp.drafts.retain(|d| d.call_id != call_id);
        self.mcp.drafts.push(Draft {
            call_id: call_id.to_owned(),
            session_id: sid.to_owned(),
            proposal,
        });
        Ok(format!(
            "The person now sees a draft to {what} in this chat. Nothing changes until they approve it; tell them it is waiting for them, and do not call its tools before then."
        ))
    }

    /// The registered server called `name` (as tool names spell it, so case and punctuation do
    /// not matter).
    fn mcp_named(&self, name: &str) -> Result<&Server, String> {
        let wanted = slug(name);
        self.mcp
            .servers
            .iter()
            .find(|s| s.slug() == wanted)
            .ok_or_else(|| format!("there is no MCP server called {name:?}; `list` shows them"))
    }

    fn mcp_propose_add(&self, new: New) -> Result<Proposal, String> {
        if self.mcp_named(&new.name).is_ok() {
            return Err(format!(
                "there is already a server called {:?}; use update to change it",
                new.name
            ));
        }
        let server = Server {
            id: uuid::Uuid::new_v4().to_string(),
            name: new.name,
            transport: new.transport,
            auth: new.auth,
            enabled: true,
            status: Status::Pending,
            detail: String::new(),
            tools: Vec::new(),
        };
        let secrets = Secrets {
            key: new.key,
            env: new.env,
            oauth: None,
        };
        Ok(Proposal::Add { server, secrets })
    }

    fn mcp_propose_update(&self, name: &str, change: Change) -> Result<Proposal, String> {
        let old = self.mcp_named(name)?;
        let mut server = old.clone();
        match &mut server.transport {
            Transport::Extension { .. } => {
                return Err(format!(
                    "{} is built in. The person turns it and its tools on or off on the Extensions page",
                    old.name
                ));
            }
            Transport::Http { url } => {
                if change.command.is_some() || change.args.is_some() || !change.env.is_empty() {
                    return Err(format!(
                        "{} is a remote server: it has no command, arguments or variables",
                        old.name
                    ));
                }
                if let Some(new) = change.url.clone() {
                    *url = new;
                }
            }
            Transport::Stdio { command, args } => {
                if change.url.is_some() || change.auth.is_some_and(|a| a != Auth::None) {
                    return Err(format!(
                        "{} is a local server: it has no address or sign-in",
                        old.name
                    ));
                }
                if let Some(new) = change.command.clone() {
                    *command = new;
                }
                if let Some(new) = change.args.clone() {
                    *args = new;
                }
            }
        }
        if let Some(auth) = change.auth {
            server.auth = auth;
        } else if change.key.is_some() && matches!(server.transport, Transport::Http { .. }) {
            server.auth = Auth::Key;
        }
        let mut touched: Vec<String> = change.env.keys().cloned().collect();
        let secret_change =
            !change.env.is_empty() || change.key.is_some() || server.auth != old.auth;
        let secrets = if secret_change {
            let mut s = match &self.store {
                Some(db) => db.mcp_secrets(&old.id).map_err(|e| e.to_string())?,
                None => Secrets::default(),
            };
            for (k, v) in change.env {
                match v {
                    Some(v) => s.env.insert(k, v),
                    None => s.env.remove(&k),
                };
            }
            if let Some(key) = change.key {
                s.key = Some(key);
                touched.push("the key".to_owned());
            }
            if server.auth != Auth::Key {
                s.key = None;
            }
            if server.auth != Auth::OAuth {
                s.oauth = None;
            }
            if server.auth == Auth::Key && s.key.is_none() {
                return Err("auth key needs the `key`".to_owned());
            }
            Some(s)
        } else {
            None
        };
        if server == *old && secrets.is_none() {
            return Err(format!("that is how {} is already set", old.name));
        }
        Ok(Proposal::Update {
            server,
            secrets,
            touched,
        })
    }

    /// The servers and their tools for the agent, one line each. Keys and variable values are
    /// never in it, only the variables' names.
    fn mcp_list_for_agent(&self) -> String {
        if self.mcp.servers.is_empty() {
            return "No MCP servers are registered.".to_owned();
        }
        let lines: Vec<String> = self
            .mcp
            .servers
            .iter()
            .map(|s| {
                let kind = match s.transport {
                    Transport::Http { .. } => "remote",
                    Transport::Stdio { .. } => "local",
                    Transport::Extension { .. } => "built in",
                };
                let state = match (s.enabled, s.status) {
                    (false, _) => "off".to_owned(),
                    (true, Status::Reachable) => "on".to_owned(),
                    (true, Status::Pending) => "not reached yet".to_owned(),
                    (true, Status::Unreachable) => format!("unreachable: {}", s.detail),
                };
                let mut line = format!("- {} ({kind}, {state}): {}", s.name, s.target());
                if s.auth != Auth::None {
                    line.push_str(&format!("; sign-in: {}", s.auth.as_str()));
                }
                if let Some(Ok(secrets)) = self.store.as_ref().map(|db| db.mcp_secrets(&s.id))
                    && !secrets.env.is_empty()
                {
                    let names: Vec<&str> = secrets.env.keys().map(String::as_str).collect();
                    line.push_str(&format!("; variables: {}", names.join(", ")));
                }
                // By the names an agent is given them by (`agents` tool), not the server's own.
                if !s.tools.is_empty() {
                    let tools: Vec<String> = s.tools.iter().map(|t| s.qualified(&t.name)).collect();
                    line.push_str(&format!("; tools: {}", tools.join(", ")));
                }
                line
            })
            .collect();
        lines.join("\n")
    }

    // ---- the draft card ----

    fn mcp_take_draft(&mut self, call_id: &str) -> Option<Draft> {
        let i = self.mcp.drafts.iter().position(|d| d.call_id == call_id)?;
        self.mcp.draft_notice = None;
        Some(self.mcp.drafts.remove(i))
    }

    /// The card's Approve: makes the change. An add that can no longer be made (the name was
    /// taken since) keeps the draft and says why on the card.
    pub fn mcp_approve_draft(&mut self, call_id: &str) {
        let Some(draft) = self.mcp_take_draft(call_id) else {
            return;
        };
        let failed = match draft.proposal.clone() {
            Proposal::Add { server, secrets } => {
                if self.mcp_named(&server.name).is_ok() {
                    Some(format!("There is already a server called {}.", server.name))
                } else if server.auth == Auth::OAuth {
                    self.mcp_sign_in_new(server, secrets, |app, server, secrets| {
                        app.mcp_save(server, secrets);
                    });
                    None
                } else {
                    self.mcp_save(server, secrets);
                    None
                }
            }
            Proposal::Update {
                server, secrets, ..
            } => self.mcp_apply_update(server, secrets).err(),
            Proposal::Remove { id, .. } => {
                self.mcp_remove(&id);
                None
            }
        };
        if let Some(why) = failed {
            self.mcp.draft_notice = Some((draft.call_id.clone(), why));
            self.mcp.drafts.push(draft);
        }
    }

    /// The card's Edit, for an add: the MCP page's form, filled in, to change and add from there.
    pub fn mcp_edit_draft(&mut self, call_id: &str) {
        let Some(Draft {
            proposal: Proposal::Add { server, secrets },
            ..
        }) = self.mcp_take_draft(call_id)
        else {
            return;
        };
        self.mcp_fill_form(&server, &secrets);
        self.go(crate::app::Route::Mcp);
    }

    pub fn mcp_reject_draft(&mut self, call_id: &str) {
        self.mcp_take_draft(call_id);
    }

    /// Saves a changed server, restarting a local one and asking it for its tools again.
    fn mcp_apply_update(&mut self, server: Server, secrets: Option<Secrets>) -> Result<(), String> {
        let id = server.id.clone();
        let Some(db) = &self.store else {
            return Err("There is no database, so nothing can be saved.".to_owned());
        };
        db.update_mcp_server(&server).map_err(|e| e.to_string())?;
        if let Some(s) = &secrets {
            db.save_mcp_secrets(&id, s).map_err(|e| e.to_string())?;
        }
        let Some(slot) = self.mcp.servers.iter_mut().find(|s| s.id == id) else {
            return Err("That server is gone.".to_owned());
        };
        let (enabled, auth) = (server.enabled, server.auth);
        *slot = Server {
            status: Status::Pending,
            detail: String::new(),
            tools: std::mem::take(&mut slot.tools),
            ..server
        };
        client::stop(&id);
        if enabled {
            if auth == Auth::OAuth && secrets.as_ref().is_some_and(|s| s.oauth.is_none()) {
                self.mcp_sign_in(&id);
            } else {
                self.mcp_resync(&id);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::TempDb;

    fn local(name: &str) -> Server {
        Server {
            id: format!("id-{name}"),
            name: name.into(),
            transport: Transport::Stdio {
                command: "/bin/false".into(),
                args: vec![],
            },
            auth: Auth::None,
            enabled: false,
            status: Status::Pending,
            detail: String::new(),
            tools: vec![],
        }
    }

    fn call(app: &mut App, args: serde_json::Value) -> Result<String, String> {
        app.mcp_server_call("chat", "c1", &args.to_string())
    }

    #[test]
    fn an_added_server_waits_for_approval_and_secrets_never_show_in_the_list() {
        let db = TempDb::new();
        let mut app = db.open();
        let told = call(
            &mut app,
            json!({"action": "add", "name": "TTS", "command": "/bin/false", "env": {"TOKEN": "s3cret"}}),
        )
        .unwrap();
        assert!(told.contains("draft to add \"TTS\""), "{told}");
        assert!(
            app.mcp.servers.is_empty(),
            "nothing is added before approval"
        );
        assert_eq!(app.mcp.drafts.len(), 1);

        app.mcp_approve_draft("c1");
        assert!(app.mcp.drafts.is_empty());
        assert_eq!(app.mcp.servers.len(), 1);
        let id = app.mcp.servers[0].id.clone();
        assert_eq!(
            app.store.as_ref().unwrap().mcp_secrets(&id).unwrap().env["TOKEN"],
            "s3cret"
        );
        let list = call(&mut app, json!({"action": "list"})).unwrap();
        assert!(
            list.contains("TTS (local") && list.contains("variables: TOKEN"),
            "{list}"
        );
        assert!(!list.contains("s3cret"), "{list}");

        let again = call(
            &mut app,
            json!({"action": "add", "name": "tts", "command": "x"}),
        );
        assert!(again.unwrap_err().contains("already a server"));
    }

    #[test]
    fn an_update_merges_variables_and_a_rejected_draft_changes_nothing() {
        let db = TempDb::new();
        let mut app = db.open();
        let s = local("TTS");
        let secrets = Secrets {
            env: [("A".into(), "1".into()), ("B".into(), "2".into())].into(),
            ..Secrets::default()
        };
        app.store
            .as_ref()
            .unwrap()
            .insert_mcp_server(&s, &secrets)
            .unwrap();
        app.mcp.servers.push(s.clone());

        call(&mut app, json!({"action": "remove", "name": "tts"})).unwrap();
        app.mcp_reject_draft("c1");
        assert_eq!(app.mcp.servers.len(), 1, "rejected");

        call(
            &mut app,
            json!({"action": "update", "name": "TTS", "args": ["--fast"], "env": {"B": null, "C": "3"}}),
        )
        .unwrap();
        assert_eq!(app.mcp.servers[0], s, "nothing changes before approval");
        app.mcp_approve_draft("c1");
        assert_eq!(
            app.mcp.servers[0].transport,
            Transport::Stdio {
                command: "/bin/false".into(),
                args: vec!["--fast".into()]
            }
        );
        let db_ = app.store.as_ref().unwrap();
        assert_eq!(
            db_.load_mcp_servers().unwrap()[0].transport,
            app.mcp.servers[0].transport
        );
        let env = db_.mcp_secrets(&s.id).unwrap().env;
        assert_eq!(
            env.into_iter().collect::<Vec<_>>(),
            [("A".into(), "1".into()), ("C".into(), "3".into())]
        );

        let wrong = call(
            &mut app,
            json!({"action": "update", "name": "TTS", "url": "https://x"}),
        );
        assert!(wrong.unwrap_err().contains("local server"));
        call(&mut app, json!({"action": "remove", "name": "TTS"})).unwrap();
        app.mcp_approve_draft("c1");
        assert!(app.mcp.servers.is_empty());
    }
}
