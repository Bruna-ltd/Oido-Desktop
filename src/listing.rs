//! Asking a provider which models a key can use: where to ask, how, and how to read the answer.
//! This is also how a key is checked when it is connected: a key that cannot list models is not
//! a working key.

use std::time::Duration;

use serde_json::Value;

use crate::models::{Caps, ModelInfo, RawModel, finish};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    /// `GET {base}/models` with a Bearer key: OpenAI and everything that copies it, and servers
    /// such as llama.cpp.
    OpenAi,
    Anthropic,
    Gemini,
    /// `GET {base}/models` with a Bearer key, answering `{"models":[{"name":..}]}`. Those are
    /// System One models, not chat models: asking only proves the key works.
    TypeSafe,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Endpoint {
    pub wire: Wire,
    /// Without a trailing slash.
    pub base: String,
}

const ANTHROPIC_VERSION: &str = "2023-06-01";
/// A list is read page by page, never more than this many, so a misbehaving server cannot loop us.
const MAX_PAGES: usize = 10;
const TIMEOUT: Duration = Duration::from_secs(20);

/// Where a provider's models are listed. `None` for a provider we do not know and a server with no URL.
pub fn endpoint(id: &str, custom: bool, base_url: Option<&str>) -> Option<Endpoint> {
    if custom {
        return base_url.map(|b| Endpoint {
            wire: Wire::OpenAi,
            base: b.trim_end_matches('/').to_owned(),
        });
    }
    let (wire, base) = match id {
        "openai" => (Wire::OpenAi, "https://api.openai.com/v1"),
        "openrouter" => (Wire::OpenAi, "https://openrouter.ai/api/v1"),
        "nvidia" => (Wire::OpenAi, "https://integrate.api.nvidia.com/v1"),
        "deepseek" => (Wire::OpenAi, "https://api.deepseek.com/v1"),
        "qwen" => (
            Wire::OpenAi,
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        ),
        "kimi" => (Wire::OpenAi, "https://api.moonshot.ai/v1"),
        "mistral" => (Wire::OpenAi, "https://api.mistral.ai/v1"),
        "anthropic" => (Wire::Anthropic, "https://api.anthropic.com/v1"),
        "typesafe" => (Wire::TypeSafe, "https://api.typesafe.ai/v1"),
        "gemini" => (
            Wire::Gemini,
            "https://generativelanguage.googleapis.com/v1beta",
        ),
        _ => return None,
    };
    Some(Endpoint {
        wire,
        base: base.to_owned(),
    })
}

/// Why a list could not be had, in terms a person can act on.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The provider answered that the key is not accepted.
    Rejected,
    /// No answer: offline, a wrong address, a server that is down, or too slow.
    Unreachable,
    /// An answer we cannot use.
    Unexpected(String),
}

impl Error {
    /// A sentence for the form, naming the provider (`who`).
    pub fn describe(&self, who: &str) -> String {
        match self {
            Self::Rejected => format!("{who} did not accept the key. Check it and try again."),
            Self::Unreachable => format!(
                "Could not reach {who} to check the key. Check your connection and try again."
            ),
            Self::Unexpected(what) => {
                format!(
                    "{who} replied in a way that was not understood ({what}). If this is your own server, check the address ends in /v1."
                )
            }
        }
    }
}

/// The models `key` can use at `ep`, ready for the picker: chat models only, named, and sorted.
/// Blocks on the network, so call it from a worker thread.
pub fn fetch(ep: &Endpoint, key: Option<&str>) -> Result<Vec<ModelInfo>, Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut raws = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let req = page_request(ep, key, cursor.as_deref());
        let body = get(&agent, ep.wire, &req)?;
        let (page, next) = parse_page(ep.wire, &body)?;
        raws.extend(page);
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(finish_all(raws))
}

/// Applies the catalog and the chat filter, drops repeats, and orders by name.
pub fn finish_all(raws: Vec<RawModel>) -> Vec<ModelInfo> {
    let mut models: Vec<ModelInfo> = raws.into_iter().filter_map(finish).collect();
    models.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    models.dedup_by(|a, b| a.id == b.id);
    models
}

pub struct Request {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
}

/// One page's request. `cursor` is where the last page left off.
fn page_request(ep: &Endpoint, key: Option<&str>, cursor: Option<&str>) -> Request {
    let mut headers = Vec::new();
    let url = match ep.wire {
        Wire::OpenAi | Wire::TypeSafe => {
            if let Some(k) = key.filter(|k| !k.is_empty()) {
                headers.push(("Authorization", format!("Bearer {k}")));
            }
            format!("{}/models", ep.base)
        }
        Wire::Anthropic => {
            if let Some(k) = key {
                headers.push(("x-api-key", k.to_owned()));
            }
            headers.push(("anthropic-version", ANTHROPIC_VERSION.to_owned()));
            match cursor {
                Some(c) => format!("{}/models?limit=1000&after_id={}", ep.base, encode(c)),
                None => format!("{}/models?limit=1000", ep.base),
            }
        }
        Wire::Gemini => {
            // The key goes in a header, not the URL, so it never lands in a log of addresses.
            if let Some(k) = key {
                headers.push(("x-goog-api-key", k.to_owned()));
            }
            match cursor {
                Some(c) => format!("{}/models?pageSize=1000&pageToken={}", ep.base, encode(c)),
                None => format!("{}/models?pageSize=1000", ep.base),
            }
        }
    };
    Request { url, headers }
}

fn get(agent: &ureq::Agent, wire: Wire, req: &Request) -> Result<String, Error> {
    let mut call = agent.get(&req.url);
    for (name, value) in &req.headers {
        call = call.header(*name, value.as_str());
    }
    let mut resp = call.call().map_err(|_| Error::Unreachable)?;
    let status = resp.status().as_u16();
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|_| Error::Unreachable);
    match status {
        200..=299 => body,
        401 | 403 => Err(Error::Rejected),
        // Google answers a bad key with 400 and a reason in the body, not 401.
        400 if wire == Wire::Gemini
            && body.as_deref().is_ok_and(|b| b.contains("API_KEY_INVALID")) =>
        {
            Err(Error::Rejected)
        }
        other => Err(Error::Unexpected(format!("HTTP {other}"))),
    }
}

/// One page of a list, and the cursor for the next, if there is one.
fn parse_page(wire: Wire, body: &str) -> Result<(Vec<RawModel>, Option<String>), Error> {
    let v: Value = serde_json::from_str(body).map_err(|_| Error::Unexpected("not JSON".into()))?;
    let list = |key: &str| {
        v.get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Unexpected(format!("no \"{key}\" list")))
    };
    match wire {
        Wire::OpenAi => Ok((
            list("data")?.iter().filter_map(openai_model).collect(),
            None,
        )),
        Wire::TypeSafe => {
            // The list must be there for the key to count as checked, but it is not for agents.
            list("models")?;
            Ok((Vec::new(), None))
        }
        Wire::Anthropic => {
            let models = list("data")?
                .iter()
                .filter_map(|m| {
                    Some(RawModel {
                        id: str_of(m, "id")?,
                        name: str_of(m, "display_name"),
                        ..RawModel::default()
                    })
                })
                .collect();
            let more = v.get("has_more").and_then(Value::as_bool).unwrap_or(false);
            let next = more.then(|| str_of(&v, "last_id")).flatten();
            Ok((models, next))
        }
        Wire::Gemini => {
            let models = list("models")?
                .iter()
                .filter_map(|m| {
                    let id = str_of(m, "name")?;
                    let chat = m
                        .get("supportedGenerationMethods")
                        .and_then(Value::as_array)
                        .map(|methods| {
                            methods
                                .iter()
                                .any(|x| x.as_str() == Some("generateContent"))
                        });
                    Some(RawModel {
                        id: id.strip_prefix("models/").unwrap_or(&id).to_owned(),
                        name: str_of(m, "displayName"),
                        chat,
                        ..RawModel::default()
                    })
                })
                .collect();
            let next = str_of(&v, "nextPageToken").filter(|t| !t.is_empty());
            Ok((models, next))
        }
    }
}

/// OpenAI's entry is just an id. OpenRouter's adds a name and what each model takes in and
/// supports, which is declared, so it is believed.
fn openai_model(m: &Value) -> Option<RawModel> {
    let strings = |v: Option<&Value>| -> Option<Vec<String>> {
        v.and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
    };
    let inputs = strings(
        m.get("architecture")
            .and_then(|a| a.get("input_modalities")),
    );
    let params = strings(m.get("supported_parameters"));
    let caps = Caps {
        vision: inputs.map(|i| i.iter().any(|x| x == "image")),
        reasoning: params.as_ref().map(|p| {
            p.iter()
                .any(|x| x == "reasoning" || x == "include_reasoning")
        }),
        tools: params.map(|p| p.iter().any(|x| x == "tools")),
    };
    Some(RawModel {
        id: str_of(m, "id")?,
        name: str_of(m, "name"),
        caps,
        chat: None,
    })
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Percent-encodes everything but the unreserved characters, for a cursor in a query string.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::net::TcpListener;

    use super::*;
    use crate::testing::serve;

    const OPENAI: &str = r#"{"object":"list","data":[
        {"id":"gpt-4o","object":"model","owned_by":"openai"},
        {"id":"text-embedding-3-large","object":"model"},
        {"id":"whisper-1","object":"model"},
        {"id":"gpt-4o-mini","object":"model"}]}"#;

    const OPENROUTER: &str = r#"{"data":[
        {"id":"anthropic/claude-opus-4-7","name":"Anthropic: Claude Opus 4.7",
         "architecture":{"input_modalities":["text","image"]},
         "supported_parameters":["tools","reasoning","temperature"]},
        {"id":"meta/llama-3","name":"Llama 3","architecture":{"input_modalities":["text"]},
         "supported_parameters":["temperature"]}]}"#;

    fn at(wire: Wire, base: &str) -> Endpoint {
        Endpoint {
            wire,
            base: base.to_owned(),
        }
    }

    #[test]
    fn typesafe_is_checked_with_its_key_and_lists_no_chat_models() {
        let (base, seen) = serve(vec![(
            200,
            r#"{"models":[{"name":"jev-latest","description":"d","release_date":"2026-09-15"}]}"#,
        )]);
        let ep = at(Wire::TypeSafe, &format!("{base}/v1"));
        assert_eq!(fetch(&ep, Some("ts-key")), Ok(Vec::new()));
        let request = seen.recv().unwrap();
        assert!(request.starts_with("get /v1/models "), "{request}");
        assert!(
            request.contains("authorization: bearer ts-key"),
            "{request}"
        );

        let (base, _) = serve(vec![(401, "{}")]);
        assert_eq!(
            fetch(&at(Wire::TypeSafe, &format!("{base}/v1")), Some("bad")),
            Err(Error::Rejected)
        );
        assert_eq!(
            endpoint("typesafe", false, None).unwrap().wire,
            Wire::TypeSafe
        );
    }

    #[test]
    fn built_in_providers_have_an_address_and_a_server_uses_its_own() {
        assert_eq!(
            endpoint("openai", false, None).unwrap().base,
            "https://api.openai.com/v1"
        );
        assert_eq!(
            endpoint("anthropic", false, None).unwrap().wire,
            Wire::Anthropic
        );
        assert_eq!(endpoint("gemini", false, None).unwrap().wire, Wire::Gemini);
        assert_eq!(
            endpoint("llama-server:mine", true, Some("http://box.local:9000/v1/"))
                .unwrap()
                .base,
            "http://box.local:9000/v1"
        );
        assert!(
            endpoint("llama-server:x", true, None).is_none(),
            "a server with no address cannot be asked"
        );
        assert!(endpoint("nobody", false, None).is_none());
    }

    #[test]
    fn requests_carry_the_key_the_way_each_provider_wants_it() {
        let r = page_request(&at(Wire::OpenAi, "http://x/v1"), Some("sk-1"), None);
        assert_eq!(r.url, "http://x/v1/models");
        assert_eq!(r.headers, vec![("Authorization", "Bearer sk-1".to_owned())]);
        assert!(
            page_request(&at(Wire::OpenAi, "http://x/v1"), None, None)
                .headers
                .is_empty(),
            "a server with no key sends none"
        );

        let r = page_request(
            &at(Wire::Anthropic, "http://x/v1"),
            Some("sk-a"),
            Some("model_9"),
        );
        assert_eq!(r.url, "http://x/v1/models?limit=1000&after_id=model_9");
        assert!(
            r.headers.contains(&("x-api-key", "sk-a".to_owned()))
                && r.headers.iter().any(|(n, _)| *n == "anthropic-version")
        );

        let r = page_request(
            &at(Wire::Gemini, "http://x/v1beta"),
            Some("g-1"),
            Some("a+b/c="),
        );
        assert_eq!(
            r.url,
            "http://x/v1beta/models?pageSize=1000&pageToken=a%2Bb%2Fc%3D"
        );
        assert!(!r.url.contains("g-1"), "the key is never in the address");
        assert_eq!(r.headers, vec![("x-goog-api-key", "g-1".to_owned())]);
    }

    #[test]
    fn an_openai_style_list_is_filtered_named_and_sorted() {
        let (page, next) = parse_page(Wire::OpenAi, OPENAI).unwrap();
        assert!(next.is_none());
        let models = finish_all(page);
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["gpt-4o", "gpt-4o-mini"],
            "embeddings and speech are gone"
        );
        assert_eq!(models[0].name, "GPT-4o", "the catalog supplies the name");
    }

    #[test]
    fn what_openrouter_declares_is_believed() {
        let models = finish_all(parse_page(Wire::OpenAi, OPENROUTER).unwrap().0);
        let opus = models
            .iter()
            .find(|m| m.id == "anthropic/claude-opus-4-7")
            .unwrap();
        assert_eq!(opus.name, "Anthropic: Claude Opus 4.7");
        assert_eq!(
            opus.caps,
            Caps {
                vision: Some(true),
                reasoning: Some(true),
                tools: Some(true)
            }
        );
        let llama = models.iter().find(|m| m.id == "meta/llama-3").unwrap();
        assert_eq!(
            llama.caps,
            Caps {
                vision: Some(false),
                reasoning: Some(false),
                tools: Some(false)
            }
        );
    }

    #[test]
    fn anthropic_pages_and_gemini_marks_which_models_chat() {
        let (page, next) =
            parse_page(Wire::Anthropic, r#"{"data":[{"id":"claude-x","display_name":"Claude X","type":"model"}],"has_more":true,"last_id":"claude-x"}"#).unwrap();
        assert_eq!(
            (page[0].name.as_deref(), next.as_deref()),
            (Some("Claude X"), Some("claude-x"))
        );
        let (_, last) = parse_page(
            Wire::Anthropic,
            r#"{"data":[],"has_more":false,"last_id":"z"}"#,
        )
        .unwrap();
        assert!(last.is_none(), "has_more false ends the list");

        let body = r#"{"models":[
            {"name":"models/gemini-2.5-pro","displayName":"Gemini 2.5 Pro","supportedGenerationMethods":["generateContent","countTokens"]},
            {"name":"models/text-embedding-004","displayName":"Embedding","supportedGenerationMethods":["embedContent"]}],
            "nextPageToken":"tok"}"#;
        let (page, next) = parse_page(Wire::Gemini, body).unwrap();
        assert_eq!(next.as_deref(), Some("tok"));
        let models = finish_all(page);
        assert_eq!(models.len(), 1);
        assert_eq!(
            (models[0].id.as_str(), models[0].name.as_str()),
            ("gemini-2.5-pro", "Gemini 2.5 Pro")
        );
    }

    #[test]
    fn a_reply_that_is_not_a_model_list_is_unexpected_not_a_panic() {
        assert!(matches!(
            parse_page(Wire::OpenAi, "<html>hi</html>"),
            Err(Error::Unexpected(_))
        ));
        assert!(matches!(
            parse_page(Wire::OpenAi, r#"{"error":"no"}"#),
            Err(Error::Unexpected(_))
        ));
        assert!(matches!(
            parse_page(Wire::Gemini, r#"{"data":[]}"#),
            Err(Error::Unexpected(_))
        ));
    }

    #[test]
    fn fetching_sends_the_key_and_returns_the_models() {
        let (base, seen) = serve(vec![(200, OPENAI)]);
        let models = fetch(
            &at(Wire::OpenAi, &format!("{base}/v1")),
            Some("sk-test-123"),
        )
        .unwrap();
        assert_eq!(models.len(), 2);
        let request = seen.recv().unwrap();
        assert!(request.starts_with("get /v1/models "), "{request}");
        assert!(request.contains("authorization: bearer sk-test-123"));
    }

    #[test]
    fn fetching_follows_anthropics_pages_with_the_cursor() {
        let (base, seen) = serve(vec![
            (
                200,
                r#"{"data":[{"id":"claude-a","display_name":"A"}],"has_more":true,"last_id":"claude-a"}"#,
            ),
            (
                200,
                r#"{"data":[{"id":"claude-b","display_name":"B"}],"has_more":false,"last_id":"claude-b"}"#,
            ),
        ]);
        let models = fetch(&at(Wire::Anthropic, &base), Some("sk-a")).unwrap();
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["claude-a", "claude-b"]
        );
        let first = seen.recv().unwrap();
        let second = seen.recv().unwrap();
        assert!(
            first.contains("x-api-key: sk-a") && first.contains("anthropic-version: 2023-06-01")
        );
        assert!(second.contains("after_id=claude-a"));
    }

    #[test]
    fn a_rejected_key_an_unreachable_server_and_a_broken_one_are_told_apart() {
        let (base, _) = serve(vec![(401, r#"{"error":"bad key"}"#)]);
        assert_eq!(
            fetch(&at(Wire::OpenAi, &base), Some("bad")),
            Err(Error::Rejected)
        );

        let (base, _) = serve(vec![(
            400,
            r#"{"error":{"status":"INVALID_ARGUMENT","details":[{"reason":"API_KEY_INVALID"}]}}"#,
        )]);
        assert_eq!(
            fetch(&at(Wire::Gemini, &base), Some("bad")),
            Err(Error::Rejected),
            "google says 400, not 401"
        );

        let (base, _) = serve(vec![(500, "oops")]);
        assert_eq!(
            fetch(&at(Wire::OpenAi, &base), None),
            Err(Error::Unexpected("HTTP 500".into()))
        );

        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        assert_eq!(
            fetch(&at(Wire::OpenAi, &closed), None),
            Err(Error::Unreachable)
        );
    }

    #[test]
    fn the_messages_name_the_provider_and_say_what_to_do() {
        assert!(
            Error::Rejected
                .describe("OpenAI")
                .contains("OpenAI did not accept the key")
        );
        assert!(
            Error::Unreachable
                .describe("Anthropic")
                .contains("Could not reach Anthropic")
        );
        let m = Error::Unexpected("HTTP 404".into()).describe("My server");
        assert!(m.contains("HTTP 404") && m.contains("/v1"));
    }
}
