//! Extensions: tool sets built into the app, for services that have no MCP server worth using
//! (Gmail, and later Calendar and Drive). To an agent an extension is a server like any other
//! (`mcp__gmail__search`); to the person it is a switch on the Extensions page. Nothing runs on
//! anyone's servers: each call goes from this computer straight to the service's own API, signed
//! in as the person, with a client they made there.
//!
//! An extension is code, so the list is fixed ([`ALL`]) and each one's tools are known without
//! asking anyone. What is saved is only whether one is on, which of its tools are off, and the
//! account (see [`google`]).

mod calendar;
mod drive;
mod flow;
mod gmail;
pub mod google;
mod marks;

use serde_json::Value;

use crate::mcp::Tool;
use crate::mcp::catalog::Logo;

pub use flow::{AccountView, Extensions};
pub use google::Failure;
pub use marks::GOOGLE as GOOGLE_LOGO;

/// What a tool does to the person's data, said on the page beside its switch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    Reads,
    Changes,
}

impl Access {
    pub fn label(self) -> &'static str {
        match self {
            Self::Reads => "reads",
            Self::Changes => "changes things",
        }
    }
}

/// One tool of an extension.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub access: Access,
    /// JSON Schema of the arguments.
    pub schema: Value,
}

impl ToolDef {
    pub fn tool(&self) -> Tool {
        Tool {
            name: self.name.to_owned(),
            description: self.description.to_owned(),
            schema: self.schema.clone(),
        }
    }
}

pub trait Extension: Sync {
    /// The name its server goes by: it is in every tool name, so tools never move to another.
    fn title(&self) -> &'static str;
    /// Its picture.
    fn logo(&self) -> Logo;
    /// What it gives, in a line.
    fn blurb(&self) -> &'static str;
    /// What Google must allow for its tools to work.
    fn scopes(&self) -> &'static [&'static str];
    fn tools(&self) -> Vec<ToolDef>;
    /// Runs one tool and says what happened in words for the model.
    fn call(&self, tool: &str, args: &Value, api: &google::Api) -> Result<String, Failure>;
}

/// Every extension, in the order the page lists them.
pub const ALL: &[&dyn Extension] = &[&gmail::Gmail, &calendar::Calendar, &drive::Drive];

/// What a server's id starts with when the server is an extension's.
pub const SERVER_PREFIX: &str = "ext:";

/// The extension a server id names.
pub fn find(server_id: &str) -> Option<&'static dyn Extension> {
    let id = server_id.strip_prefix(SERVER_PREFIX)?;
    ALL.iter().copied().find(|e| id_of(*e) == id)
}

/// An extension's id: its title as a tool name has it.
pub fn id_of(e: &dyn Extension) -> String {
    crate::mcp::slug(e.title())
}

/// The id of the server that stands for `e`.
pub fn server_id(e: &dyn Extension) -> String {
    format!("{SERVER_PREFIX}{}", id_of(e))
}
