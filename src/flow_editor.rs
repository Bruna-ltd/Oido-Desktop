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
use crate::workflow::cond::Op;
use crate::workflow::definition::{Definition, STEP_TOOLS, Step};
use crate::workflow::editor::{ArgKind, Editor, Loc, Menu, Place, Target, kind_named};
use crate::workflow::engine::{self, Memo, Outcome, StepLog};

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
    /// The log of the run as it stands while it goes, for the graph to show it so.
    pub live: Vec<StepLog>,
    /// What the steps tried one at a time made, for the next one to start from.
    pub memos: Option<StepMemos>,
}

/// The results of the top-level steps tried so far, each with the step as it was then.
pub struct StepMemos {
    inputs: Value,
    steps: Vec<(Step, Memo)>,
}

impl StepMemos {
    /// The results the steps before `last` can start from: those still the same step, up to the
    /// first that was changed, for the same inputs.
    fn before(&self, def: &Definition, inputs: &Value, last: usize) -> Vec<Memo> {
        if self.inputs != *inputs {
            return Vec::new();
        }
        self.steps
            .iter()
            .zip(&def.steps)
            .take(last)
            .take_while(|((was, _), now)| was == *now)
            .map(|((_, memo), _)| memo.clone())
            .collect()
    }

    /// How many steps have a result to start from.
    pub fn len(&self) -> usize {
        self.steps.len()
    }
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

    /// Adds the example workflows to the open agent (or the first), except any it already has by
    /// name, and says how many it added. They only run harmless commands, to try the builder on.
    pub fn add_examples(&mut self) {
        let Some((agent_id, agent_name)) = self
            .roster
            .current()
            .or_else(|| self.roster.list.first())
            .map(|a| (a.id.clone(), a.name.clone()))
        else {
            self.wf_notice = Some("Make an agent first: a workflow belongs to one.".to_owned());
            return;
        };
        let have: Vec<String> = self
            .workflows
            .iter()
            .filter(|w| w.agent_id == agent_id)
            .map(|w| w.name.to_lowercase())
            .collect();
        let mut added = 0;
        for e in crate::workflow::examples::all() {
            if have.contains(&e.name.to_lowercase()) {
                continue;
            }
            let w = crate::workflow::Saved {
                id: new_id(),
                agent_id: agent_id.clone(),
                name: e.name.to_owned(),
                goal: e.goal.to_owned(),
                // The examples are safe to run, and meant to be tried, so they start live.
                published: Some(crate::workflow::Published {
                    definition: e.definition.clone(),
                    at: now(),
                }),
                definition: e.definition,
                created_at: now(),
            };
            self.save("the workflow", |db| db.insert_workflow(&w));
            added += 1;
        }
        self.reload_workflows();
        self.wf_notice = Some(match added {
            0 => format!("{agent_name} already has the examples."),
            n => format!("Added {n} example workflows to {agent_name}."),
        });
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
        self.start_test(None);
    }

    /// Runs one step with the real results of the ones before it, so its output can be seen and
    /// the next step can use it. A step inside another runs the top-level step that holds it.
    pub fn run_step(&mut self, at: &Loc) {
        let top = at.place.first().map_or(at.index, |(i, _)| *i);
        self.start_test(Some(top));
    }

    /// Forgets what the steps made, so the next one runs everything before it again.
    pub fn forget_step_results(&mut self) {
        self.wf_test.memos = None;
    }

    /// `through` is the top-level step to run as far as, or `None` for the whole workflow. The
    /// first steps are taken from what earlier tries made, if they are still the same.
    fn start_test(&mut self, through: Option<usize>) {
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
        let go = match through {
            Some(last) => {
                let done = self
                    .wf_test
                    .memos
                    .as_ref()
                    .map_or_else(Vec::new, |m| m.before(&def, &inputs, last));
                Go::Through(inputs.clone(), last, done)
            }
            None => Go::Fresh(inputs.clone()),
        };
        let tried = (through.is_some()).then(|| (def.steps.clone(), inputs));
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
        self.wf_test.live.clear();
        // Each step beginning or ending tells the graph, so it shows the run as it goes.
        let watched = Arc::clone(&cancel);
        let watch: crate::flows::Watch = Arc::new(move |lines: Vec<StepLog>| {
            let flag = Arc::clone(&watched);
            crate::app::on_ui_thread(move |app| app.test_progress(&flag, lines));
        });
        self.spawn_flow(prep, def, go, cancel, Some(watch), move |app, outcome| {
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
                app.wf_test.live.clear();
                if let Some((steps, inputs)) = tried {
                    app.wf_test.memos = Some(StepMemos {
                        inputs,
                        steps: steps.into_iter().zip(outcome.memos.clone()).collect(),
                    });
                }
                app.wf_test.result = Some(outcome);
            }
        });
    }

    /// The log of the test run as it stands, if it is still the test the editor shows.
    pub fn test_progress(&mut self, flag: &Arc<AtomicBool>, lines: Vec<StepLog>) {
        if self
            .wf_test
            .running
            .as_ref()
            .is_some_and(|f| Arc::ptr_eq(f, flag))
        {
            self.wf_test.live = lines;
        }
    }

    /// Saves the workflow in the editor if it is good, and closes the editor; otherwise says what
    /// is wrong and stays open.
    pub fn save_editor(&mut self) {
        if let Some(id) = self.persist_editor() {
            self.wf_editor = None;
            self.wf_menu = None;
            self.reset_test();
            self.select_workflow(&id);
        }
    }

    /// Saves the draft and leaves the editor open. Returns the workflow's id, or `None` if it
    /// is not fit to save (what is wrong is in the editor).
    fn persist_editor(&mut self) -> Option<String> {
        let ed = self.wf_editor.as_mut()?;
        ed.check();
        if !ed.problems.is_empty() {
            return None;
        }
        let existing = ed
            .workflow
            .as_ref()
            .and_then(|id| self.workflows.iter().find(|w| &w.id == id));
        let created = existing.map_or_else(now, |w| w.created_at);
        let id = ed.workflow.clone().unwrap_or_else(new_id);
        let saved = ed.saved(&id, created).ok()?;
        let is_new = existing.is_none();
        // What the draft calls must at least exist; whether it is live is for publishing.
        if let Err(why) = self.flow_closure(&saved.agent_id, Some(&id), &saved.definition) {
            if let Some(ed) = &mut self.wf_editor {
                ed.problems = vec![why];
            }
            return None;
        }
        self.save("the workflow", |db| {
            if is_new {
                db.insert_workflow(&saved)
            } else {
                db.update_workflow(&saved)
            }
        });
        if let Some(ed) = &mut self.wf_editor {
            ed.workflow = Some(id.clone());
        }
        self.reload_workflows();
        Some(id)
    }

    #[cfg(test)]
    pub fn save_editor_keep_open_for_test(&mut self) {
        self.persist_editor();
    }

    /// Saves the draft, then makes it the live copy, with the schedule its trigger asks for.
    pub fn publish_editor(&mut self) {
        let Some(id) = self.persist_editor() else {
            return;
        };
        let result = self.publish_workflow(&id);
        self.after_publication(&id, result, "Published.");
    }

    /// Takes the workflow down: nothing runs it for real, and its schedule stops.
    pub fn unpublish_editor(&mut self) {
        let Some(id) = self.wf_editor.as_ref().and_then(|e| e.workflow.clone()) else {
            return;
        };
        let result = self.unpublish_workflow(&id);
        self.after_publication(
            &id,
            result,
            "Unpublished. Nothing runs it until it is published.",
        );
    }

    /// Drops what changed since the last publish: the editor shows the live copy again.
    pub fn revert_editor(&mut self) {
        let Some(id) = self.wf_editor.as_ref().and_then(|e| e.workflow.clone()) else {
            return;
        };
        match self.revert_workflow(&id) {
            Ok(()) => {
                if let Some(w) = self.workflows.iter().find(|w| w.id == id) {
                    self.wf_editor = Some(Editor::from_saved(w));
                }
                self.wf_graph.selected = None;
                self.wf_notice = Some("Back to the published version.".to_owned());
            }
            Err(why) => self.editor_says(why),
        }
    }

    fn after_publication(&mut self, id: &str, result: Result<(), String>, done: &str) {
        match result {
            Ok(()) => {
                let live = self
                    .workflows
                    .iter()
                    .find(|w| w.id == id)
                    .and_then(|w| w.published.clone());
                if let Some(ed) = &mut self.wf_editor {
                    ed.set_published(live);
                    ed.problems.clear();
                }
                self.wf_notice = Some(done.to_owned());
            }
            Err(why) => self.editor_says(why),
        }
    }

    fn editor_says(&mut self, why: String) {
        if let Some(ed) = &mut self.wf_editor {
            ed.problems = vec![why];
        }
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

    /// Chooses what starts the workflow: you (or an agent), or a schedule.
    pub fn editor_set_schedule(&mut self, on: bool) {
        if let Some(ed) = &mut self.wf_editor {
            ed.trigger.set_schedule(on, now());
        }
    }

    /// The trigger's timing boxes.
    pub fn trigger_timing(&mut self) -> &mut crate::schedules::form::TimingForm {
        match self.wf_editor.as_mut() {
            Some(ed) => &mut ed.trigger.timing,
            None => &mut self.wf_stray_timing,
        }
    }

    /// The box for what a scheduled run is given for input `name`.
    pub fn trigger_input(&mut self, name: &str) -> &mut rune::TextEdit {
        match self.wf_editor.as_mut() {
            Some(ed) => ed
                .trigger
                .inputs
                .entry(name.to_owned())
                .or_insert_with(|| TextEdit::new("")),
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
        let kind = kind_named(label);
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

    /// Chooses the tool a step uses. What the old tool was given means nothing to the new one.
    pub fn editor_set_tool(&mut self, at: &Loc, tool: &str) {
        if let Some(n) = self.wf_editor.as_mut().and_then(|e| e.node_mut(at))
            && n.tool != tool
        {
            n.tool = tool.to_owned();
            n.params.clear();
        }
        self.wf_menu = None;
    }

    /// Chooses the tool for the step whose list is open.
    pub fn editor_choose_tool(&mut self, tool: &str) {
        if let Some(at) = self.wf_tool_step.clone() {
            self.editor_set_tool(&at, tool);
        }
    }

    /// Sets an argument of a tool step from a choice: one of a list, or yes or no.
    pub fn editor_set_param(&mut self, at: &Loc, key: &str, kind: ArgKind, text: &str) {
        if let Some(ed) = &mut self.wf_editor {
            ed.set_param(at, key, kind, text);
        }
        self.wf_menu = None;
    }

    /// The tools a step of the editor's workflow can use, by the name the model knows them by:
    /// the built-in ones a flow may call, those of the MCP servers that are on, and handing a task
    /// over when the agent has a team.
    pub fn step_tools(&self, current: &str) -> Vec<String> {
        let mut tools: Vec<String> = STEP_TOOLS.iter().map(|t| (*t).to_owned()).collect();
        for server in self.mcp.servers.iter().filter(|m| m.offers()) {
            tools.extend(server.tools.iter().map(|t| server.qualified(&t.name)));
        }
        let agent = self.wf_editor.as_ref().map(|e| e.agent_id.as_str());
        if agent
            .and_then(|id| self.roster.agent_ref(id))
            .is_some_and(|a| !a.subagents.is_empty())
        {
            tools.push(crate::tools::DELEGATE.to_owned());
        }
        // The one a step already names stays, though its server is off.
        if !current.is_empty() && !tools.iter().any(|t| t == current) {
            tools.push(current.to_owned());
        }
        tools
    }

    /// The JSON schema of the arguments of a tool, as the workflow's agent would see it.
    pub fn step_tool_schema(&self, tool: &str) -> Option<serde_json::Value> {
        if tool == crate::tools::DELEGATE {
            let team = self
                .wf_editor
                .as_ref()
                .and_then(|e| self.roster.agent_ref(&e.agent_id))
                .map(|a| a.subagents.clone())?;
            return Some(crate::delegate::spec(&team).schema);
        }
        self.all_tools()
            .into_iter()
            .find(|t| t.name == tool)
            .map(|t| t.schema)
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
    /// Puts into a box the result a person chose by its words, giving the step it comes from a
    /// name for it if it has none.
    pub fn editor_use(&mut self, which: &Target, at: &Loc, key: &str, braces: bool, label: &str) {
        let name = self.wf_editor.as_mut().and_then(|ed| {
            let found = ed
                .results(at, Some(key))
                .into_iter()
                .find(|f| f.label == label)?;
            ed.resolve(&found.source)
        });
        match name {
            Some(name) => self.editor_insert(which, braces, &name),
            None => self.wf_menu = None,
        }
    }

    /// Chooses by its words the result an `if` or `while` looks at.
    pub fn editor_set_left(&mut self, at: &Loc, label: &str) {
        if let Some(ed) = &mut self.wf_editor
            && let Some(found) = ed
                .results(at, Some("cond"))
                .into_iter()
                .find(|f| f.label == label)
            && let Some(name) = ed.resolve(&found.source)
        {
            ed.set_condition_left(at, &name);
        }
        self.wf_menu = None;
    }

    pub fn editor_set_compare(&mut self, at: &Loc, label: &str) {
        if let (Some(ed), Some(op)) = (&mut self.wf_editor, Op::from_label(label)) {
            ed.set_condition_op(at, op);
        }
        self.wf_menu = None;
    }

    pub fn editor_toggle_expression(&mut self, at: &Loc) {
        if let Some(ed) = &mut self.wf_editor {
            if ed.node_mut(at).is_some_and(|n| n.when.is_some()) {
                ed.condition_as_expression(at);
            } else {
                ed.condition_in_parts(at);
            }
        }
    }

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

        app.editor_add(&Vec::new(), "Repeat for each");
        set(&mut app, &Target::Field(top(0), 0), "[\"x\", \"y\"]");
        set(&mut app, &Target::Field(top(0), 1), "name");
        set(&mut app, &Target::Bind(top(0)), "greeted");
        let body: Place = vec![(0, 0)];
        app.editor_add(&body, "Use a tool");
        let inner = Loc {
            place: body,
            index: 0,
        };
        set(
            &mut app,
            &Target::Param(inner, "command".into(), ArgKind::Text),
            "echo {{name}}-{{index}}",
        );
        app.editor_add(&Vec::new(), "Check something");
        app.editor_toggle_expression(&top(1));
        set(&mut app, &Target::Field(top(1), 0), "inputs.loud");
        let then: Place = vec![(1, 0)];
        app.editor_add(&then, "Use a tool");
        set(
            &mut app,
            &Target::Param(
                Loc {
                    place: then,
                    index: 0,
                },
                "command".into(),
                ArgKind::Text,
            ),
            "echo {{= greeted.len() }} greeted",
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

        // Saved is not live: nothing runs it for real until it is published.
        assert!(app.workflows[0].live().is_none());
        app.publish_workflow(&id).unwrap();
        let mut again = db.open();
        again.load_workflows();
        assert_eq!(
            again.workflows[0].live(),
            Some(&app.workflows[0].definition)
        );

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
        app.editor_add(&Vec::new(), "Check something");
        app.editor_toggle_expression(&top(0));
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
        app.editor_add(&Vec::new(), "Remember a value");
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
        app.editor_add(&Vec::new(), "Use a tool");
        set(
            &mut app,
            &Target::Param(top(0), "command".into(), ArgKind::Text),
            "echo got-{{inputs.word}}",
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
        set(
            &mut app,
            &Target::Param(top(0), "timeout_secs".into(), ArgKind::Whole),
            "soon",
        );
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
            &Target::Param(top(0), "timeout_secs".into(), ArgKind::Whole),
            "",
        );
        set(
            &mut app,
            &Target::Param(top(0), "command".into(), ArgKind::Text),
            "echo got-{{inputs.word}}",
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

    #[cfg(unix)]
    #[test]
    fn publishing_from_the_editor_saves_the_draft_first_and_unpublish_and_revert_follow_it() {
        let db = TempDb::new();
        let mut app = on_the_page(&db);
        app.new_workflow();
        set(&mut app, &Target::Name, "Ping");
        app.editor_add(&Vec::new(), "Use a tool");
        set(
            &mut app,
            &Target::Param(top(0), "command".into(), ArgKind::Text),
            "echo one",
        );
        // A schedule trigger, every hour.
        app.editor_set_schedule(true);
        let t = &mut app.wf_editor.as_mut().unwrap().trigger.timing;
        t.when = crate::schedules::form::When::Every;
        t.every = TextEdit::new("60");

        app.publish_editor();
        let ed = app.wf_editor.as_ref().unwrap();
        assert!(ed.problems.is_empty(), "{:?}", ed.problems);
        let id = ed.workflow.clone().unwrap();
        assert!(app.workflows[0].live().is_some());
        assert_eq!(app.schedules.len(), 1, "its trigger made a schedule");
        assert!(app.schedules[0].from_trigger);

        // More changes are a draft only; Revert brings the live copy back into the editor.
        set(
            &mut app,
            &Target::Param(top(0), "command".into(), ArgKind::Text),
            "echo two",
        );
        app.save_editor_keep_open_for_test();
        assert!(app.workflows[0].has_unpublished_changes());
        app.revert_editor();
        assert!(!app.workflows[0].has_unpublished_changes());
        assert_eq!(
            app.wf_editor.as_ref().unwrap().workflow.as_deref(),
            Some(id.as_str())
        );
        assert!(app.wf_editor.as_ref().unwrap().trigger.schedule);

        app.unpublish_editor();
        assert!(app.workflows[0].live().is_none());
        assert!(app.schedules.is_empty());
        // What is not fit to run is refused and said, and nothing goes live.
        app.editor_set_schedule(true);
        app.wf_editor.as_mut().unwrap().trigger.timing.every = TextEdit::new("soon");
        app.publish_editor();
        assert!(!app.wf_editor.as_ref().unwrap().problems.is_empty());
        assert!(app.workflows[0].live().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_step_run_alone_starts_from_the_results_of_the_ones_before_it_until_one_is_changed() {
        let db = TempDb::new();
        let mut app = on_the_page(&db);
        app.new_workflow();
        set(&mut app, &Target::Name, "Chain");
        let command = |at: usize| Target::Param(top(at), "command".into(), ArgKind::Text);
        for _ in 0..3 {
            app.editor_add(&Vec::new(), "Use a tool");
        }
        // The first makes something different each time it runs, so a run again would show.
        set(&mut app, &command(0), "date +%s%N");
        set(&mut app, &Target::Bind(top(0)), "stamp");
        set(&mut app, &command(1), "echo two-{{stamp.stdout}}");
        set(&mut app, &Target::Bind(top(1)), "two");
        set(&mut app, &command(2), "echo three-{{two.stdout}}");

        let out = |app: &App| app.wf_test.result.as_ref().unwrap().clone();
        app.run_step(&top(1));
        let first = out(&app);
        assert_eq!(first.ending, crate::workflow::engine::Ending::Succeeded);
        assert_eq!(first.steps.len(), 2, "the step and the one before it");
        let stamp = first.steps[0].output.clone();
        assert!(
            first.steps[1].output.contains("two-"),
            "{:?} / {:?}",
            first.steps[0].output,
            first.steps[1].output
        );

        // The next step starts from them: the first is not run again, and the third reads the second.
        app.run_step(&top(2));
        let second = out(&app);
        assert_eq!(second.steps.len(), 3);
        assert_eq!(second.steps[0].output, stamp, "not run again");
        assert!(
            second.steps[2].output.contains("two-"),
            "{}",
            second.steps[2].output
        );

        // Changing the second keeps the first and runs the second again.
        set(&mut app, &command(1), "echo changed-{{stamp.stdout}}");
        app.run_step(&top(2));
        let third = out(&app);
        assert_eq!(
            third.steps[0].output, stamp,
            "the first is still remembered"
        );
        assert!(third.steps[1].output.contains("changed-"));
        assert!(
            third.steps[2].output.contains("changed-"),
            "{}",
            third.steps[2].output
        );

        // Changing the first makes everything run again.
        set(&mut app, &command(0), "echo new-stamp");
        app.run_step(&top(2));
        assert!(out(&app).steps[0].output.contains("new-stamp"));

        // Forgetting what was made starts from the top again; one that is not fit to run is said.
        assert!(app.wf_test.memos.as_ref().is_some_and(|m| m.len() == 3));
        app.forget_step_results();
        assert!(app.wf_test.memos.is_none());
        set(
            &mut app,
            &Target::Param(top(1), "timeout_secs".into(), ArgKind::Whole),
            "soon",
        );
        app.run_step(&top(1));
        assert!(app.wf_test.notice.is_some());
    }
}
