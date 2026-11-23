//! The window a step opens in: what came before as a tree to pick names from, the step's own
//! boxes, and what it made when it ran. This is what the window needs from the app: the data to
//! show, and putting a name picked from a tree into a box. The screen is `ui/workflow_node.rs`.

use std::collections::HashSet;

use serde_json::Value;

use crate::app::App;
use crate::workflow::definition::{Field, FieldKind};
use crate::workflow::editor::{ArgKind, Loc, Refs, Root, Target, params_of, specs};
use crate::workflow::trace;

/// How many items of a list a tree shows.
const SHOWN: usize = 50;
/// How much of a value one row shows.
const PREVIEW: usize = 60;

/// One line of a data tree.
#[derive(Clone, PartialEq, Debug)]
pub struct Row {
    pub depth: usize,
    /// Tells it from the others, for turning it over.
    pub id: String,
    pub label: String,
    /// The way to it from the root: field names, and positions in lists.
    pub path: Vec<String>,
    pub preview: String,
    pub branch: bool,
    pub open: bool,
}

/// The lines of the tree under `root`: what `value` holds, or where it is not known (the step has
/// not run), what `fields` say it holds. The first level is open; deeper ones are until turned over.
pub fn rows(
    root: &str,
    value: Option<&Value>,
    fields: &[Field],
    flipped: &HashSet<String>,
) -> Vec<Row> {
    let mut out = Vec::new();
    match value {
        Some(v) => value_rows(root, v, &mut Vec::new(), flipped, &mut out),
        None => field_rows(root, fields, &mut Vec::new(), flipped, &mut out),
    }
    out
}

fn open_at(id: &str, depth: usize, flipped: &HashSet<String>) -> bool {
    (depth < 1) != flipped.contains(id)
}

fn value_rows(
    root: &str,
    value: &Value,
    path: &mut Vec<String>,
    flipped: &HashSet<String>,
    out: &mut Vec<Row>,
) {
    let children: Vec<(String, &Value)> = match value {
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Value::Array(items) => items
            .iter()
            .take(SHOWN)
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        _ => return,
    };
    let depth = path.len();
    for (name, v) in children {
        path.push(name.clone());
        let id = format!("{root}/{}", path.join("/"));
        let branch = matches!(v, Value::Object(m) if !m.is_empty())
            || matches!(v, Value::Array(a) if !a.is_empty());
        let open = branch && open_at(&id, depth, flipped);
        out.push(Row {
            depth,
            id,
            label: name,
            path: path.clone(),
            preview: preview(v),
            branch,
            open,
        });
        if open {
            value_rows(root, v, path, flipped, out);
        }
        path.pop();
    }
    if let Value::Array(items) = value
        && items.len() > SHOWN
    {
        out.push(Row {
            depth,
            id: format!("{root}/more"),
            label: format!("… {} more", items.len() - SHOWN),
            path: Vec::new(),
            preview: String::new(),
            branch: false,
            open: false,
        });
    }
}

fn field_rows(
    root: &str,
    fields: &[Field],
    path: &mut Vec<String>,
    flipped: &HashSet<String>,
    out: &mut Vec<Row>,
) {
    let depth = path.len();
    for f in fields {
        path.push(f.name.clone());
        let id = format!("{root}/{}", path.join("/"));
        let branch = !f.of.is_empty();
        let open = branch && open_at(&id, depth, flipped);
        out.push(Row {
            depth,
            id,
            label: f.name.clone(),
            path: path.clone(),
            preview: match f.kind {
                FieldKind::Text => "text",
                FieldKind::Number => "number",
                FieldKind::YesNo => "yes or no",
                FieldKind::List => "list",
            }
            .to_owned(),
            branch,
            open,
        });
        if open {
            // What a list's items hold is reached through one of them, the first.
            if f.kind == FieldKind::List {
                path.push("0".to_owned());
                field_rows(root, &f.of, path, flipped, out);
                path.pop();
            } else {
                field_rows(root, &f.of, path, flipped, out);
            }
        }
        path.pop();
    }
}

/// A value in a few words, for one line.
fn preview(v: &Value) -> String {
    match v {
        Value::Object(m) => format!("{} field{}", m.len(), if m.len() == 1 { "" } else { "s" }),
        Value::Array(a) => format!("{} item{}", a.len(), if a.len() == 1 { "" } else { "s" }),
        Value::String(s) => {
            let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
            match one.char_indices().nth(PREVIEW) {
                Some((i, _)) => format!("{}…", &one[..i]),
                None => one,
            }
        }
        Value::Null => "nothing".to_owned(),
        other => other.to_string(),
    }
}

/// How a root and a way into it is written in a box: `mail.messages.0.subject` in text, and
/// `mail.messages[0].subject` in an expression, where a position is not a name.
pub fn path_text(root: &str, path: &[String], expression: bool) -> String {
    let mut out = root.to_owned();
    for part in path {
        if part.bytes().all(|b| b.is_ascii_digit()) && expression {
            out.push_str(&format!("[{part}]"));
        } else {
            out.push('.');
            out.push_str(part);
        }
    }
    out
}

/// What a step made when it was run, as the window shows it.
#[derive(Clone, Debug)]
pub struct Made {
    pub value: Option<Value>,
    pub error: Option<String>,
    pub running: bool,
    /// "round 2" and how long it took.
    pub note: String,
}

impl App {
    /// What the step at `at` made in the run shown, or the last one tried.
    pub fn made_by(&self, at: &Loc) -> Option<Made> {
        let ed = self.wf_editor.as_ref()?;
        let line = self.shown_run().and_then(|r| {
            r.lines
                .iter()
                .rev()
                .find(|l| trace::loc_of(ed, &l.path).as_ref() == Some(at))
        });
        let kept = self.result_of(at);
        if line.is_none() && kept.is_none() {
            return None;
        }
        let value = kept.or_else(|| {
            let l = line.as_ref().filter(|l| l.error.is_none() && !l.running)?;
            Some(
                serde_json::from_str(&l.output).unwrap_or_else(|_| Value::String(l.output.clone())),
            )
        });
        let (error, running, note) = match &line {
            Some(l) => (
                l.error.clone(),
                l.running,
                [l.round_note(), Some(format!("{} ms", l.ms))]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · "),
            ),
            None => (None, false, String::new()),
        };
        Some(Made {
            value,
            error,
            running,
            note,
        })
    }

    /// Turns a branch of a data tree open or shut.
    pub fn flip_branch(&mut self, id: &str) {
        let flipped = &mut self.wf_graph.flipped;
        if !flipped.remove(id) {
            flipped.insert(id.to_owned());
        }
    }

    /// Puts what a row of the input tree stands for into the box of the step at `at` that has
    /// the caret, or else its first box that takes names.
    pub fn map_into(&mut self, at: &Loc, root: &Root, path: &[String]) {
        let Some(name) = self.wf_editor.as_mut().and_then(|ed| ed.root_name(root)) else {
            return;
        };
        let Some((target, refs)) = self.mapping_box(at) else {
            self.wf_mapper.notice = Some("This step has no box that takes names.".to_owned());
            return;
        };
        let text = path_text(&name, path, refs == Refs::Bare);
        self.editor_insert(&target, refs == Refs::Braces, &text);
        self.editor_text(&target).focused = true;
    }

    /// The box a picked name goes in: the one with the caret, else the first that takes names.
    fn mapping_box(&self, at: &Loc) -> Option<(Target, Refs)> {
        let ed = self.wf_editor.as_ref()?;
        let node = ed.node(at)?;
        let mut boxes: Vec<(Target, Refs, bool)> = Vec::new();
        for (i, spec) in specs(&node.kind).iter().enumerate() {
            let Some(edit) = node.fields.get(i) else {
                continue;
            };
            if spec.refs != Refs::None && !(spec.key == "cond" && node.when.is_some()) {
                boxes.push((Target::Field(at.clone(), i), spec.refs, edit.focused));
            }
        }
        let mut keys: Vec<String> = self
            .step_tool_schema(&node.tool)
            .map(|schema| {
                params_of(&schema)
                    .into_iter()
                    .filter(|p| p.kind == ArgKind::Text)
                    .map(|p| p.key)
                    .collect()
            })
            .unwrap_or_default();
        for (key, p) in &node.params {
            if p.kind == ArgKind::Text && !keys.contains(key) {
                keys.push(key.clone());
            }
        }
        for key in keys {
            let focused = node.params.get(&key).is_some_and(|p| p.edit.focused);
            boxes.push((
                Target::Param(at.clone(), key, ArgKind::Text),
                Refs::Braces,
                focused,
            ));
        }
        let pick = boxes.iter().position(|b| b.2).unwrap_or(0);
        boxes.into_iter().nth(pick).map(|(t, r, _)| (t, r))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_value_is_a_tree_whose_first_level_is_open_and_the_rest_opens_when_turned() {
        let v = json!({"messages": [{"from": "a", "subject": "Hi  there"}], "count": 1});
        let shut = HashSet::new();
        let r = rows("mail", Some(&v), &[], &shut);
        let labels: Vec<&str> = r.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["count", "messages", "0"]);
        assert_eq!(r[1].preview, "1 item");
        assert!(r[1].open && r[2].branch && !r[2].open);
        assert_eq!(r[2].path, ["messages", "0"]);
        // Turning the item over shows what is in it, with whitespace squeezed.
        let flipped: HashSet<String> = [r[2].id.clone()].into();
        let r = rows("mail", Some(&v), &[], &flipped);
        let labels: Vec<&str> = r.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["count", "messages", "0", "from", "subject"]);
        assert_eq!(r[4].preview, "Hi there");
        // Turning the first level over shuts it.
        let flipped: HashSet<String> = ["mail/messages".to_owned()].into();
        assert_eq!(rows("mail", Some(&v), &[], &flipped).len(), 2);
    }

    #[test]
    fn a_step_not_run_shows_the_fields_it_is_known_to_have_and_a_long_list_is_cut() {
        let fields = vec![Field {
            name: "messages".into(),
            kind: FieldKind::List,
            of: vec![Field {
                name: "from".into(),
                kind: FieldKind::Text,
                of: Vec::new(),
            }],
        }];
        let r = rows("mail", None, &fields, &HashSet::new());
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].preview, "list");
        assert_eq!(r[1].path, ["messages", "0", "from"]);
        let r = rows("mail", None, &fields, &["mail/messages".to_owned()].into());
        assert_eq!(r.len(), 1);
        let long = json!({"n": (0..80).collect::<Vec<_>>()});
        let flipped: HashSet<String> = HashSet::new();
        let r = rows("x", Some(&long), &[], &flipped);
        assert_eq!(r.len(), 1 + SHOWN + 1);
        assert_eq!(r.last().unwrap().label, "… 30 more");
        assert!(r.last().unwrap().path.is_empty());
    }

    #[test]
    fn a_path_is_written_the_way_the_box_reads_it() {
        let path = ["messages".to_owned(), "0".to_owned(), "subject".to_owned()];
        assert_eq!(path_text("mail", &path, false), "mail.messages.0.subject");
        assert_eq!(path_text("mail", &path, true), "mail.messages[0].subject");
        assert_eq!(path_text("mail", &[], true), "mail");
    }

    #[test]
    fn a_second_press_opens_the_step_and_a_picked_name_goes_in_the_box_with_the_caret() {
        use crate::app::Route;
        use crate::testing::{TempDb, app_with_server};
        use crate::workflow::definition::{Field, FieldKind};
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![]);
        app.roster.current_mut().unwrap().tools = vec![crate::tools::SHELL.into()];
        app.go(Route::Workflows);
        app.new_workflow();
        let label = crate::workflow::editor::KINDS
            .iter()
            .find(|k| k.kind == crate::workflow::definition::TOOL)
            .unwrap()
            .label;
        app.editor_add(&Vec::new(), label);
        app.editor_add(&Vec::new(), label);
        let first = Loc {
            place: Vec::new(),
            index: 0,
        };
        let second = Loc {
            place: Vec::new(),
            index: 1,
        };
        app.wf_editor.as_mut().unwrap().steps[0].found = vec![Field {
            name: "messages".into(),
            kind: FieldKind::List,
            of: vec![Field {
                name: "subject".into(),
                kind: FieldKind::Text,
                of: Vec::new(),
            }],
        }];
        // One press selects; a second soon after opens it; selecting another closes it.
        app.press_step(second.clone());
        assert!(!app.node_open());
        app.press_step(second.clone());
        assert!(app.node_open());
        app.select_step(Some(first.clone()));
        assert!(!app.node_open());
        // A name picked from the tree goes into the box, naming the step's result if it has none.
        app.select_step(Some(second.clone()));
        let path = ["messages".to_owned(), "0".to_owned(), "subject".to_owned()];
        app.map_into(&second, &Root::Step(first.clone()), &path);
        let ed = app.wf_editor.as_ref().unwrap();
        let bind = ed.node(&first).unwrap().bind.text.clone();
        assert!(!bind.is_empty());
        let typed = ed.node(&second).unwrap();
        let said = typed
            .params
            .values()
            .map(|p| p.edit.text.clone())
            .chain(typed.fields.iter().map(|f| f.text.clone()))
            .collect::<Vec<_>>()
            .join("|");
        assert!(
            said.contains(&format!("{{{{{bind}.messages.0.subject}}}}")),
            "{said}"
        );
    }
}
