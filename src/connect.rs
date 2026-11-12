//! Connecting a provider, keeping its model lists fresh, and choosing the default model.
//!
//! A key is sealed, then checked by asking the provider which models it can use, and only if that
//! works are the provider, its sealed key and its models saved together. Asking the network
//! happens on a worker thread (see [`App::spawn`]) so the window never freezes.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use rune::TextEdit;
use zeroize::Zeroizing;

use crate::app::{App, now};
use crate::chatgpt;
use crate::credentials::{self, Credential, Tokens};
use crate::listing::{self, Error as ListError};
use crate::models::{ModelInfo, ModelRef};
use crate::providers::{CODEX, NEW_SERVER, ProviderAuth, SigningIn, normalize_server_url};
use crate::store::Verified;

type Listed = Result<Vec<ModelInfo>, ListError>;

impl App {
    /// Connects a built-in provider with the key in the form. If the key cannot be sealed, or the
    /// provider does not accept it, nothing is saved and the form says why.
    pub fn connect_provider(&mut self, id: &str) {
        let key = Zeroizing::new(self.providers.key.text.trim().to_owned());
        if key.is_empty() || self.providers.busy.contains(id) {
            return;
        }
        let Some(p) = self.providers.get(id) else {
            return;
        };
        let (label, custom, base) = (p.label.clone(), p.custom, p.base_url.clone());
        let Some(ep) = listing::endpoint(id, custom, base.as_deref()) else {
            return self
                .providers
                .fail(id, format!("{label} is not supported yet."));
        };
        let sealed = match self.seal(id, &key) {
            Ok(sealed) => sealed,
            Err(why) => return self.providers.fail(id, why),
        };
        self.providers.error = None;
        self.providers.busy.insert(id.to_owned());
        let (id, key, ask) = (id.to_owned(), key.clone(), Credential::Key(key));
        self.spawn(
            move || listing::fetch(&ep, Some(&ask)),
            move |app, listed| app.connected(&id, &key, sealed, listed),
        );
    }

    fn connected(&mut self, id: &str, key: &str, sealed: Vec<u8>, listed: Listed) {
        self.providers.busy.remove(id);
        let label = self.providers.label_of(id);
        let models = match listed {
            Ok(models) => models,
            Err(e) => return self.providers.fail(id, e.describe(&label)),
        };
        let at = now();
        self.providers.connect(id);
        self.providers.set_hint(id, key);
        self.providers.set_models(id, models.clone(), at);
        let verified = Verified {
            id,
            sealed: Some(&sealed),
            models: &models,
            fetched_at: at,
        };
        if let Err(why) = self.write_providers(Some(verified)) {
            self.providers.disconnect(id);
            return self.providers.fail(id, why);
        }
        self.providers.key = TextEdit::new("");
    }

    /// Adds a server or saves edits to one. A blank key keeps the current one. As with a provider,
    /// the server is asked for its models first, and a server that does not answer is not saved.
    pub fn save_server(&mut self, id: &str) {
        if !self.providers.can_save(id) {
            return;
        }
        let typed = Zeroizing::new(self.providers.key.text.trim().to_owned());
        let existing = self.providers.get(id).cloned();
        let name = self.providers.name.text.trim().to_owned();
        let url = self.providers.url.text.trim().to_owned();
        let target = if id == NEW_SERVER {
            self.providers.new_server_id(&name)
        } else {
            id.to_owned()
        };
        if self.providers.busy.contains(&target) {
            return;
        }
        let base = if url.is_empty() {
            existing
                .as_ref()
                .and_then(|p| p.base_url.clone())
                .unwrap_or_default()
        } else {
            normalize_server_url(&url)
        };
        let label = if !name.is_empty() {
            name
        } else {
            existing
                .as_ref()
                .map_or_else(|| target.clone(), |p| p.label.clone())
        };
        let Some(ep) = listing::endpoint(&target, true, Some(&base)).filter(|_| !base.is_empty())
        else {
            return self.providers.fail(id, "A server needs an address.".into());
        };
        // The key to ask with: the one just typed, else the one already kept for this server.
        let (ask, sealed) = if typed.is_empty() {
            let kept = match (&self.store, &existing) {
                (Some(db), Some(_)) => db.provider_credential(&target).map_err(|e| e.to_string()),
                _ => Ok(None),
            };
            match kept {
                Ok(key) => (key, None),
                Err(why) => return self.providers.fail(id, why),
            }
        } else {
            match self.seal(&target, &typed) {
                Ok(sealed) => (Some(Credential::Key(typed.clone())), Some(sealed)),
                Err(why) => return self.providers.fail(id, why),
            }
        };
        self.providers.error = None;
        self.providers.busy.insert(target.clone());
        let id = id.to_owned();
        self.spawn(
            move || listing::fetch(&ep, ask.as_ref()),
            move |app, listed| app.server_checked(&id, &target, &label, sealed, &typed, listed),
        );
    }

    fn server_checked(
        &mut self,
        id: &str,
        target: &str,
        label: &str,
        sealed: Option<Vec<u8>>,
        typed: &str,
        listed: Listed,
    ) {
        self.providers.busy.remove(target);
        let models = match listed {
            Ok(models) => models,
            Err(e) => return self.providers.fail(id, e.describe(label)),
        };
        let Some((saved, created)) = self.providers.save_server(id) else {
            return;
        };
        let at = now();
        if sealed.is_some() {
            self.providers.set_hint(&saved, typed);
        }
        self.providers.set_models(&saved, models.clone(), at);
        let verified = Verified {
            id: &saved,
            sealed: sealed.as_deref(),
            models: &models,
            fetched_at: at,
        };
        if let Err(why) = self.write_providers(Some(verified)) {
            if created {
                self.providers.disconnect(&saved);
            }
            return self.providers.fail(id, why);
        }
        self.providers.key = TextEdit::new("");
    }

    /// Signs a provider out. Its key and models are forgotten with it, by the same write; a server
    /// is removed.
    pub fn disconnect_provider(&mut self, id: &str) {
        self.providers.disconnect(id);
        self.save_providers();
    }

    /// Asks a connected provider for its models again (the "Refresh models" button). A failure
    /// shows on its row and leaves the saved list as it was.
    pub fn refresh_models(&mut self, id: &str) {
        if self.providers.busy.contains(id) {
            return;
        }
        let Some(p) = self.providers.get(id).filter(|p| p.connected) else {
            return;
        };
        let (label, custom, base) = (p.label.clone(), p.custom, p.base_url.clone());
        let Some(ep) = listing::endpoint(id, custom, base.as_deref()) else {
            return;
        };
        let kept = match &self.store {
            Some(db) => db.provider_credential(id).map_err(|e| e.to_string()),
            None => Ok(None),
        };
        let ask = match kept {
            Ok(key) => key,
            Err(why) => {
                self.providers.row_errors.insert(id.to_owned(), why);
                return;
            }
        };
        self.providers.row_errors.remove(id);
        self.providers.busy.insert(id.to_owned());
        let id = id.to_owned();
        self.spawn(
            move || listing::fetch(&ep, ask.as_ref()),
            move |app, listed| app.refreshed(&id, &label, listed),
        );
    }

    fn refreshed(&mut self, id: &str, label: &str, listed: Listed) {
        self.providers.busy.remove(id);
        match listed {
            Ok(models) => {
                let at = now();
                self.providers.set_models(id, models.clone(), at);
                self.save("the models", |db| db.save_models(id, &models, at));
                self.providers.row_errors.remove(id);
            }
            Err(e) => {
                self.providers
                    .row_errors
                    .insert(id.to_owned(), e.describe(label));
            }
        }
    }

    /// Starts a subscription sign-in for `id`: the provider's approval page opens in the browser.
    /// A Claude Pro/Max sign-in then has its code pasted back into the form; a ChatGPT one comes
    /// back to this computer by itself.
    pub fn start_subscription_sign_in(&mut self, id: &str) {
        if self.providers.busy.contains(id) {
            return;
        }
        if id == CODEX {
            return self.start_chatgpt_sign_in(id);
        }
        let sign = match credentials::sign_in() {
            Ok(sign) => sign,
            Err(why) => return self.providers.fail(id, why),
        };
        // The address is shown on the form too, so a browser that will not open is not a dead end.
        self.providers.error = crate::browser::open(&sign.url).err();
        self.providers.signing_in = Some(SigningIn {
            id: id.to_owned(),
            sign,
            cancel: None,
        });
        self.providers.code = TextEdit::new("");
    }

    /// Opens ChatGPT's approval page and waits, on a worker thread, for the browser to come back to
    /// this computer with the code. Letting the sign-in go (see [`SigningIn`]) ends the wait.
    fn start_chatgpt_sign_in(&mut self, id: &str) {
        let (sign, listener) = match chatgpt::begin() {
            Ok(begun) => begun,
            Err(why) => return self.providers.fail(id, why),
        };
        // The address is shown on the form too, so a browser that will not open is not a dead end.
        self.providers.error = crate::browser::open(&sign.url).err();
        let cancel = Arc::new(AtomicBool::new(false));
        self.providers.signing_in = Some(SigningIn {
            id: id.to_owned(),
            sign: sign.clone(),
            cancel: Some(Arc::clone(&cancel)),
        });
        self.providers.busy.insert(id.to_owned());
        let colors = self.look.colors();
        let id = id.to_owned();
        self.spawn(
            move || {
                let tokens = chatgpt::finish(&listener, &sign, &cancel, &colors);
                (tokens, cancel.load(std::sync::atomic::Ordering::Relaxed))
            },
            move |app, (tokens, cancelled)| app.chatgpt_signed_in(&id, tokens, cancelled),
        );
    }

    /// The browser came back, or the wait ended. A sign-in that was let go is not reported.
    fn chatgpt_signed_in(&mut self, id: &str, tokens: Result<Tokens, String>, cancelled: bool) {
        self.providers.signing_in = None;
        if cancelled {
            self.providers.busy.remove(id);
            return;
        }
        self.subscribed(id, tokens);
    }

    /// Trades the pasted code for tokens. The state it carries must be the one this sign-in made,
    /// or the code belongs to another request and is refused.
    pub fn finish_subscription_sign_in(&mut self, id: &str) {
        if self.providers.busy.contains(id) {
            return;
        }
        let Some((verifier, state)) = self
            .providers
            .signing_in
            .as_ref()
            .filter(|s| s.id == id)
            .map(|s| (s.sign.verifier.clone(), s.sign.state.clone()))
        else {
            return;
        };
        let pasted = self.providers.code.text.trim().to_owned();
        if pasted.is_empty() {
            return self
                .providers
                .fail(id, "Paste the code the browser showed.".to_owned());
        }
        let (code, shown) = pasted
            .split_once('#')
            .map_or((pasted.as_str(), ""), |(c, s)| (c.trim(), s.trim()));
        if code.is_empty() {
            return self
                .providers
                .fail(id, "Paste the code the browser showed.".to_owned());
        }
        if shown.is_empty() {
            return self.providers.fail(
                id,
                "Paste the whole code the page shows, including the part after #.".to_owned(),
            );
        }
        if shown != state {
            return self.providers.fail(
                id,
                "That code came from a different sign-in. Start again.".to_owned(),
            );
        }
        let code = code.to_owned();
        self.providers.error = None;
        self.providers.busy.insert(id.to_owned());
        let id = id.to_owned();
        self.spawn(
            move || credentials::exchange(&code, &state, &verifier),
            move |app, tokens| app.subscribed(&id, tokens),
        );
    }

    /// Tokens in hand: the provider is asked for its models with them before anything is saved, so
    /// a subscription that cannot infer is not stored as connected.
    fn subscribed(&mut self, id: &str, tokens: Result<Tokens, String>) {
        self.providers.busy.remove(id);
        let tokens = match tokens {
            Ok(tokens) => tokens,
            Err(why) => return self.providers.fail(id, why),
        };
        let Some(ep) = self.list_endpoint(id) else {
            let label = self.providers.label_of(id);
            return self
                .providers
                .fail(id, format!("{label} is not supported yet."));
        };
        self.providers.busy.insert(id.to_owned());
        let ask = tokens.credential();
        let (id, tokens) = (id.to_owned(), tokens);
        self.spawn(
            move || listing::fetch(&ep, Some(&ask)),
            move |app, listed| app.subscription_checked(&id, &tokens, listed),
        );
    }

    fn subscription_checked(&mut self, id: &str, tokens: &Tokens, listed: Listed) {
        self.providers.busy.remove(id);
        let label = self.providers.label_of(id);
        let models = match listed {
            Ok(models) => models,
            Err(ListError::Rejected) => {
                return self.providers.fail(
                    id,
                    format!("{label} did not accept the sign-in. Try signing in again."),
                );
            }
            Err(e) => return self.providers.fail(id, e.describe(&label)),
        };
        let sealed = match tokens.json().and_then(|json| self.seal(id, &json)) {
            Ok(sealed) => sealed,
            Err(why) => return self.providers.fail(id, why),
        };
        let at = now();
        self.providers.connect(id);
        self.providers.set_subscription_hint(id);
        self.providers.set_models(id, models.clone(), at);
        let verified = Verified {
            id,
            sealed: Some(&sealed),
            models: &models,
            fetched_at: at,
        };
        if let Err(why) = self.write_providers(Some(verified)) {
            self.providers.disconnect(id);
            return self.providers.fail(id, why);
        }
        self.providers.code = TextEdit::new("");
    }

    /// Keeps signed-in subscriptions current, from the app's half-minute tick. Offline, the old
    /// token is left as it is and the next tick tries again.
    pub fn refresh_subscription_tokens(&mut self) {
        let at = now();
        let due: Vec<String> = self
            .providers
            .list
            .iter()
            .filter(|p| {
                p.connected
                    && p.auth == ProviderAuth::Subscription
                    && !self.providers.busy.contains(&p.id)
            })
            .map(|p| p.id.clone())
            .collect();
        for id in due {
            let tokens = match self.store.as_ref().map(|db| db.subscription_tokens(&id)) {
                Some(Ok(Some(tokens))) if tokens.is_stale(at) => tokens,
                _ => continue,
            };
            self.providers.busy.insert(id.clone());
            let renew: fn(&Tokens) -> Result<Tokens, String> = if id == CODEX {
                chatgpt::refresh
            } else {
                credentials::refresh
            };
            self.spawn(
                move || renew(&tokens),
                move |app, tokens| app.refreshed_tokens(&id, tokens),
            );
        }
    }

    /// A refreshed pair is sealed back in place: the refresh token rotates, so the old one is spent.
    fn refreshed_tokens(&mut self, id: &str, tokens: Result<Tokens, String>) {
        self.providers.busy.remove(id);
        let label = self.providers.label_of(id);
        match tokens {
            Ok(tokens) => {
                if let Some(db) = &self.store
                    && let Err(e) = db.save_subscription_tokens(id, &tokens)
                {
                    eprintln!("oido: could not keep {label}'s refreshed sign-in: {e}");
                }
            }
            Err(why) => eprintln!("oido: could not refresh {label}'s sign-in: {why}"),
        }
    }

    /// Where a provider's models are listed, if it has an address we know.
    fn list_endpoint(&self, id: &str) -> Option<listing::Endpoint> {
        self.providers
            .get(id)
            .and_then(|p| listing::endpoint(&p.id, p.custom, p.base_url.as_deref()))
    }

    /// A model picker was opened: any connected provider whose list is stale is asked again in the
    /// background, so the picker is right by the time it is looked at. Offline, the saved lists stay.
    pub fn models_opened(&mut self) {
        let at = now();
        let stale: Vec<String> = self
            .providers
            .list
            .iter()
            .filter(|p| p.connected && self.providers.is_stale(&p.id, at))
            .map(|p| p.id.clone())
            .collect();
        for id in stale {
            self.refresh_models(&id);
        }
    }

    /// Sets, or clears, the model agents use when they have none of their own.
    pub fn set_default_model(&mut self, model: Option<ModelRef>) {
        self.providers.default_model = model;
        self.save_default_model();
    }

    /// Seals `key` for provider `id`, or says in words why it could not be.
    fn seal(&self, id: &str, key: &str) -> Result<Vec<u8>, String> {
        match &self.store {
            Some(db) => db.seal_provider_key(id, key).map_err(|e| e.to_string()),
            None => Err("Saving is not available, so the key could not be kept.".to_owned()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testing::{TempDb, serve};
    use crate::vault::Vault;

    const LIST: &str =
        r#"{"data":[{"id":"llama-3.3-70b"},{"id":"qwen-2.5-coder"},{"id":"text-embedding-x"}]}"#;
    const LIST_2: &str = r#"{"data":[{"id":"llama-3.3-70b"},{"id":"mistral-small"}]}"#;

    fn ids(app: &App, provider: &str) -> Vec<String> {
        app.providers.models[provider]
            .models
            .iter()
            .map(|m| m.id.clone())
            .collect()
    }

    /// Fills the add-a-server form (a name, a url, optionally a key) and saves it.
    fn add_server(app: &mut App, name: &str, base: &str, key: &str) {
        app.providers.toggle(NEW_SERVER);
        app.providers.name = TextEdit::new(name);
        app.providers.url = TextEdit::new(base);
        app.providers.key = TextEdit::new(key);
        app.save_server(NEW_SERVER);
    }

    #[test]
    fn a_server_that_lists_models_is_added_with_them_and_its_key_is_checked_with_the_key() {
        let db = TempDb::new();
        let mut app = db.open();
        let (base, seen) = serve(vec![(200, LIST)]);
        add_server(
            &mut app,
            "Local box",
            &format!("{base}/v1"),
            "sk-local-secret",
        );

        let request = seen.recv().unwrap();
        assert!(request.starts_with("get /v1/models "), "{request}");
        assert!(
            request.contains("authorization: bearer sk-local-secret"),
            "the key it was checked with"
        );
        assert!(app.providers.error.is_none() && app.providers.busy.is_empty());
        assert_eq!(
            ids(&app, "llama-server:local-box"),
            vec!["llama-3.3-70b", "qwen-2.5-coder"],
            "embeddings are left out"
        );
        assert_eq!(app.providers.key.text, "", "the typed key is cleared");
        drop(app);

        let app = db.open();
        assert_eq!(
            ids(&app, "llama-server:local-box"),
            vec!["llama-3.3-70b", "qwen-2.5-coder"],
            "the list is saved, so the picker works offline"
        );
        let key = app
            .store
            .as_ref()
            .unwrap()
            .provider_key("llama-server:local-box")
            .unwrap()
            .unwrap();
        assert_eq!(&*key, "sk-local-secret", "and the key opens");
        assert_eq!(app.providers.groups().len(), 1);
    }

    #[test]
    fn a_server_that_rejects_the_key_or_is_down_is_not_added_and_the_form_says_why() {
        let db = TempDb::new();
        let mut app = db.open();
        let (base, _) = serve(vec![(401, r#"{"error":"no"}"#)]);
        add_server(&mut app, "Locked", &format!("{base}/v1"), "wrong");
        assert!(
            app.providers.get("llama-server:locked").is_none(),
            "nothing was added"
        );
        assert!(
            app.providers
                .error
                .as_deref()
                .unwrap()
                .contains("Locked did not accept the key")
        );
        assert_eq!(
            app.providers.open.as_deref(),
            Some(NEW_SERVER),
            "the form stays open for another try"
        );
        assert_eq!(
            app.providers.url.text,
            format!("{base}/v1"),
            "with what was typed"
        );
        assert!(app.providers.busy.is_empty());

        let gone = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}/v1", l.local_addr().unwrap())
        };
        add_server(&mut app, "Down", &gone, "");
        assert!(app.providers.get("llama-server:down").is_none());
        assert!(
            app.providers
                .error
                .as_deref()
                .unwrap()
                .contains("Could not reach Down")
        );
        assert!(
            app.store
                .as_ref()
                .unwrap()
                .load_providers()
                .unwrap()
                .iter()
                .all(|p| !p.custom),
            "and nothing was saved"
        );
    }

    #[test]
    fn a_key_that_cannot_be_sealed_stops_before_the_network_is_asked() {
        let db = TempDb::new();
        let mut app = db.open_with(Vault::unavailable());
        app.providers.toggle("openai");
        app.providers.key = TextEdit::new("sk-live-1234567890");
        app.connect_provider("openai");
        assert!(!app.providers.get("openai").unwrap().connected);
        assert!(app.providers.error.as_deref().unwrap().contains("keyring"));
        assert!(app.providers.busy.is_empty(), "no request was made");
        assert_eq!(
            app.providers.key.text, "sk-live-1234567890",
            "what was typed is kept"
        );
    }

    #[test]
    fn with_no_database_a_key_is_not_pretended_kept() {
        let mut app = App::new(None);
        app.providers.toggle("openai");
        app.providers.key = TextEdit::new("sk-live-1234567890");
        app.connect_provider("openai");
        assert!(!app.providers.get("openai").unwrap().connected);
        assert!(app.providers.error.is_some());
    }

    #[test]
    fn a_checked_key_connects_the_provider_and_is_saved_with_its_models_in_one_step() {
        let db = TempDb::new();
        let mut app = db.open();
        app.providers.toggle("anthropic");
        app.providers.key = TextEdit::new("sk-ant-live-1234567890");
        let sealed = app
            .store
            .as_ref()
            .unwrap()
            .seal_provider_key("anthropic", "sk-ant-live-1234567890")
            .unwrap();
        let models = listing::finish_all(vec![crate::models::RawModel {
            id: "claude-opus-4-7".into(),
            ..Default::default()
        }]);
        app.providers.busy.insert("anthropic".into());
        app.connected("anthropic", "sk-ant-live-1234567890", sealed, Ok(models));

        assert!(app.providers.get("anthropic").unwrap().connected);
        assert_eq!(
            app.providers
                .get("anthropic")
                .unwrap()
                .key_masked
                .as_deref(),
            Some("sk-…7890")
        );
        assert!(app.providers.busy.is_empty() && app.providers.key.text.is_empty());
        drop(app);
        let app = db.open();
        assert!(app.providers.get("anthropic").unwrap().connected);
        assert_eq!(
            app.providers.models["anthropic"].models[0].name, "Claude Opus 4.7",
            "the catalog's name came with it"
        );
        assert_eq!(
            &*app
                .store
                .as_ref()
                .unwrap()
                .provider_key("anthropic")
                .unwrap()
                .unwrap(),
            "sk-ant-live-1234567890"
        );
    }

    #[test]
    fn a_rejected_or_unreachable_provider_is_not_connected_and_nothing_is_saved() {
        let db = TempDb::new();
        let mut app = db.open();
        app.providers.toggle("openai");
        app.providers.key = TextEdit::new("sk-bad");
        let sealed = app
            .store
            .as_ref()
            .unwrap()
            .seal_provider_key("openai", "sk-bad")
            .unwrap();
        app.connected("openai", "sk-bad", sealed, Err(ListError::Rejected));
        assert!(!app.providers.get("openai").unwrap().connected);
        assert_eq!(
            app.providers.error.as_deref(),
            Some("OpenAI did not accept the key. Check it and try again.")
        );
        let sealed = app
            .store
            .as_ref()
            .unwrap()
            .seal_provider_key("openai", "sk-bad")
            .unwrap();
        app.connected("openai", "sk-bad", sealed, Err(ListError::Unreachable));
        assert!(
            app.providers
                .error
                .as_deref()
                .unwrap()
                .starts_with("Could not reach OpenAI")
        );
        assert!(
            app.store
                .as_ref()
                .unwrap()
                .provider_key("openai")
                .unwrap()
                .is_none(),
            "no key was stored"
        );
        assert!(
            app.store
                .as_ref()
                .unwrap()
                .connected_without_key()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn refreshing_replaces_the_list_and_a_failure_keeps_the_old_one_and_says_so_on_the_row() {
        let db = TempDb::new();
        let mut app = db.open();
        let (base, seen) = serve(vec![
            (200, LIST),
            (404, "{}"),
            (200, LIST_2),
            (404, "{}"),
            (401, "{}"),
        ]);
        add_server(&mut app, "Box", &format!("{base}/v1"), "sk-box");
        let _ = (seen.recv().unwrap(), seen.recv().unwrap());

        app.refresh_models("llama-server:box");
        assert!(
            seen.recv()
                .unwrap()
                .contains("authorization: bearer sk-box"),
            "a refresh uses the saved key"
        );
        let _ = seen.recv().unwrap();
        assert_eq!(
            ids(&app, "llama-server:box"),
            vec!["llama-3.3-70b", "mistral-small"]
        );
        assert!(app.providers.row_errors.is_empty() && app.providers.busy.is_empty());

        app.refresh_models("llama-server:box");
        assert!(app.providers.row_errors["llama-server:box"].contains("did not accept the key"));
        assert_eq!(
            ids(&app, "llama-server:box"),
            vec!["llama-3.3-70b", "mistral-small"],
            "the saved list stays"
        );
        drop(app);
        assert_eq!(
            ids(&db.open(), "llama-server:box"),
            vec!["llama-3.3-70b", "mistral-small"],
            "and so does the saved copy"
        );
    }

    #[test]
    fn opening_a_picker_refreshes_only_the_lists_that_are_stale() {
        let db = TempDb::new();
        let mut app = db.open();
        let (old, old_seen) = serve(vec![(200, LIST_2)]);
        let (fresh, fresh_seen) = serve(vec![(200, LIST_2)]);
        add_server(&mut app, "Old", &format!("{old}/v1"), "");
        add_server(&mut app, "Fresh", &format!("{fresh}/v1"), "");
        // the first check of each answered with LIST_2 too; now age one list
        let _ = (old_seen.recv().unwrap(), fresh_seen.recv().unwrap());
        app.providers
            .models
            .get_mut("llama-server:old")
            .unwrap()
            .fetched_at = Some(0);
        let (old2, old2_seen) = serve(vec![(200, LIST)]);
        app.providers
            .list
            .iter_mut()
            .find(|p| p.id == "llama-server:old")
            .unwrap()
            .base_url = Some(format!("{old2}/v1"));

        app.models_opened();
        assert!(
            old2_seen.try_recv().is_ok(),
            "the stale provider was asked again"
        );
        assert_eq!(
            ids(&app, "llama-server:old"),
            vec!["llama-3.3-70b", "qwen-2.5-coder"]
        );
        assert!(
            fresh_seen.try_recv().is_err(),
            "the fresh one was left alone"
        );
    }

    #[test]
    fn the_default_model_survives_a_restart_and_disconnecting_forgets_a_providers_models() {
        let db = TempDb::new();
        let mut app = db.open();
        let (base, _) = serve(vec![(200, LIST)]);
        add_server(&mut app, "Box", &format!("{base}/v1"), "");
        let model = ModelRef::new("llama-server:box", "llama-3.3-70b");
        app.set_default_model(Some(model.clone()));
        drop(app);

        let mut app = db.open();
        assert_eq!(app.providers.default_model, Some(model.clone()));
        assert_eq!(
            app.providers.status(None),
            crate::providers::ModelStatus::Ready(model)
        );
        app.disconnect_provider("llama-server:box");
        assert!(
            matches!(
                app.providers.status(None),
                crate::providers::ModelStatus::Unavailable { .. }
            ),
            "the default is kept but cannot answer"
        );
        drop(app);
        let app = db.open();
        assert!(app.providers.get("llama-server:box").is_none() && app.providers.models.is_empty());
        assert!(
            app.providers.default_model.is_some(),
            "still remembered, and flagged"
        );
    }

    /// Saves `id` as connected with a subscription, as a finished sign-in does.
    fn subscribed(app: &App, id: &str, tokens: &Tokens) {
        let store = app.store.as_ref().unwrap();
        let mut list = app.providers.list.clone();
        let p = list.iter_mut().find(|p| p.id == id).unwrap();
        p.connected = true;
        p.auth = ProviderAuth::Subscription;
        p.key_masked = Some("a plan".to_owned());
        let sealed = store
            .seal_provider_key(id, &tokens.json().unwrap())
            .unwrap();
        let verified = Verified {
            id,
            sealed: Some(&sealed),
            models: &[],
            fetched_at: now(),
        };
        store.replace_providers(&list, Some(verified)).unwrap();
    }

    #[test]
    fn an_expired_subscription_asks_for_another_sign_in_and_a_fresh_one_is_sent() {
        let db = TempDb::new();
        let fresh = Tokens {
            access_token: "sk-ant-oat-1".into(),
            refresh_token: "refresh-1".into(),
            expires_at: now() + 3600,
        };
        {
            let app = db.open();
            subscribed(&app, "anthropic", &fresh);
        }
        {
            let app = db.open();
            let (endpoint, credential) = app.target_for("anthropic").unwrap();
            assert_eq!(endpoint.base, "https://api.anthropic.com/v1");
            let credential = credential.unwrap();
            assert!(credential.is_subscription());
            assert_eq!(
                credential.token(),
                "sk-ant-oat-1",
                "the token is what is sent"
            );
        }

        let stale = Tokens {
            expires_at: now() - 1,
            ..fresh
        };
        {
            let app = db.open();
            subscribed(&app, "anthropic", &stale);
        }
        let app = db.open();
        let why = app.target_for("anthropic").unwrap_err();
        assert!(
            why.contains("expired") && why.contains("sign in again"),
            "a stale token is not sent: {why}"
        );
    }

    #[test]
    fn a_code_from_another_sign_in_is_refused_before_the_token_endpoint() {
        let db = TempDb::new();
        let mut app = db.open();
        app.providers.open = Some("anthropic".to_owned());
        app.providers.signing_in = Some(SigningIn {
            id: "anthropic".to_owned(),
            sign: crate::credentials::SignIn {
                verifier: "verifier".to_owned(),
                state: "state-1".to_owned(),
                url: "https://claude.ai/oauth/authorize".to_owned(),
            },
            cancel: None,
        });

        app.providers.code = TextEdit::new("the-code#other-state");
        app.finish_subscription_sign_in("anthropic");
        let why = app.providers.error.as_deref().unwrap();
        assert!(why.contains("different sign-in"), "{why}");
        assert!(app.providers.busy.is_empty(), "no request was made");

        app.providers.code = TextEdit::new("the-code");
        app.finish_subscription_sign_in("anthropic");
        let why = app.providers.error.as_deref().unwrap();
        assert!(
            why.contains("after #"),
            "the whole code is asked for: {why}"
        );
        assert!(app.providers.busy.is_empty());
    }

    #[test]
    fn a_chatgpt_plan_is_asked_at_the_codex_backend_with_its_token_until_it_expires() {
        let db = TempDb::new();
        let token = crate::testing::access_token(Some("acct-1"));
        let fresh = Tokens {
            access_token: token.clone(),
            refresh_token: "refresh-1".into(),
            expires_at: now() + 3600,
        };
        {
            let app = db.open();
            subscribed(&app, "codex", &fresh);
        }
        {
            let app = db.open();
            let (endpoint, credential) = app.target_for("codex").unwrap();
            assert_eq!(endpoint.api, crate::completion::Api::Codex);
            assert_eq!(endpoint.base, chatgpt::BASE);
            assert_eq!(credential.unwrap().token(), token);
        }
        let stale = Tokens {
            expires_at: now() - 1,
            ..fresh
        };
        {
            let app = db.open();
            subscribed(&app, "codex", &stale);
        }
        let app = db.open();
        let why = app.target_for("codex").unwrap_err();
        assert!(
            why.contains("OpenAI Codex") && why.contains("expired"),
            "{why}"
        );
    }

    #[test]
    fn a_chatgpt_sign_in_that_was_let_go_says_nothing_and_one_that_failed_says_why() {
        let db = TempDb::new();
        let mut app = db.open();
        let cancel = Arc::new(AtomicBool::new(false));
        let waiting = |cancel: &Arc<AtomicBool>| SigningIn {
            id: "codex".to_owned(),
            sign: crate::credentials::SignIn {
                verifier: "v".to_owned(),
                state: "s".to_owned(),
                url: "https://auth.openai.com/oauth/authorize".to_owned(),
            },
            cancel: Some(Arc::clone(cancel)),
        };

        app.providers.busy.insert("codex".to_owned());
        app.providers.signing_in = Some(waiting(&cancel));
        app.chatgpt_signed_in("codex", Err("sign-in was cancelled".to_owned()), true);
        assert!(app.providers.busy.is_empty() && app.providers.signing_in.is_none());
        assert!(app.providers.error.is_none(), "cancelling is not an error");

        app.providers.busy.insert("codex".to_owned());
        app.providers.signing_in = Some(waiting(&cancel));
        app.chatgpt_signed_in(
            "codex",
            Err("the sign-in was refused: access_denied".to_owned()),
            false,
        );
        assert!(app.providers.busy.is_empty() && app.providers.signing_in.is_none());
        assert_eq!(app.providers.open.as_deref(), Some("codex"));
        assert!(
            app.providers
                .error
                .as_deref()
                .is_some_and(|e| e.contains("access_denied")),
            "{:?}",
            app.providers.error
        );
        assert!(!app.providers.get("codex").unwrap().connected);
    }
}
