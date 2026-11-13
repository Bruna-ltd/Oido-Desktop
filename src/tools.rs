//! The app's own tools, which every agent has without choosing them (see [`offered`]), and how any
//! tool, the app's or another's, is shown to the person.

use serde_json::{Value, json};

use crate::completion::ToolSpec;

pub const ASK: &str = "ask_user_question";
pub const SHELL: &str = "shell";
pub const PROCESS: &str = "process";
pub const TEACH: &str = "teach_workflow";
pub const RUN_WORKFLOW: &str = "run_workflow";
pub const DELEGATE: &str = "delegate";

/// What is sent back when you skip a question.
pub const SKIPPED: &str = "The person skipped this question without answering. Carry on with your best judgement and say what you assumed.";

/// Every tool that is registered: what an agent can be given. All of these are built in; there is
/// no way yet to register your own, since a tool needs code that runs it.
pub fn registered() -> Vec<ToolSpec> {
    let mut all = vec![
        ask(),
        shell(),
        process(),
        ToolSpec {
            name: TEACH.into(),
            description: "Save what you did in this chat as a reusable workflow: fixed steps that can be run again without planning. Use it when the person asks to save or repeat what you just did. A draft is shown to them to confirm or change; nothing runs until they save it.".into(),
            schema: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: RUN_WORKFLOW.into(),
            description: "Run one of your saved workflows by name, with inputs if it takes any, and get its result. Use it when the person asks for something a saved workflow already does.".into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "The workflow's name."},
                    "inputs": {"type": "object", "description": "Values for the workflow's inputs, by name."}
                },
                "required": ["name"]
            }),
        },
        crate::schedule::tool::spec(),
        crate::mcp::tool::spec(),
        crate::agent_tool::spec(),
        crate::web::spec(),
        file_tool(
            crate::files::READ,
            "Read a text file in the working folder. Long files are cut; use `offset` (first line, from 1) and `limit` (number of lines) to read the rest in parts.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The file, relative to the working folder."},
                    "offset": {"type": "integer", "description": "First line to read, counting from 1."},
                    "limit": {"type": "integer", "description": "How many lines to read."}
                },
                "required": ["path"]
            }),
        ),
        file_tool(
            crate::files::WRITE,
            "Write a text file in the working folder, making any folders it needs. It replaces the whole file if it exists, so to change part of a file use edit_file.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The file, relative to the working folder."},
                    "content": {"type": "string", "description": "The whole new content."}
                },
                "required": ["path", "content"]
            }),
        ),
        file_tool(
            crate::files::EDIT,
            "Change part of an existing text file in the working folder by replacing exact text. `old_text` must match the file exactly, spaces and line breaks included, and must appear once (or set `replace_all`). Read the file first so it matches.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The file, relative to the working folder."},
                    "old_text": {"type": "string", "description": "The exact text to replace."},
                    "new_text": {"type": "string", "description": "What to put there instead."},
                    "replace_all": {"type": "boolean", "description": "Replace every occurrence, not just a single one."}
                },
                "required": ["path", "old_text", "new_text"]
            }),
        ),
    ];
    all.extend(crate::typesafe::specs());
    all
}

/// Tools that only work while TypeSafe is connected: they are not offered, or run, without it.
pub fn needs_typesafe(name: &str) -> bool {
    crate::typesafe::is_tool(name)
}

fn file_tool(name: &'static str, description: &'static str, schema: Value) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: description.into(),
        schema,
    }
}

fn shell() -> ToolSpec {
    ToolSpec {
        name: SHELL.into(),
        description: "Run a shell command on the person's computer and get its output. It runs at once, unless it looks risky (running as administrator, deleting outside the working folder, changing system or login files and the like), when it waits for the person's go-ahead; so keep commands few, focused and easy to read back to the person. It runs with `sh -c` in the agent's working folder (the person's home folder if none is set), cannot read input, and is ended after a time limit. Long output is cut. For something that should keep running, such as a dev server or a watcher, set `background` to true: it starts at once, without a time limit, and you follow it with the `process` tool. Give it the command alone, with no `&` or `nohup`. Do not use it for anything destructive unless they asked for exactly that.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The command line to run."},
                "reason": {"type": "string", "description": "One short sentence on why, shown to the person with the command."},
                "timeout_secs": {"type": "integer", "description": "Time limit in seconds. Default 30, at most 120. Ignored for a background command."},
                "background": {"type": "boolean", "description": "Start the command and carry on without waiting for it to end. It is ended when the person stops the chat or closes it, and when Oido quits."}
            },
            "required": ["command"]
        }),
    }
}

fn process() -> ToolSpec {
    ToolSpec {
        name: PROCESS.into(),
        description: "Follow the commands you started in the background with `shell`. `list` shows them and how each stands. `read` gives one's state and the end of what it printed, so call it again later to see more. `stop` ends it and everything it started. Only this chat's processes can be seen.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "read", "stop"], "description": "What to do."},
                "id": {"type": "string", "description": "The process, as `shell` or `list` named it, such as bg1. Not needed for list."}
            },
            "required": ["action"]
        }),
    }
}

fn ask() -> ToolSpec {
    ToolSpec {
        name: ASK.into(),
        description: "Ask the person a question and wait for the answer. Use it when you need a decision or a missing detail that you cannot reasonably work out, instead of guessing. Ask one clear question at a time, and offer options when the answer is a choice.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "question": {"type": "string", "description": "The question, in a sentence or two."},
                "options": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "2 to 4 short choices. The person can also write their own answer. Leave out for an open question."
                },
                "multiple": {"type": "boolean", "description": "True if more than one option can be chosen."}
            },
            "required": ["question"]
        }),
    }
}

/// Whether a tool is one of the app's own, which every agent has without choosing it. TypeSafe's
/// and MCP servers' tools are chosen per agent.
pub fn is_builtin(name: &str) -> bool {
    !needs_typesafe(name) && registered().iter().any(|t| t.name == name)
}

/// What an agent is offered: every built-in tool, then the TypeSafe and MCP tools it has chosen,
/// in the register's order, then MCP servers' (`mcp`, as [`crate::mcp::specs`] gives them). A
/// chosen name that is no longer registered is ignored, and so are TypeSafe's while TypeSafe is
/// not connected (`typesafe`).
pub fn offered(chosen: &[String], typesafe: bool, mcp: &[ToolSpec]) -> Vec<ToolSpec> {
    registered()
        .into_iter()
        .chain(mcp.iter().cloned())
        .filter(|t| {
            is_builtin(&t.name)
                || (chosen.contains(&t.name) && (typesafe || !needs_typesafe(&t.name)))
        })
        .collect()
}

/// A tool that runs in the background and ends later: a command, a workflow, a page fetched from
/// the web, a question put to TypeSafe, a call to an MCP server, or a file sent on Telegram.
pub fn is_job(name: &str) -> bool {
    name == SHELL
        || name == RUN_WORKFLOW
        || name == crate::web::NAME
        || needs_typesafe(name)
        || crate::mcp::is_tool(name)
        || name == crate::channels::SEND_FILE
}

/// Whether a tool waits for you (an answer, or a go-ahead) before its result exists.
pub fn needs_you(name: &str) -> bool {
    name == ASK || name == SHELL
}

/// A line saying what a call is, for lists.
pub fn summary(name: &str, args: &str) -> String {
    match name {
        ASK => question(args).text,
        SHELL => crate::command::parse(args).map_or_else(|| args.to_owned(), |s| s.command),
        PROCESS => process_summary(args),
        n if crate::files::is_file_tool(n) || n == crate::channels::SEND_FILE => {
            crate::files::path_of(args)
        }
        DELEGATE => crate::delegate::summary(args),
        RUN_WORKFLOW => serde_json::from_str::<Value>(args)
            .ok()
            .and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| args.to_owned()),
        TEACH => "Save this as a workflow".to_owned(),
        crate::schedule::tool::NAME => crate::schedule::tool::summary(args),
        crate::mcp::tool::NAME => crate::mcp::tool::summary(args),
        crate::agent_tool::NAME => crate::agent_tool::summary(args),
        crate::web::NAME => crate::web::summary(args),
        n if needs_typesafe(n) => crate::typesafe::summary(args),
        _ => args.to_owned(),
    }
}

/// "stop bg1", for a list.
fn process_summary(args: &str) -> String {
    let v: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let field = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    format!("{} {}", field("action"), field("id"))
        .trim()
        .to_owned()
}

/// The group the app's own tools are listed under.
pub const DEFAULTS: &str = "Defaults";
/// The group TypeSafe's tools are listed under.
pub const TYPESAFE: &str = "TypeSafe";

/// How a tool is shown to the person, apart from the name the model calls it by: a plain label, a
/// line on what it does, and the group it is listed under (the app's own, TypeSafe, or the MCP
/// server it comes from).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Shown {
    pub label: String,
    pub about: String,
    pub group: String,
}

/// How the tool called `name` is shown. An MCP tool is labelled from its server's name for it and
/// grouped under the server; a name nothing knows shows as itself.
pub fn shown(name: &str, servers: &[crate::mcp::Server]) -> Shown {
    if let Some((label, about)) = own(name) {
        let group = if needs_typesafe(name) {
            TYPESAFE
        } else {
            DEFAULTS
        };
        return Shown {
            label: label.to_owned(),
            about: about.to_owned(),
            group: group.to_owned(),
        };
    }
    match crate::mcp::resolve(servers, name) {
        Some((server, tool)) => Shown {
            label: humanize(&tool.name),
            about: first_sentence(&tool.description),
            group: server.name.clone(),
        },
        None => Shown {
            label: name.to_owned(),
            about: String::new(),
            group: DEFAULTS.to_owned(),
        },
    }
}

/// The label and line for one of the app's own tools, written for the person rather than the
/// model.
fn own(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        ASK => (
            "Ask you a question",
            "Stops to ask you for a decision instead of guessing.",
        ),
        SHELL => (
            "Run commands",
            "Runs shell commands on this computer; risky ones wait for your go-ahead.",
        ),
        PROCESS => (
            "Follow background commands",
            "Lists, reads and stops the commands it started in the background.",
        ),
        TEACH => (
            "Save as a workflow",
            "Turns what it just did into a workflow you can run again.",
        ),
        RUN_WORKFLOW => ("Run workflows", "Runs one of your saved workflows by name."),
        crate::schedule::tool::NAME => (
            "Schedule itself",
            "Sets prompts or workflows to run at set times; new ones need your approval.",
        ),
        crate::mcp::tool::NAME => (
            "Manage MCP servers",
            "Lists MCP servers, and drafts adding, changing or removing them for your approval.",
        ),
        crate::agent_tool::NAME => (
            "Manage agents",
            "Lists agents, and drafts creating, changing or deleting them for your approval.",
        ),
        crate::web::NAME => (
            "Fetch web pages",
            "Reads web pages and other text from the internet.",
        ),
        crate::files::READ => ("Read files", "Reads text files in its working folder."),
        crate::files::WRITE => (
            "Write files",
            "Creates or replaces files in its working folder.",
        ),
        crate::files::EDIT => (
            "Edit files",
            "Changes part of a file in its working folder.",
        ),
        crate::typesafe::YES_NO => (
            "Yes or no check",
            "Asks TypeSafe whether something is true of a text or image.",
        ),
        crate::typesafe::CHOOSE => (
            "Pick an option",
            "Asks TypeSafe which option fits a text or image best.",
        ),
        crate::typesafe::SCORE => (
            "Rate on a scale",
            "Asks TypeSafe to score a text or image on a scale you describe.",
        ),
        crate::typesafe::LOCATE => (
            "Find in an image",
            "Asks TypeSafe where something is in an image, such as a button in a screenshot.",
        ),
        _ => return None,
    })
}

/// A server's tool name as words: "send_email" and "sendEmail" become "Send email". A word in
/// capitals ("URL") stays so.
fn humanize(name: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_alphanumeric() {
            words.extend((!word.is_empty()).then(|| std::mem::take(&mut word)));
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower {
            words.push(std::mem::take(&mut word));
        }
        // The capital before a lowercase letter starts a new word: "APIKeys" is "API" "Keys".
        if c.is_lowercase() && word.chars().count() > 1 && word.chars().all(char::is_uppercase) {
            let last = word.pop();
            words.push(std::mem::take(&mut word));
            word.extend(last);
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        word.push(c);
    }
    words.extend((!word.is_empty()).then_some(word));
    let mut out = words
        .iter()
        .map(|w| {
            if w.len() > 1 && w.chars().all(|c| !c.is_lowercase()) {
                w.clone()
            } else {
                w.to_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(first) = out.chars().next() {
        out.replace_range(..first.len_utf8(), &first.to_uppercase().to_string());
    }
    if out.is_empty() { name.to_owned() } else { out }
}

/// The first sentence of a description, on one line.
fn first_sentence(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match text.find(". ") {
        Some(end) => text[..=end].to_owned(),
        None => text,
    }
}

pub fn knows(name: &str) -> bool {
    // `send_file` is offered only in chats carried on from Telegram; elsewhere it says so.
    name == DELEGATE
        || name == crate::channels::SEND_FILE
        || registered().iter().any(|t| t.name == name)
}

/// A question as the model wrote it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Question {
    pub text: String,
    pub options: Vec<String>,
    pub multiple: bool,
}

/// Reads the arguments of a call. A model that wrote something else still gets its words shown.
pub fn question(args: &str) -> Question {
    let v: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let text = v
        .get("question")
        .and_then(Value::as_str)
        .map_or_else(|| args.trim().to_owned(), |q| q.trim().to_owned());
    let options = v
        .get("options")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|o| o.trim().to_owned())
                .filter(|o| !o.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Question {
        text,
        options,
        multiple: v.get("multiple").and_then(Value::as_bool).unwrap_or(false),
    }
}

/// The answer to send: the options chosen, then anything typed.
pub fn answer(picks: &[String], typed: &str) -> String {
    let mut parts: Vec<&str> = picks.iter().map(String::as_str).collect();
    let typed = typed.trim();
    if !typed.is_empty() {
        parts.push(typed);
    }
    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_is_read_from_its_arguments_and_tolerates_odd_ones() {
        let q = question(r#"{"question":" Which? ","options":["A"," ","B"],"multiple":true}"#);
        assert_eq!(
            q,
            Question {
                text: "Which?".into(),
                options: vec!["A".into(), "B".into()],
                multiple: true
            }
        );
        assert_eq!(question("not json").text, "not json");
        assert!(question(r#"{"question":"Why?"}"#).options.is_empty());
    }

    #[test]
    fn an_answer_joins_what_was_chosen_and_typed() {
        assert_eq!(
            answer(&["A".into(), "B".into()], "  also C "),
            "A; B; also C"
        );
        assert_eq!(answer(&[], " yes "), "yes");
        assert_eq!(answer(&[], " "), "");
    }

    #[test]
    fn every_agent_is_offered_the_built_in_tools_and_nothing_unregistered() {
        let names = |chosen: &[&str]| -> Vec<String> {
            let chosen: Vec<String> = chosen.iter().map(|c| (*c).to_owned()).collect();
            offered(&chosen, false, &[])
                .into_iter()
                .map(|t| t.name)
                .collect()
        };
        let builtin: Vec<String> = registered()
            .into_iter()
            .map(|t| t.name)
            .filter(|n| is_builtin(n))
            .collect();
        assert!(builtin.contains(&SHELL.to_owned()) && builtin.contains(&ASK.to_owned()));
        assert_eq!(names(&[]), builtin, "without choosing any");
        assert_eq!(
            names(&["shell.run"]),
            builtin,
            "an unregistered name is ignored"
        );
        assert!(!is_builtin(crate::typesafe::YES_NO) && !is_builtin("mcp__x__y"));
        assert!(knows(ASK) && !knows("shell.run"));
    }

    #[test]
    fn every_registered_tool_has_a_plain_label_and_group() {
        for t in registered() {
            let s = shown(&t.name, &[]);
            assert_ne!(s.label, t.name, "{} has no label", t.name);
            assert!(!s.about.is_empty(), "{} has no line", t.name);
        }
        assert_eq!(shown(ASK, &[]).group, DEFAULTS);
        assert_eq!(shown(crate::typesafe::SCORE, &[]).group, TYPESAFE);
    }

    #[test]
    fn an_mcp_tool_is_labelled_in_words_and_grouped_under_its_server() {
        let mut server = crate::mcp::tests::server("Resend", &["send_email"]);
        server.tools[0].description = "Send an email.  Needs a verified domain.".into();
        let s = shown(&server.qualified("send_email"), &[server]);
        assert_eq!(
            s,
            Shown {
                label: "Send email".into(),
                about: "Send an email.".into(),
                group: "Resend".into(),
            }
        );
        assert_eq!(humanize("listAPIKeys"), "List API keys");
        assert_eq!(humanize("get-URL"), "Get URL");
        assert_eq!(humanize("search"), "Search");
        assert_eq!(shown("mystery", &[]).label, "mystery");
    }

    #[test]
    fn typesafe_tools_are_offered_only_while_typesafe_is_connected() {
        let chosen: Vec<String> = vec![ASK.into(), crate::typesafe::YES_NO.into()];
        let names = |on| -> Vec<String> {
            offered(&chosen, on, &[])
                .into_iter()
                .map(|t| t.name)
                .filter(|n| !is_builtin(n))
                .collect()
        };
        assert!(names(false).is_empty());
        assert_eq!(names(true), [crate::typesafe::YES_NO]);
        assert!(is_job(crate::typesafe::SCORE) && knows(crate::typesafe::CHOOSE));
    }
}
