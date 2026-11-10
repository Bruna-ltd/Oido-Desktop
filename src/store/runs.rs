//! Runs and their steps.

use super::usage::Counted;
use super::{Result, Store};
use crate::models::ModelRef;
use crate::runs::{Run, Status, Step, StepKind};
use crate::usage::{Source, Totals};

const COLUMNS: usize = 19;

impl Store {
    /// A run begins: saved as running, so one that is cut off can be told from one that finished.
    pub fn start_run(&self, r: &Run) -> Result<()> {
        self.conn.prepare_cached(self.sql("InsertRun")?)?.execute((
            &r.id,
            &r.agent_id,
            &r.agent_name,
            &r.session_id,
            &r.model.provider,
            &r.model.id,
            r.started_at,
            &r.parent,
            &r.title,
        ))?;
        Ok(())
    }

    /// A run moves on: how it stands now, and the steps it took since it was last saved, together.
    /// Each model call among the steps is counted in the usage totals, in the same transaction, so
    /// a call is counted exactly when it is recorded.
    pub fn save_run(&self, r: &Run, steps: &[Step]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.conn.prepare_cached(self.sql("UpdateRun")?)?.execute((
            &r.id,
            r.status.as_str(),
            &r.error,
            r.input_tokens,
            r.output_tokens,
            r.duration_ms,
            r.ended_at,
            &r.title,
            r.cache_read_tokens,
            r.cache_write_tokens,
        ))?;
        let mut insert = self.conn.prepare_cached(self.sql("InsertStep")?)?;
        for s in steps {
            insert.execute(rusqlite::params![
                &r.id,
                s.seq,
                s.kind.as_str(),
                &s.name,
                s.status.as_str(),
                &s.input,
                &s.output,
                &s.reasoning,
                s.input_tokens,
                s.output_tokens,
                s.duration_ms,
                s.started_at,
                s.at_ms,
                s.first_ms,
                i64::from(s.truncated),
                s.cache_read_tokens,
                s.cache_write_tokens,
            ])?;
        }
        self.count_calls(r, steps)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds the model calls among `steps` to the day's totals and the chat's.
    fn count_calls(&self, r: &Run, steps: &[Step]) -> Result<()> {
        let mut calls = steps.iter().filter(|s| s.kind == StepKind::Llm).peekable();
        if calls.peek().is_none() {
            return Ok(());
        }
        let source = if r.parent.is_some() {
            Source::Member
        } else if self.is_scheduled_session(&r.session_id)? {
            Source::Schedule
        } else {
            Source::Chat
        };
        for s in calls {
            let who = Counted {
                model: &r.model,
                agent_id: &r.agent_id,
                agent_name: &r.agent_name,
                // A summary written to make room is counted apart, whatever the run was for.
                source: if s.name == crate::window::SUMMARY_STEP {
                    Source::Compaction
                } else {
                    source
                },
            };
            let t = Totals::of_call(s);
            self.add_usage(s.started_at, &who, &t)?;
            self.add_session_usage(&r.session_id, &r.model, &t)?;
        }
        Ok(())
    }

    /// The newest runs, newest first.
    pub fn list_runs(&self, limit: usize) -> Result<Vec<Run>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListRuns")?)?;
        let rows = stmt.query_map([limit as i64], |r| {
            debug_assert!(r.as_ref().column_count() >= COLUMNS);
            Ok(Run {
                id: r.get(0)?,
                agent_id: r.get(1)?,
                agent_name: r.get(2)?,
                session_id: r.get(3)?,
                model: ModelRef {
                    provider: r.get(4)?,
                    id: r.get(5)?,
                },
                status: Status::parse(&r.get::<_, String>(6)?),
                error: r.get(7)?,
                input_tokens: r.get(8)?,
                output_tokens: r.get(9)?,
                duration_ms: r.get(10)?,
                started_at: r.get(11)?,
                ended_at: r.get(12)?,
                parent: r.get(13)?,
                title: r.get(14)?,
                calls: r.get(15)?,
                tools: r.get(16)?,
                cache_read_tokens: r.get(17)?,
                cache_write_tokens: r.get(18)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// A run's steps, in order.
    pub fn list_steps(&self, run_id: &str) -> Result<Vec<Step>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListSteps")?)?;
        let rows = stmt.query_map([run_id], |r| {
            Ok(Step {
                seq: r.get(0)?,
                kind: StepKind::parse(&r.get::<_, String>(1)?),
                name: r.get(2)?,
                status: Status::parse(&r.get::<_, String>(3)?),
                input: r.get(4)?,
                output: r.get(5)?,
                reasoning: r.get(6)?,
                input_tokens: r.get(7)?,
                output_tokens: r.get(8)?,
                duration_ms: r.get(9)?,
                started_at: r.get(10)?,
                at_ms: r.get(11)?,
                first_ms: r.get(12)?,
                truncated: r.get::<_, i64>(13)? != 0,
                cache_read_tokens: r.get(14)?,
                cache_write_tokens: r.get(15)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Start-up housekeeping: removes runs older than `keep_secs`, and marks runs still running as
    /// failed, since the app that was running them is gone.
    pub fn tidy_runs(&self, now: i64, keep_secs: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.conn
            .prepare_cached(self.sql("DeleteRunsBefore")?)?
            .execute([now - keep_secs])?;
        self.conn
            .prepare_cached(self.sql("InterruptRunningRuns")?)?
            .execute([now])?;
        tx.commit()?;
        Ok(())
    }

    /// Removes the chat's messages from `seq` on (a retry drops the reply it redoes).
    pub fn delete_messages_from(&self, session_id: &str, seq: usize) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteMessagesFrom")?)?
            .execute((session_id, seq as i64))?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::model::Message;
    use crate::vault::Vault;

    fn store() -> Store {
        Store::in_memory(Vault::fixed([1; 32])).unwrap()
    }

    fn run(id: &str, started_at: i64) -> Run {
        let mut r = Run::new(id, "a", "Atlas", "s", ModelRef::new("openai", "gpt-x"));
        r.started_at = started_at;
        r
    }

    fn step(seq: u32, kind: StepKind) -> Step {
        let mut s = Step::new(seq, kind, 0);
        s.output = Some(format!("out{seq}"));
        s
    }

    #[test]
    fn a_run_is_saved_running_then_finished_with_its_steps_and_listed_newest_first() {
        let db = store();
        db.start_run(&run("old", 10)).unwrap();
        db.start_run(&run("new", 20)).unwrap();
        assert_eq!(db.list_runs(10).unwrap()[0].status, Status::Running);

        let mut done = run("new", 20);
        done.title = "Sum the files".into();
        db.save_run(&done, &[step(1, StepKind::User), step(2, StepKind::Llm)])
            .unwrap();
        let mut tool = step(3, StepKind::Tool);
        (tool.at_ms, tool.duration_ms) = (Some(1200), Some(40));
        (done.status, done.input_tokens, done.output_tokens) = (Status::Ok, Some(5), Some(7));
        (done.duration_ms, done.ended_at) = (Some(900), Some(21));
        (done.calls, done.tools) = (1, 1);
        db.save_run(&done, &[tool.clone()]).unwrap();

        let runs = db.list_runs(10).unwrap();
        assert_eq!(
            runs.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["new", "old"]
        );
        assert_eq!(runs[0], done, "saved as it went, counted when read");
        assert_eq!(db.list_runs(1).unwrap().len(), 1, "limited");
        let steps = db.list_steps("new").unwrap();
        assert_eq!(steps.iter().map(|s| s.seq).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(steps[1].kind, StepKind::Llm);
        assert_eq!(steps[1].output.as_deref(), Some("out2"));
        assert_eq!(steps[2], tool);
    }

    #[test]
    fn starting_the_app_removes_old_runs_with_their_steps_and_ends_runs_that_were_cut_off() {
        let db = store();
        let day = 24 * 60 * 60;
        let now = 100 * day;
        for (id, at) in [("ancient", now - 31 * day), ("recent", now - day)] {
            db.start_run(&run(id, at)).unwrap();
            let mut r = run(id, at);
            r.status = Status::Ok;
            db.save_run(&r, &[step(1, StepKind::User)]).unwrap();
        }
        db.start_run(&run("cut-off", now - 60)).unwrap();

        db.tidy_runs(now, 30 * day).unwrap();
        let runs = db.list_runs(10).unwrap();
        let ids: Vec<_> = runs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["cut-off", "recent"]);
        assert!(
            db.list_steps("ancient").unwrap().is_empty(),
            "its steps went with it"
        );
        assert_eq!(runs[0].status, Status::Error);
        assert!(runs[0].error.as_deref().unwrap().contains("closed"));
        assert_eq!(runs[1].status, Status::Ok, "a finished run is left alone");
    }

    #[test]
    fn deleting_messages_from_a_place_keeps_the_ones_before() {
        let db = store();
        let agent = crate::agents::Agent::blank("a".into());
        let mut chat = agent.sessions[0].clone();
        chat.messages = vec![
            Message::User("1".into()),
            Message::reply("2", None, crate::model::Outcome::Done),
            Message::User("3".into()),
        ];
        let mut agent = agent;
        agent.name = "Atlas".into();
        agent.sessions = vec![chat.clone()];
        db.save_agent_tree(&agent, None, 0).unwrap();
        db.delete_messages_from(&chat.id, 1).unwrap();
        let loaded = db.load_agents().unwrap();
        assert_eq!(
            loaded[0].sessions[0].messages,
            vec![Message::User("1".into())]
        );
    }
}
