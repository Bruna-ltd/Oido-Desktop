//! Taint: text from outside (a web page, an MCP tool's result, a message from a channel person not
//! marked trusted, what a command or file says in a folder not trusted) can carry instructions meant
//! to steer the agent. While any of it is still in what a model is sent, every call the model makes
//! that acts (see [`acts`]) waits for the person's go-ahead.
//!
//! Taint is worked out for each model call from what is actually sent, so it lasts as long as the
//! text does: trimmed to save room, summarized away (a summary of it stays tainted), or left behind
//! by Start fresh, it clears.

/// Why the result of the tool `name` is outside text, in words for a trip card, or `None` when it
/// is not. `trust_folder`: whether the agent's working folder counts as the person's own.
pub fn result_source(name: &str, trust_folder: bool) -> Option<&'static str> {
    if crate::channels::tools::says_own(name) {
        // How a send went, in the app's own words.
        None
    } else if name == crate::tools::SCRIPT {
        // What its steps read, web pages and servers among it, is in what it gives back.
        Some("a script's result")
    } else if name == crate::web::NAME || crate::connector::is_tool(name) {
        Some("a web page")
    } else if crate::mcp::is_tool(name) {
        Some("an MCP tool's result")
    } else if trust_folder {
        None
    } else if name == crate::files::READ {
        Some("a file it read")
    } else if name == crate::tools::SHELL || name == crate::tools::PROCESS {
        Some("a command's output")
    } else if name == crate::tools::RUN_WORKFLOW {
        Some("a workflow's result")
    } else {
        None
    }
}

/// Whether a call to the tool `name` acts: runs something, changes something, or sends something
/// away. Tools that only draft a change (schedules, agents, MCP servers, workflows to save) are
/// left out: what they draft already waits for the person's approval. So are the browser's: when
/// they wait is the person's setting (see [`crate::connector::Approvals`]).
pub fn acts(name: &str) -> bool {
    name == crate::tools::SHELL
        || name == crate::files::WRITE
        || name == crate::files::EDIT
        || name == crate::web::NAME
        || name == crate::tools::RUN_WORKFLOW
        || name == crate::tools::SCRIPT
        || name == crate::tools::DELEGATE
        || crate::mcp::is_tool(name)
        || crate::typesafe::is_tool(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_pages_and_mcp_results_are_outside_text_and_a_trusted_folder_is_not() {
        assert_eq!(result_source(crate::web::NAME, true), Some("a web page"));
        assert!(result_source("mcp__mail__read", true).is_some());
        assert_eq!(result_source(crate::files::READ, true), None);
        assert_eq!(result_source(crate::tools::SHELL, true), None);
        assert_eq!(
            result_source(crate::files::READ, false),
            Some("a file it read")
        );
        assert_eq!(
            result_source(crate::tools::SHELL, false),
            Some("a command's output")
        );
        assert_eq!(
            result_source(crate::files::WRITE, false),
            None,
            "its own write"
        );
        assert_eq!(result_source(crate::tools::ASK, false), None);
    }

    #[test]
    fn calls_that_act_are_told_from_those_that_only_read_or_draft() {
        for acting in [
            crate::tools::SHELL,
            crate::files::WRITE,
            crate::files::EDIT,
            crate::web::NAME,
            "mcp__telegram__send_file",
            "mcp__whatsapp__send_message",
            crate::tools::DELEGATE,
            crate::tools::RUN_WORKFLOW,
            crate::tools::SCRIPT,
            "mcp__mail__send",
        ] {
            assert!(acts(acting), "{acting}");
        }
        for not in [
            crate::files::READ,
            crate::tools::ASK,
            crate::tools::TEACH,
            crate::agent_tool::NAME,
            crate::schedule::tool::NAME,
            crate::mcp::tool::NAME,
        ] {
            assert!(!acts(not), "{not}");
        }
    }
}
