//! MCP servers: other programs that offer tools to agents (the Model Context Protocol).
//!
//! Any number of servers can be registered. A server is either remote (a URL, reached over HTTP,
//! with no sign-in, a pasted key, or an OAuth sign-in) or local (a command this app starts and
//! talks to over its standard input and output). Registering one lists its tools and saves them,
//! so an agent can be given them without the server being reached again until a tool is called.
//!
//! A server's tools join the register under the name `mcp__<server>__<tool>`, which is what an
//! agent chooses and what the model calls. Nothing here asks before a tool runs: an agent that was
//! given an MCP tool runs it as soon as the model calls it, as it does the shell tool.

pub mod catalog;
pub mod client;
mod flow;
pub mod oauth;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::completion::ToolSpec;

pub use catalog::{PRESETS, Preset};
pub use flow::{Kind, Mcp};

/// What every MCP tool name starts with.
pub const PREFIX: &str = "mcp__";
/// The longest tool name a model provider accepts.
const MAX_NAME: usize = 64;

pub fn is_tool(name: &str) -> bool {
    name.starts_with(PREFIX)
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Transport {
    Http { url: String },
    Stdio { command: String, args: Vec<String> },
}

/// How a remote server knows who is calling.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(
    clippy::enum_variant_names,
    reason = "OAuth is the protocol's name, not a suffix"
)]
pub enum Auth {
    None,
    Key,
    OAuth,
}

impl Auth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Key => "key",
            Self::OAuth => "oauth",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "key" => Self::Key,
            "oauth" => Self::OAuth,
            _ => Self::None,
        }
    }
}

/// What the last attempt to list a server's tools found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// Not reached yet.
    Pending,
    Reachable,
    Unreachable,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Reachable => "reachable",
            Self::Unreachable => "unreachable",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "reachable" => Self::Reachable,
            "unreachable" => Self::Unreachable,
            _ => Self::Pending,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Tool {
    /// The name the server knows it by.
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments.
    pub schema: Value,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub transport: Transport,
    pub auth: Auth,
    pub enabled: bool,
    pub status: Status,
    /// Why the server is unreachable, or empty.
    pub detail: String,
    pub tools: Vec<Tool>,
}

/// What must not sit in the database as readable text. Sealed as one value (see
/// [`crate::store::Store::save_mcp_secrets`]).
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Secrets {
    /// A pasted key, sent as a bearer token.
    #[serde(default)]
    pub key: Option<String>,
    /// Environment variables a local server is started with.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub oauth: Option<oauth::Tokens>,
}

// Secrets are never printed.
impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secrets(..)")
    }
}

impl Server {
    /// The server's name as it appears in tool names: lowercase letters, digits and underscores.
    pub fn slug(&self) -> String {
        slug(&self.name)
    }

    /// The name the register and the model know one of its tools by.
    pub fn qualified(&self, tool: &str) -> String {
        let mut name = format!("{PREFIX}{}__{}", self.slug(), clean(tool));
        // Only ASCII is left, so cutting at a byte is cutting at a character.
        name.truncate(MAX_NAME);
        name
    }

    /// What the server is, in a line, for lists.
    pub fn target(&self) -> String {
        match &self.transport {
            Transport::Http { url } => url.clone(),
            Transport::Stdio { command, args } => std::iter::once(command.as_str())
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    /// Whether its tools are offered to agents: it is on and was reached.
    pub fn offers(&self) -> bool {
        self.enabled && self.status == Status::Reachable
    }
}

/// Lowercase letters, digits and underscores; anything else becomes an underscore.
pub fn slug(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    s.trim_matches('_').to_owned()
}

/// A tool name made of what a provider accepts: letters, digits, `_` and `-`.
fn clean(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The tools of every server that is on and reached, ready to offer.
pub fn specs(servers: &[Server]) -> Vec<ToolSpec> {
    servers
        .iter()
        .filter(|s| s.offers())
        .flat_map(|s| s.tools.iter().map(|t| spec(s, t)))
        .collect()
}

fn spec(server: &Server, tool: &Tool) -> ToolSpec {
    let from = format!("(From {}.)", server.name);
    ToolSpec {
        name: server.qualified(&tool.name),
        description: if tool.description.trim().is_empty() {
            from
        } else {
            format!("{} {from}", tool.description.trim())
        },
        schema: object_schema(&tool.schema),
    }
}

/// A model provider wants an object schema; a server that sends none, or something else, gets one
/// that takes nothing.
fn object_schema(schema: &Value) -> Value {
    if schema.get("type").and_then(Value::as_str) == Some("object") {
        schema.clone()
    } else {
        json!({"type": "object", "properties": {}})
    }
}

/// The server and tool a call's name stands for, if the name is one of theirs.
pub fn resolve<'a>(servers: &'a [Server], name: &str) -> Option<(&'a Server, &'a Tool)> {
    servers.iter().find_map(|s| {
        s.tools
            .iter()
            .find(|t| s.qualified(&t.name) == name)
            .map(|t| (s, t))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;

    pub fn server(name: &str, tools: &[&str]) -> Server {
        Server {
            id: format!("id-{name}"),
            name: name.to_owned(),
            transport: Transport::Http {
                url: "https://example.test/mcp".into(),
            },
            auth: Auth::None,
            enabled: true,
            status: Status::Reachable,
            detail: String::new(),
            tools: tools
                .iter()
                .map(|t| Tool {
                    name: (*t).to_owned(),
                    description: format!("Does {t}."),
                    schema: json!({"type": "object", "properties": {"q": {"type": "string"}}}),
                })
                .collect(),
        }
    }

    #[test]
    fn a_tool_is_named_for_its_server_in_characters_a_provider_accepts() {
        let s = server("Context 7!", &["resolve library.id"]);
        assert_eq!(
            s.qualified("resolve library.id"),
            "mcp__context_7__resolve_library_id"
        );
        let long = s.qualified(&"x".repeat(100));
        assert_eq!(long.len(), MAX_NAME);
    }

    #[test]
    fn only_servers_that_are_on_and_reached_offer_tools_and_names_resolve_back() {
        let mut a = server("docs", &["search"]);
        let mut b = server("wiki", &["ask", "read"]);
        assert_eq!(specs(&[a.clone(), b.clone()]).len(), 3);
        b.enabled = false;
        a.status = Status::Unreachable;
        assert!(specs(&[a.clone(), b.clone()]).is_empty());
        let both = [a, b];
        let (s, t) = resolve(&both, "mcp__wiki__read").unwrap();
        assert_eq!((s.name.as_str(), t.name.as_str()), ("wiki", "read"));
    }

    #[test]
    fn a_schema_that_is_not_an_object_is_replaced_by_one_that_takes_nothing() {
        assert_eq!(
            object_schema(&json!(true)),
            json!({"type": "object", "properties": {}})
        );
        let ok = json!({"type": "object", "properties": {"a": {}}});
        assert_eq!(object_schema(&ok), ok);
    }
}
