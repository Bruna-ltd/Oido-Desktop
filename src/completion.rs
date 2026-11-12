//! Asking a model for a reply, streamed: where to ask, how each provider family wants the request
//! (and how it spells "think harder"), and how to read the stream back. Gemini is asked through
//! its OpenAI-compatible endpoint, as `oido-core` does, so there are three formats: OpenAI's, which
//! most providers copy, Anthropic's, and the Responses API that ChatGPT's Codex backend speaks.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::chatgpt;
use crate::credentials::{
    CLAUDE_CODE_SYSTEM, CLAUDE_CODE_USER_AGENT, Credential, OAUTH_BETA, TOOL_PREFIX,
};
use crate::guard::Guarded;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Api {
    /// `POST {base}/chat/completions`, server-sent events of `choices[0].delta`.
    OpenAi,
    /// `POST {base}/messages`, server-sent events of typed blocks.
    Anthropic,
    /// `POST {base}/responses`, OpenAI's Responses API: server-sent events of typed response
    /// events. It is what a ChatGPT plan is spent on.
    Codex,
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
    /// `"reasoning": {"effort": "low", "summary": "auto"}` (the Responses API).
    Responses,
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
        "codex" => make(Api::Codex, chatgpt::BASE, Thinking::Responses),
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
    /// Pictures sent with a user turn.
    pub images: Vec<Image>,
}

/// A picture in a user turn: its type (`image/png`, `image/jpeg`) and its bytes, base64.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    pub media_type: String,
    pub data: String,
}

impl Turn {
    pub fn user(text: impl Into<String>) -> Turn {
        Turn {
            role: Role::User,
            text: text.into(),
            calls: Vec::new(),
            call_id: None,
            images: Vec::new(),
        }
    }

    /// A user turn with pictures after its words.
    pub fn user_with(text: impl Into<String>, images: Vec<Image>) -> Turn {
        Turn {
            images,
            ..Turn::user(text)
        }
    }

    pub fn assistant(text: impl Into<String>, calls: Vec<Call>) -> Turn {
        Turn {
            role: Role::Assistant,
            text: text.into(),
            calls,
            call_id: None,
            images: Vec::new(),
        }
    }

    pub fn tool(call_id: impl Into<String>, result: impl Into<String>) -> Turn {
        Turn {
            role: Role::Tool,
            text: result.into(),
            calls: Vec::new(),
            call_id: Some(call_id.into()),
            images: Vec::new(),
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

/// The tokens a model call used, as far as the provider has said. Cache reads and writes are
/// part of `input`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Tokens {
    pub input: Option<u32>,
    pub output: Option<u32>,
    pub cache_read: Option<u32>,
    pub cache_write: Option<u32>,
}

impl Tokens {
    /// Takes what a later usage event says, keeping what it leaves out.
    pub fn update(&mut self, later: Tokens) {
        self.input = later.input.or(self.input);
        self.output = later.output.or(self.output);
        self.cache_read = later.cache_read.or(self.cache_read);
        self.cache_write = later.cache_write.or(self.cache_write);
    }
}

/// What a stream says as it goes.
#[derive(Clone, PartialEq, Debug)]
pub enum Event {
    Text(String),
    /// The model's thinking, when it shares it.
    Reasoning(String),
    /// What the call used so far. Each event says only what it knows; see [`Tokens::update`].
    Usage(Tokens),
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
                "This chat is too long for the model, even with older parts trimmed and summarized. Shorten your last message, or press Start fresh."
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
/// Only a request that has been through the guard can be sent (see [`crate::guard`]).
pub fn run(
    ep: &Endpoint,
    credential: Option<&Credential>,
    req: &Guarded,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Event),
) -> Result<(), Error> {
    let oauth = credential.is_some_and(Credential::is_subscription);
    let (url, headers, body) = prepare(ep, credential, req.request());
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
        oauth,
        &req.request().tools,
        BufReader::new(resp.into_body().into_reader()),
        cancel,
        emit,
    )
}

/// A whole reply, for a call that does not need to be shown as it arrives.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Completion {
    pub text: String,
    pub tokens: Tokens,
}

/// Asks and waits for the whole answer. Blocks, so call it from a worker thread.
pub fn complete(
    ep: &Endpoint,
    credential: Option<&Credential>,
    req: &Guarded,
    cancel: &AtomicBool,
) -> Result<Completion, Error> {
    let mut done = Completion::default();
    run(ep, credential, req, cancel, &mut |ev| match ev {
        Event::Text(t) => done.text.push_str(&t),
        Event::Usage(t) => done.tokens.update(t),
        Event::Reasoning(_) | Event::ToolCall(_) => {}
    })?;
    Ok(done)
}

/// The address, headers and body for a request.
fn prepare(
    ep: &Endpoint,
    credential: Option<&Credential>,
    req: &Request,
) -> (String, Vec<(&'static str, String)>, String) {
    let mut headers = Vec::new();
    let oauth = credential.is_some_and(Credential::is_subscription);
    let (path, body) = match ep.api {
        Api::OpenAi => {
            if let Some(c) = credential.filter(|c| !c.token().is_empty()) {
                headers.push(("Authorization", format!("Bearer {}", c.token())));
            }
            ("chat/completions", openai_body(ep.thinking, req))
        }
        Api::Anthropic => {
            match credential {
                Some(Credential::Subscription(token)) if !token.is_empty() => {
                    headers.push(("Authorization", format!("Bearer {}", token.as_str())));
                    headers.push(("anthropic-beta", OAUTH_BETA.to_owned()));
                    headers.push(("user-agent", CLAUDE_CODE_USER_AGENT.to_owned()));
                    headers.push(("x-app", "cli".to_owned()));
                }
                Some(Credential::Key(key)) if !key.is_empty() => {
                    headers.push(("x-api-key", key.to_string()));
                }
                _ => {}
            }
            headers.push(("anthropic-version", ANTHROPIC_VERSION.to_owned()));
            ("messages", anthropic_body(req, oauth))
        }
        Api::Codex => {
            if let Some(Credential::Subscription(token)) =
                credential.filter(|c| !c.token().is_empty())
            {
                headers.extend(chatgpt::headers(token));
            }
            ("responses", codex_body(ep.thinking, req))
        }
    };
    (format!("{}/{path}", ep.base), headers, body.to_string())
}

/// A call's arguments as the JSON object they should be, or `None` when the model's were cut off
/// or malformed. Providers parse the arguments of earlier calls when they are sent back, and refuse
/// the whole request over one that is not an object, so such a call goes back with `{}`: its
/// result already says what went wrong.
fn args_object(args: &str) -> Option<Value> {
    serde_json::from_str::<Value>(args)
        .ok()
        .filter(Value::is_object)
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
            Role::User if t.images.is_empty() => json!({"role": "user", "content": t.text}),
            Role::User => {
                let mut parts = Vec::new();
                if !t.text.is_empty() {
                    parts.push(json!({"type": "text", "text": t.text}));
                }
                for i in &t.images {
                    let url = format!("data:{};base64,{}", i.media_type, i.data);
                    parts.push(json!({"type": "image_url", "image_url": {"url": url}}));
                }
                json!({"role": "user", "content": parts})
            }
            Role::Tool => json!({"role": "tool", "tool_call_id": t.call_id, "content": t.text}),
            Role::Assistant if t.calls.is_empty() => {
                json!({"role": "assistant", "content": t.text})
            }
            Role::Assistant => {
                let calls: Vec<Value> = t
                    .calls
                    .iter()
                    .map(|c| {
                        let args = if args_object(&c.args).is_some() { c.args.as_str() } else { "{}" };
                        json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": args}})
                    })
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
                Thinking::None | Thinking::Responses => {}
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

/// The Responses API's input: the chat as a list of items, a tool call and its result each an item
/// of their own, matched by `call_id`.
fn codex_input(req: &Request) -> Vec<Value> {
    let mut input = Vec::new();
    for t in &req.turns {
        match t.role {
            Role::User => {
                let mut parts = Vec::new();
                if !t.text.is_empty() || t.images.is_empty() {
                    parts.push(json!({"type": "input_text", "text": t.text}));
                }
                for i in &t.images {
                    let url = format!("data:{};base64,{}", i.media_type, i.data);
                    parts.push(json!({"type": "input_image", "image_url": url}));
                }
                input.push(json!({"type": "message", "role": "user", "content": parts}));
            }
            Role::Assistant => {
                if !t.text.is_empty() {
                    input.push(json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": t.text}]}));
                }
                for c in &t.calls {
                    let args = if args_object(&c.args).is_some() {
                        c.args.as_str()
                    } else {
                        "{}"
                    };
                    input.push(json!({"type": "function_call", "call_id": c.id, "name": c.name, "arguments": args}));
                }
            }
            Role::Tool => input.push(
                json!({"type": "function_call_output", "call_id": t.call_id, "output": t.text}),
            ),
        }
    }
    input
}

/// A Responses request as ChatGPT's Codex backend takes it: always streamed, and never stored there
/// (the chat is sent whole each time, as with every other provider).
fn codex_body(thinking: Thinking, req: &Request) -> Value {
    // This backend has no system message: the brief goes in `instructions`, the stable part first.
    let instructions: Vec<&str> = [req.system.as_deref(), req.context.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    let mut body = json!({
        "model": req.model,
        "instructions": instructions.join("\n\n"),
        "input": codex_input(req),
        "stream": true,
        "store": false,
    });
    if let Some(map) = body.as_object_mut() {
        if !req.tools.is_empty() {
            let tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| json!({"type": "function", "name": t.name, "description": t.description, "parameters": t.schema}))
                .collect();
            map.insert("tools".into(), json!(tools));
            map.insert("tool_choice".into(), json!("auto"));
        }
        if let (Some(think), Thinking::Responses) = (req.think, thinking) {
            map.insert(
                "reasoning".into(),
                json!({"effort": think.name(), "summary": "auto"}),
            );
        }
    }
    body
}

/// Anthropic wants tool results inside a user message, together with whatever the user says next,
/// so consecutive user and tool turns become one message.
fn anthropic_messages(req: &Request, oauth: bool) -> Vec<Value> {
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
            Role::User => {
                if !t.text.is_empty() || t.images.is_empty() {
                    user_blocks.push(json!({"type": "text", "text": t.text}));
                }
                for i in &t.images {
                    user_blocks.push(json!({"type": "image", "source": {"type": "base64", "media_type": i.media_type, "data": i.data}}));
                }
            }
            Role::Assistant => {
                flush(&mut out, &mut user_blocks);
                let mut blocks = Vec::new();
                if !t.text.is_empty() {
                    blocks.push(json!({"type": "text", "text": t.text}));
                }
                for c in &t.calls {
                    let input = args_object(&c.args).unwrap_or_else(|| json!({}));
                    blocks.push(
                        json!({"type": "tool_use", "id": c.id, "name": wire_tool_name(&c.name, oauth), "input": input}),
                    );
                }
                out.push(json!({"role": "assistant", "content": blocks}));
            }
        }
    }
    flush(&mut out, &mut user_blocks);
    out
}

/// A tool name as the wire takes it. On a subscription Anthropic only runs tools under the `mcp__`
/// namespace, so every name carries it; the prefix is stripped again from the reply.
fn wire_tool_name(name: &str, oauth: bool) -> String {
    if oauth && !name.starts_with(TOOL_PREFIX) {
        format!("{TOOL_PREFIX}{name}")
    } else {
        name.to_owned()
    }
}

/// A wire tool name back to the name the app dispatches by. Not every name gets the prefix on the
/// way out: an MCP tool is already `mcp__<server>__<tool>` and is sent as it is, so the request's
/// own tool names say which form a reply means. A name not among them is left alone.
fn app_tool_name(wire: &str, oauth: bool, tools: &[ToolSpec]) -> String {
    if !oauth || tools.iter().any(|t| t.name == wire) {
        return wire.to_owned();
    }
    match wire.strip_prefix(TOOL_PREFIX) {
        Some(bare) if tools.iter().any(|t| t.name == bare) => bare.to_owned(),
        _ => wire.to_owned(),
    }
}

fn anthropic_body(req: &Request, oauth: bool) -> Value {
    // Thinking is left off once a tool has been used in the chat: Anthropic would need the
    // earlier thinking blocks sent back with their signatures, which are not kept.
    let used_tools = req.turns.iter().any(|t| !t.calls.is_empty());
    let budget = req.think.map(Think::budget).filter(|_| !used_tools);
    let mut body = json!({
        "model": req.model,
        // Thinking counts toward the limit and the limit must exceed the budget, so the reply keeps its room.
        "max_tokens": REPLY_TOKENS + budget.unwrap_or(0),
        "messages": anthropic_messages(req, oauth),
        "stream": true,
    });
    if let Some(map) = body.as_object_mut() {
        // OAuth inference must introduce itself as Claude Code, or Anthropic refuses it.
        let mut system = Vec::new();
        if oauth {
            system.push(json!({"type": "text", "text": CLAUDE_CODE_SYSTEM}));
        }
        // The brief and skills are the same on every message of a chat, so they are marked
        // cacheable: after the first the provider reads them from its cache at a fraction of the
        // price. The context changes each time, so it follows as a block of its own.
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
                .map(|t| json!({"name": wire_tool_name(&t.name, oauth), "description": t.description, "input_schema": t.schema}))
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
    const TOO_LONG: [&str; 8] = [
        "context length",
        "context window",
        // llama-server: "the request exceeds the available context size".
        "context size",
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

/// The message in an error body (`{"error": {"message": ..}}`, `{"error": ".."}`, `{"message": ..}`,
/// `{"detail": ..}`), shortened.
fn error_message(body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let found = v
        .pointer("/error/message")
        .or_else(|| v.get("error").filter(|e| e.is_string()))
        .or_else(|| v.get("message"))
        .or_else(|| v.get("detail").filter(|d| d.is_string()))
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
    oauth: bool,
    tools: &[ToolSpec],
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
            Api::Anthropic => anthropic_event(&v, oauth, tools, &mut pending, emit)?,
            Api::Codex => codex_event(&v, emit)?,
        };
        if finished && api != Api::OpenAi {
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
        let n = |k: &str| u.pointer(k).and_then(Value::as_u64).map(|n| n as u32);
        emit(Event::Usage(Tokens {
            input: n("/prompt_tokens"),
            output: n("/completion_tokens"),
            // OpenAI and those that copy it; DeepSeek names its own.
            cache_read: n("/prompt_tokens_details/cached_tokens")
                .or_else(|| n("/prompt_cache_hit_tokens")),
            cache_write: None,
        }));
    }
    choice
        .and_then(|c| c.get("finish_reason"))
        .is_some_and(|f| !f.is_null())
}

/// One event of a Responses stream. `Ok(true)` when the response is complete; a failure is an
/// `Err`. A tool call arrives whole, in the event that closes its item.
fn codex_event(v: &Value, emit: &mut dyn FnMut(Event)) -> Result<bool, Error> {
    let text = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    match v.get("type").and_then(Value::as_str) {
        Some("response.output_text.delta") => {
            if let Some(t) = text("delta") {
                emit(Event::Text(t));
            }
        }
        Some("response.reasoning_summary_text.delta" | "response.reasoning_text.delta") => {
            if let Some(t) = text("delta") {
                emit(Event::Reasoning(t));
            }
        }
        Some("response.output_item.done") => {
            let item = v
                .get("item")
                .filter(|i| i.get("type").and_then(Value::as_str) == Some("function_call"));
            if let Some(item) = item {
                let field = |k: &str| {
                    item.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                let index = v.get("output_index").and_then(Value::as_u64).unwrap_or(0);
                emit(call_event(
                    index,
                    field("call_id"),
                    field("name"),
                    field("arguments"),
                ));
            }
        }
        Some("response.completed" | "response.incomplete") => {
            let u = v.pointer("/response/usage");
            let n = |k: &str| {
                u.and_then(|u| u.pointer(k))
                    .and_then(Value::as_u64)
                    .map(|n| n as u32)
            };
            emit(Event::Usage(Tokens {
                input: n("/input_tokens"),
                output: n("/output_tokens"),
                cache_read: n("/input_tokens_details/cached_tokens"),
                cache_write: None,
            }));
            return Ok(true);
        }
        Some("response.failed") => return Err(codex_failure(v.pointer("/response/error"))),
        Some("error") => return Err(codex_failure(Some(v))),
        _ => {}
    }
    Ok(false)
}

/// What a failed Responses stream says went wrong, by its error `code`.
fn codex_failure(error: Option<&Value>) -> Error {
    let field = |k: &str| {
        error
            .and_then(|e| e.get(k))
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    let message = field("message").to_owned();
    match field("code") {
        "rate_limit_exceeded" | "usage_limit_reached" | "insufficient_quota" => {
            Error::RateLimited(message)
        }
        "context_length_exceeded" => Error::TooLong,
        "invalid_api_key" | "token_expired" | "invalid_token" => Error::Rejected,
        _ => Error::Provider(500, message),
    }
}

/// One Anthropic event. `Ok(true)` when the message is complete; an error event is an `Err`.
fn anthropic_event(
    v: &Value,
    oauth: bool,
    tools: &[ToolSpec],
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
            emit(Event::Usage(Tokens {
                input,
                cache_read: usage(u, "cache_read_input_tokens"),
                cache_write: usage(u, "cache_creation_input_tokens"),
                ..Tokens::default()
            }));
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
                let name = app_tool_name(&field("name"), oauth, tools);
                pending.insert(index, (field("id"), name, String::new()));
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
            emit(Event::Usage(Tokens {
                output,
                ..Tokens::default()
            }));
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
        let (_, _, b) = prepare(&ep(api, thinking), Some(&Credential::key("k")), &req(think));
        serde_json::from_str(&b).unwrap()
    }

    fn read(api: Api, stream: &str) -> (Vec<Event>, Result<(), Error>) {
        let mut events = Vec::new();
        let r = read_stream(
            api,
            false,
            &[],
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
    fn a_subscription_introduces_itself_as_claude_code_and_names_every_tool_under_mcp() {
        let (_, h, raw) = prepare(
            &ep(Api::Anthropic, Thinking::Budget),
            Some(&Credential::subscription("sk-ant-oat-1")),
            &req(None),
        );
        assert!(
            h.contains(&("Authorization", "Bearer sk-ant-oat-1".to_owned())),
            "{h:?}"
        );
        assert!(h.contains(&("x-app", "cli".to_owned())));
        assert!(
            h.iter()
                .any(|(n, v)| *n == "anthropic-beta" && v.contains("oauth-2025-04-20"))
        );
        assert!(!h.iter().any(|(n, _)| *n == "x-api-key"));
        assert!(
            !raw.contains("sk-ant-oat-1"),
            "the token stays out of the body"
        );

        let mut r = req(None);
        r.tools = vec![ToolSpec {
            name: "read_file".into(),
            description: "read".into(),
            schema: json!({"type": "object"}),
        }];
        r.turns = vec![
            Turn::user("Hi"),
            Turn::assistant(
                "",
                vec![Call {
                    id: "c1".into(),
                    name: "read_file".into(),
                    args: "{}".into(),
                }],
            ),
            Turn::tool("c1", "done"),
        ];
        let b = anthropic_body(&r, true);
        assert_eq!(b["system"][0]["text"], CLAUDE_CODE_SYSTEM);
        assert_eq!(b["system"][1]["text"], "Be brief.");
        assert_eq!(b["tools"][0]["name"], "mcp__read_file");
        assert_eq!(b["messages"][1]["content"][0]["name"], "mcp__read_file");
    }

    #[test]
    fn a_subscriptions_reply_names_the_tools_the_app_knows() {
        let mut events = Vec::new();
        let r = read_stream(
            Api::Anthropic,
            true,
            &[ToolSpec {
                name: "read_file".into(),
                description: "read".into(),
                schema: json!({"type": "object"}),
            }],
            Cursor::new(
                "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"mcp__read_file\"}}\n\n\
                 data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n\
                 data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
                 data: {\"type\":\"message_stop\"}\n\n"
                    .to_owned(),
            ),
            &AtomicBool::new(false),
            &mut |e| events.push(e),
        );
        assert_eq!(r, Ok(()));
        assert_eq!(
            events,
            vec![Event::ToolCall(Call {
                id: "t1".into(),
                name: "read_file".into(),
                args: "{}".into(),
            })]
        );
    }

    /// A tool of an MCP server is already `mcp__<server>__<tool>` and goes out untouched, so the
    /// reply must come back untouched too: taking the prefix off would name a tool that is not there.
    #[test]
    fn a_subscriptions_mcp_tool_keeps_the_name_the_app_registered() {
        let mut events = Vec::new();
        let tools = [ToolSpec {
            name: "mcp__demo__ping".into(),
            description: "ping".into(),
            schema: json!({"type": "object"}),
        }];
        let r = read_stream(
            Api::Anthropic,
            true,
            &tools,
            Cursor::new(
                "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"mcp__demo__ping\"}}\n\n\
                 data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n\
                 data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
                 data: {\"type\":\"message_stop\"}\n\n"
                    .to_owned(),
            ),
            &AtomicBool::new(false),
            &mut |e| events.push(e),
        );
        assert_eq!(r, Ok(()));
        assert_eq!(
            events,
            vec![Event::ToolCall(Call {
                id: "t1".into(),
                name: "mcp__demo__ping".into(),
                args: "{}".into(),
            })]
        );
    }

    #[test]
    fn the_key_goes_in_the_header_each_provider_expects_and_nowhere_else() {
        let (url, h, b) = prepare(
            &ep(Api::OpenAi, Thinking::None),
            Some(&Credential::key("sk-1")),
            &req(None),
        );
        assert_eq!(url, "http://x/v1/chat/completions");
        assert_eq!(h, vec![("Authorization", "Bearer sk-1".to_owned())]);
        assert!(!b.contains("sk-1"));
        let (url, h, _) = prepare(
            &ep(Api::Anthropic, Thinking::Budget),
            Some(&Credential::key("sk-2")),
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
             data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"prompt_tokens_details\":{\"cached_tokens\":4}}}\n\n\
             data: [DONE]\n\n",
        );
        assert_eq!(r, Ok(()));
        assert_eq!(
            events,
            vec![
                Event::Reasoning("hm".into()),
                Event::Text("Hel".into()),
                Event::Text("lo".into()),
                Event::Usage(Tokens {
                    input: Some(7),
                    output: Some(3),
                    cache_read: Some(4),
                    cache_write: None,
                }),
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
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":2,\"cache_read_input_tokens\":10,\"cache_creation_input_tokens\":5}}}\n\n\
             data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hm\"}}\n\n\
             data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
             data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":2}}\n\n\
             data: {\"type\":\"message_stop\"}\n\n",
        );
        assert_eq!(r, Ok(()));
        assert_eq!(
            events[0],
            Event::Usage(Tokens {
                input: Some(17),
                output: None,
                cache_read: Some(10),
                cache_write: Some(5),
            })
        );
        assert!(events.contains(&Event::Reasoning("hm".into())));
        assert!(events.contains(&Event::Text("Hi".into())));
        assert_eq!(
            events.last(),
            Some(&Event::Usage(Tokens {
                output: Some(2),
                ..Tokens::default()
            }))
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
            false,
            &[],
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
            refusal(
                400,
                r#"{"error":{"code":400,"message":"the request exceeds the available context size, try increasing it","type":"exceed_context_size_error"}}"#
            ),
            Error::TooLong,
            "llama-server"
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
    fn pictures_follow_the_words_in_each_providers_shape() {
        let mut r = req(None);
        let pic = Image {
            media_type: "image/png".into(),
            data: "AAAA".into(),
        };
        r.turns = vec![Turn::user_with("what is this", vec![pic])];

        let b = openai_body(Thinking::None, &r);
        let parts = &b["messages"][1]["content"];
        assert_eq!(parts[0], json!({"type": "text", "text": "what is this"}));
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,AAAA");

        let b = anthropic_body(&r, false);
        let blocks = &b["messages"][0]["content"];
        assert_eq!(blocks[0]["text"], "what is this");
        assert_eq!(
            blocks[1]["source"],
            json!({"type": "base64", "media_type": "image/png", "data": "AAAA"})
        );

        // A picture sent with no words carries no empty text part.
        r.turns[0].text.clear();
        assert_eq!(
            openai_body(Thinking::None, &r)["messages"][1]["content"][0]["type"],
            "image_url"
        );
        assert_eq!(
            anthropic_body(&r, false)["messages"][0]["content"][0]["type"],
            "image"
        );
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
        let b = anthropic_body(&r, false);
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
        let b = anthropic_body(&r, false);
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
        let b = anthropic_body(&r, false);
        assert_eq!(
            b["system"].as_array().unwrap().len(),
            1,
            "context alone still goes"
        );
        r.context = None;
        assert!(anthropic_body(&r, false).get("system").is_none());
        assert_eq!(
            openai_body(Thinking::None, &r)["messages"][0]["role"],
            "user"
        );
    }

    #[test]
    fn a_call_with_cut_off_arguments_goes_back_with_an_empty_object() {
        let mut r = req(None);
        for (id, args) in [("c1", r#"{"path": "a.md""#), ("c2", ""), ("c3", "[1]")] {
            r.turns.push(Turn::assistant(
                "",
                vec![Call {
                    id: id.into(),
                    name: "write_file".into(),
                    args: args.into(),
                }],
            ));
            r.turns
                .push(Turn::tool(id, "Error: the arguments were not valid JSON."));
        }

        let b = openai_body(Thinking::None, &r);
        let sent: Vec<&Value> = b["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m.get("tool_calls"))
            .map(|c| &c[0]["function"]["arguments"])
            .collect();
        assert_eq!(sent, [&json!("{}"); 3]);

        let b = anthropic_body(&r, false);
        let inputs: Vec<&Value> = b["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "assistant")
            .map(|m| &m["content"][0]["input"])
            .collect();
        assert_eq!(inputs, [&json!({}); 3]);
    }
    fn codex_chat() -> Request {
        Request {
            model: "gpt-5.3-codex".into(),
            system: Some("Be brief.".into()),
            context: Some("It is Friday.".into()),
            turns: vec![
                Turn::user_with(
                    "Look",
                    vec![Image {
                        media_type: "image/png".into(),
                        data: "AAAA".into(),
                    }],
                ),
                Turn::assistant(
                    "Reading it.",
                    vec![Call {
                        id: "call_1".into(),
                        name: "read_file".into(),
                        args: r#"{"path":"a"}"#.into(),
                    }],
                ),
                Turn::tool("call_1", "contents"),
                Turn::user("Thanks"),
            ],
            think: Some(Think::Medium),
            tools: vec![ToolSpec {
                name: "read_file".into(),
                description: "Reads a file".into(),
                schema: json!({"type": "object"}),
            }],
        }
    }

    #[test]
    fn a_codex_request_puts_the_brief_in_instructions_and_the_chat_in_items() {
        let b = codex_body(Thinking::Responses, &codex_chat());
        assert_eq!(b["instructions"], "Be brief.\n\nIt is Friday.");
        assert_eq!((&b["stream"], &b["store"]), (&json!(true), &json!(false)));
        assert!(
            b.get("messages").is_none(),
            "the Responses API has no messages"
        );
        assert_eq!(
            b["reasoning"],
            json!({"effort": "medium", "summary": "auto"})
        );
        assert_eq!(b["tools"][0]["name"], "read_file");
        assert_eq!(b["tools"][0]["type"], "function");
        assert_eq!(b["tool_choice"], "auto");

        let input = b["input"].as_array().unwrap();
        let kinds: Vec<(&str, &str)> = input
            .iter()
            .map(|i| {
                (
                    i["type"].as_str().unwrap(),
                    i["role"].as_str().unwrap_or(""),
                )
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ("message", "user"),
                ("message", "assistant"),
                ("function_call", ""),
                ("function_call_output", ""),
                ("message", "user"),
            ]
        );
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(
            input[0]["content"][1],
            json!({"type": "input_image", "image_url": "data:image/png;base64,AAAA"})
        );
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[2]["arguments"], r#"{"path":"a"}"#);
        assert_eq!(
            input[3],
            json!({"type": "function_call_output", "call_id": "call_1", "output": "contents"})
        );

        let mut quiet = codex_chat();
        (quiet.think, quiet.tools) = (None, Vec::new());
        let b = codex_body(Thinking::Responses, &quiet);
        assert!(
            b.get("reasoning").is_none()
                && b.get("tools").is_none()
                && b.get("tool_choice").is_none(),
            "off and no tools send nothing: {b}"
        );
    }

    #[test]
    fn a_codex_request_goes_to_responses_with_the_plans_token_and_account() {
        let token = crate::testing::access_token(Some("acct-1"));
        let ep = Endpoint {
            api: Api::Codex,
            base: "https://chatgpt.com/backend-api/codex".into(),
            thinking: Thinking::Responses,
        };
        let (url, headers, _) =
            prepare(&ep, Some(&Credential::subscription(&token)), &codex_chat());
        assert_eq!(url, "https://chatgpt.com/backend-api/codex/responses");
        let get = |name: &str| {
            headers
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("Authorization"), Some(format!("Bearer {token}")));
        assert_eq!(get("chatgpt-account-id").as_deref(), Some("acct-1"));
        assert!(get("OpenAI-Beta").is_some());
        let at = endpoint("codex", false, None).unwrap();
        assert_eq!((at.api, at.thinking), (Api::Codex, Thinking::Responses));
    }

    #[test]
    fn a_codex_stream_gives_text_thinking_whole_tool_calls_and_usage() {
        let stream = [
            r#"{"type":"response.created","response":{}}"#,
            r#"{"type":"response.reasoning_summary_text.delta","delta":"Checking."}"#,
            r#"{"type":"response.output_text.delta","delta":"Hel"}"#,
            r#"{"type":"response.output_text.delta","delta":"lo"}"#,
            r#"{"type":"response.output_item.done","output_index":1,"item":{"type":"message","role":"assistant"}}"#,
            r#"{"type":"response.output_item.done","output_index":2,"item":{"type":"function_call","call_id":"call_9","name":"read_file","arguments":"{\"path\":\"a\"}"}}"#,
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":120,"output_tokens":30,"input_tokens_details":{"cached_tokens":100}}}}"#,
        ]
        .map(|data| format!("event: x\ndata: {data}\n\n"))
        .concat();
        let (events, done) = read(Api::Codex, &stream);
        assert_eq!(done, Ok(()));
        assert_eq!(
            events,
            [
                Event::Reasoning("Checking.".into()),
                Event::Text("Hel".into()),
                Event::Text("lo".into()),
                Event::ToolCall(Call {
                    id: "call_9".into(),
                    name: "read_file".into(),
                    args: r#"{"path":"a"}"#.into(),
                }),
                Event::Usage(Tokens {
                    input: Some(120),
                    output: Some(30),
                    cache_read: Some(100),
                    cache_write: None,
                }),
            ]
        );
    }

    #[test]
    fn a_codex_stream_that_ends_without_completing_was_cut_and_failures_are_told_apart() {
        let (_, done) = read(
            Api::Codex,
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n",
        );
        assert_eq!(done, Err(Error::Cut));

        let failed = |code: &str| {
            let data = json!({"type": "response.failed", "response": {"error": {"code": code, "message": "m"}}});
            read(Api::Codex, &format!("data: {data}\n\n")).1
        };
        assert_eq!(
            failed("usage_limit_reached"),
            Err(Error::RateLimited("m".into()))
        );
        assert_eq!(failed("context_length_exceeded"), Err(Error::TooLong));
        assert_eq!(
            failed("something_else"),
            Err(Error::Provider(500, "m".into()))
        );
        let plain = json!({"type": "error", "code": "rate_limit_exceeded", "message": "slow down"});
        assert_eq!(
            read(Api::Codex, &format!("data: {plain}\n\n")).1,
            Err(Error::RateLimited("slow down".into()))
        );
    }

    #[test]
    fn a_refusal_reads_the_detail_chatgpts_backend_gives() {
        assert_eq!(
            refusal(400, r#"{"detail":"Instructions are required"}"#),
            Error::Provider(400, "Instructions are required".into())
        );
        assert_eq!(
            refusal(
                429,
                r#"{"error":{"type":"usage_limit_reached","message":"The usage limit has been reached"}}"#
            ),
            Error::RateLimited("The usage limit has been reached".into())
        );
    }
}
