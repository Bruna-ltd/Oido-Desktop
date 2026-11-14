//! The Extensions page's state, and what each press changes: turning an extension or one of its
//! tools on or off, saving the Google client, signing in and out, and running a call in the
//! background. An extension shows up to agents as a server (see [`App::rebuild_extension_servers`])
//! so everything that offers or runs MCP tools handles it unchanged.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rune::TextEdit;
use serde_json::Value;

use super::google::{self, Account, EMAIL_SCOPE};
use super::{ALL, Extension, find, id_of, server_id};
use crate::app::App;
use crate::command::Finish;
use crate::mcp::{Auth, Server, Status, Transport, oauth};
use crate::store::Setting;

/// What the page knows of the Google account, without its secrets.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct AccountView {
    pub client_id: String,
    pub has_client: bool,
    pub email: Option<String>,
    pub scopes: Vec<String>,
    pub signed_in: bool,
}

impl AccountView {
    fn of(account: &Account) -> AccountView {
        AccountView {
            client_id: account.client_id.clone(),
            has_client: account.has_client(),
            email: account.email.clone(),
            scopes: account.scopes.clone(),
            signed_in: account.tokens.is_some(),
        }
    }

    fn allows(&self, scopes: &[&str]) -> bool {
        self.signed_in && scopes.iter().all(|s| self.scopes.iter().any(|g| g == s))
    }

    /// What Google left out of the sign-in, by the last part of each name, so a refusal can be
    /// told from a mistake.
    fn missing(&self, scopes: &[&str]) -> String {
        let short = |s: &str| s.rsplit('/').next().unwrap_or(s).to_owned();
        let missing: Vec<String> = scopes
            .iter()
            .filter(|s| !self.scopes.iter().any(|g| g == *s))
            .map(|s| short(s))
            .collect();
        let granted: Vec<String> = self.scopes.iter().map(|s| short(s)).collect();
        format!(
            "Missing: {}. Google granted: {}.",
            missing.join(", "),
            if granted.is_empty() {
                "nothing".to_owned()
            } else {
                granted.join(", ")
            }
        )
    }
}

pub struct Extensions {
    settings: BTreeMap<String, Setting>,
    pub account: AccountView,
    /// The client id and secret being typed.
    pub client_id: TextEdit,
    pub client_secret: TextEdit,
    /// The extension whose tools are listed.
    pub open: Option<String>,
    /// Stops the sign-in that is waiting for the browser.
    pub signing_in: Option<Arc<AtomicBool>>,
    pub notice: Option<String>,
}

impl Extensions {
    pub fn new() -> Extensions {
        Extensions {
            settings: BTreeMap::new(),
            account: AccountView::default(),
            client_id: TextEdit::new(""),
            client_secret: TextEdit::new(""),
            open: None,
            signing_in: None,
            notice: None,
        }
    }

    pub fn enabled(&self, ext: &dyn Extension) -> bool {
        self.settings.get(&id_of(ext)).is_some_and(|s| s.enabled)
    }

    pub fn tool_on(&self, ext: &dyn Extension, tool: &str) -> bool {
        self.settings
            .get(&id_of(ext))
            .is_none_or(|s| !s.off_tools.contains(tool))
    }

    /// What Google must allow for everything that is on: the extensions' scopes, and the address.
    pub fn wanted_scopes(&self) -> Vec<&'static str> {
        let mut scopes = vec![EMAIL_SCOPE];
        for ext in ALL.iter().filter(|e| self.enabled(**e)) {
            for s in ext.scopes() {
                if !scopes.contains(s) {
                    scopes.push(s);
                }
            }
        }
        scopes
    }

    /// Why an extension's tools are not offered even though it is on, or nothing.
    pub fn blocked(&self, ext: &dyn Extension) -> Option<String> {
        if !self.account.has_client {
            Some("Add your Google client first.".to_owned())
        } else if !self.account.signed_in {
            Some("Sign in to Google.".to_owned())
        } else if !self.account.allows(ext.scopes()) {
            Some(format!(
                "Sign in again to allow {}. Tick every permission box on Google's page. {}",
                ext.title(),
                self.account.missing(ext.scopes())
            ))
        } else {
            None
        }
    }
}

impl App {
    /// Reads what was saved: which extensions are on, and the account.
    pub fn load_extensions(&mut self) {
        let Some(db) = &self.store else { return };
        match db.load_extensions() {
            Ok(rows) => self.ext.settings = rows.into_iter().collect(),
            Err(e) => eprintln!("oido: could not read the extensions: {e}"),
        }
        match db.extension_account(google::PROVIDER) {
            Ok(account) => self.ext_show_account(&account.unwrap_or_default()),
            Err(e) => eprintln!("oido: could not read the Google account: {e}"),
        }
        self.rebuild_extension_servers();
    }

    fn ext_show_account(&mut self, account: &Account) {
        self.ext.account = AccountView::of(account);
        self.ext.client_id = TextEdit::new(&account.client_id);
        self.ext.client_secret = TextEdit::new("");
    }

    /// Makes each extension that is on a server in the register, offering the tools that are on. One that is off is no server at all. Never saved with
    /// the person's own servers.
    pub fn rebuild_extension_servers(&mut self) {
        self.mcp.servers.retain(|s| !s.is_extension());
        for ext in ALL.iter().copied().filter(|e| self.ext.enabled(*e)) {
            let blocked = self.ext.blocked(ext);
            self.mcp.servers.push(Server {
                id: server_id(ext),
                name: ext.title().to_owned(),
                transport: Transport::Extension { id: id_of(ext) },
                auth: Auth::OAuth,
                enabled: true,
                // Offered as soon as it is on, so agents can be given its tools before signing
                // in; until then a call says to sign in.
                status: Status::Reachable,
                detail: blocked.unwrap_or_default(),
                tools: ext
                    .tools()
                    .iter()
                    .filter(|t| self.ext.tool_on(ext, t.name))
                    .map(|t| t.tool())
                    .collect(),
            });
        }
    }

    fn ext_notice(&mut self, text: impl Into<String>) {
        self.ext.notice = Some(text.into());
    }

    fn ext_save_setting(&mut self, ext: &dyn Extension) {
        let id = id_of(ext);
        let setting = self.ext.settings.get(&id).cloned().unwrap_or_default();
        if let Some(db) = &self.store
            && let Err(e) = db.save_extension(&id, &setting)
        {
            self.ext_notice(format!("Could not save that: {e}"));
        }
        self.rebuild_extension_servers();
    }

    pub fn ext_toggle(&mut self, ext: &'static dyn Extension) {
        let on = !self.ext.enabled(ext);
        if on && !self.ext.account.has_client {
            return self.ext_notice("Add your Google client first, then turn it on.");
        }
        self.ext.notice = None;
        self.ext.settings.entry(id_of(ext)).or_default().enabled = on;
        self.ext_save_setting(ext);
        if on && self.ext.account.signed_in && !self.ext.account.allows(ext.scopes()) {
            self.ext_notice(format!("Sign in again to allow {}.", ext.title()));
        }
    }

    pub fn ext_toggle_tool(&mut self, ext: &'static dyn Extension, tool: &str) {
        let off = &mut self.ext.settings.entry(id_of(ext)).or_default().off_tools;
        if !off.remove(tool) {
            off.insert(tool.to_owned());
        }
        self.ext_save_setting(ext);
    }

    pub fn ext_toggle_open(&mut self, ext: &dyn Extension) {
        let id = id_of(ext);
        self.ext.open = if self.ext.open.as_deref() == Some(id.as_str()) {
            None
        } else {
            Some(id)
        };
    }

    /// Whether the typed client can be saved.
    pub fn ext_can_save_client(&self) -> bool {
        !self.ext.client_id.text.trim().is_empty() && !self.ext.client_secret.text.trim().is_empty()
    }

    /// Saves the typed client. A different client makes the old sign-in useless, so it is dropped.
    pub fn ext_save_client(&mut self) {
        if !self.ext_can_save_client() {
            return;
        }
        let Some(db) = &self.store else { return };
        let mut account = match db.extension_account(google::PROVIDER) {
            Ok(a) => a.unwrap_or_default(),
            Err(e) => return self.ext_notice(format!("The saved account could not be read: {e}")),
        };
        account.client_id = self.ext.client_id.text.trim().to_owned();
        account.client_secret = self.ext.client_secret.text.trim().to_owned();
        account.sign_out();
        if let Err(e) = db.save_extension_account(google::PROVIDER, &account) {
            return self.ext_notice(format!("Could not save the client: {e}"));
        }
        self.ext_show_account(&account);
        self.ext.notice = None;
        self.rebuild_extension_servers();
    }

    /// The Sign in button: asks Google for what everything that is on needs.
    pub fn ext_sign_in(&mut self) {
        if self.ext.signing_in.is_some() {
            return;
        }
        let scopes = self.ext.wanted_scopes();
        if scopes.len() == 1 {
            return self.ext_notice("Turn an extension on first, then sign in.");
        }
        let account = match self
            .store
            .as_ref()
            .map(|db| db.extension_account(google::PROVIDER))
        {
            Some(Ok(Some(a))) if a.has_client() => a,
            Some(Err(e)) => {
                return self.ext_notice(format!("The saved account could not be read: {e}"));
            }
            _ => return self.ext_notice("Add your Google client first."),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.ext.signing_in = Some(Arc::clone(&cancel));
        self.ext_notice("Finish signing in in your browser.");
        let colors = self.look.colors();
        self.spawn(
            move || {
                let (tokens, given) = oauth::sign_in_google(
                    &account.client_id,
                    &account.client_secret,
                    &scopes,
                    &cancel,
                    colors,
                )?;
                let email = google::email_of(&tokens);
                Ok::<_, String>((tokens, given, email))
            },
            move |app, result| {
                app.ext.signing_in = None;
                app.ext.notice = None;
                match result {
                    Ok((tokens, given, email)) => app.ext_signed_in(tokens, given, email),
                    Err(why) => app.ext_notice(format!("Sign-in did not finish: {why}.")),
                }
            },
        );
    }

    fn ext_signed_in(&mut self, tokens: oauth::Tokens, scopes: Vec<String>, email: Option<String>) {
        let Some(db) = &self.store else { return };
        let mut account = match db.extension_account(google::PROVIDER) {
            Ok(a) => a.unwrap_or_default(),
            Err(e) => return self.ext_notice(format!("The saved account could not be read: {e}")),
        };
        account.tokens = Some(tokens);
        account.scopes = scopes;
        account.email = email;
        if let Err(e) = db.save_extension_account(google::PROVIDER, &account) {
            return self.ext_notice(format!("Could not save the sign-in: {e}"));
        }
        self.ext_show_account(&account);
        self.rebuild_extension_servers();
        // Google lets people untick permissions; say so when that left an extension short.
        let short: Vec<&str> = ALL
            .iter()
            .filter(|e| self.ext.enabled(**e) && !self.ext.account.allows(e.scopes()))
            .map(|e| e.title())
            .collect();
        if !short.is_empty() {
            self.ext_notice(format!(
                "Google did not allow everything {} needs. Sign in again and tick every permission box.",
                short.join(" and ")
            ));
        }
    }

    /// The Cancel button while a sign-in waits for the browser.
    pub fn ext_cancel_sign_in(&mut self) {
        if let Some(cancel) = &self.ext.signing_in {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Signs out here at once, and tells Google to forget it in the background.
    pub fn ext_sign_out(&mut self) {
        let Some(db) = &self.store else { return };
        let mut account = match db.extension_account(google::PROVIDER) {
            Ok(Some(a)) => a,
            Ok(None) => return,
            Err(e) => return self.ext_notice(format!("The saved account could not be read: {e}")),
        };
        let old = account.tokens.clone();
        account.sign_out();
        if let Err(e) = db.save_extension_account(google::PROVIDER, &account) {
            return self.ext_notice(format!("Could not sign out: {e}"));
        }
        self.ext_show_account(&account);
        self.ext.notice = None;
        self.rebuild_extension_servers();
        if let Some(tokens) = old {
            self.spawn(move || google::revoke(&tokens), |_, ()| {});
        }
    }

    /// Saves a sign-in that was renewed during a call.
    fn ext_keep_tokens(&mut self, tokens: oauth::Tokens) {
        let Some(db) = &self.store else { return };
        match db.extension_account(google::PROVIDER) {
            Ok(Some(mut account)) => {
                account.tokens = Some(tokens);
                if let Err(e) = db.save_extension_account(google::PROVIDER, &account) {
                    eprintln!("oido: could not save the renewed Google sign-in: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("oido: could not read the Google account: {e}"),
        }
    }

    /// The agent called an extension's tool: it runs in the background like any MCP call, and its
    /// result goes back when it arrives. `refuse` is told why when it cannot start.
    pub(crate) fn start_extension_call(
        &mut self,
        sid: &str,
        call_id: &str,
        server_id: &str,
        tool: &str,
        args: Value,
    ) -> Result<(), String> {
        let ext = find(server_id).ok_or("that extension is gone.")?;
        let account = match self
            .store
            .as_ref()
            .map(|db| db.extension_account(google::PROVIDER))
        {
            Some(Ok(Some(a))) => a,
            Some(Err(e)) => return Err(format!("the saved Google account could not be read: {e}")),
            _ => {
                return Err(
                    "there is no Google account. The person can add one on the Extensions page."
                        .to_owned(),
                );
            }
        };
        let cancel = self.track(sid, call_id);
        let (sid, id, tool) = (sid.to_owned(), call_id.to_owned(), tool.to_owned());
        self.spawn(
            move || {
                let outcome = google::call(ext, &tool, &args, account, &cancel);
                (outcome, cancel.load(Ordering::Relaxed))
            },
            move |app, (outcome, stopped)| {
                if let Some(tokens) = outcome.tokens {
                    app.ext_keep_tokens(tokens);
                }
                let finish = match (outcome.result, stopped) {
                    (_, true) => Finish::Stopped("Stopped before Google answered.".into()),
                    (Ok(text), false) => Finish::Done(text),
                    (Err(why), false) => Finish::Done(format!("Error: {why}.")),
                };
                app.command_done(&sid, &id, finish);
            },
        );
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn gmail() -> &'static dyn Extension {
        ALL[0]
    }

    #[test]
    fn it_says_what_stands_between_an_extension_and_its_tools() {
        let mut e = Extensions::new();
        assert_eq!(
            e.blocked(gmail()).as_deref(),
            Some("Add your Google client first.")
        );
        e.account.has_client = true;
        assert_eq!(e.blocked(gmail()).as_deref(), Some("Sign in to Google."));
        e.account.signed_in = true;
        let why = e.blocked(gmail()).unwrap();
        assert!(why.starts_with("Sign in again to allow Gmail."));
        assert!(why.contains("Missing: gmail.readonly. Google granted: nothing."));
        e.account.scopes = gmail().scopes().iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(e.blocked(gmail()), None);
    }

    #[test]
    fn sign_in_asks_only_for_what_is_on() {
        let mut e = Extensions::new();
        assert_eq!(e.wanted_scopes(), [EMAIL_SCOPE]);
        e.settings.insert(
            "gmail".into(),
            Setting {
                enabled: true,
                ..Setting::default()
            },
        );
        let wanted = e.wanted_scopes();
        assert_eq!(wanted.len(), 2);
        assert!(wanted.contains(&gmail().scopes()[0]));
    }

    #[test]
    fn a_tool_is_on_until_switched_off() {
        let mut e = Extensions::new();
        assert!(e.tool_on(gmail(), "search"));
        e.settings
            .entry("gmail".into())
            .or_default()
            .off_tools
            .insert("search".into());
        assert!(!e.tool_on(gmail(), "search") && e.tool_on(gmail(), "list_labels"));
    }

    /// An app whose store holds a Google client and a sign-in that allows Gmail.
    fn signed_in_app(db: &crate::testing::TempDb) -> App {
        let mut app = db.open();
        let account = Account {
            client_id: "cid".into(),
            client_secret: "secret".into(),
            tokens: Some(oauth::Tokens {
                access_token: "t".into(),
                refresh_token: None,
                expires_at: None,
                client_id: "cid".into(),
                client_secret: None,
                token_endpoint: String::new(),
            }),
            email: Some("me@x.org".into()),
            scopes: gmail().scopes().iter().map(|s| (*s).to_owned()).collect(),
        };
        app.store
            .as_ref()
            .unwrap()
            .save_extension_account(google::PROVIDER, &account)
            .unwrap();
        app.load_extensions();
        app
    }

    #[test]
    fn an_extension_is_no_server_until_it_is_on_and_then_offers_the_tools_that_are_on() {
        let db = crate::testing::TempDb::new();
        let mut app = signed_in_app(&db);
        assert!(app.mcp.servers.is_empty());
        assert!(
            app.all_tools()
                .iter()
                .all(|t| !t.name.starts_with("mcp__gmail__"))
        );

        app.ext_toggle(gmail());
        let names = |app: &App| -> Vec<String> {
            app.all_tools()
                .into_iter()
                .map(|t| t.name)
                .filter(|n| n.starts_with("mcp__gmail__"))
                .collect()
        };
        assert_eq!(
            names(&app),
            [
                "mcp__gmail__search",
                "mcp__gmail__read_message",
                "mcp__gmail__read_thread",
                "mcp__gmail__list_labels"
            ]
        );
        assert!(app.tool_known("mcp__gmail__search"));

        app.ext_toggle_tool(gmail(), "read_thread");
        assert!(!names(&app).contains(&"mcp__gmail__read_thread".to_owned()));
        assert!(!app.tool_known("mcp__gmail__read_thread"));

        // What was chosen is there on the next launch.
        let again = db.open();
        assert!(again.ext.enabled(gmail()) && !again.ext.tool_on(gmail(), "read_thread"));
        assert!(
            again.tool_known("mcp__gmail__search") && !again.tool_known("mcp__gmail__read_thread")
        );
        assert_eq!(again.ext.account.email.as_deref(), Some("me@x.org"));

        app.ext_toggle(gmail());
        assert!(app.mcp.servers.is_empty());
    }

    #[test]
    fn turning_on_needs_a_client_and_signing_out_keeps_the_tools_offered_with_the_reason() {
        let db = crate::testing::TempDb::new();
        let mut app = db.open();
        app.ext_toggle(gmail());
        assert!(!app.ext.enabled(gmail()) && app.ext.notice.is_some());

        let mut app = signed_in_app(&db);
        app.ext_toggle(gmail());
        app.ext_sign_out();
        // Signed out: still on and its tools still offered; the page says why they will not work.
        assert!(
            app.ext.enabled(gmail()) && app.ext.account.has_client && !app.ext.account.signed_in
        );
        assert!(
            app.all_tools()
                .iter()
                .any(|t| t.name == "mcp__gmail__search")
        );
        assert_eq!(app.mcp.servers[0].detail, "Sign in to Google.");
    }

    #[test]
    fn saving_a_different_client_drops_the_old_sign_in() {
        let db = crate::testing::TempDb::new();
        let mut app = signed_in_app(&db);
        assert!(!app.ext_can_save_client());
        app.ext.client_id = TextEdit::new("other-id");
        app.ext.client_secret = TextEdit::new("other-secret");
        app.ext_save_client();
        assert!(app.ext.account.has_client && !app.ext.account.signed_in);
        assert_eq!(app.ext.account.client_id, "other-id");
        assert!(
            app.ext.client_secret.text.is_empty(),
            "the secret is not kept on screen"
        );
    }

    #[test]
    fn a_call_to_an_extension_that_is_off_is_refused_in_words() {
        let db = crate::testing::TempDb::new();
        let mut app = signed_in_app(&db);
        assert!(
            app.start_extension_call("s", "c", "ext:nothing", "x", Value::Null)
                .is_err()
        );
    }
}
