//! The `mcp_servers` tool: what an agent may ask about MCP servers, and reading its arguments.
//! What the app does with a call (drafts the person approves) is in `super::manage`.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::flow::split_words;
use super::{Auth, Transport, slug};
use crate::completion::ToolSpec;

pub const NAME: &str = "mcp_servers";

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: NAME.into(),
        description: "Manage the MCP servers registered in this app: programs that give agents more tools. `list` shows them at once, with their tools (never their keys or variable values). `add`, `update` and `remove` show the person a draft and take effect only once they approve it; say so, and stop there. A local server is a command started on this computer (`command` and `args`); a remote one is an address (`url`). Servers are shared by every agent; once one is added, its tools are given to an agent in its settings, or with the `agents` tool by the names `list` shows (mcp__server__tool). A server's name cannot be changed: remove it and add it again. Use absolute paths in commands and arguments.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "add", "update", "remove"]},
                "name": {"type": "string", "description": "The server's name: for add, a new one; otherwise one from `list`."},
                "url": {"type": "string", "description": "A remote server's address, https://..."},
                "command": {"type": "string", "description": "A local server's program, such as npx or an absolute path."},
                "args": {"type": "array", "items": {"type": "string"}, "description": "The program's arguments, one per item."},
                "env": {"type": "object", "additionalProperties": {"type": ["string", "null"]}, "description": "Environment variables a local server is started with, by name. For update, only those given change; null removes one."},
                "auth": {"type": "string", "enum": ["none", "key", "oauth"], "description": "How a remote server knows the person: no sign-in, a key sent as a bearer token, or a sign-in in the browser. Default none, or key when `key` is given."},
                "key": {"type": "string", "description": "With auth key: the key. Only one the person gave you."}
            },
            "required": ["action"]
        }),
    }
}

/// A line saying what a call does, for lists: "add: GitHub".
pub fn summary(args: &str) -> String {
    let v: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let part = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    match (part("action"), part("name")) {
        (Some(action), Some(name)) => format!("{action}: {name}"),
        (Some(action), None) => action.to_owned(),
        _ => args.to_owned(),
    }
}

/// What a call asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    List,
    Add(New),
    Update { name: String, change: Change },
    Remove(String),
}

/// A server to add, as the call gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct New {
    pub name: String,
    pub transport: Transport,
    pub auth: Auth,
    pub key: Option<String>,
    pub env: BTreeMap<String, String>,
}

/// What an update changes; `None` keeps what the server has.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Change {
    pub url: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub auth: Option<Auth>,
    pub key: Option<String>,
    /// A variable to set, or with `None` to remove.
    pub env: BTreeMap<String, Option<String>>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        *self == Change::default()
    }
}

/// Reads a call's arguments. Errs, in words the model can act on.
pub fn parse(args: &str) -> Result<Call, String> {
    let v: Value = serde_json::from_str(args).map_err(|_| "the arguments are not JSON")?;
    let text = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let name = || {
        let name = text("name").ok_or("give the server's `name`")?;
        if slug(&name).is_empty() {
            return Err("the `name` needs at least one letter or digit".to_owned());
        }
        Ok(name)
    };
    let url = text("url").map(|u| checked_url(&u)).transpose()?;
    let command = text("command");
    let args_ = arguments(v.get("args"))?;
    let key = text("key");
    let auth = match text("auth").as_deref() {
        None => None,
        Some(a @ ("none" | "key" | "oauth")) => Some(Auth::parse(a)),
        Some(other) => return Err(format!("there is no auth {other:?}: none, key or oauth")),
    };
    match text("action").as_deref() {
        Some("list") => Ok(Call::List),
        Some("add") => {
            let name = name()?;
            let auth = auth.unwrap_or(if key.is_some() { Auth::Key } else { Auth::None });
            let transport = match (url, command) {
                (Some(url), None) => Transport::Http { url },
                (None, Some(command)) => {
                    if auth != Auth::None {
                        return Err("a local server takes no `auth`: give it `env` instead".into());
                    }
                    Transport::Stdio {
                        command,
                        args: args_.unwrap_or_default(),
                    }
                }
                (Some(_), Some(_)) => return Err("give a `url` or a `command`, not both".into()),
                (None, None) => {
                    return Err("give a remote server's `url` or a local `command`".into());
                }
            };
            if auth == Auth::Key && key.is_none() {
                return Err("auth key needs the `key`".into());
            }
            let env = env(v.get("env"))?
                .into_iter()
                .filter_map(|(k, v)| v.map(|v| (k, v)))
                .collect();
            Ok(Call::Add(New {
                name,
                transport,
                auth,
                key: key.filter(|_| auth == Auth::Key),
                env,
            }))
        }
        Some("update") => {
            let change = Change {
                url,
                command,
                args: args_,
                auth,
                key,
                env: env(v.get("env"))?,
            };
            if change.is_empty() {
                return Err("say what to change: url, command, args, env, auth or key".into());
            }
            Ok(Call::Update {
                name: name()?,
                change,
            })
        }
        Some("remove") => Ok(Call::Remove(name()?)),
        Some(other) => Err(format!("there is no action {other:?}")),
        None => Err("give an `action`".into()),
    }
}

fn checked_url(url: &str) -> Result<String, String> {
    if url.starts_with("https://") || url.starts_with("http://") {
        Ok(url.to_owned())
    } else {
        Err(format!(
            "{url:?} is not an address: it should start with https://"
        ))
    }
}

/// `args` as a list; a single string is split as on a command line.
fn arguments(v: Option<&Value>) -> Result<Option<Vec<String>>, String> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => split_words(s).map(Some),
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| {
                i.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "every item of `args` must be text".to_owned())
            })
            .collect::<Result<_, _>>()
            .map(Some),
        Some(_) => Err("`args` must be a list of text".into()),
    }
}

fn env(v: Option<&Value>) -> Result<BTreeMap<String, Option<String>>, String> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(BTreeMap::new());
    };
    let map = v
        .as_object()
        .ok_or("`env` must be an object of NAME: value")?;
    map.iter()
        .map(|(k, val)| {
            if k.is_empty() || k.contains('=') {
                return Err(format!("{k:?} is not a variable name"));
            }
            let val = match val {
                Value::Null => None,
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => return Err(format!("the value of {k} must be text")),
            };
            Ok((k.clone(), val))
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_local_server_is_a_command_with_its_arguments_and_variables() {
        let call = parse(
            r#"{"action":"add","name":"TTS","command":"/bin/tts-mcp","args":["--fast","a b"],"env":{"LANG":"en","N":2}}"#,
        )
        .unwrap();
        let Call::Add(n) = call else {
            panic!("{call:?}")
        };
        assert_eq!(n.name, "TTS");
        assert_eq!(
            n.transport,
            Transport::Stdio {
                command: "/bin/tts-mcp".into(),
                args: vec!["--fast".into(), "a b".into()]
            }
        );
        assert_eq!(n.auth, Auth::None);
        assert_eq!(n.env.get("N").map(String::as_str), Some("2"));
        assert!(
            matches!(parse(r#"{"action":"add","name":"x","command":"npx","args":"-y \"@a/b c\""}"#).unwrap(),
                Call::Add(New { transport: Transport::Stdio { args, .. }, .. }) if args == ["-y", "@a/b c"]),
            "a string of arguments is split as on a command line"
        );
    }

    #[test]
    fn a_remote_server_is_an_address_and_a_key_means_key_sign_in() {
        let Call::Add(n) =
            parse(r#"{"action":"add","name":"Docs","url":"https://x.dev/mcp","key":"k1"}"#)
                .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            n.transport,
            Transport::Http {
                url: "https://x.dev/mcp".into()
            }
        );
        assert_eq!((n.auth, n.key.as_deref()), (Auth::Key, Some("k1")));
    }

    #[test]
    fn what_cannot_be_added_is_refused_in_words() {
        for (args, said) in [
            (
                r#"{"action":"add","name":"x"}"#,
                "`url` or a local `command`",
            ),
            (
                r#"{"action":"add","name":"x","url":"https://a","command":"b"}"#,
                "not both",
            ),
            (r#"{"action":"add","name":"x","url":"ftp://a"}"#, "https://"),
            (
                r#"{"action":"add","name":"x","url":"https://a","auth":"key"}"#,
                "needs the `key`",
            ),
            (
                r#"{"action":"add","name":"x","command":"a","auth":"oauth"}"#,
                "takes no `auth`",
            ),
            (
                r#"{"action":"add","name":"--","command":"a"}"#,
                "letter or digit",
            ),
            (r#"{"action":"add","command":"a"}"#, "`name`"),
            (r#"{"action":"update","name":"x"}"#, "say what to change"),
            (
                r#"{"action":"add","name":"x","command":"a","env":{"A=B":"c"}}"#,
                "not a variable name",
            ),
            (r#"{"action":"fly"}"#, "no action"),
        ] {
            let why = parse(args).unwrap_err();
            assert!(why.contains(said), "{args}: {why}");
        }
    }

    #[test]
    fn an_update_keeps_what_it_does_not_name_and_null_removes_a_variable() {
        let call = parse(r#"{"action":"update","name":"TTS","env":{"A":"1","B":null}}"#).unwrap();
        let Call::Update { name, change } = call else {
            panic!()
        };
        assert_eq!(name, "TTS");
        assert_eq!(change.command, None);
        assert_eq!(change.env.get("A"), Some(&Some("1".into())));
        assert_eq!(change.env.get("B"), Some(&None));
        assert_eq!(
            parse(r#"{"action":"remove","name":"TTS"}"#).unwrap(),
            Call::Remove("TTS".into())
        );
        assert_eq!(
            summary(r#"{"action":"remove","name":"TTS"}"#),
            "remove: TTS"
        );
    }
}
