//! What a run did to each step of a workflow, and what is wrong with each step, found from the run's
//! log and the editor's problems so the graph can draw them on the steps. Pure.
//!
//! A log line says where its step is by a path (`2.then.0`, `3.news.1`): the position in the top
//! list, then for each frame the name of the list in it and the position there. The editor names a
//! step by the lists it went through, counted by number. [`loc_of`] turns one into the other.

use std::collections::BTreeMap;

use super::definition::{FOR_EACH, IF, PARALLEL, SWITCH, TRY, WHILE};
use super::editor::{Editor, Loc, Node};
use super::engine::StepLog;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// It has begun and not ended.
    Running,
    Done,
    Failed,
    /// The run stopped here to wait for an answer or a delay.
    Waiting,
}

/// What a run did at one step, all its rounds together.
#[derive(Clone, PartialEq, Debug)]
pub struct NodeRun {
    pub state: State,
    /// How many times it ran: one for a step outside any loop.
    pub rounds: usize,
    /// What it gave last, as the log kept it.
    pub output: String,
    /// Why it failed, if a round did.
    pub error: Option<String>,
    pub ms: u64,
}

/// Which list of `node` a path names, as the editor numbers its lists.
fn block_of(node: &Node, name: &str) -> Option<usize> {
    let last = node.blocks.len().checked_sub(1)?;
    match (node.kind.as_str(), name) {
        (IF, "then") | (FOR_EACH | WHILE | TRY, "body") => Some(0),
        (IF, "else") | (TRY, "catch") => Some(1).filter(|n| *n <= last),
        (SWITCH, "default") => Some(last),
        (SWITCH, case) => case
            .strip_prefix("case")
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|n| *n < last),
        (PARALLEL, branch) => node
            .blocks
            .iter()
            .position(|b| b.when.text.trim() == branch),
        _ => None,
    }
}

/// The step in the editor a log path is, or none if the editor does not have one there: the path of
/// a step in another workflow (run by a `flow` step), or of one the editor has since changed.
pub fn loc_of(ed: &Editor, path: &str) -> Option<Loc> {
    let mut parts = path.split('.');
    let mut index: usize = parts.next()?.parse().ok()?;
    let mut list = &ed.steps;
    let mut place = Vec::new();
    loop {
        let node = list.get(index)?;
        let Some(name) = parts.next() else {
            return Some(Loc { place, index });
        };
        let block = block_of(node, name)?;
        place.push((index, block));
        list = &node.blocks.get(block)?.steps;
        index = parts.next()?.parse().ok()?;
    }
}

/// What the log says happened at each step it reaches. `waiting` is the id of the `wait` step the
/// run is stopped at, if it is.
pub fn runs_by_step(ed: &Editor, log: &[StepLog], waiting: Option<&str>) -> Vec<(Loc, NodeRun)> {
    let mut out: Vec<(Loc, NodeRun)> = Vec::new();
    for line in log {
        let Some(at) = loc_of(ed, &line.path) else {
            continue;
        };
        let state = if line.error.is_some() {
            State::Failed
        } else if line.running {
            State::Running
        } else if waiting == Some(line.id.as_str()) {
            State::Waiting
        } else {
            State::Done
        };
        match out.iter_mut().find(|(loc, _)| *loc == at) {
            Some((_, run)) => {
                run.rounds += 1;
                run.ms += line.ms;
                run.output = line.output.clone();
                // Failed outranks running, and running outranks done: the worst of its rounds.
                run.state = match (run.state, state) {
                    (State::Failed, _) | (_, State::Failed) => State::Failed,
                    (State::Waiting, _) | (_, State::Waiting) => State::Waiting,
                    (State::Running, _) | (_, State::Running) => State::Running,
                    _ => State::Done,
                };
                if run.error.is_none() {
                    run.error = line.error.clone();
                }
            }
            None => out.push((
                at,
                NodeRun {
                    state,
                    rounds: 1,
                    output: line.output.clone(),
                    error: line.error.clone(),
                    ms: line.ms,
                },
            )),
        }
    }
    out
}

/// The first problem of each step that has one, by the step's id. The editor's problems name the
/// step as `step 2.1 ("id"): what is wrong`; those that name no step (a missing name, an input) are
/// not here.
pub fn problems_by_id(problems: &[String]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for p in problems {
        let Some(rest) = p.strip_prefix("step ") else {
            continue;
        };
        let Some((_, after)) = rest.split_once("(\"") else {
            continue;
        };
        let Some((id, said)) = after.split_once("\")") else {
            continue;
        };
        let said = said.trim_start_matches(':').trim();
        out.entry(id.to_owned()).or_insert_with(|| said.to_owned());
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::workflow::Saved;

    fn editor() -> Editor {
        let tool =
            |id: &str| json!({"id": id, "kind": "tool", "tool": "shell", "args": {"command": "x"}});
        let w = Saved {
            id: "w".into(),
            agent_id: "a".into(),
            name: "F".into(),
            goal: String::new(),
            definition: serde_json::from_value(json!({"steps": [
                tool("a"),
                {"id": "i", "kind": "if", "cond": "true", "then": [tool("t")], "else": [tool("e")]},
                {"id": "e", "kind": "for_each", "over": "[1]", "steps": [tool("body")]},
                {"id": "s", "kind": "switch", "on": "1",
                 "cases": [{"when": "1", "steps": [tool("c0")]}, {"when": "2", "steps": [tool("c1")]}],
                 "default": [tool("d")]},
                {"id": "p", "kind": "parallel", "branches": [
                    {"name": "left", "steps": [tool("l")]}, {"name": "right", "steps": [tool("r")]}
                ]},
                {"id": "y", "kind": "try", "steps": [tool("tried")], "catch": [tool("caught")]},
                {"id": "f", "kind": "flow", "workflow": "Other", "bind": "o"}
            ]}))
            .unwrap(),
            created_at: 0,
        };
        Editor::from_saved(&w)
    }

    fn at(place: &[(usize, usize)], index: usize) -> Loc {
        Loc {
            place: place.to_vec(),
            index,
        }
    }

    fn line(path: &str, id: &str) -> StepLog {
        StepLog {
            id: id.into(),
            kind: "tool".into(),
            what: String::new(),
            output: format!("out-{id}"),
            error: None,
            ms: 5,
            path: path.into(),
            rounds: Vec::new(),
            running: false,
        }
    }

    #[test]
    fn a_log_path_is_the_step_the_editor_numbers_the_same_way_for_every_kind_of_list() {
        let ed = editor();
        let cases = [
            ("0", at(&[], 0)),
            ("1.then.0", at(&[(1, 0)], 0)),
            ("1.else.0", at(&[(1, 1)], 0)),
            ("2.body.0", at(&[(2, 0)], 0)),
            ("3.case0.0", at(&[(3, 0)], 0)),
            ("3.case1.0", at(&[(3, 1)], 0)),
            ("3.default.0", at(&[(3, 2)], 0)),
            ("4.left.0", at(&[(4, 0)], 0)),
            ("4.right.0", at(&[(4, 1)], 0)),
            ("5.body.0", at(&[(5, 0)], 0)),
            ("5.catch.0", at(&[(5, 1)], 0)),
            ("6", at(&[], 6)),
        ];
        for (path, want) in cases {
            assert_eq!(loc_of(&ed, path), Some(want), "{path}");
        }
        // Not there: another workflow's steps, a list or branch the step has not, past the end.
        for path in [
            "6.flow.0",
            "1.body.0",
            "3.case2.0",
            "4.middle.0",
            "9",
            "1.then.5",
            "",
            "x",
            "1.then",
        ] {
            assert_eq!(loc_of(&ed, path), None, "{path:?}");
        }
    }

    #[test]
    fn the_rounds_of_a_step_are_one_state_the_worst_of_them() {
        let ed = editor();
        let mut failed = line("2.body.0", "body");
        failed.error = Some("exit code 1".into());
        let log = [
            line("0", "a"),
            line("2.body.0", "body"),
            failed,
            line("2.body.0", "body"),
            line("6.flow.0", "inner"),
            line("1.then.0", "t"),
        ];
        let runs = runs_by_step(&ed, &log, None);
        let of = |l: Loc| runs.iter().find(|(x, _)| *x == l).map(|(_, r)| r.clone());
        let body = of(at(&[(2, 0)], 0)).unwrap();
        assert_eq!((body.state, body.rounds, body.ms), (State::Failed, 3, 15));
        assert_eq!(body.error.as_deref(), Some("exit code 1"));
        assert_eq!(body.output, "out-body", "what the last round gave");
        assert_eq!(of(at(&[], 0)).unwrap().state, State::Done);
        assert_eq!(runs.len(), 3, "the other workflow's line is not here");
        assert_eq!(
            of(at(&[(1, 1)], 0)),
            None,
            "a step that did not run has nothing"
        );
    }

    #[test]
    fn a_step_still_going_or_waiting_says_so() {
        let ed = editor();
        let mut going = line("1.then.0", "t");
        going.running = true;
        let ask = line("0", "a");
        let runs = runs_by_step(&ed, &[ask, going], Some("a"));
        let state = |l: Loc| runs.iter().find(|(x, _)| *x == l).unwrap().1.state;
        assert_eq!(state(at(&[], 0)), State::Waiting);
        assert_eq!(state(at(&[(1, 0)], 0)), State::Running);
    }

    #[test]
    fn a_problem_is_found_for_its_step_by_the_id_it_names() {
        let problems = vec![
            "The workflow needs a name.".to_owned(),
            "step 2.1 (\"t\"): a tool step needs a tool".to_owned(),
            "step 2.1 (\"t\"): a second thing".to_owned(),
            "step 3 (\"s\"): {{x}} refers to \"x\", which no earlier step binds".to_owned(),
            "input \"n\": the default is not a number".to_owned(),
            "step 4: the arguments are not JSON: oops".to_owned(),
        ];
        let by = problems_by_id(&problems);
        assert_eq!(by.len(), 2);
        assert_eq!(by["t"], "a tool step needs a tool", "the first one");
        assert!(by["s"].starts_with("{{x}} refers"));
    }
}
