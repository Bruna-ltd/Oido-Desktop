//! What the buttons and boxes of the workflow editor do to the app: opening it on a workflow or a
//! new one, changing what is in it, and saving. The state is [`crate::workflow::editor`]; the
//! screen is `ui/workflow_editor.rs`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rune::TextEdit;
use serde_json::{Map, Value};

use crate::app::{App, now};
use crate::flows::Go;
use crate::model::new_id;
use crate::models::ModelRef;
use crate::store::Counted;
use crate::usage::Source;
use crate::workflow::editor::{Editor, KINDS, Loc, Menu, Place, Target};
use crate::workflow::engine::{self, Outcome};

/// What a card raised by a test run of a workflow that is not saved yet is filed under.
pub const TEST_CARD: &str = "workflow-test";

/// The editor's test run of the workflow as it stands, saved or not.
#[derive(Default)]
pub struct TestRun {
    /// What to give it, one per line: `name = value`.
    pub inputs: TextEdit,
    /// Stops it, while it runs.
    pub running: Option<Arc<AtomicBool>>,
    pub result: Option<Outcome>,
    pub notice: Option<String>,
}

impl TestRun {
    /// The inputs as the workflow takes them, or the line that is not one.
    fn given(&self) -> Result<Map<String, Value>, String> {
        let mut given = Map::new();
        for line in self.inputs.text.lines().filter(|l| !l.trim().is_empty()) {
            let (name, value) = line.split_once('=').ok_or_else(|| {
                format!("Write each input as name = value, not {:?}.", line.trim())
            })?;
            given.insert(
                name.trim().to_owned(),
                Value::String(value.trim().to_owned()),
            );
        }
        Ok(given)
    }
}

impl App {
    /// Every agent and team member, by name, with its id.
    pub fn agent_choices(&self) -> Vec<(String, String)> {
        self.roster
            .list
            .iter()
            .flat_map(|a| std::iter::once(a).chain(a.subagents.iter()))
            .map(|a| (a.name.clone(), a.id.clone()))
            .collect()
    }

    /// Opens the editor on a new workflow, for the first agent until another is chosen.
    pub fn new_workflow(&mut self) {
        let agent = self
            .roster
            .list
            .first()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        self.wf_menu = None;
        self.reset_test();
        self.wf_editor = Some(Editor::blank(&agent));
    }

    /// Opens the editor on the open workflow, unless it is running.
    pub fn edit_open_workflow(&mut self) {
        let Some(wf) = self
            .wf_open
            .as_ref()
            .and_then(|id| self.workflows.iter().find(|w| &w.id == id))
        else {
            return;
        };
        if self.wf_jobs.contains_key(&wf.id) {
            return;
        }
        let ed = Editor::from_saved(wf);
        self.wf_menu = None;
        self.reset_test();
        self.wf_editor = Some(ed);
    }

    pub fn cancel_editor(&mut self) {
        self.wf_editor = None;
        self.wf_menu = None;
        self.reset_test();
    }

    /// Ends a test run still going and forgets the last one.
    fn reset_test(&mut self) {
        self.stop_test();
        self.wf_test = TestRun::default();
        // A workflow opened in the graph is fitted to the view and has nothing selected.
        self.wf_graph.selected = None;
        self.wf_graph.needs_fit = true;
    }

    pub fn stop_test(&mut self) {
        if let Some(flag) = &self.wf_test.running {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Runs the workflow as it stands in the editor, for real, with the inputs given. Nothing is
    /// saved: not the workflow, and not the run.
    pub fn test_editor(&mut self) {
        if self.wf_test.running.is_some() {
            return;
        }
        let Some(ed) = &mut self.wf_editor else {
            return;
        };
        self.wf_test.result = None;
        self.wf_test.notice = None;
        ed.check();
        if !ed.problems.is_empty() {
            self.wf_test.notice = Some("Fix what is listed above first.".to_owned());
            return;
        }
        let Ok(def) = ed.definition() else {
            return;
        };
        let (agent_id, own) = (ed.agent_id.clone(), ed.workflow.clone());
        let name = match ed.name.text.trim() {
            "" => "this workflow".to_owned(),
            n => n.to_owned(),
        };
        let started = self
            .wf_test
            .given()
            .and_then(|given| engine::bind_inputs(&def, &given))
            .and_then(|inputs| {
                let at = crate::guard::ask::Where::Workflow(
                    own.clone().unwrap_or_else(|| TEST_CARD.to_owned()),
                );
                let prep = self.prep_run(&def, own.as_deref(), &agent_id, at, &name)?;
                Ok((inputs, prep))
            });
        let (inputs, prep) = match started {
            Ok(ok) => ok,
            Err(why) => {
                self.wf_test.notice = Some(why);
                return;
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.wf_test.running = Some(Arc::clone(&cancel));
        let model = ModelRef::new(&prep.provider, &prep.model);
        let agent_name = self
            .roster
            .agent_ref(&agent_id)
            .map(|a| a.name.clone())
            .unwrap_or_default();
        let began = now();
        let flag = Arc::clone(&cancel);
        self.spawn_flow(prep, def, Go::Fresh(inputs), cancel, move |app, outcome| {
            let who = Counted {
                model: &model,
                agent_id: &agent_id,
                agent_name: &agent_name,
                source: Source::Workflow,
            };
            app.count_flow_usage(began, outcome.usage, &who);
            // Only if it is still the test the editor shows: the editor may have closed.
            if app
                .wf_test
                .running
                .as_ref()
                .is_some_and(|f| Arc::ptr_eq(f, &flag))
            {
                app.wf_test.running = None;
                app.wf_test.result = Some(outcome);
            }
        });
    }

    /// Saves the workflow in the editor if it is good, and closes the editor; otherwise says what
    /// is wrong and stays open.
    pub fn save_editor(&mut self) {
        let Some(ed) = &mut self.wf_editor else {
            return;
        };
        ed.check();
        if !ed.problems.is_empty() {
            return;
        }
        let existing = ed
            .workflow
            .as_ref()
            .and_then(|id| self.workflows.iter().find(|w| &w.id == id));
        let created = existing.map_or_else(now, |w| w.created_at);
        let id = ed.workflow.clone().unwrap_or_else(new_id);
        let Ok(saved) = ed.saved(&id, created) else {
            return;
        };
        let is_new = existing.is_none();
        if let Err(why) = self.flow_closure(&saved.agent_id, Some(&id), &saved.definition) {
            if let Some(ed) = &mut self.wf_editor {
                ed.problems = vec![why];
            }
            return;
        }
        self.save("the workflow", |db| {
            if is_new {
                db.insert_workflow(&saved)
            } else {
                db.update_workflow(&saved)
            }
        });
        self.wf_editor = None;
        self.wf_menu = None;
        self.reset_test();
        self.reload_workflows();
        self.select_workflow(&id);
    }

    /// Looks again at what is typed.
    pub fn check_editor(&mut self) {
        if let Some(ed) = &mut self.wf_editor {
            ed.check();
        }
    }

    pub fn toggle_menu(&mut self, menu: &Menu) {
        self.wf_menu = if self.wf_menu.as_ref() == Some(menu) {
            None
        } else {
            Some(menu.clone())
        };
    }

    /// The box something is being typed into.
    pub fn editor_text(&mut self, which: &Target) -> &mut rune::TextEdit {
        match self.wf_editor.as_mut().and_then(|e| e.text_mut(which)) {
            Some(edit) => edit,
            None => &mut self.wf_stray,
        }
    }

    pub fn choose_agent(&mut self, name: &str) {
        let id = self
            .agent_choices()
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| id);
        if let (Some(ed), Some(id)) = (&mut self.wf_editor, id) {
            ed.agent_id = id;
        }
        self.wf_menu = None;
    }

    /// Adds the kind of step the menu entry `label` names to the end of a list.
    pub fn editor_add(&mut self, place: &Place, label: &str) {
        let kind = KINDS.iter().find(|(_, l)| *l == label).map(|(k, _)| *k);
        if let (Some(ed), Some(kind)) = (&mut self.wf_editor, kind) {
            ed.add(place, kind);
            ed.check();
        }
        self.wf_menu = None;
    }

    pub fn editor_remove(&mut self, at: &Loc) {
        if let Some(ed) = &mut self.wf_editor {
            ed.remove(at);
            ed.check();
        }
    }

    pub fn editor_shift(&mut self, at: &Loc, by: isize) {
        if let Some(ed) = &mut self.wf_editor {
            ed.shift(at, by);
            ed.check();
        }
    }

    pub fn editor_add_case(&mut self, at: &Loc) {
        if let Some(ed) = &mut self.wf_editor {
            ed.add_case(at);
        }
    }

    pub fn editor_add_branch(&mut self, at: &Loc) {
        if let Some(ed) = &mut self.wf_editor {
            ed.add_branch(at);
        }
    }

    pub fn editor_remove_branch(&mut self, at: &Loc, block: usize) {
        if let Some(ed) = &mut self.wf_editor {
            ed.remove_branch(at, block);
            ed.check();
        }
    }

    pub fn editor_remove_case(&mut self, at: &Loc, block: usize) {
        if let Some(ed) = &mut self.wf_editor {
            ed.remove_case(at, block);
            ed.check();
        }
    }

    pub fn editor_set_tool(&mut self, at: &Loc, tool: &str) {
        if let Some(n) = self.wf_editor.as_mut().and_then(|e| e.node_mut(at)) {
            n.tool = tool.to_owned();
        }
        self.wf_menu = None;
    }

    pub fn editor_toggle_go_on(&mut self, at: &Loc) {
        if let Some(n) = self.wf_editor.as_mut().and_then(|e| e.node_mut(at)) {
            n.go_on = !n.go_on;
        }
    }

    pub fn editor_add_input(&mut self) {
        if let Some(ed) = &mut self.wf_editor {
            ed.inputs.push(crate::workflow::editor::InputRow {
                name: rune::TextEdit::new(""),
                kind: "string",
                description: rune::TextEdit::new(""),
                default: rune::TextEdit::new(""),
            });
        }
    }

    pub fn editor_remove_input(&mut self, i: usize) {
        if let Some(ed) = &mut self.wf_editor
            && i < ed.inputs.len()
        {
            ed.inputs.remove(i);
            ed.check();
        }
    }

    pub fn editor_set_input_kind(&mut self, i: usize, kind: &str) {
        if let Some(row) = self.wf_editor.as_mut().and_then(|e| e.inputs.get_mut(i))
            && let Some(k) = crate::workflow::editor::INPUT_KINDS
                .into_iter()
                .find(|k| *k == kind)
        {
            row.kind = k;
        }
        self.wf_menu = None;
    }

    /// Puts a name into the box it was picked for, as `{{name}}` where the box reads braces.
    pub fn editor_insert(&mut self, which: &Target, braces: bool, name: &str) {
        let text = if braces {
            format!("{{{{{name}}}}}")
        } else {
            name.to_owned()
        };
        self.editor_text(which).insert(&text);
        self.wf_menu = None;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use rune::TextEdit;

    use super::*;
    use crate::app::Route;
    use crate::testing::{TempDb, app_with_server};
    use crate::workflow::RunStatus;

    fn set(app: &mut App, at: &Target, text: &str) {
        *app.editor_text(at) = TextEdit::new(text);
    }

    fn top(index: usize) -> Loc {
        Loc {
            place: Vec::new(),
            index,
        }
    }

    /// An app on the Workflows page whose agent has the shell.
    fn on_the_page(db: &TempDb) -> App {
        let (mut app, _) = app_with_server(db, vec![]);
        app.roster.current_mut().unwrap().tools = vec![crate::tools::SHELL.into()];
        app.go(Route::Workflows);
        app
    }

    #[cfg(unix)]
    #[test]
    fn a_workflow_is_built_in_the_editor_saved_found_again_after_a_restart_and_run() {
        let db = TempDb::new();
        let mut app = on_the_page(&db);
        app.new_workflow();
        set(&mut app, &Target::Name, "Greet");
        app.editor_add_input();
        set(&mut app, &Target::InputName(0), "loud");
        app.editor_set_input_kind(0, "boolean");
        set(&mut app, &Target::InputDefault(0), "true");

        app.editor_add(&Vec::new(), "For each");
        set(&mut app, &Target::Field(top(0), 0), "[\"x\", \"y\"]");
        set(&mut app, &Target::Field(top(0), 1), "name");
        set(&mut app, &Target::Bind(top(0)), "greeted");
        let body: Place = vec![(0, 0)];
        app.editor_add(&body, "Run a tool");
        let inner = Loc {
            place: body,
            index: 0,
        };
        set(
            &mut app,
            &Target::Field(inner, 0),
            r#"{"command": "echo {{name}}-{{index}}"}"#,
        );
        app.editor_add(&Vec::new(), "If");
        set(&mut app, &Target::Field(top(1), 0), "inputs.loud");
        let then: Place = vec![(1, 0)];
        app.editor_add(&then, "Run a tool");
        set(
            &mut app,
            &Target::Field(
                Loc {
                    place: then,
                    index: 0,
                },
                0,
            ),
            r#"{"command": "echo {{= greeted.len() }} greeted"}"#,
        );

        app.save_editor();
        assert!(
            app.wf_editor.is_none(),
            "{:?}",
            app.wf_editor.map(|e| e.problems)
        );
        assert_eq!(app.workflows.len(), 1);
        let id = app.workflows[0].id.clone();
        assert_eq!(app.wf_open.as_deref(), Some(id.as_str()));

        // Still there, whole, when the app is opened again.
        let mut again = db.open();
        again.load_workflows();
        assert_eq!(again.workflows.len(), 1);
        assert_eq!(again.workflows[0].definition, app.workflows[0].definition);

        app.run_open_workflow();
        let run = &app.wf_runs[0];
        assert_eq!(run.status, RunStatus::Succeeded, "{}", run.error);
        let said: Vec<&str> = run.steps.iter().map(|s| s.output.as_str()).collect();
        assert!(said.iter().any(|o| o.contains("x-0")), "{said:?}");
        assert!(said.iter().any(|o| o.contains("y-1")), "{said:?}");
        assert!(said.last().unwrap().contains("2 greeted"), "{said:?}");
        assert_eq!(run.steps.len(), 5);
        assert_eq!(run.steps[1].path, "0.body.0");
        assert_eq!(run.steps[1].rounds, [1]);
    }

    #[test]
    fn a_workflow_with_problems_is_not_saved_and_the_editor_says_what_they_are() {
        let db = TempDb::new();
        let mut app = on_the_page(&db);
        app.new_workflow();
        app.save_editor();
        let ed = app.wf_editor.as_ref().unwrap();
        assert!(
            ed.problems.iter().any(|p| p.contains("needs a name")),
            "{:?}",
            ed.problems
        );
        assert!(app.workflows.is_empty());

        set(&mut app, &Target::Name, "X");
        app.editor_add(&Vec::new(), "If");
        set(&mut app, &Target::Field(top(0), 0), "ghost > 1");
        app.save_editor();
        let ed = app.wf_editor.as_ref().unwrap();
        assert!(
            ed.problems.iter().any(|p| p.contains("ghost")),
            "{:?}",
            ed.problems
        );
        assert!(
            ed.problems
                .iter()
                .any(|p| p.contains("needs at least one step")),
            "{:?}",
            ed.problems
        );
        assert!(app.workflows.is_empty());
        app.cancel_editor();
        assert!(app.wf_editor.is_none() && app.workflows.is_empty());
    }

    #[test]
    fn changing_a_saved_workflow_keeps_it_the_same_workflow_and_a_running_one_cannot_be_changed() {
        let db = TempDb::new();
        let mut app = on_the_page(&db);
        app.new_workflow();
        set(&mut app, &Target::Name, "Old name");
        app.editor_add(&Vec::new(), "Set values");
        set(&mut app, &Target::Field(top(0), 0), "a = 1");
        set(&mut app, &Target::Bind(top(0)), "made");
        app.save_editor();
        let id = app.workflows[0].id.clone();
        let made = app.workflows[0].created_at;

        app.wf_jobs.insert(
            id.clone(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        app.edit_open_workflow();
        assert!(app.wf_editor.is_none(), "not while it runs");
        app.wf_jobs.clear();

        app.edit_open_workflow();
        assert_eq!(app.wf_editor.as_ref().unwrap().name.text, "Old name");
        set(&mut app, &Target::Name, "New name");
        set(&mut app, &Target::Field(top(0), 0), "a = 2\nb = 3");
        app.save_editor();
        assert_eq!(app.workflows.len(), 1);
        assert_eq!(
            (
                app.workflows[0].id.as_str(),
                app.workflows[0].name.as_str(),
                app.workflows[0].created_at
            ),
            (id.as_str(), "New name", made)
        );
        assert_eq!(
            app.workflows[0].definition.steps[0]
                .values
                .as_ref()
                .unwrap()
                .len(),
            2
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_test_run_runs_the_workflow_as_it_stands_without_saving_it_or_the_run() {
        let db = TempDb::new();
        let mut app = on_the_page(&db);
        app.new_workflow();
        set(&mut app, &Target::Name, "Echo");
        app.editor_add_input();
        set(&mut app, &Target::InputName(0), "word");
        app.editor_add(&Vec::new(), "Run a tool");
        set(
            &mut app,
            &Target::Field(top(0), 0),
            r#"{"command": "echo got-{{inputs.word}}"}"#,
        );

        // Not ready: the box that is not a line, then an input that is missing.
        app.wf_test.inputs = TextEdit::new("word");
        app.test_editor();
        assert!(app.wf_test.running.is_none());
        assert!(
            app.wf_test
                .notice
                .as_deref()
                .is_some_and(|n| n.contains("name = value")),
            "{:?}",
            app.wf_test.notice
        );
        app.wf_test.inputs = TextEdit::new("");
        app.test_editor();
        assert!(
            app.wf_test
                .notice
                .as_deref()
                .is_some_and(|n| n.contains("word"))
        );

        // A step with a problem is not run.
        app.wf_test.inputs = TextEdit::new("word = hi");
        set(&mut app, &Target::Field(top(0), 0), "{not json");
        app.test_editor();
        assert!(
            app.wf_test
                .notice
                .as_deref()
                .is_some_and(|n| n.contains("Fix"))
        );
        assert!(app.wf_test.result.is_none());

        // Good: it runs for real and shows each step.
        set(
            &mut app,
            &Target::Field(top(0), 0),
            r#"{"command": "echo got-{{inputs.word}}"}"#,
        );
        app.test_editor();
        assert!(
            app.wf_test.running.is_none(),
            "the test worker has finished"
        );
        let out = app.wf_test.result.as_ref().unwrap();
        assert_eq!(out.ending, crate::workflow::engine::Ending::Succeeded);
        assert!(out.output.contains("got-hi"), "{}", out.output);
        assert!(app.workflows.is_empty(), "nothing was saved");
        assert!(
            app.store
                .as_ref()
                .unwrap()
                .list_workflows()
                .unwrap()
                .is_empty()
        );

        // Closing the editor forgets it.
        app.cancel_editor();
        assert!(app.wf_test.result.is_none() && app.wf_test.inputs.text.is_empty());
    }
}
