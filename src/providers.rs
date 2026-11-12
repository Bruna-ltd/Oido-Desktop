//! AI providers: which services run the agents. Mock state until `oido-core` serves
//! `/api/providers`; the shapes and rules mirror Studio's `ProvidersClient`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rune::TextEdit;

use crate::models::{ModelInfo, ModelRef, ProviderModels};
use crate::picker::{Group, PickerState};

/// The id Studio gives a user-added server before it has a name; the form's slot for "new".
pub const NEW_SERVER: &str = "llama-server:new";

/// How a provider is asked: with its own API key, or with a subscription (Claude Pro/Max, or a
/// ChatGPT plan).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ProviderAuth {
    #[default]
    ApiKey,
    Subscription,
}

#[derive(Clone)]
pub struct Provider {
    pub id: String,
    pub label: String,
    /// The provider's own brand colour, as hex.
    pub color: &'static str,
    pub connected: bool,
    pub key_masked: Option<String>,
    pub base_url: Option<String>,
    pub custom: bool,
    /// How a connected provider is asked.
    pub auth: ProviderAuth,
}

/// A subscription sign-in waiting on the browser: for the code it showed, or, when it comes back to
/// this computer by itself, for the browser to do so.
pub struct SigningIn {
    pub id: String,
    pub sign: crate::credentials::SignIn,
    /// Set for a sign-in that waits on this computer's port. Letting the sign-in go, by closing its
    /// form or signing the provider out, ends that wait.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Drop for SigningIn {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

pub struct Providers {
    pub list: Vec<Provider>,
    /// Each connected provider's models, as last fetched.
    pub models: HashMap<String, ProviderModels>,
    /// What an agent with no model of its own uses.
    pub default_model: Option<ModelRef>,
    pub default_picker: PickerState,
    /// Providers being asked for their models right now.
    pub busy: HashSet<String>,
    /// Why a connected provider's last refresh failed, shown on its row.
    pub row_errors: HashMap<String, String>,
    /// The one connect form that is open, by provider id. Closed rows stay one quiet line.
    pub open: Option<String>,
    /// Why the last connect or save failed, shown in the open form.
    pub error: Option<String>,
    pub key: TextEdit,
    pub url: TextEdit,
    pub name: TextEdit,
    /// The subscription sign-in in progress, if any.
    pub signing_in: Option<SigningIn>,
    /// The code the browser showed, to be pasted back.
    pub code: TextEdit,
}

/// TypeSafe's System One models answer typed questions for tools; they are not chat models. It is
/// connected like a provider, but its models are never listed for agents (see [`serves_chat`]).
pub const TYPESAFE: &str = "typesafe";

/// ChatGPT's Codex: a ChatGPT plan's models, signed in to rather than given a key (see
/// [`crate::chatgpt`]).
pub const CODEX: &str = "codex";

/// Whether a provider's models can run an agent's chat, and so are listed, refreshed and picked.
pub fn serves_chat(id: &str) -> bool {
    id != TYPESAFE
}

const BUILT_IN: [(&str, &str, &str); 11] = [
    ("openai", "OpenAI", "#10A37F"),
    (CODEX, "OpenAI Codex", "#7C6CF6"),
    ("anthropic", "Anthropic", "#CC785C"),
    ("gemini", "Gemini", "#4285F4"),
    ("openrouter", "OpenRouter", "#6566F1"),
    ("nvidia", "NVIDIA", "#76B900"),
    ("deepseek", "DeepSeek", "#4D6BFE"),
    ("mistral", "Mistral", "#000000"),
    ("kimi", "Kimi", "#027AFF"),
    ("qwen", "Qwen", "#615CED"),
    (TYPESAFE, "TypeSafe", "#3B82F6"),
];

/// A saved model list is asked for again, when a picker opens, once it is this old (as in core).
const STALE_AFTER_SECS: i64 = 300;

/// Whether a model can answer: what the composer shows and whether it lets you send.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ModelStatus {
    Ready(ModelRef),
    /// No model chosen and no default: nothing to answer with yet.
    Choose,
    /// A model is chosen but its provider is gone or no longer lists it.
    Unavailable {
        model: ModelRef,
        why: String,
    },
}

pub const SERVER_COLOR: &str = "#48484A";

/// A provider's brand colour, from its id: built-ins have their own, servers share one.
pub fn color_of(id: &str, custom: bool) -> &'static str {
    if custom {
        return SERVER_COLOR;
    }
    BUILT_IN
        .iter()
        .find(|(b, _, _)| *b == id)
        .map_or(SERVER_COLOR, |&(_, _, color)| color)
}

fn built_in(&(id, label, color): &(&'static str, &str, &'static str)) -> Provider {
    Provider {
        id: id.into(),
        label: label.into(),
        color,
        connected: false,
        key_masked: None,
        base_url: None,
        custom: false,
        auth: ProviderAuth::ApiKey,
    }
}

pub fn init() -> Providers {
    let list: Vec<Provider> = BUILT_IN.iter().map(built_in).collect();
    Providers {
        list,
        models: HashMap::new(),
        default_model: None,
        default_picker: PickerState::default(),
        busy: HashSet::new(),
        row_errors: HashMap::new(),
        open: None,
        error: None,
        key: TextEdit::new(""),
        url: TextEdit::new(""),
        name: TextEdit::new(""),
        signing_in: None,
        code: TextEdit::new(""),
    }
}

impl Providers {
    /// The saved list with the models fetched for it and the default model.
    pub fn from_saved(
        mut list: Vec<Provider>,
        models: HashMap<String, ProviderModels>,
        default_model: Option<ModelRef>,
    ) -> Providers {
        // A built-in added since the list was saved is offered too, ahead of the user's servers.
        let at = list.iter().position(|p| p.custom).unwrap_or(list.len());
        let missing: Vec<Provider> = BUILT_IN
            .iter()
            .filter(|(id, ..)| !list.iter().any(|p| p.id == *id))
            .map(built_in)
            .collect();
        list.splice(at..at, missing);
        Providers {
            list,
            models,
            default_model,
            default_picker: PickerState::default(),
            busy: HashSet::new(),
            row_errors: HashMap::new(),
            open: None,
            error: None,
            key: TextEdit::new(""),
            url: TextEdit::new(""),
            name: TextEdit::new(""),
            signing_in: None,
            code: TextEdit::new(""),
        }
    }
}

/// Users paste whatever the endpoint page shows (often the full `/v1/chat/completions` URL).
/// Reduce it to the `/v1` root the backend appends `/models` and `/chat/completions` onto.
pub fn normalize_server_url(raw: &str) -> String {
    let mut url = raw.trim().trim_end_matches('/').to_owned();
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        url = format!("http://{url}");
    }
    for tail in ["/chat/completions", "/completions", "/models"] {
        if url.to_ascii_lowercase().ends_with(tail) {
            url.truncate(url.len() - tail.len());
            break;
        }
    }
    let has_version = url.rsplit('/').next().is_some_and(|seg| {
        seg.len() > 1
            && seg[..1].eq_ignore_ascii_case("v")
            && seg[1..].chars().all(|c| c.is_ascii_digit())
    });
    if !has_version {
        url.push_str("/v1");
    }
    url
}

fn mask(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() > 8 {
        let head: String = chars[..3].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}…{tail}")
    } else {
        "••••".into()
    }
}

fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

impl Providers {
    pub fn get(&self, id: &str) -> Option<&Provider> {
        self.list.iter().find(|p| p.id == id)
    }

    /// A provider's name, or its id if it is not (or no longer) in the list.
    pub fn label_of(&self, id: &str) -> String {
        self.get(id)
            .map_or_else(|| id.to_owned(), |p| p.label.clone())
    }

    /// The connected providers and the models they offer, for the picker. A provider with no
    /// models has no group.
    pub fn groups(&self) -> Vec<Group> {
        self.list
            .iter()
            .filter(|p| p.connected)
            .filter_map(|p| {
                let models = self.models.get(&p.id)?.models.clone();
                (!models.is_empty()).then(|| Group {
                    provider: p.id.clone(),
                    label: p.label.clone(),
                    models,
                })
            })
            .collect()
    }

    pub fn model_info(&self, m: &ModelRef) -> Option<&ModelInfo> {
        self.models
            .get(&m.provider)?
            .models
            .iter()
            .find(|i| i.id == m.id)
    }

    /// What to show for a model: its friendly name, else its id.
    pub fn model_name(&self, m: &ModelRef) -> String {
        self.model_info(m)
            .map_or_else(|| m.id.clone(), |i| i.name.clone())
    }

    /// Replaces a provider's models with a fresh list.
    pub fn set_models(&mut self, id: &str, models: Vec<ModelInfo>, fetched_at: i64) {
        self.models.insert(
            id.to_owned(),
            ProviderModels {
                fetched_at: Some(fetched_at),
                models,
            },
        );
    }

    /// Whether a connected provider's list is old enough to ask again. Offline, the saved one stays.
    pub fn is_stale(&self, id: &str, now: i64) -> bool {
        if !serves_chat(id) {
            return false;
        }
        match self.models.get(id).and_then(|m| m.fetched_at) {
            Some(at) => now - at > STALE_AFTER_SECS,
            None => true,
        }
    }

    /// Whether `chosen` (or, with none, the default model) can answer right now.
    pub fn status(&self, chosen: Option<&ModelRef>) -> ModelStatus {
        let Some(model) = chosen.or(self.default_model.as_ref()) else {
            return ModelStatus::Choose;
        };
        let why = match self.get(&model.provider) {
            Some(p) if p.connected => {
                if self.model_info(model).is_some() {
                    return ModelStatus::Ready(model.clone());
                }
                format!("{} no longer offers {}.", p.label, model.id)
            }
            _ => format!("{} is not connected.", self.label_of(&model.provider)),
        };
        ModelStatus::Unavailable {
            model: model.clone(),
            why,
        }
    }

    /// Opens or closes a provider's form. Only one is open at a time; its fields start clean,
    /// or filled from the server being edited.
    pub fn toggle(&mut self, id: &str) {
        self.error = None;
        if self.open.as_deref() == Some(id) {
            self.open = None;
            return;
        }
        let (name, url) = self
            .get(id)
            .map(|p| (p.label.clone(), p.base_url.clone().unwrap_or_default()))
            .unwrap_or_default();
        let custom = self.get(id).is_some_and(|p| p.custom);
        self.name = TextEdit::new(if custom { name } else { String::new() });
        self.url = TextEdit::new(if custom { url } else { String::new() });
        self.key = TextEdit::new("");
        self.signing_in = None;
        self.code = TextEdit::new("");
        self.open = Some(id.to_owned());
        // The first field takes focus, as Studio's `autoFocus` does.
        if id == NEW_SERVER || custom {
            self.name.focused = true;
        } else {
            self.key.focused = true;
        }
    }

    pub fn can_connect(&self) -> bool {
        !self.key.text.trim().is_empty()
    }

    pub fn can_save(&self, id: &str) -> bool {
        let (name, url) = (self.name.text.trim(), self.url.text.trim());
        if id == NEW_SERVER {
            !name.is_empty() && !url.is_empty()
        } else {
            !name.is_empty() || !url.is_empty() || !self.key.text.trim().is_empty()
        }
    }

    /// Connects a built-in provider with the key in the form.
    pub fn connect(&mut self, id: &str) {
        if let Some(p) = self.list.iter_mut().find(|p| p.id == id) {
            p.connected = true;
            p.auth = ProviderAuth::ApiKey;
        }
        self.open = None;
        self.signing_in = None;
        self.code = TextEdit::new("");
    }

    /// Shows a masked hint of the key that was just kept for `id`. Only called once the key is
    /// safely stored, so the hint never describes a key that is not there.
    pub fn set_hint(&mut self, id: &str, key: &str) {
        if let Some(p) = self.list.iter_mut().find(|p| p.id == id) {
            p.key_masked = Some(mask(key));
        }
    }

    /// Shows what a subscription is signed in as, in place of a masked key.
    pub fn set_subscription_hint(&mut self, id: &str) {
        if let Some(p) = self.list.iter_mut().find(|p| p.id == id) {
            p.auth = ProviderAuth::Subscription;
            let plan = if id == CODEX {
                "ChatGPT plan"
            } else {
                "Claude Pro/Max"
            };
            p.key_masked = Some(plan.to_owned());
        }
    }

    /// Reopens `id`'s form with `why`, keeping what was typed so it can be tried again.
    pub fn fail(&mut self, id: &str, why: String) {
        self.open = Some(id.to_owned());
        self.error = Some(why);
    }

    /// The id a server named `name` would get: its slug, made unique among the saved ones.
    pub fn new_server_id(&self, name: &str) -> String {
        let slug = slug(name.trim());
        let id = format!("llama-server:{slug}");
        if slug.is_empty() || self.get(&id).is_some() {
            format!("llama-server:{slug}-{}", self.list.len())
        } else {
            id
        }
    }

    /// Adds a server, or saves edits to one, and says which and whether it is new. A blank key
    /// keeps the current one. The key itself is the caller's to store.
    pub fn save_server(&mut self, id: &str) -> Option<(String, bool)> {
        if !self.can_save(id) {
            return None;
        }
        let (name, url) = (
            self.name.text.trim().to_owned(),
            self.url.text.trim().to_owned(),
        );
        let saved = if id == NEW_SERVER {
            let id = self.new_server_id(&name);
            self.list.push(Provider {
                id: id.clone(),
                label: name,
                color: SERVER_COLOR,
                connected: true,
                key_masked: None,
                base_url: Some(normalize_server_url(&url)),
                custom: true,
                auth: ProviderAuth::ApiKey,
            });
            (id, true)
        } else {
            let p = self.list.iter_mut().find(|p| p.id == id)?;
            if !name.is_empty() {
                p.label = name;
            }
            if !url.is_empty() {
                p.base_url = Some(normalize_server_url(&url));
            }
            (id.to_owned(), false)
        };
        self.open = None;
        Some(saved)
    }

    /// Signs a built-in provider out; a custom server is removed. The provider in use falls back
    /// to the first one still connected.
    pub fn disconnect(&mut self, id: &str) {
        if self.get(id).is_some_and(|p| p.custom) {
            self.list.retain(|p| p.id != id);
        } else if let Some(p) = self.list.iter_mut().find(|p| p.id == id) {
            p.connected = false;
            p.key_masked = None;
            p.auth = ProviderAuth::ApiKey;
        }
        self.models.remove(id);
        self.row_errors.remove(id);
        if self.signing_in.as_ref().is_some_and(|s| s.id == id) {
            self.signing_in = None;
            self.code = TextEdit::new("");
        }
        if self.open.as_deref() == Some(id) {
            self.open = None;
        }
    }

    pub fn connected(&self) -> Vec<&Provider> {
        self.list
            .iter()
            .filter(|p| p.connected && !p.custom)
            .collect()
    }

    pub fn servers(&self) -> Vec<&Provider> {
        self.list.iter().filter(|p| p.custom).collect()
    }

    pub fn available(&self) -> Vec<&Provider> {
        self.list
            .iter()
            .filter(|p| !p.connected && !p.custom)
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn urls_normalise_to_the_v1_root() {
        assert_eq!(
            normalize_server_url("localhost:8080"),
            "http://localhost:8080/v1"
        );
        assert_eq!(
            normalize_server_url("https://x.example/v1/chat/completions"),
            "https://x.example/v1"
        );
        assert_eq!(
            normalize_server_url(" https://x.example/v1/ "),
            "https://x.example/v1"
        );
        assert_eq!(
            normalize_server_url("https://x.example/v2/models"),
            "https://x.example/v2"
        );
        assert_eq!(
            normalize_server_url("https://x.example/openai/completions"),
            "https://x.example/openai/v1"
        );
    }

    #[test]
    fn nothing_is_connected_until_you_connect_it() {
        let mut p = init();
        assert!(p.connected().is_empty());
        p.connect("openai");
        assert!(p.get("openai").unwrap().connected);
        assert!(p.open.is_none());
    }

    #[test]
    fn the_hint_shows_the_ends_of_a_key_and_never_the_middle() {
        let mut p = init();
        p.set_hint("openai", "sk-live-1234567890");
        assert_eq!(
            p.get("openai").unwrap().key_masked.as_deref(),
            Some("sk-…7890")
        );
        p.set_hint("openai", "short");
        assert_eq!(p.get("openai").unwrap().key_masked.as_deref(), Some("••••"));
    }

    #[test]
    fn a_failure_reopens_the_form_and_closing_it_clears_the_message() {
        let mut p = init();
        p.key = TextEdit::new("typed");
        p.fail("openai", "no keyring".into());
        assert_eq!(p.open.as_deref(), Some("openai"));
        assert_eq!(p.error.as_deref(), Some("no keyring"));
        assert_eq!(
            p.key.text, "typed",
            "what was typed is kept for another try"
        );
        p.toggle("openai");
        assert!(p.open.is_none() && p.error.is_none());
    }

    #[test]
    fn disconnecting_forgets_the_providers_models_and_its_errors() {
        let mut p = init();
        p.connect("anthropic");
        p.set_models(
            "anthropic",
            vec![model("claude-opus-4-7", "Claude Opus 4.7")],
            100,
        );
        p.row_errors.insert("anthropic".into(), "x".into());
        p.disconnect("anthropic");
        assert!(!p.get("anthropic").unwrap().connected);
        assert!(!p.models.contains_key("anthropic") && p.row_errors.is_empty());
    }

    fn model(id: &str, name: &str) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            name: name.into(),
            caps: Default::default(),
            context: None,
        }
    }

    fn with_models() -> Providers {
        let mut p = init();
        p.connect("anthropic");
        p.set_models(
            "anthropic",
            vec![model("claude-opus-4-7", "Claude Opus 4.7")],
            1_000,
        );
        p.connect("openai");
        p.set_models("openai", vec![model("gpt-4o", "GPT-4o")], 1_000);
        p
    }

    #[test]
    fn a_built_in_added_after_the_list_was_saved_still_appears() {
        let mut saved = init().list;
        saved.retain(|p| p.id != TYPESAFE);
        let p = Providers::from_saved(saved, HashMap::new(), None);
        assert!(p.get(TYPESAFE).is_some_and(|t| !t.connected && !t.custom));
        assert_eq!(p.list.len(), BUILT_IN.len());
    }

    #[test]
    fn typesafe_is_a_provider_that_never_serves_chat_models() {
        let mut p = init();
        assert!(p.get(TYPESAFE).is_some_and(|t| !t.connected));
        p.connect(TYPESAFE);
        p.set_models(TYPESAFE, Vec::new(), 1_000);
        assert!(p.groups().is_empty(), "no picker group");
        assert!(
            !p.is_stale(TYPESAFE, 1_000_000),
            "never asked again for chat models"
        );
        assert!(p.is_stale("openai", 1_000_000));
    }

    #[test]
    fn only_connected_providers_with_models_make_picker_groups() {
        let mut p = with_models();
        p.connect("gemini");
        assert_eq!(
            p.groups()
                .iter()
                .map(|g| g.label.as_str())
                .collect::<Vec<_>>(),
            vec!["OpenAI", "Anthropic"],
            "no models, no group"
        );
        p.list
            .iter_mut()
            .find(|x| x.id == "openai")
            .unwrap()
            .connected = false;
        assert_eq!(p.groups().len(), 1);
    }

    #[test]
    fn a_model_is_ready_only_when_its_provider_is_connected_and_lists_it() {
        let mut p = with_models();
        let opus = ModelRef::new("anthropic", "claude-opus-4-7");
        assert_eq!(p.status(Some(&opus)), ModelStatus::Ready(opus.clone()));
        assert_eq!(
            p.status(None),
            ModelStatus::Choose,
            "no model and no default: choose one"
        );
        p.default_model = Some(ModelRef::new("openai", "gpt-4o"));
        assert_eq!(
            p.status(None),
            ModelStatus::Ready(ModelRef::new("openai", "gpt-4o")),
            "no model of its own uses the default"
        );

        let gone = ModelRef::new("anthropic", "claude-retired");
        assert!(
            matches!(p.status(Some(&gone)), ModelStatus::Unavailable { why, .. } if why == "Anthropic no longer offers claude-retired.")
        );
        p.disconnect("anthropic");
        assert!(
            matches!(p.status(Some(&opus)), ModelStatus::Unavailable { why, .. } if why == "Anthropic is not connected.")
        );
        assert!(
            matches!(p.status(Some(&ModelRef::new("ghost", "m"))), ModelStatus::Unavailable { why, .. } if why == "ghost is not connected.")
        );
    }

    #[test]
    fn a_list_is_stale_after_five_minutes_or_when_never_fetched() {
        let p = with_models();
        assert!(!p.is_stale("openai", 1_000 + 299));
        assert!(p.is_stale("openai", 1_000 + 301));
        assert!(p.is_stale("gemini", 1_000), "never fetched");
    }

    #[test]
    fn a_models_name_is_its_friendly_name_else_its_id() {
        let p = with_models();
        assert_eq!(p.model_name(&ModelRef::new("openai", "gpt-4o")), "GPT-4o");
        assert_eq!(
            p.model_name(&ModelRef::new("openai", "unlisted")),
            "unlisted"
        );
    }

    #[test]
    fn a_server_needs_a_name_and_url_and_disconnecting_removes_it() {
        let mut p = init();
        p.toggle(NEW_SERVER);
        p.name = TextEdit::new("My Box");
        assert!(p.save_server(NEW_SERVER).is_none());
        assert!(p.get("llama-server:my-box").is_none(), "no url yet");
        p.url = TextEdit::new("box.local:9000");
        assert_eq!(
            p.save_server(NEW_SERVER),
            Some(("llama-server:my-box".to_owned(), true))
        );
        let s = p.get("llama-server:my-box").unwrap();
        assert_eq!(s.base_url.as_deref(), Some("http://box.local:9000/v1"));
        p.disconnect("llama-server:my-box");
        assert!(p.get("llama-server:my-box").is_none());
    }

    fn waiting(id: &str, cancel: &Arc<AtomicBool>) -> SigningIn {
        SigningIn {
            id: id.to_owned(),
            sign: crate::credentials::SignIn {
                verifier: "v".to_owned(),
                state: "s".to_owned(),
                url: "https://auth.openai.com/oauth/authorize".to_owned(),
            },
            cancel: Some(Arc::clone(cancel)),
        }
    }

    #[test]
    fn letting_a_chatgpt_sign_in_go_ends_its_wait() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut p = init();
        p.signing_in = Some(waiting(CODEX, &cancel));
        p.toggle("openai");
        assert!(
            p.signing_in.is_none() && cancel.load(Ordering::Relaxed),
            "closing the form"
        );

        let cancel = Arc::new(AtomicBool::new(false));
        p.signing_in = Some(waiting(CODEX, &cancel));
        p.disconnect(CODEX);
        assert!(cancel.load(Ordering::Relaxed), "signing the provider out");
    }

    #[test]
    fn codex_is_offered_to_everyone_and_signed_in_as_a_plan() {
        let mut p = init();
        assert!(
            p.get(CODEX)
                .is_some_and(|c| !c.connected && c.label == "OpenAI Codex")
        );
        p.set_subscription_hint(CODEX);
        assert_eq!(
            p.get(CODEX).unwrap().key_masked.as_deref(),
            Some("ChatGPT plan")
        );

        // A list saved before Codex existed still gets it, ahead of the user's own servers.
        let mut saved: Vec<Provider> = init().list.into_iter().filter(|p| p.id != CODEX).collect();
        saved.push(Provider {
            id: "llama-server:box".into(),
            label: "Box".into(),
            color: SERVER_COLOR,
            connected: true,
            key_masked: None,
            base_url: Some("http://box/v1".into()),
            custom: true,
            auth: ProviderAuth::ApiKey,
        });
        let p = Providers::from_saved(saved, HashMap::new(), None);
        let ids: Vec<&str> = p.list.iter().map(|p| p.id.as_str()).collect();
        let (codex, server) = (
            ids.iter().position(|i| *i == CODEX).unwrap(),
            ids.iter().position(|i| *i == "llama-server:box").unwrap(),
        );
        assert!(codex < server, "{ids:?}");
    }
}
