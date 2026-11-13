//! Workflows, their drafts and their runs.

use super::{Result, Store};
use crate::workflow::definition::Definition;
use crate::workflow::{Draft, RunRecord, RunStatus, Saved};

fn definition(text: &str) -> Result<Definition> {
    Ok(serde_json::from_str(text)?)
}

impl Store {
    pub fn insert_workflow(&self, w: &Saved) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("InsertWorkflow")?)?
            .execute((
                &w.id,
                &w.agent_id,
                &w.name,
                &w.goal,
                serde_json::to_string(&w.definition)?,
                w.created_at,
            ))?;
        Ok(())
    }

    /// Saves what the editor changed: the name, the goal and the steps.
    pub fn update_workflow(&self, w: &Saved) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("UpdateWorkflow")?)?
            .execute((
                &w.id,
                &w.name,
                &w.goal,
                serde_json::to_string(&w.definition)?,
            ))?;
        Ok(())
    }

    pub fn list_workflows(&self) -> Result<Vec<Saved>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListWorkflows")?)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, agent_id, name, goal, def, created_at) = row?;
            out.push(Saved {
                id,
                agent_id,
                name,
                goal,
                definition: definition(&def)?,
                created_at,
            });
        }
        Ok(out)
    }

    pub fn delete_workflow(&self, id: &str) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteWorkflow")?)?
            .execute([id])?;
        Ok(())
    }

    pub fn put_draft(&self, d: &Draft) -> Result<()> {
        self.conn.prepare_cached(self.sql("PutDraft")?)?.execute((
            &d.session_id,
            &d.agent_id,
            &d.name,
            &d.goal,
            serde_json::to_string(&d.definition)?,
        ))?;
        Ok(())
    }

    pub fn list_drafts(&self) -> Result<Vec<Draft>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListDrafts")?)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (session_id, agent_id, name, goal, def) = row?;
            out.push(Draft {
                session_id,
                agent_id,
                name,
                goal,
                definition: definition(&def)?,
            });
        }
        Ok(out)
    }

    pub fn delete_draft(&self, session_id: &str) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteDraft")?)?
            .execute([session_id])?;
        Ok(())
    }

    pub fn start_workflow_run(&self, r: &RunRecord) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("StartWorkflowRun")?)?
            .execute((
                &r.id,
                &r.workflow_id,
                &r.started_by,
                &r.inputs,
                r.started_at,
            ))?;
        Ok(())
    }

    pub fn finish_workflow_run(&self, r: &RunRecord) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("FinishWorkflowRun")?)?
            .execute((
                &r.id,
                r.status.as_str(),
                &r.output,
                &r.failed_step,
                &r.error,
                serde_json::to_string(&r.steps)?,
                r.input_tokens,
                r.output_tokens,
                r.finished_at,
            ))?;
        Ok(())
    }

    /// A workflow's newest runs, newest first.
    pub fn list_workflow_runs(&self, workflow_id: &str, limit: usize) -> Result<Vec<RunRecord>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListWorkflowRuns")?)?;
        let rows = stmt.query_map((workflow_id, limit as i64), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, u32>(9)?,
                r.get::<_, u32>(10)?,
                r.get::<_, i64>(11)?,
                r.get::<_, Option<i64>>(12)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (
                id,
                workflow_id,
                started_by,
                status,
                inputs,
                output,
                failed_step,
                error,
                steps,
                input_tokens,
                output_tokens,
                started_at,
                finished_at,
            ) = row?;
            out.push(RunRecord {
                id,
                workflow_id,
                started_by,
                status: RunStatus::parse(&status),
                inputs,
                output,
                failed_step,
                error,
                steps: serde_json::from_str(&steps)?,
                input_tokens,
                output_tokens,
                started_at,
                finished_at,
            });
        }
        Ok(out)
    }

    /// Start-up housekeeping: runs left running by a closed app are marked failed, and old runs go.
    pub fn tidy_workflow_runs(&self, now: i64, keep_secs: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.conn
            .prepare_cached(self.sql("InterruptWorkflowRuns")?)?
            .execute([now])?;
        self.conn
            .prepare_cached(self.sql("DeleteWorkflowRunsBefore")?)?
            .execute([now - keep_secs])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::agents::Agent;
    use crate::vault::Vault;
    use crate::workflow::definition::Step;
    use crate::workflow::engine::StepLog;

    fn store_with_agent() -> (Store, String, String) {
        let db = Store::in_memory(Vault::fixed([1; 32])).unwrap();
        let mut a = Agent::blank("a1".into());
        a.name = "Atlas".into();
        let session = a.sessions[0].id.clone();
        db.save_roster(&[a]).unwrap();
        (db, "a1".into(), session)
    }

    fn def() -> Definition {
        Definition {
            steps: vec![Step {
                id: "s".into(),
                kind: "generate".into(),
                instruction: Some("hi".into()),
                ..Step::default()
            }],
            ..Definition::default()
        }
    }

    fn saved(agent: &str) -> Saved {
        Saved {
            id: "w1".into(),
            agent_id: agent.into(),
            name: "Daily".into(),
            goal: "Does it.".into(),
            definition: def(),
            created_at: 5,
        }
    }

    fn run(status: RunStatus, at: i64) -> RunRecord {
        RunRecord {
            id: format!("r{at}"),
            workflow_id: "w1".into(),
            started_by: "you".into(),
            status,
            inputs: "{}".into(),
            output: String::new(),
            failed_step: String::new(),
            error: String::new(),
            steps: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            started_at: at,
            finished_at: None,
        }
    }

    #[test]
    fn a_workflow_its_draft_and_its_runs_round_trip_and_go_with_their_owners() {
        let (db, agent, session) = store_with_agent();
        db.insert_workflow(&saved(&agent)).unwrap();
        assert_eq!(db.list_workflows().unwrap(), vec![saved(&agent)]);

        let draft = Draft {
            session_id: session.clone(),
            agent_id: agent.clone(),
            name: "D".into(),
            goal: "g".into(),
            definition: def(),
        };
        db.put_draft(&draft).unwrap();
        let mut again = draft.clone();
        again.name = "D2".into();
        db.put_draft(&again).unwrap();
        assert_eq!(
            db.list_drafts().unwrap(),
            vec![again],
            "one draft per chat, replaced in place"
        );
        db.delete_draft(&session).unwrap();
        assert!(db.list_drafts().unwrap().is_empty());

        db.start_workflow_run(&run(RunStatus::Running, 10)).unwrap();
        let mut done = run(RunStatus::Running, 10);
        done.status = RunStatus::Failed;
        done.failed_step = "s".into();
        done.error = "boom".into();
        done.input_tokens = 7;
        done.finished_at = Some(11);
        done.steps = vec![StepLog {
            id: "s".into(),
            kind: "generate".into(),
            what: "w".into(),
            output: "o".into(),
            error: Some("boom".into()),
            ms: 3,
            path: "0.then.1".into(),
            rounds: vec![2],
        }];
        db.finish_workflow_run(&done).unwrap();
        assert_eq!(db.list_workflow_runs("w1", 10).unwrap(), vec![done]);

        db.delete_agent(&agent).unwrap();
        assert!(
            db.list_workflows().unwrap().is_empty(),
            "the workflow went with its agent"
        );
        assert!(
            db.list_workflow_runs("w1", 10).unwrap().is_empty(),
            "and its runs"
        );
    }

    #[test]
    fn starting_the_app_ends_runs_that_were_cut_off_and_removes_old_finished_ones() {
        let (db, agent, _) = store_with_agent();
        db.insert_workflow(&saved(&agent)).unwrap();
        let day = 24 * 60 * 60;
        let now = 100 * day;
        for (status, at) in [
            (RunStatus::Succeeded, now - 40 * day),
            (RunStatus::Succeeded, now - day),
            (RunStatus::Running, now - 60),
        ] {
            db.start_workflow_run(&run(status, at)).unwrap();
            if status != RunStatus::Running {
                db.finish_workflow_run(&run(status, at)).unwrap();
            }
        }
        db.tidy_workflow_runs(now, 30 * day).unwrap();
        let runs = db.list_workflow_runs("w1", 10).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].status, RunStatus::Failed);
        assert!(runs[0].error.contains("closed"));
        assert_eq!(runs[1].status, RunStatus::Succeeded);
    }

    #[test]
    fn editing_a_workflow_keeps_its_agent_its_runs_and_when_it_was_made() {
        let (db, agent, _) = store_with_agent();
        db.insert_workflow(&saved(&agent)).unwrap();
        db.start_workflow_run(&run(RunStatus::Running, 7)).unwrap();
        let mut w = saved(&agent);
        w.name = "Weekly".into();
        w.goal = "Better.".into();
        w.created_at = 99;
        w.definition.steps[0].instruction = Some("changed".into());
        db.update_workflow(&w).unwrap();
        let got = db.list_workflows().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(
            (
                got[0].name.as_str(),
                got[0].goal.as_str(),
                got[0].created_at
            ),
            ("Weekly", "Better.", 5)
        );
        assert_eq!(
            got[0].definition.steps[0].instruction.as_deref(),
            Some("changed")
        );
        assert_eq!(db.list_workflow_runs("w1", 10).unwrap().len(), 1);
    }
}
