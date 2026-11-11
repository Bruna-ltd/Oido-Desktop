//! Registering, signing in to, refreshing and calling MCP servers from the app: the form, the
//! background work, and what each result changes. The protocol itself is in [`super::client`] and
//! [`super::oauth`].

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rune::TextEdit;
use serde_json::Value;

use super::client::{self, Outcome, Target};
use super::{Auth, Preset, Secrets, Server, Status, Tool, Transport, oauth, resolve, slug};
use crate::app::App;
use crate::command::Finish;
use crate::completion::ToolSpec;

/// What the add form is registering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Remote,
    Local,
}

/// The add form.
pub struct Form {
    pub kind: Kind,
    pub name: TextEdit,
    pub url: TextEdit,
    pub auth: Auth,
    pub key: TextEdit,
    pub command: TextEdit,
    /// Arguments, written as on a command line.
    pub args: TextEdit,
    /// `NAME=value` pairs, written as on a command line.
    pub env: TextEdit,
}

impl Form {
    fn new() -> Form {
        Form {
            kind: Kind::Remote,
            name: TextEdit::new(""),
            url: TextEdit::new(""),
            auth: Auth::None,
            key: TextEdit::new(""),
            command: TextEdit::new(""),
            args: TextEdit::new(""),
            env: TextEdit::new(""),
        }
    }
}

/// The MCP page's state, and the servers themselves.
pub struct Mcp {
    pub servers: Vec<Server>,
    pub form: Form,
    /// The add form is showing.
    pub adding: bool,
    /// A local server's command was written and waits for the go-ahead to be run.
    pub confirming: bool,
    /// The server whose tools are listed.
    pub open: Option<String>,
    /// Servers being reached or signed in to right now.
    pub busy: HashSet<String>,
    /// Stops the sign-in that is waiting for the browser.
    pub signing_in: Option<Arc<AtomicBool>>,
    pub notice: Option<String>,
    /// What the page is filtered by.
    pub search: TextEdit,
    /// Changes agents asked for, each waiting for the person in its chat.
    pub drafts: Vec<super::Draft>,
    /// Why approving a draft did not go through, by the draft's tool call.
    pub draft_notice: Option<(String, String)>,
}

impl Mcp {
    pub fn new(servers: Vec<Server>) -> Mcp {
        Mcp {
            servers,
            form: Form::new(),
            adding: false,
            confirming: false,
            open: None,
            busy: HashSet::new(),
            signing_in: None,
            notice: None,
            search: TextEdit::new(""),
            drafts: Vec::new(),
            draft_notice: None,
        }
    }

    /// Whether `fields` (a name, an address, a description) answer the search: every word typed
    /// is in one of them, in any case. No search matches everything.
    pub fn matches(&self, fields: &[&str]) -> bool {
        let query = self.search.text.to_lowercase();
        query
            .split_whitespace()
            .all(|word| fields.iter().any(|f| f.to_lowercase().contains(word)))
    }

    /// Whether the form holds enough to register something.
    pub fn can_add(&self) -> bool {
        let f = &self.form;
        !f.name.text.trim().is_empty()
            && match f.kind {
                Kind::Remote => !f.url.text.trim().is_empty(),
                Kind::Local => !f.command.text.trim().is_empty(),
            }
    }

    /// The command line a local server would be started with, as it will be run.
    pub fn command_line(&self) -> String {
        let f = &self.form;
        let mut line = f.command.text.trim().to_owned();
        let args = f.args.text.trim();
        if !args.is_empty() {
            line.push(' ');
            line.push_str(args);
        }
        line
    }

    /// Whether `preset` is registered already (its address is).
    pub fn has_preset(&self, preset: &Preset) -> bool {
        self.servers
            .iter()
            .any(|s| matches!(&s.transport, Transport::Http { url } if url == preset.url))
    }
}

/// Splits what was typed as on a command line: spaces separate, quotes group, a backslash escapes.
pub fn split_words(s: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') | (None, '\\') => {
                cur.push(chars.next().ok_or("a backslash ends the line")?);
                started = true;
            }
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (None, c) => {
                cur.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err("a quote is not closed".to_owned());
    }
    if started {
        words.push(cur);
    }
    Ok(words)
}

/// Words written so [`split_words`] reads them back: plain when they can be, else in double quotes.
fn join_words(words: &[String]) -> String {
    words
        .iter()
        .map(|w| {
            let plain = !w.is_empty()
                && !w
                    .chars()
                    .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'));
            if plain {
                w.clone()
            } else {
                format!("\"{}\"", w.replace('\\', "\\\\").replace('"', "\\\""))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `NAME=value` words as pairs. A word with no `=` or no name is refused, not guessed at.
fn env_pairs(s: &str) -> Result<Vec<(String, String)>, String> {
    split_words(s)?
        .into_iter()
        .map(|w| match w.split_once('=') {
            Some((k, v)) if !k.is_empty() => Ok((k.to_owned(), v.to_owned())),
            _ => Err(format!("\"{w}\" is not NAME=value")),
        })
        .collect()
}

impl App {
    pub fn load_mcp(&mut self) {
        let Some(db) = &self.store else { return };
        match db.load_mcp_servers() {
            Ok(servers) => self.mcp.servers = servers,
            Err(e) => eprintln!("oido: could not read the MCP servers: {e}"),
        }
    }

    /// Every tool that can be given to an agent: the built-in ones and those of MCP servers that
    /// are on and reached.
    pub fn all_tools(&self) -> Vec<ToolSpec> {
        let mut all = crate::tools::registered();
        all.extend(super::specs(&self.mcp.servers));
        all
    }

    /// Whether a call's name is a tool this app has.
    pub fn tool_known(&self, name: &str) -> bool {
        crate::tools::knows(name) || resolve(&self.mcp.servers, name).is_some()
    }

    fn mcp_notice(&mut self, text: impl Into<String>) {
        self.mcp.notice = Some(text.into());
    }

    // ---- the form ----

    pub fn mcp_toggle_form(&mut self) {
        self.mcp.adding = !self.mcp.adding;
        self.mcp.confirming = false;
        self.mcp.notice = None;
    }

    /// Opens the form with a preset's name and address filled in.
    pub fn mcp_use_preset(&mut self, preset: &Preset) {
        self.mcp.form = Form::new();
        self.mcp.form.name = TextEdit::new(preset.name);
        self.mcp.form.url = TextEdit::new(preset.url);
        self.mcp.form.auth = preset.auth;
        self.mcp.adding = true;
        self.mcp.confirming = false;
        self.mcp.notice = None;
    }

    pub fn mcp_set_kind(&mut self, kind: Kind) {
        self.mcp.form.kind = kind;
        self.mcp.confirming = false;
    }

    pub fn mcp_set_auth(&mut self, auth: Auth) {
        self.mcp.form.auth = auth;
    }

    /// The Add button. A local server first shows the exact command and waits for a second press:
    /// adding it runs that command on this computer.
    pub fn mcp_add(&mut self) {
        if !self.mcp.can_add() {
            return;
        }
        let name = self.mcp.form.name.text.trim().to_owned();
        if slug(&name).is_empty() {
            return self.mcp_notice("Give it a name with at least one letter or digit.");
        }
        if self.mcp.servers.iter().any(|s| s.slug() == slug(&name)) {
            return self.mcp_notice(format!("There is already a server called {name}."));
        }
        match self.mcp.form.kind {
            Kind::Remote => self.mcp_add_remote(name),
            Kind::Local => {
                if self.mcp.confirming {
                    self.mcp_add_local(name);
                } else {
                    self.mcp.confirming = true;
                    self.mcp.notice = None;
                }
            }
        }
    }

    fn mcp_add_remote(&mut self, name: String) {
        let f = &self.mcp.form;
        let url = f.url.text.trim().to_owned();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return self.mcp_notice("The address should start with https://.");
        }
        let key = f.key.text.trim().to_owned();
        let auth = match f.auth {
            Auth::Key if key.is_empty() => {
                return self.mcp_notice("Paste the key, or choose another way to sign in.");
            }
            other => other,
        };
        let server = Server {
            id: new_id(),
            name,
            transport: Transport::Http { url: url.clone() },
            auth,
            enabled: true,
            status: Status::Pending,
            detail: String::new(),
            tools: Vec::new(),
        };
        let secrets = Secrets {
            key: (auth == Auth::Key).then_some(key),
            ..Secrets::default()
        };
        if auth == Auth::OAuth {
            self.mcp_sign_in_new(server, secrets, App::mcp_register);
        } else {
            self.mcp_register(server, secrets);
        }
    }

    fn mcp_add_local(&mut self, name: String) {
        let f = &self.mcp.form;
        let command = f.command.text.trim().to_owned();
        let (args, env) = match (split_words(&f.args.text), env_pairs(&f.env.text)) {
            (Ok(a), Ok(e)) => (a, e),
            (Err(why), _) => return self.mcp_notice(format!("The arguments: {why}.")),
            (_, Err(why)) => return self.mcp_notice(format!("The variables: {why}.")),
        };
        let server = Server {
            id: new_id(),
            name,
            transport: Transport::Stdio { command, args },
            auth: Auth::None,
            enabled: true,
            status: Status::Pending,
            detail: String::new(),
            tools: Vec::new(),
        };
        let secrets = Secrets {
            env: env.into_iter().collect(),
            ..Secrets::default()
        };
        self.mcp_register(server, secrets);
    }

    /// Registers what the form holds, then empties it.
    fn mcp_register(&mut self, server: Server, secrets: Secrets) {
        if self.mcp_save(server, secrets) {
            self.mcp.form = Form::new();
            self.mcp.adding = false;
            self.mcp.confirming = false;
        }
    }

    /// Saves the server and asks it for its tools. It stays registered if it cannot be reached,
    /// marked so, since the cause (a key to fix, a program not installed) is the person's to fix.
    /// False, with why on the page, if it could not be saved.
    pub(super) fn mcp_save(&mut self, server: Server, secrets: Secrets) -> bool {
        let Some(db) = &self.store else {
            self.mcp_notice("There is no database, so nothing can be saved.");
            return false;
        };
        if let Err(e) = db.insert_mcp_server(&server, &secrets) {
            self.mcp_notice(format!("{e}"));
            return false;
        }
        let id = server.id.clone();
        self.mcp.servers.push(server);
        self.mcp.notice = None;
        self.mcp.open = Some(id.clone());
        self.mcp_resync(&id);
        true
    }

    /// Fills the add form with a server and its secrets, opened, to change before adding.
    pub(super) fn mcp_fill_form(&mut self, server: &Server, secrets: &Secrets) {
        let mut form = Form::new();
        form.name = TextEdit::new(&server.name);
        form.auth = server.auth;
        form.key = TextEdit::new(secrets.key.as_deref().unwrap_or_default());
        match &server.transport {
            Transport::Http { url } => form.url = TextEdit::new(url),
            Transport::Stdio { command, args } => {
                form.kind = Kind::Local;
                form.command = TextEdit::new(command);
                form.args = TextEdit::new(join_words(args));
                let pairs: Vec<String> = secrets
                    .env
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect();
                form.env = TextEdit::new(join_words(&pairs));
            }
        }
        self.mcp.form = form;
        self.mcp.adding = true;
        self.mcp.confirming = false;
        self.mcp.notice = None;
    }

    // ---- signing in ----

    /// Signs in before the server is saved: a sign-in that never finishes leaves nothing behind.
    /// `save` takes the server once signed in.
    pub(super) fn mcp_sign_in_new(
        &mut self,
        server: Server,
        secrets: Secrets,
        save: fn(&mut App, Server, Secrets),
    ) {
        let Transport::Http { url } = server.transport.clone() else {
            return;
        };
        let cancel = self.mcp_begin_sign_in(&server.id);
        self.spawn(
            move || oauth::sign_in(&url, &cancel),
            move |app, result| {
                app.mcp_end_sign_in(&server.id);
                match result {
                    Ok(tokens) => {
                        let secrets = Secrets {
                            oauth: Some(tokens),
                            ..secrets
                        };
                        save(app, server, secrets);
                    }
                    Err(why) => app.mcp_notice(format!("Sign-in did not finish: {why}.")),
                }
            },
        );
    }

    /// Signs in again to a saved server whose sign-in ran out or was refused.
    pub fn mcp_sign_in(&mut self, id: &str) {
        let Some(Transport::Http { url }) = self
            .mcp
            .servers
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.transport.clone())
        else {
            return;
        };
        let cancel = self.mcp_begin_sign_in(id);
        let id = id.to_owned();
        self.spawn(
            move || oauth::sign_in(&url, &cancel),
            move |app, result| {
                app.mcp_end_sign_in(&id);
                match result {
                    Ok(tokens) => app.mcp_keep_tokens(&id, tokens, true),
                    Err(why) => app.mcp_notice(format!("Sign-in did not finish: {why}.")),
                }
            },
        );
    }

    fn mcp_begin_sign_in(&mut self, id: &str) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        self.mcp.signing_in = Some(Arc::clone(&cancel));
        self.mcp.busy.insert(id.to_owned());
        self.mcp.notice = Some("Finish signing in in your browser.".to_owned());
        cancel
    }

    fn mcp_end_sign_in(&mut self, id: &str) {
        self.mcp.signing_in = None;
        self.mcp.busy.remove(id);
        self.mcp.notice = None;
    }

    /// The Cancel button while a sign-in waits for the browser.
    pub fn mcp_cancel_sign_in(&mut self) {
        if let Some(cancel) = &self.mcp.signing_in {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Saves new sign-in tokens; `then_list` asks the server for its tools with them.
    fn mcp_keep_tokens(&mut self, id: &str, tokens: oauth::Tokens, then_list: bool) {
        let Some(db) = &self.store else { return };
        let saved = db.mcp_secrets(id).and_then(|mut s| {
            s.oauth = Some(tokens);
            db.save_mcp_secrets(id, &s)
        });
        match saved {
            Ok(()) if then_list => self.mcp_resync(id),
            Ok(()) => {}
            Err(e) => self.mcp_notice(format!("The sign-in could not be saved: {e}")),
        }
    }

    // ---- listing ----

    /// Asks the server for its tools again.
    pub fn mcp_resync(&mut self, id: &str) {
        if self.mcp.busy.contains(id) {
            return;
        }
        let Some(target) = self.mcp_target(id) else {
            return;
        };
        self.mcp.busy.insert(id.to_owned());
        let id = id.to_owned();
        let cancel = AtomicBool::new(false);
        self.spawn(
            move || client::list_tools(target, &cancel),
            move |app, outcome| app.mcp_listed(&id, outcome),
        );
    }

    fn mcp_listed(&mut self, id: &str, outcome: Outcome<Vec<Tool>>) {
        self.mcp.busy.remove(id);
        if let Some(tokens) = outcome.tokens {
            self.mcp_keep_tokens(id, tokens, false);
        }
        let Some(server) = self.mcp.servers.iter_mut().find(|s| s.id == id) else {
            return;
        };
        match outcome.result {
            Ok(tools) => {
                server.status = Status::Reachable;
                server.detail.clear();
                server.tools = tools;
            }
            // The tools it had stay, so they are not lost to a server that is only down for now.
            Err(why) => {
                server.status = Status::Unreachable;
                server.detail = why;
            }
        }
        if let Some(db) = &self.store
            && let Err(e) = db.save_mcp_listing(id, server.status, &server.detail, &server.tools)
        {
            eprintln!("oido: could not save the MCP server's tools: {e}");
        }
    }

    // ---- turning on and off, removing ----

    pub fn mcp_toggle(&mut self, id: &str) {
        let Some(server) = self.mcp.servers.iter_mut().find(|s| s.id == id) else {
            return;
        };
        server.enabled = !server.enabled;
        let (enabled, reached) = (server.enabled, server.status == Status::Reachable);
        if let Some(db) = &self.store
            && let Err(e) = db.set_mcp_enabled(id, enabled)
        {
            eprintln!("oido: could not save that the MCP server is {enabled}: {e}");
        }
        if enabled {
            if !reached {
                self.mcp_resync(id);
            }
        } else {
            client::stop(id);
        }
    }

    pub fn mcp_remove(&mut self, id: &str) {
        client::stop(id);
        if let Some(db) = &self.store
            && let Err(e) = db.delete_mcp_server(id)
        {
            return self.mcp_notice(format!("Could not remove it: {e}"));
        }
        self.mcp.servers.retain(|s| s.id != id);
        if self.mcp.open.as_deref() == Some(id) {
            self.mcp.open = None;
        }
    }

    pub fn mcp_toggle_tools(&mut self, id: &str) {
        self.mcp.open = if self.mcp.open.as_deref() == Some(id) {
            None
        } else {
            Some(id.to_owned())
        };
    }

    // ---- calling ----

    /// Opens what a call to the server needs. The vault stays on this thread, so it is done here,
    /// before the work goes to another.
    fn mcp_target(&mut self, id: &str) -> Option<Target> {
        let server = self.mcp.servers.iter().find(|s| s.id == id)?;
        let transport = server.transport.clone();
        let secrets = match self.store.as_ref().map(|db| db.mcp_secrets(id)) {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                self.mcp_notice(format!("The server's saved secrets could not be read: {e}"));
                return None;
            }
            None => Secrets::default(),
        };
        Some(Target {
            id: id.to_owned(),
            transport,
            secrets,
        })
    }

    /// The agent called an MCP tool: it runs in the background like a command, and its result goes
    /// back to the agent when it arrives. Always handles the call, even to refuse it.
    pub(crate) fn start_mcp_call(&mut self, sid: &str, call_id: &str, name: &str, args: &str) {
        let refuse = |app: &mut App, why: String| {
            app.resolve(sid, call_id, format!("Error: {why}"), true);
            app.start_next_command(sid);
        };
        let Some((server, tool)) = resolve(&self.mcp.servers, name) else {
            return refuse(self, format!("there is no tool called {name}."));
        };
        if !server.enabled {
            return refuse(
                self,
                format!(
                    "{} is turned off. The person can turn it on on the MCP page.",
                    server.name
                ),
            );
        }
        let (server_id, tool_name) = (server.id.clone(), tool.name.clone());
        let arguments: Value = if args.trim().is_empty() {
            Value::Object(Default::default())
        } else {
            match serde_json::from_str(args) {
                Ok(v) => v,
                Err(_) => return refuse(self, "the arguments were not JSON.".to_owned()),
            }
        };
        let Some(target) = self.mcp_target(&server_id) else {
            return refuse(
                self,
                "the server's saved secrets could not be read.".to_owned(),
            );
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.commands.insert(sid.to_owned(), Arc::clone(&cancel));
        let (sid, id) = (sid.to_owned(), call_id.to_owned());
        self.spawn(
            move || {
                let outcome = client::call_tool(target, &tool_name, arguments, &cancel);
                (outcome, cancel.load(Ordering::Relaxed))
            },
            move |app, (outcome, stopped)| {
                if let Some(tokens) = outcome.tokens {
                    app.mcp_keep_tokens(&server_id, tokens, false);
                }
                let finish = match (outcome.result, stopped) {
                    (_, true) => Finish::Stopped("Stopped before the server answered.".into()),
                    (Ok(text), false) => Finish::Done(text),
                    (Err(why), false) => Finish::Done(format!("Error: {why}.")),
                };
                app.command_done(&sid, &id, finish);
            },
        );
    }
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_search_needs_every_word_somewhere_in_any_case_and_none_matches_all() {
        let mut m = Mcp::new(Vec::new());
        let fields = [
            "GitHub",
            "Repos, issues and pull requests.",
            "https://api.githubcopilot.com/mcp/",
        ];
        assert!(m.matches(&fields));
        for (typed, hit) in [
            ("git", true),
            ("  ISSUES  hub", true),
            ("pull copilot", true),
            ("jira", false),
            ("git jira", false),
        ] {
            m.search = TextEdit::new(typed);
            assert_eq!(m.matches(&fields), hit, "{typed:?}");
        }
    }

    #[test]
    fn words_are_split_as_on_a_command_line() {
        assert_eq!(
            split_words(r#"-y @scope/pkg --root "/my files" 'a b' x\ y"#).unwrap(),
            ["-y", "@scope/pkg", "--root", "/my files", "a b", "x y"]
        );
        assert_eq!(split_words("  ").unwrap(), Vec::<String>::new());
        assert_eq!(split_words(r#"a "" b"#).unwrap(), ["a", "", "b"]);
        assert!(split_words(r#"a "b"#).is_err());
    }

    #[test]
    fn joined_words_split_back_to_the_same_words() {
        let words: Vec<String> = ["-y", "a b", "", r#"say "hi""#, r"c:\x", "it's"]
            .map(String::from)
            .to_vec();
        assert_eq!(split_words(&join_words(&words)).unwrap(), words);
    }

    #[test]
    fn variables_are_name_value_pairs_and_anything_else_is_refused() {
        assert_eq!(
            env_pairs(r#"A=1 B="two words" C="#).unwrap(),
            [
                ("A".into(), "1".into()),
                ("B".into(), "two words".into()),
                ("C".into(), String::new())
            ]
        );
        assert!(env_pairs("plain").is_err());
        assert!(env_pairs("=x").is_err());
    }
}
