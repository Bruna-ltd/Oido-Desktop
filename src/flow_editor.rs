//! What the buttons and boxes of the workflow editor do to the app: opening it on a workflow or a
//! new one, changing what is in it, and saving. The state is [`crate::workflow::editor`]; the
//! screen is `ui/workflow_editor.rs`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
use crate::workflow::editor::{
    ArgKind, Completion, Editor, Loc, Menu, Place, Refs, Target, kind_named,
};
use crate::workflow::engine::{self, Memo, Outcome, StepLog};

/// What a card raised by a test run of a workflow that is not saved yet is filed under.
pub const TEST_CARD: &str = "workflow-test";

/// A box that finishes the names typed in it: which box it is, what it may name, and the element
/// the list of ways to finish opens under.
#[derive(Clone)]
pub struct Complete {
    pub at: Loc,
    pub key: String,
    pub refs: Refs,
    pub target: Target,
    pub anchor: String,
}

/// Which way to finish a name is chosen, and the text where the list was put away with Escape.
#[derive(Default)]
pub struct Suggest {
    pub active: usize,
    pub dismissed: Option<String>,
}

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

/// The model that finds the fields of a step's result, and how the last try went.
#[derive(Default)]
pub struct Mapper {
    /// None: the workflow's agent's own.
    pub model: Option<crate::models::ModelRef>,
    pub picker: crate::picker::PickerState,
    pub running: bool,
    pub notice: Option<String>,
    /// The step that was run so its result could be looked at, once that run ends.
    pub after_run: Option<Loc>,
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

/// A name a step's result can be kept under, from the step's id: letters, digits and
/// underscores, not starting with a digit, and none of the names the workflow uses itself.
fn result_name(id: &str) -> String {
    let name: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    match name.as_str() {
        "" | "inputs" | "item" | "index" | "error" => "result".to_owned(),
        _ if name.starts_with(|c: char| c.is_ascii_digit()) => format!("step_{name}"),
        _ => name,
    }
}

/// A line, then each problem on a line of its own.
fn problems_notice(head: &str, problems: &[String]) -> String {
    let mut out = head.to_owned();
    for p in problems {
        out.push_str("\n• ");
        out.push_str(p);
    }
    out
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
        self.wf_editor = Some(Editor::blank_named(
            &agent,
            self.workflows.iter().map(|w| w.name.as_str()),
        ));
    }

    /// A card was pressed: open that workflow in the builder.
    pub fn open_in_builder(&mut self, id: &str) {
        self.select_workflow(id);
        self.edit_open_workflow();
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
        self.autosave();
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

    /// Runs the workflow as far as one step, with the real results of the ones before it, so its
    /// output can be seen and the next step can use it. A step inside another runs the step that
    /// holds it, cut off after this one. What comes after is not run, and not checked: a step
    /// not finished yet elsewhere cannot stop it.
    pub fn run_step(&mut self, at: &Loc) {
        self.start_test(Some(at.clone()));
    }

    /// Runs what comes before a step, for real, so what it needs is there to see and use. The step
    /// itself is not run, so it can be unfinished.
    pub fn run_before(&mut self, at: &Loc) {
        match self.wf_editor.as_ref().and_then(|ed| ed.step_before(at)) {
            Some(before) => self.start_test(Some(before)),
            None => {
                self.wf_test.result = None;
                self.wf_test.notice = Some("Nothing comes before this step.".to_owned());
                self.wf_logs_open = true;
            }
        }
    }

    /// Forgets what the steps made, so the next one runs everything before it again.
    pub fn forget_step_results(&mut self) {
        self.wf_test.memos = None;
    }

    /// `through` is the step to run as far as, or `None` for the whole workflow. The first steps
    /// are taken from what earlier tries made, if they are still the same.
    fn start_test(&mut self, through: Option<Loc>) {
        if self.wf_test.running.is_some() {
            return;
        }
        if let Some(ed) = &mut self.wf_editor {
            ed.name_required_results();
        }
        self.autosave();
        let Some(ed) = &mut self.wf_editor else {
            return;
        };
        self.wf_test.result = None;
        self.wf_test.notice = None;
        self.wf_logs_open = true;
        self.wf_log_pick = None;
        let def = match &through {
            None => {
                ed.check();
                if !ed.problems.is_empty() {
                    self.wf_test.notice = Some(problems_notice(
                        "This workflow cannot run yet:",
                        &ed.problems,
                    ));
                    return;
                }
                match ed.definition() {
                    Ok(def) => def,
                    Err(why) => {
                        self.wf_test.notice =
                            Some(problems_notice("This workflow cannot run yet:", &why));
                        return;
                    }
                }
            }
            // Only the steps that run are checked, and what is wrong there is said here, on the
            // run, so the way out is clear.
            Some(at) => match ed.definition_through(at) {
                Ok(def) if !ed.agent_id.is_empty() => def,
                Ok(_) => {
                    self.wf_test.notice =
                        Some("Choose the agent this workflow belongs to first.".to_owned());
                    return;
                }
                Err(why) => {
                    self.wf_test.notice = Some(problems_notice(
                        "This step cannot run yet, because of what is before it or in it:",
                        &why,
                    ));
                    return;
                }
            },
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
        let go = match through.as_ref() {
            Some(at) => {
                let last = at.place.first().map_or(at.index, |(step, _)| *step);
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
        // Each step beginning or ending tells the graph, so it shows the run as it goes. Only the
        // newest log is kept: a fast run makes one redraw per frame, not a backlog of one a step.
        let watched = Arc::clone(&cancel);
        let newest: Arc<Mutex<Option<Vec<StepLog>>>> = Arc::default();
        let watch: crate::flows::Watch = Arc::new(move |lines: Vec<StepLog>| {
            let Ok(mut slot) = newest.lock() else {
                return;
            };
            if slot.replace(lines).is_some() {
                return;
            }
            drop(slot);
            let (flag, newest) = (Arc::clone(&watched), Arc::clone(&newest));
            crate::app::on_ui_thread(move |app| {
                let lines = newest.lock().ok().and_then(|mut slot| slot.take());
                if let Some(lines) = lines {
                    app.test_progress(&flag, lines);
                }
            });
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
                app.discover_after_run();
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

    /// Saves what is in the editor as it stands, finished or not, and says what is still wrong
    /// with it. The editor stays open: leaving it is [`App::cancel_editor`], and nothing is lost
    /// by that either, because the draft is kept as it is changed.
    pub fn save_editor(&mut self) {
        if let Some(ed) = &mut self.wf_editor {
            ed.check();
        }
        self.persist_draft();
    }

    /// Keeps the draft as it is changed, like n8n does. Nothing to say if it cannot be (no agent
    /// yet, or a new workflow with no step): the next change tries again.
    pub fn autosave(&mut self) {
        self.persist_draft();
    }

    /// Writes the editor's workflow as a draft whatever is wrong with it, when it is not the
    /// same as what is kept. A new workflow is first kept when it has a step. Returns its id.
    fn persist_draft(&mut self) -> Option<String> {
        let ed = self.wf_editor.as_ref()?;
        if ed.agent_id.is_empty() {
            return None;
        }
        let existing = ed
            .workflow
            .as_ref()
            .and_then(|id| self.workflows.iter().find(|w| &w.id == id));
        if existing.is_none() && ed.steps.is_empty() {
            return None;
        }
        let created = existing.map_or_else(now, |w| w.created_at);
        let id = ed.workflow.clone().unwrap_or_else(new_id);
        let draft = ed.draft(&id, created);
        let is_new = existing.is_none();
        let same = existing.is_some_and(|w| {
            w.name == draft.name && w.goal == draft.goal && w.definition == draft.definition
        });
        if !same {
            self.save("the workflow", |db| {
                if is_new {
                    db.insert_workflow(&draft)
                } else {
                    db.update_workflow(&draft)
                }
            });
            self.load_workflows();
        }
        if let Some(ed) = &mut self.wf_editor {
            ed.workflow = Some(id.clone());
        }
        Some(id)
    }

    /// Whether what is kept is what the editor shows.
    pub fn editor_saved(&self) -> bool {
        let Some(ed) = &self.wf_editor else {
            return true;
        };
        let Some(kept) = ed
            .workflow
            .as_ref()
            .and_then(|id| self.workflows.iter().find(|w| &w.id == id))
        else {
            return ed.steps.is_empty();
        };
        let now = ed.draft(&kept.id, kept.created_at);
        kept.name == now.name && kept.goal == now.goal && kept.definition == now.definition
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
        // The draft is kept even when it is not fit to publish.
        self.persist_draft();
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
        let Some(ed) = &mut self.wf_editor else {
            return;
        };
        ed.check();
        // What is wrong, said where the logs are, with the step each is about.
        self.wf_test.result = None;
        self.wf_test.notice = Some(if ed.problems.is_empty() {
            "Nothing is wrong: every step is ready to run.".to_owned()
        } else {
            problems_notice("What is wrong:", &ed.problems)
        });
        self.wf_logs_open = true;
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
        self.autosave();
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
        self.autosave();
    }

    pub fn editor_remove(&mut self, at: &Loc) {
        if let Some(ed) = &mut self.wf_editor {
            ed.remove(at);
            ed.check();
        }
        self.autosave();
    }

    pub fn editor_shift(&mut self, at: &Loc, by: isize) {
        if let Some(ed) = &mut self.wf_editor {
            ed.shift(at, by);
            ed.check();
        }
        self.autosave();
    }

    pub fn editor_add_case(&mut self, at: &Loc) {
        if let Some(ed) = &mut self.wf_editor {
            ed.add_case(at);
        }
        self.autosave();
    }

    pub fn editor_add_branch(&mut self, at: &Loc) {
        if let Some(ed) = &mut self.wf_editor {
            ed.add_branch(at);
        }
        self.autosave();
    }

    pub fn editor_remove_branch(&mut self, at: &Loc, block: usize) {
        if let Some(ed) = &mut self.wf_editor {
            ed.remove_branch(at, block);
            ed.check();
        }
        self.autosave();
    }

    pub fn editor_remove_case(&mut self, at: &Loc, block: usize) {
        if let Some(ed) = &mut self.wf_editor {
            ed.remove_case(at, block);
            ed.check();
        }
        self.autosave();
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
        self.autosave();
    }

    /// Chooses the tool for the step whose list is open.
    pub fn editor_choose_tool(&mut self, tool: &str) {
        if let Some(at) = self.wf_tool_step.clone() {
            self.editor_set_tool(&at, tool);
        }
        self.autosave();
    }

    /// Sets an argument of a tool step from a choice: one of a list, or yes or no.
    pub fn editor_set_param(&mut self, at: &Loc, key: &str, kind: ArgKind, text: &str) {
        if let Some(ed) = &mut self.wf_editor {
            ed.set_param(at, key, kind, text);
        }
        self.wf_menu = None;
        self.autosave();
    }

    /// The tools a step of the editor's workflow can use, by the name the model knows them by:
    /// the built-in ones a flow may call, TypeSafe's while it is connected, and those of the MCP
    /// servers that are on. Handing a task to an agent is its own kind of step, not a tool to
    /// choose.
    pub fn step_tools(&self, current: &str) -> Vec<String> {
        let mut tools: Vec<String> = STEP_TOOLS.iter().map(|t| (*t).to_owned()).collect();
        if self.typesafe_connected() {
            tools.extend(
                crate::typesafe::specs()
                    .into_iter()
                    .map(|t| t.name)
                    .filter(|n| crate::typesafe::is_tool(n)),
            );
        }
        for server in self.mcp.servers.iter().filter(|m| m.offers()) {
            tools.extend(server.tools.iter().map(|t| server.qualified(&t.name)));
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
            let agents: Vec<_> = self
                .roster
                .list
                .iter()
                .flat_map(|a| std::iter::once(a).chain(a.subagents.iter()))
                .cloned()
                .collect();
            return Some(crate::delegate::step_schema(&agents));
        }
        self.all_tools()
            .into_iter()
            .find(|t| t.name == tool)
            .map(|t| t.schema)
    }

    /// Has a model look at the result the step at `at` made when it was tried, and keep the fields
    /// it finds on the step, so boxes after it can offer them by name. The model is the one
    /// chosen for it, or else the workflow's agent's.
    /// What the step at `at` made in the last run of it: a top-level step's bound result, kept as
    /// it is; for a step inside another, what its last round wrote in the log, read as JSON if it
    /// is.
    pub(crate) fn result_of(&self, at: &Loc) -> Option<Value> {
        if at.place.is_empty() {
            return self
                .wf_test
                .memos
                .as_ref()
                .and_then(|m| m.steps.get(at.index))
                .and_then(|(_, memo)| memo.bound.as_ref())
                .map(|(_, v)| v.clone());
        }
        let ed = self.wf_editor.as_ref()?;
        let lines = match (&self.wf_test.result, self.wf_test.live.is_empty()) {
            (Some(r), _) => &r.steps,
            (None, false) => &self.wf_test.live,
            (None, true) => return None,
        };
        let line = lines.iter().rev().find(|l| {
            !l.running
                && l.error.is_none()
                && crate::workflow::trace::loc_of(ed, &l.path).as_ref() == Some(at)
        })?;
        Some(
            serde_json::from_str(&line.output)
                .unwrap_or_else(|_| Value::String(line.output.clone())),
        )
    }

    /// Runs the step at `at` as far as it needs, and finds its fields when that run is over. A
    /// step that keeps its result under no name is given one, because a top-level result is
    /// only kept by its name.
    fn run_then_discover(&mut self, at: &Loc) {
        // A step that is not finished cannot be run to look at what it makes, but the step before
        // it can, and what that makes is what this one is about to use: so look at that.
        let mut at = at.clone();
        let mut moved = false;
        while let Some(ed) = &self.wf_editor
            && ed.definition_through(&at).is_err()
            && let Some(before) = ed.step_before(&at)
        {
            at = before;
            moved = true;
        }
        let at = &at;
        if let Some(ed) = &mut self.wf_editor
            && at.place.is_empty()
            && let Some(node) = ed.node_mut(at)
            && node.bind.text.trim().is_empty()
        {
            node.bind = rune::TextEdit::new(result_name(node.id.text.trim()));
        }
        self.wf_mapper.after_run = Some(at.clone());
        self.wf_mapper.notice = Some(
            if moved {
                "This step is not finished, so the step before it is being run, to see what it makes…"
            } else {
                "Running the step first, to see what it makes…"
            }
            .to_owned(),
        );
        self.run_step(at);
        if self.wf_test.running.is_none() {
            // It did not start: the notice of the run says why.
            self.wf_mapper.after_run = None;
            self.wf_mapper.notice = Some(
                self.wf_test
                    .notice
                    .clone()
                    .unwrap_or_else(|| "The step could not be run.".to_owned()),
            );
        }
    }

    /// A run started by [`App::discover_fields`] has ended: look at what the step made, or say
    /// why there is nothing to look at.
    fn discover_after_run(&mut self) {
        let Some(at) = self.wf_mapper.after_run.take() else {
            return;
        };
        let failed = self.wf_test.result.as_ref().and_then(|r| match &r.ending {
            engine::Ending::Failed { error, .. } => Some(error.clone()),
            engine::Ending::Stopped => Some("it was stopped".to_owned()),
            engine::Ending::Waiting { .. } => Some("it is waiting for an answer".to_owned()),
            engine::Ending::Succeeded => None,
        });
        match failed {
            Some(why) => {
                self.wf_mapper.notice = Some(format!(
                    "The step did not finish, so there is nothing to look at: {why}"
                ));
            }
            None => self.discover_fields(&at),
        }
    }

    pub fn discover_fields(&mut self, at: &Loc) {
        if self.wf_test.running.is_some() || self.wf_mapper.running {
            return;
        }
        let Some(ed) = &self.wf_editor else { return };
        let say = |app: &mut App, why: &str| app.wf_mapper.notice = Some(why.to_owned());
        self.wf_mapper.notice = None;
        let Some(result) = self.result_of(at) else {
            // There is nothing to look at yet: run the step, with what is before it, and look at
            // what it made once it is done.
            return self.run_then_discover(at);
        };
        let message = match crate::workflow::discover::message(&result) {
            Ok(m) => m,
            Err(why) => return say(self, &why),
        };
        let agent_id = ed.agent_id.clone();
        let prep = match self.prepare_model_with(&agent_id, self.wf_mapper.model.as_ref()) {
            Ok(p) => p,
            Err(why) => return say(self, &why),
        };
        self.wf_mapper.running = true;
        let at = at.clone();
        self.spawn(
            move || {
                let cancel = AtomicBool::new(false);
                crate::flows::ask(&prep, &cancel, crate::workflow::discover::SYSTEM, &message)
                    .and_then(|c| crate::workflow::discover::read(&c.text, &result))
            },
            move |app, found| {
                app.wf_mapper.running = false;
                match found {
                    Ok(fields) => {
                        let n = fields.len();
                        if let Some(node) = app.wf_editor.as_mut().and_then(|e| e.node_mut(&at)) {
                            node.found = fields;
                            app.wf_mapper.notice = Some(format!(
                                "Found {n} field{}. They are offered under Use a result.",
                                if n == 1 { "" } else { "s" }
                            ));
                        } else {
                            app.wf_mapper.notice = Some("That step is gone.".to_owned());
                        }
                    }
                    Err(why) => app.wf_mapper.notice = Some(why),
                }
            },
        );
    }

    pub fn editor_add_reply(&mut self, at: &Loc) {
        if let Some(n) = self.wf_editor.as_mut().and_then(|e| e.node_mut(at)) {
            n.reply.push(crate::workflow::editor::ReplyRow {
                name: rune::TextEdit::new(""),
                kind: crate::workflow::definition::FieldKind::default(),
            });
        }
        self.autosave();
    }

    pub fn editor_remove_reply(&mut self, at: &Loc, i: usize) {
        if let Some(ed) = &mut self.wf_editor {
            if let Some(n) = ed.node_mut(at)
                && i < n.reply.len()
            {
                n.reply.remove(i);
            }
            ed.check();
        }
        self.autosave();
    }

    pub fn editor_set_reply_kind(&mut self, at: &Loc, i: usize, label: &str) {
        use crate::workflow::definition::FieldKind;
        if let Some(row) = self
            .wf_editor
            .as_mut()
            .and_then(|e| e.node_mut(at))
            .and_then(|n| n.reply.get_mut(i))
            && let Some(kind) = FieldKind::ALL.into_iter().find(|k| k.label() == label)
        {
            row.kind = kind;
        }
        self.wf_menu = None;
        self.autosave();
    }

    pub fn editor_toggle_all(&mut self, at: &Loc) {
        if let Some(n) = self.wf_editor.as_mut().and_then(|e| e.node_mut(at)) {
            n.all = !n.all;
        }
        self.autosave();
    }

    pub fn editor_toggle_go_on(&mut self, at: &Loc) {
        if let Some(n) = self.wf_editor.as_mut().and_then(|e| e.node_mut(at)) {
            n.go_on = !n.go_on;
        }
        self.autosave();
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
        self.autosave();
    }

    pub fn editor_remove_input(&mut self, i: usize) {
        if let Some(ed) = &mut self.wf_editor
            && i < ed.inputs.len()
        {
            ed.inputs.remove(i);
            ed.check();
        }
        self.autosave();
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
        self.autosave();
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
        self.autosave();
    }

    pub fn editor_set_compare(&mut self, at: &Loc, label: &str) {
        if let (Some(ed), Some(op)) = (&mut self.wf_editor, Op::from_label(label)) {
            ed.set_condition_op(at, op);
        }
        self.wf_menu = None;
        self.autosave();
    }

    pub fn editor_toggle_expression(&mut self, at: &Loc) {
        if let Some(ed) = &mut self.wf_editor {
            if ed.node_mut(at).is_some_and(|n| n.when.is_some()) {
                ed.condition_as_expression(at);
            } else {
                ed.condition_in_parts(at);
            }
        }
        self.autosave();
    }

    /// The ways to finish the name typed in a box, if it is one that does and has one.
    pub fn wf_completion(&self, c: &Complete, edit: &TextEdit) -> Option<Completion> {
        let (anchor, caret) = edit.range();
        if anchor != caret || self.wf_suggest.dismissed.as_deref() == Some(edit.text.as_str()) {
            return None;
        }
        self.wf_editor
            .as_ref()?
            .completions(&c.at, Some(&c.key), c.refs, &edit.text, caret)
    }

    /// A key typed in a box that finishes names, before the box sees it. True when it was taken.
    pub fn wf_complete_key(&mut self, c: &Complete, key: &rune::KeyEvent) -> bool {
        use rune::Key;
        let edit = self.editor_text(&c.target).clone();
        let Some(found) = self.wf_completion(c, &edit) else {
            return false;
        };
        let n = found.options.len();
        let active = self.wf_suggest.active.min(n - 1);
        let m = key.mods;
        if m.shift || m.cmd || m.ctrl || m.alt {
            return false;
        }
        match key.key {
            Key::Escape => {
                self.wf_suggest.dismissed = Some(edit.text.clone());
                true
            }
            Key::Up => {
                self.wf_suggest.active = (active + n - 1) % n;
                true
            }
            Key::Down => {
                self.wf_suggest.active = (active + 1) % n;
                true
            }
            Key::Enter | Key::Tab => {
                self.editor_complete(c, &found.options[active].clone());
                true
            }
            _ => false,
        }
    }

    /// Finishes the name being typed in a box with `option`.
    pub fn editor_complete(&mut self, c: &Complete, option: &str) {
        let (text, caret) = {
            let edit = self.editor_text(&c.target);
            (edit.text.clone(), edit.range().1)
        };
        let Some(from) = self
            .wf_editor
            .as_ref()
            .and_then(|ed| ed.completions(&c.at, Some(&c.key), c.refs, &text, caret))
            .map(|f| f.from)
        else {
            return;
        };
        let edit = self.editor_text(&c.target);
        edit.set_selection(from, caret);
        edit.insert(option);
        edit.focused = true;
        self.wf_suggest.active = 0;
        self.autosave();
    }

    pub fn editor_insert(&mut self, which: &Target, braces: bool, name: &str) {
        let text = if braces {
            format!("{{{{{name}}}}}")
        } else {
            name.to_owned()
        };
        self.editor_text(which).insert(&text);
        self.wf_menu = None;
        self.autosave();
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

    #[test]
    fn typesafe_tools_can_be_chosen_for_a_step_only_while_typesafe_is_connected() {
        let mut app = crate::testing::samples();
        let has = |app: &App| {
            app.step_tools("")
                .iter()
                .any(|t| t == crate::typesafe::YES_NO)
        };
        assert!(!has(&app));
        app.providers.connect(crate::providers::TYPESAFE);
        assert!(has(&app));
        let offered = app
            .step_tools("")
            .into_iter()
            .filter(|t| crate::typesafe::is_tool(t))
            .count();
        assert_eq!(offered, 4, "yes/no, choose, score and locate");
    }

    #[test]
    fn a_name_is_finished_by_key_or_click_and_put_away_with_escape() {
        use rune::{Key, KeyEvent};
        let db = TempDb::new();
        let mut app = on_the_page(&db);
        app.new_workflow();
        let label = crate::workflow::editor::KINDS
            .iter()
            .find(|k| k.kind == crate::workflow::definition::TOOL)
            .unwrap()
            .label;
        app.editor_add(&Vec::new(), label);
        app.editor_add(&Vec::new(), label);
        {
            let ed = app.wf_editor.as_mut().unwrap();
            ed.steps[0].bind = TextEdit::new("files");
        }
        let target = Target::Param(top(1), "command".into(), ArgKind::Text);
        let c = Complete {
            at: top(1),
            key: "command".into(),
            refs: Refs::Braces,
            target: target.clone(),
            anchor: "box".into(),
        };
        let edit = app.editor_text(&target);
        *edit = TextEdit::new("echo {{files.s");
        edit.set_caret(edit.text.len());
        let press = |key| KeyEvent::from(key);
        // Tab takes the first way to finish it.
        assert!(app.wf_complete_key(&c, &press(Key::Tab)));
        assert_eq!(app.editor_text(&target).text, "echo {{files.stdout");
        // Escape puts the list away for that text; other keys are the box's.
        *app.editor_text(&target) = TextEdit::new("{{fi");
        let edit = app.editor_text(&target);
        edit.set_caret(4);
        let shown = edit.clone();
        assert!(app.wf_completion(&c, &shown).is_some());
        assert!(app.wf_complete_key(&c, &press(Key::Escape)));
        assert!(app.wf_completion(&c, &shown).is_none());
        assert!(!app.wf_complete_key(&c, &press(Key::Text("x".into()))));
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

        app.editor_add(&Vec::new(), "Loop Over Items");
        set(&mut app, &Target::Field(top(0), 0), "[\"x\", \"y\"]");
        set(&mut app, &Target::Field(top(0), 1), "name");
        set(&mut app, &Target::Bind(top(0)), "greeted");
        let body: Place = vec![(0, 0)];
        app.editor_add(&body, "Tool");
        let inner = Loc {
            place: body,
            index: 0,
        };
        set(
            &mut app,
            &Target::Param(inner, "command".into(), ArgKind::Text),
            "echo {{name}}-{{index}}",
        );
        app.editor_add(&Vec::new(), "If");
        app.editor_toggle_expression(&top(1));
        set(&mut app, &Target::Field(top(1), 0), "inputs.loud");
        let then: Place = vec![(1, 0)];
        app.editor_add(&then, "Tool");
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
            app.wf_editor
                .as_ref()
                .is_some_and(|e| e.problems.is_empty()),
            "{:?}",
            app.wf_editor.as_ref().map(|e| &e.problems)
        );
        assert_eq!(app.workflows.len(), 1);
        assert!(app.editor_saved());
        // Saving keeps the editor open; leaving it goes back to the list.
        app.cancel_editor();
        assert!(app.wf_editor.is_none() && app.wf_open.is_none());
        let id = app.workflows[0].id.clone();

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

        app.select_workflow(&id);
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
        assert_eq!(app.wf_editor.as_ref().unwrap().name.text, "My workflow");
        set(&mut app, &Target::Name, "");
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
        // A workflow that is not fit to run is kept as a draft all the same, and cannot be
        // published.
        assert_eq!(app.workflows.len(), 1);
        assert!(app.workflows[0].live().is_none());
        app.publish_editor();
        assert!(app.workflows[0].live().is_none(), "not published");
        app.cancel_editor();
        assert!(app.wf_editor.is_none() && app.workflows.len() == 1);
    }

    #[test]
    fn changing_a_saved_workflow_keeps_it_the_same_workflow_and_a_running_one_cannot_be_changed() {
        let db = TempDb::new();
        let mut app = on_the_page(&db);
        app.new_workflow();
        set(&mut app, &Target::Name, "Old name");
        app.editor_add(&Vec::new(), "Edit Fields (Set)");
        set(&mut app, &Target::Field(top(0), 0), "a = 1");
        set(&mut app, &Target::Bind(top(0)), "made");
        app.save_editor();
        app.cancel_editor();
        let id = app.workflows[0].id.clone();
        let made = app.workflows[0].created_at;

        app.wf_jobs.insert(
            id.clone(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        app.open_in_builder(&id);
        assert!(app.wf_editor.is_none(), "not while it runs");
        app.wf_jobs.clear();

        app.open_in_builder(&id);
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
        app.editor_add(&Vec::new(), "Tool");
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
                .is_some_and(|n| n.contains("cannot run yet") && n.contains("timeout")),
            "the notice says what is wrong: {:?}",
            app.wf_test.notice
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
        // The draft is kept, as it is made; the run is not.
        assert_eq!(app.workflows.len(), 1);
        let id = app.workflows[0].id.clone();
        assert!(
            app.store
                .as_ref()
                .unwrap()
                .list_workflow_runs(&id, 10)
                .unwrap()
                .is_empty(),
            "no run was kept"
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
        app.editor_add(&Vec::new(), "Tool");
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
            app.editor_add(&Vec::new(), "Tool");
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

    #[cfg(unix)]
    #[test]
    fn a_step_can_be_run_as_far_as_while_others_are_not_finished_and_one_in_a_loop_gives_a_result()
    {
        let db = TempDb::new();
        let mut app = on_the_page(&db);
        app.new_workflow();
        let command = |at: Loc| Target::Param(at, "command".into(), ArgKind::Text);
        let inside = |index: usize| Loc {
            place: vec![(1, 0)],
            index,
        };
        app.editor_add(&Vec::new(), "Tool");
        app.editor_add(&Vec::new(), "Loop Over Items");
        app.editor_add(&vec![(1, 0)], "Tool");
        app.editor_add(&vec![(1, 0)], "Tool");
        app.editor_add(&Vec::new(), "Tool");
        set(&mut app, &command(top(0)), "echo first");
        set(&mut app, &Target::Field(top(1), 0), "[1, 2]");
        set(&mut app, &command(inside(0)), "echo item");
        // The second step in the loop and the step after it are not fit to run.
        let soon = |at: Loc| Target::Param(at, "timeout_secs".into(), ArgKind::Whole);
        set(&mut app, &soon(inside(1)), "soon");
        set(&mut app, &soon(top(2)), "soon");
        app.test_editor();
        assert!(
            app.wf_test.notice.is_some(),
            "the whole workflow is not fit to run"
        );

        app.run_step(&top(0));
        assert_eq!(app.wf_test.notice, None);
        let done = app.wf_test.result.as_ref().unwrap();
        assert_eq!(done.ending, crate::workflow::engine::Ending::Succeeded);
        assert_eq!(done.steps.len(), 1);

        // Inside the loop: the loop runs with that step last in it, once for each round.
        app.run_step(&inside(0));
        assert_eq!(app.wf_test.notice, None);
        let done = app.wf_test.result.as_ref().unwrap();
        assert_eq!(done.ending, crate::workflow::engine::Ending::Succeeded);
        assert_eq!(
            done.steps
                .iter()
                .filter(|l| l.output.contains("item"))
                .count(),
            2
        );
        let result = app.result_of(&inside(0)).unwrap();
        assert!(result.to_string().contains("item"), "{result}");

        // The one that is not finished says why it cannot run.
        app.run_step(&inside(1));
        assert!(
            app.wf_test
                .notice
                .as_deref()
                .is_some_and(|n| n.contains("cannot run yet")),
            "{:?}",
            app.wf_test.notice
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_workflow_is_kept_as_it_is_made_even_when_broken_and_a_step_can_run_what_is_before_it() {
        let db = TempDb::new();
        let mut app = on_the_page(&db);
        app.new_workflow();
        assert!(app.workflows.is_empty(), "nothing to keep before a step");
        // The first step is made: the workflow is kept at once, with the name it started with.
        app.editor_add(&Vec::new(), "Tool");
        assert_eq!(app.workflows.len(), 1);
        assert_eq!(app.workflows[0].name, "My workflow");
        assert!(app.editor_saved());
        let command = |at: usize| Target::Param(top(at), "command".into(), ArgKind::Text);
        set(&mut app, &command(0), "echo before");
        assert!(
            !app.editor_saved(),
            "what is typed waits for the next change"
        );
        app.editor_add(&Vec::new(), "Tool");
        set(
            &mut app,
            &Target::Param(top(1), "timeout_secs".into(), ArgKind::Whole),
            "soon",
        );
        // Selecting another step keeps what was typed, broken or not.
        app.select_step(None);
        assert!(app.editor_saved());
        assert_eq!(app.workflows[0].definition.steps.len(), 2);
        assert!(app.wf_editor.as_ref().unwrap().definition().is_err());

        // The broken step cannot run, but what is before it can.
        app.run_before(&top(1));
        assert_eq!(app.wf_test.notice, None);
        let done = app.wf_test.result.as_ref().unwrap();
        assert_eq!(done.steps.len(), 1);
        assert!(done.steps[0].output.contains("before"));
        // The first has nothing before it.
        app.run_before(&top(0));
        assert_eq!(
            app.wf_test.notice.as_deref(),
            Some("Nothing comes before this step.")
        );
    }

    #[cfg(unix)]
    #[test]
    fn finding_fields_runs_the_step_first_by_itself_and_names_its_result() {
        let db = TempDb::new();
        let mut app = on_the_page(&db);
        app.new_workflow();
        app.editor_add(&Vec::new(), "Tool");
        set(
            &mut app,
            &Target::Param(top(0), "command".into(), ArgKind::Text),
            "echo hello",
        );
        assert!(
            app.wf_editor.as_ref().unwrap().steps[0]
                .bind
                .text
                .is_empty()
        );
        app.discover_fields(&top(0));
        // It ran, kept its result under a name, and went on to look at it (there is no model in
        // this test to look, which it says).
        assert!(app.wf_test.running.is_none());
        let why = format!("{:?} / {:?}", app.wf_test.notice, app.wf_mapper.notice);
        let done = app.wf_test.result.as_ref().expect(&why);
        assert_eq!(done.ending, crate::workflow::engine::Ending::Succeeded);
        assert!(
            !app.wf_editor.as_ref().unwrap().steps[0]
                .bind
                .text
                .is_empty()
        );
        assert_eq!(app.wf_mapper.after_run, None);
        assert!(app.wf_mapper.notice.is_some());
        assert!(app.result_of(&top(0)).is_some());
    }

    #[test]
    fn a_result_is_named_from_the_step_in_letters_digits_and_underscores() {
        assert_eq!(result_name("step-1"), "step_1");
        assert_eq!(result_name("Get Mail"), "get_mail");
        assert_eq!(result_name("7"), "step_7");
        assert_eq!(result_name("error"), "result");
        assert_eq!(result_name(""), "result");
    }

    #[test]
    fn what_a_server_answers_is_read_as_fields_when_it_is_json_and_kept_as_words_when_not() {
        use crate::flows::structured;
        assert_eq!(structured(r#" {"a": 1} "#), serde_json::json!({"a": 1}));
        assert_eq!(structured("[1, 2]"), serde_json::json!([1, 2]));
        assert_eq!(structured("42"), Value::String("42".into()));
        assert_eq!(
            structured("No messages."),
            Value::String("No messages.".into())
        );
    }

    #[cfg(unix)]
    #[test]
    fn finding_fields_on_a_step_not_finished_runs_and_looks_at_the_step_before_it() {
        let db = TempDb::new();
        let mut app = on_the_page(&db);
        app.new_workflow();
        app.editor_add(&Vec::new(), "Tool");
        app.editor_add(&Vec::new(), "Loop Over Items");
        set(
            &mut app,
            &Target::Param(top(0), "command".into(), ArgKind::Text),
            "echo hello",
        );
        // The loop has no list to go over yet, so it cannot be run; the step before can.
        assert!(
            app.wf_editor
                .as_ref()
                .unwrap()
                .definition_through(&top(1))
                .is_err()
        );
        app.discover_fields(&top(1));
        let done = app
            .wf_test
            .result
            .as_ref()
            .expect("the step before was run");
        assert_eq!(done.ending, crate::workflow::engine::Ending::Succeeded);
        assert_eq!(done.steps.len(), 1);
        assert!(
            !app.wf_editor.as_ref().unwrap().steps[0]
                .bind
                .text
                .is_empty()
        );
        assert!(
            app.wf_editor.as_ref().unwrap().steps[1]
                .bind
                .text
                .is_empty()
        );
        // Nothing before the first step: it says what is wrong with the step itself.
        app.discover_fields(&top(0));
        assert!(app.result_of(&top(0)).is_some());
    }
}
