//! Workflows to try the builder on. Each is small, and between them they use every kind of step.
//! They only run harmless commands (`echo`, `ls`, `sleep`, `date`) and change nothing on the
//! computer, so they are safe to run, edit and break.

use serde_json::{Value, json};

use super::definition::Definition;

/// One example: what it is called and for, and its steps.
pub struct Example {
    pub name: &'static str,
    pub goal: &'static str,
    pub definition: Definition,
}

fn def(v: Value) -> Definition {
    // The definitions below are written here and checked by this module's tests.
    serde_json::from_value(v).unwrap_or_default()
}

fn sh(id: &str, command: &str, bind: Option<&str>) -> Value {
    let mut step = json!({"id": id, "kind": "tool", "tool": "shell", "args": {"command": command}});
    if let Some(b) = bind {
        step["bind"] = json!(b);
    }
    step
}

/// The examples, in the order they are listed. Where one runs another it is by the name here.
pub fn all() -> Vec<Example> {
    vec![
        Example {
            name: "Example: sort files (dry run)",
            goal: "Goes through a folder and says where each file would go. Branches by extension; moves nothing.",
            definition: def(json!({
                "inputs": {"folder": {"type": "string", "default": ".", "description": "The folder to look at"}},
                "steps": [
                    sh("list", "ls -1 {{inputs.folder}}", Some("files")),
                    {"id": "each", "kind": "for_each", "over": "files.stdout.trimmed().split(\"\\n\")", "as": "name",
                     "on_error": "continue", "bind": "decided", "steps": [
                        {"id": "kind", "kind": "switch", "on": "name.split(\".\").pop()",
                         "cases": [
                            {"when": "\"md\"", "steps": [sh("notes", "echo '{{name}} would go to notes/'", None)]},
                            {"when": "\"txt\"", "steps": [sh("text", "echo '{{name}} would go to text/'", None)]}
                         ],
                         "default": [sh("other", "echo '{{name}} would stay where it is'", None)]}
                    ]},
                    {"id": "count", "kind": "set", "bind": "summary",
                     "values": {"files": "files.stdout.trimmed().split(\"\\n\").len()", "looked_at": "decided.len()"}}
                ],
                "success": "Every file was looked at."
            })),
        },
        Example {
            name: "Example: retry and recover",
            goal: "Tries something that fails about two times in three, tries again, and says what happened either way.",
            definition: def(json!({
                "steps": [
                    {"id": "try", "kind": "try", "retries": 3, "backoff_ms": 400, "bind": "result", "steps": [
                        sh("flaky",
                           "if [ $(( $(date +%s%N) / 1000 % 3 )) -eq 0 ]; then echo worked; else echo 'not this time' >&2; exit 1; fi",
                           None)
                     ],
                     "catch": [
                        {"id": "gave-up", "kind": "set", "bind": "note", "values": {"note": "\"gave up: \" + error"}}
                     ]},
                    {"id": "say", "kind": "code", "bind": "said",
                     "script": "if result.contains(\"stdout\") { \"it worked: \" + result.stdout.trimmed() } else { result.note }"}
                ],
                "success": "It said what happened."
            })),
        },
        Example {
            name: "Example: three things at once",
            goal: "Runs three one-second jobs side by side, so the whole thing takes about one second, not three.",
            definition: def(json!({
                "steps": [
                    {"id": "all", "kind": "parallel", "bind": "got", "branches": [
                        {"name": "first", "steps": [sh("a", "sleep 1; echo first", None)]},
                        {"name": "second", "steps": [sh("b", "sleep 1; echo second", None)]},
                        {"name": "third", "steps": [sh("c", "sleep 1; echo third", None)]}
                    ]},
                    {"id": "join", "kind": "code", "bind": "joined",
                     "script": "got.first.stdout.trimmed() + \", \" + got.second.stdout.trimmed() + \", \" + got.third.stdout.trimmed()"}
                ],
                "success": "All three came back."
            })),
        },
        Example {
            name: "Example: ask before it goes",
            goal: "Does some work, waits a few seconds, then stops to ask you before the last step. It keeps waiting if the app is closed.",
            definition: def(json!({
                "steps": [
                    sh("prepare", "echo report ready", Some("prepared")),
                    {"id": "nap", "kind": "wait", "seconds": 3},
                    {"id": "ask", "kind": "wait", "approve": "Send it? The last step said: {{prepared.stdout}}"},
                    sh("send", "echo sent", Some("sent"))
                ],
                "success": "It was sent after you said yes."
            })),
        },
        Example {
            name: "Example: model steps",
            goal: "A model decides one thing and writes one thing; the rest is fixed. The two steps that use a model are drawn dashed.",
            definition: def(json!({
                "inputs": {"text": {"type": "string", "default": "The quick brown fox jumps over the lazy dog.",
                                    "description": "Something to look at"}},
                "steps": [
                    {"id": "judge", "kind": "decide", "question": "Is this text about an animal?",
                     "options": ["yes", "no"], "input": "{{inputs.text}}", "bind": "animal"},
                    {"id": "branch", "kind": "if", "cond": "animal == \"yes\"", "bind": "line",
                     "then": [{"id": "praise", "kind": "generate",
                               "instruction": "Write one friendly sentence about the animals in: {{inputs.text}}"}],
                     "else": [sh("shrug", "echo no animals here", None)]}
                ],
                "success": "It said something about the text."
            })),
        },
        Example {
            name: "Example: greet",
            goal: "Says hello to one person. The next example runs this one.",
            definition: def(json!({
                "inputs": {"who": {"type": "string", "default": "friend", "description": "Who to greet"}},
                "steps": [sh("hello", "echo hello {{inputs.who}}", Some("said"))]
            })),
        },
        Example {
            name: "Example: greet three",
            goal: "Runs the Example: greet workflow once for each of three people.",
            definition: def(json!({
                "steps": [
                    {"id": "each", "kind": "for_each", "over": "[\"Ann\", \"Bo\", \"Cy\"]", "as": "person", "bind": "all",
                     "steps": [{"id": "call", "kind": "flow", "workflow": "Example: greet",
                                "inputs": {"who": "{{person}}"}, "bind": "greeting"}]},
                    {"id": "list", "kind": "code", "bind": "names",
                     "script": "let out = []; for g in all { out.push(g.stdout.trimmed()); } out"}
                ],
                "success": "All three were greeted."
            })),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::definition::{each_step, flows_used, validate};

    #[test]
    fn every_example_is_a_good_workflow_and_between_them_they_use_every_kind_of_step() {
        let all = all();
        assert!(all.len() >= 6);
        let mut kinds = std::collections::BTreeSet::new();
        for e in &all {
            assert_eq!(validate(&e.definition), Ok(()), "{}", e.name);
            assert!(
                !e.goal.is_empty() && e.name.starts_with("Example: "),
                "{}",
                e.name
            );
            each_step(&e.definition.steps, &mut |s| {
                kinds.insert(s.kind.clone());
            });
        }
        for kind in [
            "tool", "generate", "decide", "if", "switch", "for_each", "set", "try", "flow", "code",
            "wait", "parallel",
        ] {
            assert!(kinds.contains(kind), "no example has a {kind} step");
        }
        // `while` and `delegate` need a loop that ends on something and a team: the builder's
        // own Add step list is the way to try them.
    }

    #[test]
    fn the_names_are_their_own_and_a_workflow_an_example_runs_is_another_example() {
        let all = all();
        let names: Vec<&str> = all.iter().map(|e| e.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
        for e in &all {
            for called in flows_used(&e.definition) {
                assert!(
                    names.iter().any(|n| n.eq_ignore_ascii_case(&called)),
                    "{} runs {called:?}, which is not an example",
                    e.name
                );
            }
        }
    }

    #[test]
    fn an_example_runs_nothing_that_changes_the_computer() {
        for e in all() {
            each_step(&e.definition.steps, &mut |s| {
                if s.kind == "tool" && s.tool.as_deref() != Some("shell") {
                    panic!("{}: {:?} is not a plain shell command", e.name, s.tool);
                }
                let cmd = s
                    .args
                    .as_ref()
                    .and_then(|a| a.get("command"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                for bad in [
                    "rm ", "mv ", "cp ", "> ", ">>", "chmod", "sudo", "curl", "git ",
                ] {
                    assert!(!cmd.contains(bad), "{}: {cmd:?} has {bad:?}", e.name);
                }
            });
        }
    }
}
