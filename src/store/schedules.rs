//! Schedules, their fires, and the drafts agents propose.

use serde_json::Map;

use super::{Result, Store};
use crate::schedule::{Fire, FireStatus, Proposal, Schedule, Target};

impl Store {
    /// Adds the schedule, or replaces the saved one with its id (its agent and creation time stay).
    pub fn put_schedule(&self, s: &Schedule) -> Result<()> {
        let (workflow_id, prompt, inputs) = match &s.target {
            Target::Prompt(p) => (None, p.as_str(), String::from("{}")),
            Target::Workflow { id, inputs } => {
                (Some(id.as_str()), "", serde_json::to_string(inputs)?)
            }
        };
        self.conn
            .prepare_cached(self.sql("PutSchedule")?)?
            .execute((
                &s.id,
                &s.agent_id,
                workflow_id,
                &s.name,
                prompt,
                inputs,
                serde_json::to_string(&s.timing)?,
                s.enabled,
                s.next_at,
                s.created_at,
            ))?;
        Ok(())
    }

    pub fn list_schedules(&self) -> Result<Vec<Schedule>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListSchedules")?)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, bool>(7)?,
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, i64>(9)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (
                id,
                agent_id,
                workflow_id,
                name,
                prompt,
                inputs,
                timing,
                enabled,
                next_at,
                created_at,
            ) = row?;
            let target = match workflow_id {
                Some(id) => Target::Workflow {
                    id,
                    inputs: serde_json::from_str::<Map<_, _>>(&inputs)?,
                },
                None => Target::Prompt(prompt),
            };
            out.push(Schedule {
                id,
                agent_id,
                name,
                target,
                timing: serde_json::from_str(&timing)?,
                enabled,
                next_at,
                created_at,
            });
        }
        Ok(out)
    }

    /// Records a pause, a resume, or the next fire time after a fire.
    pub fn set_schedule_state(&self, id: &str, enabled: bool, next_at: Option<i64>) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("SetScheduleState")?)?
            .execute((id, enabled, next_at))?;
        Ok(())
    }

    pub fn delete_schedule(&self, id: &str) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteSchedule")?)?
            .execute([id])?;
        Ok(())
    }

    pub fn insert_fire(&self, f: &Fire) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("InsertFire")?)?
            .execute((
                &f.id,
                &f.schedule_id,
                f.due_at,
                f.started_at,
                f.finished_at,
                f.status.as_str(),
                &f.note,
                &f.session_id,
                &f.workflow_run_id,
            ))?;
        Ok(())
    }

    /// Records how a fire ended.
    pub fn finish_fire(&self, f: &Fire) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("FinishFire")?)?
            .execute((
                &f.id,
                f.status.as_str(),
                &f.note,
                f.finished_at,
                &f.workflow_run_id,
            ))?;
        Ok(())
    }

    /// A schedule's newest fires, newest first.
    pub fn list_fires(&self, schedule_id: &str, limit: usize) -> Result<Vec<Fire>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListFires")?)?;
        let rows = stmt.query_map((schedule_id, limit as i64), |r| {
            Ok(Fire {
                id: r.get(0)?,
                schedule_id: r.get(1)?,
                due_at: r.get(2)?,
                started_at: r.get(3)?,
                finished_at: r.get(4)?,
                status: FireStatus::parse(&r.get::<_, String>(5)?),
                note: r.get(6)?,
                session_id: r.get(7)?,
                workflow_run_id: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A schedule's fires past its newest `keep`: each one's id and chat.
    pub fn old_fires(
        &self,
        schedule_id: &str,
        keep: usize,
    ) -> Result<Vec<(String, Option<String>)>> {
        let mut stmt = self.conn.prepare_cached(self.sql("OldFires")?)?;
        let rows = stmt.query_map((schedule_id, keep as i64), |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_fire(&self, id: &str) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteFire")?)?
            .execute([id])?;
        Ok(())
    }

    /// Start-up housekeeping: fires left running by a closed app are marked failed.
    pub fn interrupt_fires(&self, now: i64) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("InterruptFires")?)?
            .execute([now])?;
        Ok(())
    }

    pub fn put_proposal(&self, p: &Proposal) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("PutScheduleDraft")?)?
            .execute((
                &p.call_id,
                &p.session_id,
                p.updates.then_some(&p.schedule.id),
                serde_json::to_string(&p.schedule)?,
            ))?;
        Ok(())
    }

    pub fn list_proposals(&self) -> Result<Vec<Proposal>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListScheduleDrafts")?)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (call_id, session_id, schedule_id, body) = row?;
            out.push(Proposal {
                call_id,
                session_id,
                updates: schedule_id.is_some(),
                schedule: serde_json::from_str(&body)?,
            });
        }
        Ok(out)
    }

    pub fn delete_proposal(&self, call_id: &str) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteScheduleDraft")?)?
            .execute([call_id])?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;
    use crate::agents::Agent;
    use crate::schedule::Timing;
    use crate::vault::Vault;
    use crate::workflow::Saved;
    use crate::workflow::definition::{Definition, Step};

    /// A store with agent `a1`, its first chat, and its workflow `w1`.
    fn store() -> (Store, String) {
        let db = Store::in_memory(Vault::fixed([1; 32])).unwrap();
        let mut a = Agent::blank("a1".into());
        a.name = "Atlas".into();
        let session = a.sessions[0].id.clone();
        db.save_roster(&[a]).unwrap();
        db.insert_workflow(&Saved {
            id: "w1".into(),
            agent_id: "a1".into(),
            name: "Digest".into(),
            goal: String::new(),
            definition: Definition {
                steps: vec![Step {
                    id: "s".into(),
                    kind: "generate".into(),
                    instruction: Some("hi".into()),
                    ..Step::default()
                }],
                ..Definition::default()
            },
            created_at: 0,
        })
        .unwrap();
        (db, session)
    }

    fn prompt_schedule() -> Schedule {
        let t = Timing::cron("0 9 * * 1-5").unwrap();
        Schedule::new(
            "a1",
            "Morning",
            Target::Prompt("Plan my day".into()),
            t,
            0,
            &Utc,
        )
        .unwrap()
    }

    fn workflow_schedule() -> Schedule {
        let target = Target::Workflow {
            id: "w1".into(),
            inputs: json!({"folder": "notes"}).as_object().unwrap().clone(),
        };
        Schedule::new(
            "a1",
            "Digest",
            target,
            Timing::every(0, 900).unwrap(),
            0,
            &Utc,
        )
        .unwrap()
    }

    fn fire(schedule_id: &str, id: &str, session: Option<&str>) -> Fire {
        Fire {
            id: id.into(),
            schedule_id: schedule_id.into(),
            due_at: 10,
            started_at: 11,
            finished_at: None,
            status: FireStatus::Running,
            note: String::new(),
            session_id: session.map(Into::into),
            workflow_run_id: None,
        }
    }

    #[test]
    fn schedules_round_trip_and_update_in_place() {
        let (db, _) = store();
        let (mut p, w) = (prompt_schedule(), workflow_schedule());
        db.put_schedule(&p).unwrap();
        db.put_schedule(&w).unwrap();
        let mut all = db.list_schedules().unwrap();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(all, vec![w.clone(), p.clone()]);

        p.name = "Mornings".into();
        p.timing = Timing::Once(99);
        db.put_schedule(&p).unwrap();
        db.set_schedule_state(&w.id, false, None).unwrap();
        let all = db.list_schedules().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&p));
        let w2 = all.iter().find(|s| s.id == w.id).unwrap();
        assert_eq!((w2.enabled, w2.next_at), (false, None));

        db.delete_schedule(&w.id).unwrap();
        assert_eq!(db.list_schedules().unwrap(), vec![p]);
    }

    #[test]
    fn a_schedule_goes_with_its_agent_or_its_workflow() {
        let (db, _) = store();
        db.put_schedule(&prompt_schedule()).unwrap();
        db.put_schedule(&workflow_schedule()).unwrap();
        db.delete_workflow("w1").unwrap();
        let left: Vec<String> = db
            .list_schedules()
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(left, ["Morning"]);
        db.delete_agent("a1").unwrap();
        assert!(db.list_schedules().unwrap().is_empty());
    }

    #[test]
    fn the_table_refuses_a_prompt_schedule_with_no_prompt() {
        let (db, _) = store();
        let mut s = prompt_schedule();
        s.target = Target::Prompt("  ".into());
        assert!(db.put_schedule(&s).is_err());
    }

    #[test]
    fn fires_are_listed_newest_first_and_finish_with_their_outcome() {
        let (db, session) = store();
        let s = prompt_schedule();
        db.put_schedule(&s).unwrap();
        let mut first = fire(&s.id, "f1", Some(&session));
        db.insert_fire(&first).unwrap();
        let mut second = fire(&s.id, "f2", None);
        second.started_at = 20;
        second.status = FireStatus::Skipped;
        second.finished_at = Some(20);
        second.note = "still running".into();
        db.insert_fire(&second).unwrap();

        first.status = FireStatus::Succeeded;
        first.finished_at = Some(30);
        db.finish_fire(&first).unwrap();
        assert_eq!(db.list_fires(&s.id, 10).unwrap(), vec![second, first]);
        assert_eq!(db.list_fires(&s.id, 1).unwrap().len(), 1);
    }

    #[test]
    fn old_fires_are_those_past_the_newest_few() {
        let (db, _) = store();
        let s = prompt_schedule();
        db.put_schedule(&s).unwrap();
        for i in 0..5 {
            let mut f = fire(&s.id, &format!("f{i}"), None);
            f.started_at = i;
            db.insert_fire(&f).unwrap();
        }
        let old: Vec<String> = db
            .old_fires(&s.id, 3)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(old, ["f1", "f0"]);
        db.delete_fire("f0").unwrap();
        assert_eq!(db.list_fires(&s.id, 10).unwrap().len(), 4);
    }

    #[test]
    fn a_fire_left_running_reads_as_failed_and_loses_its_chat_link_when_the_chat_goes() {
        let (db, session) = store();
        let s = prompt_schedule();
        db.put_schedule(&s).unwrap();
        db.insert_fire(&fire(&s.id, "f1", Some(&session))).unwrap();
        db.interrupt_fires(50).unwrap();
        db.delete_session(&session).unwrap();
        let f = &db.list_fires(&s.id, 10).unwrap()[0];
        assert_eq!(f.status, FireStatus::Failed);
        assert_eq!(f.finished_at, Some(50));
        assert!(f.note.contains("closed"));
        assert_eq!(f.session_id, None);
    }

    #[test]
    fn proposals_round_trip_and_go_with_their_chat() {
        let (db, session) = store();
        let saved = prompt_schedule();
        db.put_schedule(&saved).unwrap();
        let new = Proposal {
            call_id: "c1".into(),
            session_id: session.clone(),
            updates: false,
            schedule: workflow_schedule(),
        };
        let mut change = saved.clone();
        change.name = "Later".into();
        let edit = Proposal {
            call_id: "c2".into(),
            session_id: session.clone(),
            updates: true,
            schedule: change,
        };
        db.put_proposal(&new).unwrap();
        db.put_proposal(&edit).unwrap();
        assert_eq!(db.list_proposals().unwrap(), vec![new, edit.clone()]);

        db.delete_proposal("c1").unwrap();
        assert_eq!(db.list_proposals().unwrap(), vec![edit]);
        db.delete_session(&session).unwrap();
        assert!(db.list_proposals().unwrap().is_empty());
    }
}
