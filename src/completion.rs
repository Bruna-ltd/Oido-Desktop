//! Asking a model for a reply, streamed: where to ask, how each provider family wants the request
//! (and how it spells "think harder"), and how to read the stream back. Gemini is asked through
//! its OpenAI-compatible endpoint, as `oido-core` does, so there are two formats: OpenAI's, which
//! most providers copy, and Anthropic's.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Api {
    /// `POST {base}/chat/completions`, server-sent events of `choices[0].delta`.
    OpenAi,
    /// `POST {base}/messages`, server-sent events of typed blocks.
    Anthropic,
}

/// How a provider takes "think harder", from `oido-core`'s provider profiles.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Thinking {
    /// No setting: it thinks as it thinks, or not at all.
    None,
    /// `"reasoning_effort": "low"` (OpenAI, and Gemini's compatible endpoint).
    Effort,
    /// `"thinking": {"type": "enabled", "budget_tokens": N}` (Anthropic, DeepSeek).
    Budget,
    /// `"reasoning": {"effort": "low"}` (OpenRouter).
    OpenRouter,
    /// `"enable_thinking": true, "thinking_budget": N` (Qwen).
    Qwen,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Endpoint {
    pub api: Api,
    /// Without a trailing slash.
    pub base: String,
    pub thinking: Thinking,
}

/// Where a provider's chat is, and how it takes its settings. `None` for a provider we do not
/// know and a server with no address.
pub fn endpoint(id: &str, custom: bool, base_url: Option<&str>) -> Option<Endpoint> {
    let make = |api, base: &str, thinking| {
        Some(Endpoint {
            api,
            base: base.trim_end_matches('/').to_owned(),
            thinking,
        })
    };
    if custom {
        return base_url.and_then(|b| make(Api::OpenAi, b, Thinking::None));
    }
    match id {
        "openai" => make(Api::OpenAi, "https://api.openai.com/v1", Thinking::Effort),
        "openrouter" => make(
            Api::OpenAi,
            "https://openrouter.ai/api/v1",
            Thinking::OpenRouter,
        ),
        "nvidia" => make(
            Api::OpenAi,
            "https://integrate.api.nvidia.com/v1",
            Thinking::None,
        ),
        "deepseek" => make(Api::OpenAi, "https://api.deepseek.com/v1", Thinking::Budget),
        "qwen" => make(
            Api::OpenAi,
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
            Thinking::Qwen,
        ),
        "kimi" => make(Api::OpenAi, "https://api.moonshot.ai/v1", Thinking::None),
        "mistral" => make(Api::OpenAi, "https://api.mistral.ai/v1", Thinking::None),
        "gemini" => make(
            Api::OpenAi,
            "https://generativelanguage.googleapis.com/v1beta/openai",
            Thinking::Effort,
        ),
        "anthropic" => make(
            Api::Anthropic,
            "https://api.anthropic.com/v1",
            Thinking::Budget,
        ),
        _ => None,
    }
}

/// How hard to think, when it is not off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Think {
    Low,
    Medium,
    High,
}

impl Think {
    fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Tokens the model may spend thinking, for providers that take a budget.
    fn budget(self) -> u32 {
        match self {
            Self::Low => 1024,
            Self::Medium => 4096,
            Self::High => 16384,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    User,
    Assistant,
    /// The result of a tool the assistant called, sent back to it.
    Tool,
}

/// A tool the model asked to use.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Call {
    pub id: String,
    pub name: String,
    /// The arguments, as the JSON text the model wrote.
    pub args: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Turn {
    pub role: Role,
    pub text: String,
    /// What an assistant turn asked for.
    pub calls: Vec<Call>,
    /// For a tool turn, the call it answers.
    pub call_id: Option<String>,
}

impl Turn {
    pub fn user(text: impl Into<String>) -> Turn {
        Turn {
            role: Role::User,
            text: text.into(),
            calls: Vec::new(),
            call_id: None,
        }
    }

    pub fn assistant(text: impl Into<String>, calls: Vec<Call>) -> Turn {
        Turn {
            role: Role::Assistant,
            text: text.into(),
            calls,
            call_id: None,
        }
    }

    pub fn tool(call_id: impl Into<String>, result: impl Into<String>) -> Turn {
        Turn {
            role: Role::Tool,
            text: result.into(),
            calls: Vec::new(),
            call_id: Some(call_id.into()),
        }
    }
}

/// A tool offered to the model.
#[derive(Clone, PartialEq, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments.
    pub schema: Value,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Request {
    pub model: String,
    pub system: Option<String>,
    /// What changes with every message (the date and time). Kept apart from `system` so that
    /// the part a provider caches stays the same.
    pub context: Option<String>,
    /// Starting with the user; after a tool call, ending with its result.
    pub turns: Vec<Turn>,
    pub think: Option<Think>,
    pub tools: Vec<ToolSpec>,
}

/// What a stream says as it goes.
#[derive(Clone, PartialEq, Debug)]
pub enum Event {
    Text(String),
    /// The model's thinking, when it shares it.
    Reasoning(String),
    Usage {
        input: Option<u32>,
        output: Option<u32>,
    },
    /// The model wants to use a tool. Sent once its arguments are complete.
    ToolCall(Call),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The provider does not accept the key.
    Rejected,
    RateLimited(String),
    /// The chat is longer than the model takes.
    TooLong,
    /// Any other refusal, with the provider's own words.
    Provider(u16, String),
    /// No answer: offline, a wrong address, a server that is down.
    Unreachable,
    /// The request could not be made at all (no key, no address), in words.
    Setup(String),
    /// The stream ended before the reply was complete.
    Cut,
    Cancelled,
}

impl Error {
    /// A sentence for the chat, naming the provider (`who`).
    pub fn describe(&self, who: &str) -> String {
        match self {
            Self::Rejected => {
                format!("{who} did not accept the key. Connect it again on the Providers page.")
            }
            Self::RateLimited(m) if m.is_empty() => {
                format!("{who} is limiting requests. Wait a moment and retry.")
            }
            Self::RateLimited(m) => format!("{who} is limiting requests: {m}"),
            Self::TooLong => {
                "This chat is too long for the model. Press Start fresh, then send it again."
                    .to_owned()
            }
            Self::Provider(status, m) if m.is_empty() => {
                format!("{who} returned an error (HTTP {status}).")
            }
            Self::Provider(status, m) => format!("{who} returned an error (HTTP {status}): {m}"),
            Self::Unreachable => format!("Could not reach {who}. Check your connection and retry."),
            Self::Setup(why) => why.clone(),
            Self::Cut => format!(
                "The reply stopped early: the connection to {who} ended before it finished."
            ),
            Self::Cancelled => "Stopped.".to_owned(),
        }
    }
}

const ANTHROPIC_VERSION: &str = "2023-06-01";
/// The most a reply may run to, besides any thinking budget (Anthropic requires a limit).
const REPLY_TOKENS: u32 = 8192;
/// Waiting for the provider to start answering, and for the whole stream to end.
const FIRST_BYTE: Duration = Duration::from_secs(120);
const WHOLE_STREAM: Duration = Duration::from_secs(600);

/// Streams the reply to `emit`. Blocks on the network, so call it from a worker thread. `cancel` is
/// checked between lines; a stream that is silent cannot be interrupted until it speaks or times out.
pub fn run(
    ep: &Endpoint,
    key: Option<&str>,
    req: &Request,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Event),
) -> Result<(), Error> {
    let (url, headers, body) = prepare(ep, key, req);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(FIRST_BYTE))
        .timeout_global(Some(WHOLE_STREAM))
        .http_status_as_error(false)
        .build()
        .into();
    let mut call = agent.post(&url).header("Content-Type", "application/json");
    for (name, value) in &headers {
        call = call.header(*name, value.as_str());
    }
    let mut resp = call.send(body.as_str()).map_err(|_| Error::Unreachable)?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        return Err(refusal(status, &text));
    }
    read_stream(
        ep.api,
        BufReader::new(resp.into_body().into_reader()),
        cancel,
        emit,
    )
}

/// A whole reply, for a call that does not need to be shown as it arrives.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Completion {
    pub text: String,
    pub input: u32,
    pub output: u32,
}

/// Asks and waits for the whole answer. Blocks, so call it from a worker thread.
pub fn complete(
    ep: &Endpoint,
    key: Option<&str>,
    req: &Request,
    cancel: &AtomicBool,
) -> Result<Completion, Error> {
    let mut done = Completion::default();
    run(ep, key, req, cancel, &mut |ev| match ev {
        Event::Text(t) => done.text.push_str(&t),
        Event::Usage { input, output } => {
            done.input = input.unwrap_or(done.input);
            done.output = output.unwrap_or(done.output);
        }
        Event::Reasoning(_) | Event::ToolCall(_) => {}
    })?;
    Ok(done)
}

/// The address, headers and body for a request.
fn prepare(
    ep: &Endpoint,
    key: Option<&str>,
    req: &Request,
) -> (String, Vec<(&'static str, String)>, String) {
    let mut headers = Vec::new();
    let (path, body) = match ep.api {
        Api::OpenAi => {
            if let Some(k) = key.filter(|k| !k.is_empty()) {
                headers.push(("Authorization", format!("Bearer {k}")));
            }
            ("chat/completions", openai_body(ep.thinking, req))
        }
        Api::Anthropic => {
            if let Some(k) = key {
                headers.push(("x-api-key", k.to_owned()));
            }
            headers.push(("anthropic-version", ANTHROPIC_VERSION.to_owned()));
            ("messages", anthropic_body(req))
        }
    };
    (format!("{}/{path}", ep.base), headers, body.to_string())
}

fn openai_messages(req: &Request) -> Vec<Value> {
    let mut messages = Vec::new();
    // One system message, the stable part first so a provider that caches by prefix keeps it.
    let system: Vec<&str> = [req.system.as_deref(), req.context.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    if !system.is_empty() {
        messages.push(json!({"role": "system", "content": system.join("\n\n")}));
    }
    for t in &req.turns {
        messages.push(match t.role {
            Role::User => json!({"role": "user", "content": t.text}),
            Role::Tool => json!({"role": "tool", "tool_call_id": t.call_id, "content": t.text}),
            Role::Assistant if t.calls.is_empty() => {
                json!({"role": "assistant", "content": t.text})
            }
            Role::Assistant => {
                let calls: Vec<Value> = t
                    .calls
                    .iter()
                    .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.args}}))
                    .collect();
                let content = if t.text.is_empty() { Value::Null } else { json!(t.text) };
                json!({"role": "assistant", "content": content, "tool_calls": calls})
            }
        });
    }
    messages
}

fn openai_body(thinking: Thinking, req: &Request) -> Value {
    let mut body = json!({
        "model": req.model,
        "messages": openai_messages(req),
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    if let Some(map) = body.as_object_mut() {
        if !req.tools.is_empty() {
            let tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.schema}}))
                .collect();
            map.insert("tools".into(), json!(tools));
        }
        if let Some(think) = req.think {
            match thinking {
                Thinking::None => {}
                Thinking::Effort => {
                    map.insert("reasoning_effort".into(), json!(think.name()));
                }
                Thinking::OpenRouter => {
                    map.insert("reasoning".into(), json!({"effort": think.name()}));
                }
                Thinking::Budget => {
                    map.insert(
                        "thinking".into(),
                        json!({"type": "enabled", "budget_tokens": think.budget()}),
                    );
                }
                Thinking::Qwen => {
                    map.insert("enable_thinking".into(), json!(true));
                    map.insert("thinking_budget".into(), json!(think.budget()));
                }
            }
        }
    }
    body
}

/// Anthropic wants tool results inside a user message, together with whatever the user says next,
/// so consecutive user and tool turns become one message.
fn anthropic_messages(req: &Request) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut user_blocks: Vec<Value> = Vec::new();
    let flush = |out: &mut Vec<Value>, blocks: &mut Vec<Value>| {
        if !blocks.is_empty() {
            out.push(json!({"role": "user", "content": std::mem::take(blocks)}));
        }
    };
    for t in &req.turns {
        match t.role {
            Role::Tool => user_blocks
                .push(json!({"type": "tool_result", "tool_use_id": t.call_id, "content": t.text})),
            Role::User => user_blocks.push(json!({"type": "text", "text": t.text})),
            Role::Assistant => {
                flush(&mut out, &mut user_blocks);
                let mut blocks = Vec::new();
                if !t.text.is_empty() {
                    blocks.push(json!({"type": "text", "text": t.text}));
                }
                for c in &t.calls {
                    let input: Value = serde_json::from_str(&c.args).unwrap_or_else(|_| json!({}));
                    blocks.push(
                        json!({"type": "tool_use", "id": c.id, "name": c.name, "input": input}),
                    );
                }
                out.push(json!({"role": "assistant", "content": blocks}));
            }
        }
    }
    flush(&mut out, &mut user_blocks);
    out
}

fn anthropic_body(req: &Request) -> Value {
    // Thinking is left off once a tool has been used in the chat: Anthropic would need the
    // earlier thinking blocks sent back with their signatures, which are not kept.
    let used_tools = req.turns.iter().any(|t| !t.calls.is_empty());
    let budget = req.think.map(Think::budget).filter(|_| !used_tools);
    let mut body = json!({
        "model": req.model,
        // Thinking counts toward the limit and the limit must exceed the budget, so the reply keeps its room.
        "max_tokens": REPLY_TOKENS + budget.unwrap_or(0),
        "messages": anthropic_messages(req),
        "stream": true,
    });
    if let Some(map) = body.as_object_mut() {
        // The brief and skills are the same on every message of a chat, so they are marked
        // cacheable: after the first the provider reads them from its cache at a fraction of the
        // price. The context changes each time, so it follows as a block of its own.
        let mut system = Vec::new();
        if let Some(s) = req.system.as_deref().filter(|s| !s.is_empty()) {
            system.push(json!({"type": "text", "text": s, "cache_control": {"type": "ephemeral"}}));
        }
        if let Some(c) = req.context.as_deref().filter(|c| !c.is_empty()) {
            system.push(json!({"type": "text", "text": c}));
        }
        if !system.is_empty() {
            map.insert("system".into(), json!(system));
        }
        if !req.tools.is_empty() {
            let tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.schema}))
                .collect();
            map.insert("tools".into(), json!(tools));
        }
        if let Some(b) = budget {
            map.insert(
                "thinking".into(),
                json!({"type": "enabled", "budget_tokens": b}),
            );
        }
    }
    body
}

/// What a non-2xx answer means, using the provider's own words where it gave them.
fn refusal(status: u16, body: &str) -> Error {
    let message = error_message(body);
    let lower = message.to_lowercase();
    const TOO_LONG: [&str; 7] = [
        "context length",
        "context window",
        "maximum context",
        "too long",
        "too many tokens",
        "exceeds the maximum",
        "reduce the length",
    ];
    match status {
        401 | 403 => Error::Rejected,
        429 => Error::RateLimited(message),
        400 | 413 | 422 if TOO_LONG.iter().any(|p| lower.contains(p)) => Error::TooLong,
        _ => Error::Provider(status, message),
    }
}

/// The message in an error body (`{"error": {"message": ..}}`, `{"error": ".."}`, `{"message": ..}`),
/// shortened.
fn error_message(body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let found = v
        .pointer("/error/message")
        .or_else(|| v.get("error").filter(|e| e.is_string()))
        .or_else(|| v.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if v.is_null() {
                body.trim().to_owned()
            } else {
                String::new()
            }
        });
    let (short, cut) = crate::runs::clip(&found, 300);
    if cut { format!("{short}…") } else { short }
}

/// Tool calls being written: by position, the id, name and the arguments so far.
type Pending = BTreeMap<u64, (String, String, String)>;

fn read_stream(
    api: Api,
    mut reader: impl BufRead,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Event),
) -> Result<(), Error> {
    let (mut line, mut finished, mut pending) = (String::new(), false, Pending::new());
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => return Err(Error::Cut),
        }
        let Some(data) = line.trim_end().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim_start();
        if data == "[DONE]" {
            finished = true;
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        finished |= match api {
            Api::OpenAi => openai_chunk(&v, &mut pending, emit),
            Api::Anthropic => anthropic_event(&v, &mut pending, emit)?,
        };
        if finished && api == Api::Anthropic {
            break;
        }
    }
    if !finished {
        return Err(Error::Cut);
    }
    // OpenAI-style streams give a call in pieces and never say when one is done.
    for (index, (id, name, args)) in std::mem::take(&mut pending) {
        emit(call_event(index, id, name, args));
    }
    Ok(())
}

fn call_event(index: u64, id: String, name: String, args: String) -> Event {
    let id = if id.is_empty() {
        format!("call_{index}")
    } else {
        id
    };
    let args = if args.trim().is_empty() {
        "{}".to_owned()
    } else {
        args
    };
    Event::ToolCall(Call { id, name, args })
}

/// One OpenAI-style chunk. True when it says the reply is finished.
fn openai_chunk(v: &Value, pending: &mut Pending, emit: &mut dyn FnMut(Event)) -> bool {
    let choice = v.pointer("/choices/0");
    let delta = choice.and_then(|c| c.get("delta"));
    // Reasoning arrives as `reasoning_content` (DeepSeek and most) or `reasoning` (OpenRouter).
    for key in ["reasoning_content", "reasoning"] {
        if let Some(t) = delta
            .and_then(|d| d.get(key))
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
        {
            emit(Event::Reasoning(t.to_owned()));
        }
    }
    if let Some(t) = delta
        .and_then(|d| d.get("content"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        emit(Event::Text(t.to_owned()));
    }
    let calls = delta
        .and_then(|d| d.get("tool_calls"))
        .and_then(Value::as_array);
    for (n, c) in calls.into_iter().flatten().enumerate() {
        let index = c.get("index").and_then(Value::as_u64).unwrap_or(n as u64);
        let entry = pending.entry(index).or_default();
        if let Some(id) = c.get("id").and_then(Value::as_str) {
            entry.0 = id.to_owned();
        }
        if let Some(name) = c.pointer("/function/name").and_then(Value::as_str) {
            entry.1.push_str(name);
        }
        if let Some(args) = c.pointer("/function/arguments").and_then(Value::as_str) {
            entry.2.push_str(args);
        }
    }
    if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
        let n = |k: &str| u.get(k).and_then(Value::as_u64).map(|n| n as u32);
        emit(Event::Usage {
            input: n("prompt_tokens"),
            output: n("completion_tokens"),
        });
    }
    choice
        .and_then(|c| c.get("finish_reason"))
        .is_some_and(|f| !f.is_null())
}

/// One Anthropic event. `Ok(true)` when the message is complete; an error event is an `Err`.
fn anthropic_event(
    v: &Value,
    pending: &mut Pending,
    emit: &mut dyn FnMut(Event),
) -> Result<bool, Error> {
    let usage = |u: Option<&Value>, key: &str| {
        u.and_then(|u| u.get(key))
            .and_then(Value::as_u64)
            .map(|n| n as u32)
    };
    let index = v.get("index").and_then(Value::as_u64).unwrap_or(0);
    match v.get("type").and_then(Value::as_str) {
        Some("message_start") => {
            // Cached input is counted apart from the rest; the run's input is all of it.
            let u = v.pointer("/message/usage");
            let parts = [
                "input_tokens",
                "cache_creation_input_tokens",
                "cache_read_input_tokens",
            ];
            let input = parts
                .iter()
                .filter_map(|k| usage(u, k))
                .reduce(|a, b| a + b);
            emit(Event::Usage {
                input,
                output: None,
            });
        }
        Some("content_block_start") => {
            let block = v.get("content_block");
            if block.and_then(|b| b.get("type")).and_then(Value::as_str) == Some("tool_use") {
                let field = |k: &str| {
                    block
                        .and_then(|b| b.get(k))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                pending.insert(index, (field("id"), field("name"), String::new()));
            }
        }
        Some("content_block_delta") => {
            let delta = v.get("delta");
            match delta.and_then(|d| d.get("type")).and_then(Value::as_str) {
                Some("text_delta") => {
                    if let Some(t) = delta.and_then(|d| d.get("text")).and_then(Value::as_str) {
                        emit(Event::Text(t.to_owned()));
                    }
                }
                Some("thinking_delta") => {
                    if let Some(t) = delta
                        .and_then(|d| d.get("thinking"))
                        .and_then(Value::as_str)
                    {
                        emit(Event::Reasoning(t.to_owned()));
                    }
                }
                Some("input_json_delta") => {
                    if let (Some(t), Some(entry)) = (
                        delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(Value::as_str),
                        pending.get_mut(&index),
                    ) {
                        entry.2.push_str(t);
                    }
                }
                _ => {}
            }
        }
        Some("content_block_stop") => {
            if let Some((id, name, args)) = pending.remove(&index) {
                emit(call_event(index, id, name, args));
            }
        }
        Some("message_delta") => {
            let output = usage(v.get("usage"), "output_tokens");
            emit(Event::Usage {
                input: None,
                output,
            });
        }
        Some("message_stop") => return Ok(true),
        Some("error") => {
            let m = v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let kind = v
                .pointer("/error/type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(match kind {
                "rate_limit_error" => Error::RateLimited(m.to_owned()),
                "authentication_error" | "permission_error" => Error::Rejected,
                _ => Error::Provider(500, m.to_owned()),
            });
        }
        _ => {}
    }
    Ok(false)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn ep(api: Api, thinking: Thinking) -> Endpoint {
        Endpoint {
            api,
            base: "http://x/v1".into(),
            thinking,
        }
    }

    fn req(think: Option<Think>) -> Request {
        Request {
            model: "m".into(),
            system: Some("Be brief.".into()),
            context: None,
            turns: vec![Turn::user("Hi")],
            think,
            tools: Vec::new(),
        }
    }

    fn body(api: Api, thinking: Thinking, think: Option<Think>) -> Value {
        let (_, _, b) = prepare(&ep(api, thinking), Some("k"), &req(think));
        serde_json::from_str(&b).unwrap()
    }

    fn read(api: Api, stream: &str) -> (Vec<Event>, Result<(), Error>) {
        let mut events = Vec::new();
        let r = read_stream(
            api,
            Cursor::new(stream.to_owned()),
            &AtomicBool::new(false),
            &mut |e| events.push(e),
        );
        (events, r)
    }

    #[test]
    fn openai_style_bodies_carry_the_brief_the_chat_and_each_providers_thinking_setting() {
        let b = body(Api::OpenAi, Thinking::Effort, Some(Think::High));
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["content"], "Hi");
        assert_eq!(b["stream"], true);
        assert_eq!(b["reasoning_effort"], "high");
        let b = body(Api::OpenAi, Thinking::OpenRouter, Some(Think::Low));
        assert_eq!(b["reasoning"]["effort"], "low");
        let b = body(Api::OpenAi, Thinking::Budget, Some(Think::Medium));
        assert_eq!(b["thinking"]["budget_tokens"], 4096);
        let b = body(Api::OpenAi, Thinking::Qwen, Some(Think::Low));
        assert_eq!(
            (&b["enable_thinking"], &b["thinking_budget"]),
            (&json!(true), &json!(1024))
        );
        let b = body(Api::OpenAi, Thinking::Effort, None);
        assert!(b.get("reasoning_effort").is_none(), "off sends nothing");
        let b = body(Api::OpenAi, Thinking::None, Some(Think::High));
        assert!(b.get("reasoning_effort").is_none() && b.get("thinking").is_none());
    }

    #[test]
    fn anthropic_bodies_keep_the_brief_apart_and_leave_room_for_the_reply_beyond_the_thinking() {
        let b = body(Api::Anthropic, Thinking::Budget, Some(Think::High));
        assert_eq!(b["system"][0]["text"], "Be brief.");
        assert_eq!(b["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(b["messages"].as_array().unwrap().len(), 1);
        assert_eq!(b["thinking"]["budget_tokens"], 16384);
        assert!(b["max_tokens"].as_u64().unwrap() > 16384);
        let b = body(Api::Anthropic, Thinking::Budget, None);
        assert!(b.get("thinking").is_none());
        assert_eq!(b["max_tokens"], REPLY_TOKENS);
    }

    #[test]
    fn the_key_goes_in_the_header_each_provider_expects_and_nowhere_else() {
        let (url, h, b) = prepare(&ep(Api::OpenAi, Thinking::None), Some("sk-1"), &req(None));
        assert_eq!(url, "http://x/v1/chat/completions");
        assert_eq!(h, vec![("Authorization", "Bearer sk-1".to_owned())]);
        assert!(!b.contains("sk-1"));
        let (url, h, _) = prepare(
            &ep(Api::Anthropic, Thinking::Budget),
            Some("sk-2"),
            &req(None),
        );
        assert_eq!(url, "http://x/v1/messages");
        assert!(h.contains(&("x-api-key", "sk-2".to_owned())));
        let (_, h, _) = prepare(&ep(Api::OpenAi, Thinking::None), None, &req(None));
        assert!(h.is_empty(), "a server with no key gets no header");
    }

    #[test]
    fn an_openai_stream_gives_text_reasoning_and_usage_and_ends_on_done() {
        let (events, r) = read(
            Api::OpenAi,
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hm\"}}]}\n\n\
             data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n\
             : keep-alive\n\n\
             data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n\
             data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n\
             data: [DONE]\n\n",
        );
        assert_eq!(r, Ok(()));
        assert_eq!(
            events,
            vec![
                Event::Reasoning("hm".into()),
                Event::Text("Hel".into()),
                Event::Text("lo".into()),
                Event::Usage {
                    input: Some(7),
                    output: Some(3)
                },
            ]
        );
    }

    #[test]
    fn a_stream_that_ends_before_it_finished_is_cut_and_keeps_what_came() {
        let (events, r) = read(
            Api::OpenAi,
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
        );
        assert_eq!(r, Err(Error::Cut));
        assert_eq!(events, vec![Event::Text("Hel".into())]);
    }

    #[test]
    fn an_anthropic_stream_gives_text_thinking_and_usage_and_an_error_event_is_an_error() {
        let (events, r) = read(
            Api::Anthropic,
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":2,\"cache_read_input_tokens\":10}}}\n\n\
             data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hm\"}}\n\n\
             data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
             data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":2}}\n\n\
             data: {\"type\":\"message_stop\"}\n\n",
        );
        assert_eq!(r, Ok(()));
        assert_eq!(
            events[0],
            Event::Usage {
                input: Some(12),
                output: None
            }
        );
        assert!(events.contains(&Event::Reasoning("hm".into())));
        assert!(events.contains(&Event::Text("Hi".into())));
        assert_eq!(
            events.last(),
            Some(&Event::Usage {
                input: None,
                output: Some(2)
            })
        );

        let (_, r) = read(
            Api::Anthropic,
            "data: {\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"message\":\"slow down\"}}\n\n",
        );
        assert_eq!(r, Err(Error::RateLimited("slow down".into())));
    }

    #[test]
    fn a_cancelled_stream_stops_at_the_next_line() {
        let cancel = AtomicBool::new(true);
        let r = read_stream(
            Api::OpenAi,
            Cursor::new("data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n".to_owned()),
            &cancel,
            &mut |_| {},
        );
        assert_eq!(r, Err(Error::Cancelled));
    }

    #[test]
    fn refusals_are_told_apart_and_use_the_providers_own_words() {
        assert_eq!(refusal(401, "{}"), Error::Rejected);
        assert_eq!(
            refusal(429, r#"{"error":{"message":"Quota used"}}"#),
            Error::RateLimited("Quota used".into())
        );
        assert_eq!(
            refusal(
                400,
                r#"{"error":{"message":"This model's maximum context length is 8192 tokens"}}"#
            ),
            Error::TooLong
        );
        assert_eq!(
            refusal(404, r#"{"error":"no such model"}"#),
            Error::Provider(404, "no such model".into())
        );
        assert_eq!(refusal(500, "oops"), Error::Provider(500, "oops".into()));
        assert!(
            Error::Rejected
                .describe("OpenAI")
                .contains("OpenAI did not accept the key")
        );
    }

    #[test]
    fn each_provider_has_its_chat_address_and_a_server_uses_its_own() {
        let a = endpoint("anthropic", false, None).unwrap();
        assert_eq!((a.api, a.thinking), (Api::Anthropic, Thinking::Budget));
        assert_eq!(endpoint("gemini", false, None).unwrap().api, Api::OpenAi);
        let s = endpoint("llama-server:box", true, Some("http://localhost:8080/v1/")).unwrap();
        assert_eq!(s.base, "http://localhost:8080/v1");
        assert!(endpoint("llama-server:box", true, None).is_none());
        assert!(endpoint("nonesuch", false, None).is_none());
    }

    #[test]
    fn tool_calls_are_offered_and_sent_back_in_each_providers_shape() {
        let mut r = req(None);
        r.tools = vec![ToolSpec {
            name: "ask".into(),
            description: "Ask.".into(),
            schema: json!({"type": "object"}),
        }];
        r.turns.push(Turn::assistant(
            "",
            vec![Call {
                id: "c1".into(),
                name: "ask".into(),
                args: r#"{"question":"Q?"}"#.into(),
            }],
        ));
        r.turns.push(Turn::tool("c1", "Blue"));
        r.turns.push(Turn::user("thanks"));

        let b = openai_body(Thinking::None, &r);
        assert_eq!(b["tools"][0]["function"]["name"], "ask");
        let m = b["messages"].as_array().unwrap();
        assert_eq!(m[2]["tool_calls"][0]["id"], "c1");
        assert_eq!(
            m[2]["tool_calls"][0]["function"]["arguments"],
            r#"{"question":"Q?"}"#
        );
        assert_eq!(m[2]["content"], Value::Null);
        assert_eq!(
            (&m[3]["role"], &m[3]["tool_call_id"]),
            (&json!("tool"), &json!("c1"))
        );
        assert_eq!(m[4]["role"], "user");

        r.think = Some(Think::High);
        let b = anthropic_body(&r);
        assert_eq!(b["tools"][0]["input_schema"]["type"], "object");
        let m = b["messages"].as_array().unwrap();
        assert_eq!(
            m.len(),
            3,
            "the result and what you said next share one user message"
        );
        assert_eq!(m[1]["content"][0]["input"]["question"], "Q?");
        assert_eq!(m[2]["content"][0]["type"], "tool_result");
        assert_eq!(m[2]["content"][0]["tool_use_id"], "c1");
        assert_eq!(m[2]["content"][1]["text"], "thanks");
        assert!(
            b.get("thinking").is_none(),
            "no thinking once a tool was used"
        );
    }

    #[test]
    fn an_openai_tool_call_arrives_in_pieces_and_is_one_event_at_the_end() {
        let (events, r) = read(
            Api::OpenAi,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c9","function":{"name":"ask","arguments":"{\"question\":\"Wh"}}]}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"y?\"}"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#,
        );
        assert_eq!(r, Ok(()));
        assert_eq!(
            events,
            vec![Event::ToolCall(Call {
                id: "c9".into(),
                name: "ask".into(),
                args: r#"{"question":"Why?"}"#.into()
            })]
        );
    }

    #[test]
    fn an_anthropic_tool_call_is_an_event_when_its_block_stops() {
        let (events, r) = read(
            Api::Anthropic,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu1","name":"ask","input":{}}}

data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"question\":"}}

data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"Why?\"}"}}

data: {"type":"content_block_stop","index":1}

data: {"type":"message_stop"}

"#,
        );
        assert_eq!(r, Ok(()));
        assert_eq!(
            events,
            vec![Event::ToolCall(Call {
                id: "tu1".into(),
                name: "ask".into(),
                args: r#"{"question":"Why?"}"#.into()
            })]
        );
    }

    #[test]
    fn the_context_follows_the_brief_and_stays_out_of_what_is_cached() {
        let mut r = req(None);
        r.context = Some("## Context\n- Current date and time: now".into());
        let b = openai_body(Thinking::None, &r);
        assert_eq!(
            b["messages"][0]["content"],
            "Be brief.\n\n## Context\n- Current date and time: now"
        );
        let b = anthropic_body(&r);
        let system = b["system"].as_array().unwrap();
        assert_eq!(system.len(), 2);
        assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
        assert!(
            system[1].get("cache_control").is_none(),
            "it changes, so it is not cached"
        );
        assert!(
            system[1]["text"]
                .as_str()
                .unwrap()
                .contains("Current date and time")
        );

        r.system = None;
        let b = anthropic_body(&r);
        assert_eq!(
            b["system"].as_array().unwrap().len(),
            1,
            "context alone still goes"
        );
        r.context = None;
        assert!(anthropic_body(&r).get("system").is_none());
        assert_eq!(
            openai_body(Thinking::None, &r)["messages"][0]["role"],
            "user"
        );
    }
}
