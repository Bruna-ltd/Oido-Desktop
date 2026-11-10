//! The models a connected provider offers: what a list says about each, what we know that the list
//! doesn't (names, abilities), and which ones an agent can chat with at all.

use serde::{Deserialize, Serialize};

/// A model is a provider and an id together: the same id can come from OpenRouter and NVIDIA.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider: String,
    pub id: String,
}

impl ModelRef {
    pub fn new(provider: impl Into<String>, id: impl Into<String>) -> ModelRef {
        ModelRef {
            provider: provider.into(),
            id: id.into(),
        }
    }
}

/// What a model can do. `None` is unknown, never "no": a list that says nothing about vision
/// leaves it unknown, and only a source that actually declared it sets `Some(false)`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Caps {
    pub vision: Option<bool>,
    pub reasoning: Option<bool>,
    pub tools: Option<bool>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ModelInfo {
    pub id: String,
    /// What the picker shows: the provider's own name, else the catalog's, else the id.
    pub name: String,
    pub caps: Caps,
    /// How many tokens it takes at once, as its provider declared; `None` when it did not say.
    pub context: Option<u32>,
}

/// One entry of a provider's list as parsed, before the catalog and the chat filter.
#[derive(Clone, Default, Debug)]
pub struct RawModel {
    pub id: String,
    pub name: Option<String>,
    /// What the list itself declared.
    pub caps: Caps,
    /// Whether the list says the model can chat: `Some` when it states so (Gemini's methods).
    pub chat: Option<bool>,
    /// The context limit the list declared.
    pub context: Option<u32>,
}

/// Completes a listed model, or drops it if it is not one an agent can chat with.
pub fn finish(raw: RawModel) -> Option<ModelInfo> {
    match raw.chat {
        Some(false) => return None,
        Some(true) => {}
        None if !is_chat_model(&raw.id) => return None,
        None => {}
    }
    let known = lookup(&raw.id);
    let name = raw
        .name
        .filter(|n| !n.trim().is_empty())
        .or_else(|| known.map(|k| k.name.to_owned()))
        .unwrap_or_else(|| raw.id.clone());
    // The catalog is hand-declared, so it answers all three; what the list declared wins over it.
    let from_catalog = known.map(|k| Caps {
        vision: Some(k.vision),
        reasoning: Some(k.reasoning),
        tools: Some(k.tools),
    });
    let c = from_catalog.unwrap_or_default();
    let caps = Caps {
        vision: raw.caps.vision.or(c.vision),
        reasoning: raw.caps.reasoning.or(c.reasoning),
        tools: raw.caps.tools.or(c.tools),
    };
    Some(ModelInfo {
        id: raw.id,
        name,
        caps,
        context: raw.context,
    })
}

/// Embeddings, speech, image and moderation models also come back from `/models`. An id we cannot
/// place is kept: hiding a real model is worse than showing an odd one.
pub fn is_chat_model(id: &str) -> bool {
    const NOT_CHAT: [&str; 12] = [
        "embed",
        "whisper",
        "tts",
        "dall-e",
        "moderation",
        "transcribe",
        "rerank",
        "imagen",
        "-image",
        "speech",
        "stable-diffusion",
        "sdxl",
    ];
    let id = id.to_lowercase();
    !NOT_CHAT.iter().any(|p| id.contains(p))
}

#[derive(Clone, Copy)]
struct Known {
    name: &'static str,
    vision: bool,
    reasoning: bool,
    tools: bool,
}

/// Names and abilities for models whose lists don't say. Ported from `oido-core`'s catalog.
/// A provider's own names and declarations win over these.
const CATALOG: [(&str, &str, bool, bool, bool); 25] = [
    ("gemini-2.5-flash", "Gemini 2.5 Flash", false, false, true),
    (
        "gemini-2.5-flash-lite",
        "Gemini 2.5 Flash Lite",
        false,
        false,
        true,
    ),
    ("gemini-2.5-pro", "Gemini 2.5 Pro", false, true, true),
    (
        "gemini-3-flash-preview",
        "Gemini 3 Flash (Preview)",
        false,
        false,
        true,
    ),
    (
        "gemini-3-pro-preview",
        "Gemini 3 Pro (Preview)",
        false,
        true,
        true,
    ),
    (
        "gemini-3.1-flash-lite-preview",
        "Gemini 3.1 Flash Lite (Preview)",
        false,
        false,
        true,
    ),
    (
        "gemini-3.1-pro-preview",
        "Gemini 3.1 Pro (Preview)",
        false,
        true,
        true,
    ),
    ("gpt-4o", "GPT-4o", true, false, true),
    ("gpt-4o-mini", "GPT-4o Mini", true, false, true),
    ("o3", "o3", false, false, true),
    (
        "deepseek-ai/deepseek-v4.1-flash",
        "Deepseek V4.1 Flash",
        true,
        false,
        true,
    ),
    ("z-ai/glm-5.3", "GLM 5.3", true, false, true),
    ("moonshotai/kimi-k2.6", "Kimi K2.6", true, false, true),
    ("deepseek-v4-pro", "DeepSeek V4 Pro", false, false, true),
    ("deepseek-v4-flash", "DeepSeek V4 Flash", false, false, true),
    ("deepseek-v3.2", "DeepSeek V3.2", false, false, true),
    ("deepseek-v3.1", "DeepSeek V3.1", false, false, true),
    ("deepseek-v3", "DeepSeek V3", false, false, true),
    (
        "deepseek-reasoner",
        "DeepSeek Reasoner (R1)",
        false,
        true,
        true,
    ),
    (
        "deepseek-reasoner-0528",
        "DeepSeek Reasoner R1 (0528)",
        false,
        true,
        true,
    ),
    ("claude-opus-4-7", "Claude Opus 4.7", true, true, true),
    ("claude-sonnet-4-6", "Claude Sonnet 4.6", true, true, true),
    (
        "claude-haiku-4-5-20251001",
        "Claude Haiku 4.5",
        true,
        false,
        true,
    ),
    (
        "claude-3-5-sonnet-20241022",
        "Claude 3.5 Sonnet",
        true,
        false,
        true,
    ),
    (
        "claude-3-5-haiku-20241022",
        "Claude 3.5 Haiku",
        true,
        false,
        true,
    ),
];

/// The catalog entry for an id, also when an aggregator prefixes it ("anthropic/claude-opus-4-7").
fn lookup(id: &str) -> Option<Known> {
    let find = |wanted: &str| {
        CATALOG.iter().find(|(cid, ..)| *cid == wanted).map(
            |&(_, name, vision, reasoning, tools)| Known {
                name,
                vision,
                reasoning,
                tools,
            },
        )
    };
    find(id).or_else(|| id.rsplit_once('/').and_then(|(_, tail)| find(tail)))
}

/// Whether a model reasons: `Some(false)` only when something declared it does not.
pub fn can_think(caps: &Caps) -> bool {
    caps.reasoning != Some(false)
}

/// Provider's models as saved, and when they were fetched (seconds since the epoch).
#[derive(Clone, Default, Debug)]
pub struct ProviderModels {
    pub fetched_at: Option<i64>,
    pub models: Vec<ModelInfo>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn raw(id: &str) -> RawModel {
        RawModel {
            id: id.into(),
            ..RawModel::default()
        }
    }

    #[test]
    fn non_chat_models_are_dropped_and_unplaceable_ones_kept() {
        for id in [
            "text-embedding-3-large",
            "whisper-1",
            "gpt-4o-mini-tts",
            "dall-e-3",
            "omni-moderation-latest",
            "gpt-image-1",
            "rerank-v3",
            "imagen-4",
        ] {
            assert!(finish(raw(id)).is_none(), "{id} is not a chat model");
        }
        for id in [
            "gpt-4o",
            "claude-opus-4-7",
            "my-finetune-v2",
            "llama-3.3-70b-instruct",
        ] {
            assert!(finish(raw(id)).is_some(), "{id} is kept");
        }
    }

    #[test]
    fn a_list_that_states_whether_a_model_chats_is_believed_over_the_name() {
        let yes = RawModel {
            id: "weird-embedding-chat".into(),
            chat: Some(true),
            ..RawModel::default()
        };
        assert!(finish(yes).is_some());
        let no = RawModel {
            id: "gemini-9".into(),
            chat: Some(false),
            ..RawModel::default()
        };
        assert!(finish(no).is_none());
    }

    #[test]
    fn the_catalog_names_and_describes_known_models() {
        let m = finish(raw("claude-opus-4-7")).unwrap();
        assert_eq!(m.name, "Claude Opus 4.7");
        assert_eq!(
            m.caps,
            Caps {
                vision: Some(true),
                reasoning: Some(true),
                tools: Some(true)
            }
        );
        let m = finish(raw("gpt-4o-mini")).unwrap();
        assert_eq!(
            m.caps.reasoning,
            Some(false),
            "the catalog declares what a model cannot do too"
        );
    }

    #[test]
    fn an_aggregators_prefix_still_finds_the_catalog_entry() {
        let m = finish(raw("anthropic/claude-sonnet-4-6")).unwrap();
        assert_eq!(m.name, "Claude Sonnet 4.6");
    }

    #[test]
    fn an_unknown_model_shows_its_id_and_its_abilities_are_unknown_not_no() {
        let m = finish(raw("some-new-model")).unwrap();
        assert_eq!(m.name, "some-new-model");
        assert_eq!(m.caps, Caps::default());
        assert!(can_think(&m.caps), "unknown is not 'cannot think'");
        assert!(!can_think(&Caps {
            reasoning: Some(false),
            ..Caps::default()
        }));
    }

    #[test]
    fn the_providers_own_name_and_declarations_win_over_the_catalog() {
        let m = finish(RawModel {
            id: "gpt-4o".into(),
            name: Some("GPT-4o (custom)".into()),
            caps: Caps {
                vision: Some(false),
                ..Caps::default()
            },
            ..RawModel::default()
        })
        .unwrap();
        assert_eq!(m.name, "GPT-4o (custom)");
        assert_eq!(m.caps.vision, Some(false));
        assert_eq!(
            m.caps.tools,
            Some(true),
            "what it did not declare comes from the catalog"
        );
    }
}
