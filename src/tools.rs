//! The tools every agent is given. For now there is one: it lets the agent stop and ask you, rather
//! than guess. Its answer comes back as the tool's result, in a new run (so a question can wait as
//! long as you like, even across a restart).

use serde_json::{Value, json};

use crate::completion::ToolSpec;

pub const ASK: &str = "ask_user_question";
pub const SHELL: &str = "shell";
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
        description: "Run a shell command on the person's computer and get its output. It runs at once, with no approval, so keep commands few, focused and easy to read back to the person. It runs with `sh -c` in the agent's working folder (the person's home folder if none is set), cannot read input, and is ended after a time limit. Long output is cut. Do not use it for anything destructive unless they asked for exactly that.".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The command line to run."},
                "reason": {"type": "string", "description": "One short sentence on why, shown to the person with the command."},
                "timeout_secs": {"type": "integer", "description": "Time limit in seconds. Default 30, at most 120."}
            },
            "required": ["command"]
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

/// The registered tools an agent has chosen, in the register's order, then MCP servers' (`mcp`, as
/// [`crate::mcp::specs`] gives them). A chosen name that is no longer registered is ignored, and so
/// are TypeSafe's while TypeSafe is not connected (`typesafe`).
pub fn offered(chosen: &[String], typesafe: bool, mcp: &[ToolSpec]) -> Vec<ToolSpec> {
    registered()
        .into_iter()
        .chain(mcp.iter().cloned())
        .filter(|t| chosen.contains(&t.name) && (typesafe || !needs_typesafe(&t.name)))
        .collect()
}

/// A tool that runs in the background and ends later: a command, a workflow, a question put to
/// TypeSafe, or a call to an MCP server.
pub fn is_job(name: &str) -> bool {
    name == SHELL || name == RUN_WORKFLOW || needs_typesafe(name) || crate::mcp::is_tool(name)
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
        n if crate::files::is_file_tool(n) => crate::files::path_of(args),
        DELEGATE => crate::delegate::summary(args),
        RUN_WORKFLOW => serde_json::from_str::<Value>(args)
            .ok()
            .and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| args.to_owned()),
        TEACH => "Save this as a workflow".to_owned(),
        crate::schedule::tool::NAME => crate::schedule::tool::summary(args),
        n if needs_typesafe(n) => crate::typesafe::summary(args),
        _ => args.to_owned(),
    }
}

pub fn knows(name: &str) -> bool {
    name == DELEGATE || registered().iter().any(|t| t.name == name)
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
    fn an_agent_is_offered_only_the_registered_tools_it_chose() {
        let names = |chosen: &[&str]| -> Vec<String> {
            let chosen: Vec<String> = chosen.iter().map(|c| (*c).to_owned()).collect();
            offered(&chosen, false, &[])
                .into_iter()
                .map(|t| t.name)
                .collect()
        };
        assert!(names(&[]).is_empty(), "no tools by default");
        assert_eq!(names(&[ASK]), [ASK.to_owned()]);
        assert!(
            names(&["shell.run"]).is_empty(),
            "an unregistered name is ignored"
        );
        assert!(knows(ASK) && !knows("shell.run"));
    }

    #[test]
    fn typesafe_tools_are_offered_only_while_typesafe_is_connected() {
        let chosen: Vec<String> = vec![ASK.into(), crate::typesafe::YES_NO.into()];
        let names = |on| -> Vec<String> {
            offered(&chosen, on, &[])
                .into_iter()
                .map(|t| t.name)
                .collect()
        };
        assert_eq!(names(false), [ASK]);
        assert_eq!(names(true), [ASK, crate::typesafe::YES_NO]);
        assert!(is_job(crate::typesafe::SCORE) && knows(crate::typesafe::CHOOSE));
    }
}
