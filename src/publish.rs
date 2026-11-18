//! Publishing a workflow: the draft the editor changes goes live, and what its trigger asks for
//! (a schedule) goes with it. Only the live copy runs for real, so a schedule never runs a
//! half-edited draft.

use chrono::Local;

use crate::app::{App, now};
use crate::schedule::{Schedule, Target};
use crate::workflow::definition::{Trigger, validate};
use crate::workflow::{Published, Saved, engine};

impl App {
    /// Makes the draft of workflow `id` its live copy. Errs, in words, if the draft is not fit to
    /// run (steps, a workflow it calls that is not live, a schedule that cannot fire), and then
    /// nothing changes.
    pub fn publish_workflow(&mut self, id: &str) -> Result<(), String> {
        let wf = self.find_workflow(id)?;
        let draft = &wf.definition;
        validate(draft).map_err(|why| why.join(" "))?;
        self.flow_closure(&wf.agent_id, Some(id), draft)?;
        let at = now();
        let schedule = match &draft.trigger {
            Trigger::Manual => None,
            Trigger::Schedule { timing, inputs } => {
                engine::bind_inputs(draft, inputs)?;
                let target = Target::Workflow {
                    id: id.to_owned(),
                    inputs: inputs.clone(),
                };
                let mut s =
                    Schedule::new(&wf.agent_id, &wf.name, target, timing.clone(), at, &Local)?;
                s.from_trigger = true;
                // The same row as before, so the fires it has made stay with it.
                if let Some(old) = self.trigger_schedule(id) {
                    s.id = old.id.clone();
                    s.created_at = old.created_at;
                    s.enabled = old.enabled;
                }
                Some(s)
            }
        };
        let published = Published {
            definition: draft.clone(),
            at,
        };
        self.store_publication(id, Some(published), schedule)
    }

    /// Takes workflow `id` down: nothing runs it for real until it is published again. Its draft
    /// stays.
    pub fn unpublish_workflow(&mut self, id: &str) -> Result<(), String> {
        self.find_workflow(id)?;
        self.store_publication(id, None, None)
    }

    /// Throws away the changes made since the last publish: the draft becomes the live copy again.
    pub fn revert_workflow(&mut self, id: &str) -> Result<(), String> {
        let mut wf = self.find_workflow(id)?;
        let live = wf
            .live()
            .cloned()
            .ok_or("This workflow was never published, so there is nothing to go back to.")?;
        wf.definition = live;
        if let Some(db) = &self.store {
            db.update_workflow(&wf).map_err(|e| e.to_string())?;
        }
        if let Some(slot) = self.workflows.iter_mut().find(|w| w.id == id) {
            *slot = wf;
        }
        Ok(())
    }

    fn find_workflow(&self, id: &str) -> Result<Saved, String> {
        self.workflows
            .iter()
            .find(|w| w.id == id)
            .cloned()
            .ok_or_else(|| "That workflow is gone.".to_owned())
    }

    /// The schedule workflow `id`'s trigger made, if it has one.
    pub(crate) fn trigger_schedule(&self, id: &str) -> Option<&Schedule> {
        self.schedules
            .iter()
            .find(|s| s.from_trigger && is_for(s, id))
    }

    /// Saves the live copy and its schedule together, then does the same in memory.
    fn store_publication(
        &mut self,
        id: &str,
        published: Option<Published>,
        schedule: Option<Schedule>,
    ) -> Result<(), String> {
        if let Some(db) = &self.store {
            db.set_published(id, published.as_ref(), schedule.as_ref())
                .map_err(|e| format!("Could not save that: {e}"))?;
        }
        let keep = schedule.as_ref().map(|s| s.id.clone());
        self.schedules
            .retain(|s| !(s.from_trigger && is_for(s, id)) || keep.as_deref() == Some(&s.id));
        if let Some(new) = schedule {
            match self.schedules.iter_mut().find(|s| s.id == new.id) {
                Some(slot) => *slot = new,
                None => self.schedules.push(new),
            }
        }
        if let Some(wf) = self.workflows.iter_mut().find(|w| w.id == id) {
            wf.published = published;
        }
        Ok(())
    }
}

/// Whether schedule `s` runs workflow `id`.
fn is_for(s: &Schedule, id: &str) -> bool {
    matches!(&s.target, Target::Workflow { id: w, .. } if w == id)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::app::Route;
    use crate::schedule::timing::Timing;
    use crate::testing::{TempDb, app_with_server};
    use crate::workflow::definition::{Definition, Input, Step};

    fn definition(command: &str) -> Definition {
        Definition {
            steps: vec![Step {
                id: "say".into(),
                kind: "tool".into(),
                tool: Some("shell".into()),
                args: json!({ "command": command }).as_object().cloned(),
                ..Step::default()
            }],
            ..Definition::default()
        }
    }

    /// An app with one saved, never published workflow.
    fn app_with_draft(db: &TempDb, def: Definition) -> (App, String) {
        let (mut app, _seen) = app_with_server(db, vec![]);
        app.roster.current_mut().unwrap().tools = vec![crate::tools::SHELL.into()];
        let wf = Saved {
            id: "w1".into(),
            agent_id: app.current().unwrap().id.clone(),
            name: "Flow".into(),
            definition: def,
            created_at: now(),
            ..Saved::default()
        };
        app.save("w", |db| db.insert_workflow(&wf));
        app.workflows.push(wf);
        (app, "w1".into())
    }

    fn reloaded(db: &TempDb) -> App {
        let mut later = db.open();
        later.load_workflows();
        later.load_schedules();
        later
    }

    #[test]
    fn a_draft_goes_live_when_published_and_later_edits_wait_until_the_next_publish() {
        let db = TempDb::new();
        let (mut app, id) = app_with_draft(&db, definition("echo one"));
        assert!(app.workflows[0].live().is_none());
        assert!(app.workflows[0].has_unpublished_changes());

        app.publish_workflow(&id).unwrap();
        let live = app.workflows[0].live().cloned().unwrap();
        assert_eq!(live, app.workflows[0].definition);
        assert!(!app.workflows[0].has_unpublished_changes());
        assert_eq!(reloaded(&db).workflows[0].live(), Some(&live));

        // Editing and saving changes the draft; what is live stays.
        let mut edited = app.workflows[0].clone();
        edited.definition = definition("echo two");
        app.save("w", |db| db.update_workflow(&edited));
        app.workflows[0] = edited;
        assert!(app.workflows[0].has_unpublished_changes());
        let later = reloaded(&db);
        assert_eq!(later.workflows[0].live(), Some(&live));
        assert_ne!(later.workflows[0].definition, live);

        // Going back to what is live drops the edit; publishing again takes it live.
        app.revert_workflow(&id).unwrap();
        assert_eq!(app.workflows[0].definition, live);
        assert_eq!(reloaded(&db).workflows[0].definition, live);
        app.workflows[0].definition = definition("echo two");
        app.publish_workflow(&id).unwrap();
        assert_eq!(app.workflows[0].live(), Some(&app.workflows[0].definition));
    }

    #[test]
    fn nothing_runs_for_real_until_it_is_published_and_unpublishing_takes_it_down() {
        let db = TempDb::new();
        let (mut app, id) = app_with_draft(&db, definition("echo hi"));
        app.go(Route::Workflows);
        app.select_workflow(&id);
        app.run_open_workflow();
        assert!(app.wf_runs.is_empty(), "no run was recorded");
        assert!(
            app.wf_notice
                .as_deref()
                .unwrap_or_default()
                .contains("not published"),
            "{:?}",
            app.wf_notice
        );

        app.publish_workflow(&id).unwrap();
        app.run_open_workflow();
        assert_eq!(app.wf_runs.len(), 1);

        app.unpublish_workflow(&id).unwrap();
        assert!(app.workflows[0].live().is_none());
        assert!(reloaded(&db).workflows[0].live().is_none());
        assert!(
            app.revert_workflow(&id).is_err(),
            "nothing live to go back to"
        );
    }

    #[test]
    fn a_schedule_trigger_makes_one_schedule_that_publishing_again_keeps_and_unpublishing_removes()
    {
        let db = TempDb::new();
        let mut def = definition("echo tick");
        def.trigger = Trigger::Schedule {
            timing: Timing::every(now() + 600, 3600).unwrap(),
            inputs: Default::default(),
        };
        let (mut app, id) = app_with_draft(&db, def);
        assert!(app.schedules.is_empty(), "a draft makes no schedule");

        app.publish_workflow(&id).unwrap();
        assert_eq!(app.schedules.len(), 1);
        let s = app.trigger_schedule(&id).unwrap().clone();
        assert!(s.from_trigger && s.enabled && s.next_at.is_some());
        assert_eq!(reloaded(&db).schedules, vec![s.clone()]);

        // Publishing again is the same schedule, so its fires stay with it.
        app.publish_workflow(&id).unwrap();
        assert_eq!(app.schedules.len(), 1);
        assert_eq!(app.schedules[0].id, s.id);
        assert_eq!(reloaded(&db).schedules.len(), 1);

        // A manual trigger has none: publishing it removes the schedule.
        app.workflows[0].definition.trigger = Trigger::Manual;
        app.publish_workflow(&id).unwrap();
        assert!(app.schedules.is_empty());
        assert!(reloaded(&db).schedules.is_empty());

        app.workflows[0].definition.trigger = Trigger::Schedule {
            timing: Timing::every(now() + 600, 3600).unwrap(),
            inputs: Default::default(),
        };
        app.publish_workflow(&id).unwrap();
        app.unpublish_workflow(&id).unwrap();
        assert!(app.schedules.is_empty());
        assert!(reloaded(&db).schedules.is_empty());
    }

    #[test]
    fn a_draft_that_cannot_run_is_refused_and_nothing_changes() {
        let db = TempDb::new();
        // A required input the schedule gives no value for.
        let mut def = definition("echo {{inputs.who}}");
        def.inputs.insert(
            "who".into(),
            Input {
                kind: "string".into(),
                description: String::new(),
                default: None,
            },
        );
        def.trigger = Trigger::Schedule {
            timing: Timing::every(now() + 600, 3600).unwrap(),
            inputs: Default::default(),
        };
        let (mut app, id) = app_with_draft(&db, def);
        let why = app.publish_workflow(&id).unwrap_err();
        assert!(why.contains("who"), "{why}");
        assert!(app.workflows[0].live().is_none() && app.schedules.is_empty());

        // A one-off time that has passed has nothing left to fire.
        app.workflows[0].definition.trigger = Trigger::Schedule {
            timing: Timing::Once(now() - 60),
            inputs: json!({"who": "me"}).as_object().cloned().unwrap(),
        };
        assert!(app.publish_workflow(&id).is_err());
        assert!(app.workflows[0].live().is_none() && app.schedules.is_empty());

        // A workflow it calls that is not live stops it too.
        let mut calls = definition("echo x");
        calls.steps[0] = Step {
            id: "call".into(),
            kind: "flow".into(),
            workflow: Some("Other".into()),
            bind: Some("r".into()),
            ..Step::default()
        };
        app.workflows[0].definition = calls;
        let other = Saved {
            id: "w2".into(),
            agent_id: app.workflows[0].agent_id.clone(),
            name: "Other".into(),
            definition: definition("echo y"),
            ..Saved::default()
        };
        app.workflows.push(other);
        let why = app.publish_workflow(&id).unwrap_err();
        assert!(why.contains("not published"), "{why}");
    }
}
