//! Usage totals, model prices and the catalog's context limits.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, Row as SqlRow};

use super::{Result, Store};
use crate::models::ModelRef;
use crate::pricing::Catalog;
use crate::usage::{Price, Prices, Row, Source, Totals};
use crate::window::Limits;

/// Who a model call was for, as it is counted.
pub struct Counted<'a> {
    pub model: &'a ModelRef,
    pub agent_id: &'a str,
    pub agent_name: &'a str,
    pub source: Source,
}

impl Store {
    /// Adds `t` to the day of `at` (seconds since the epoch, counted in local time).
    pub fn add_usage(&self, at: i64, who: &Counted, t: &Totals) -> Result<()> {
        let n = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        self.conn.prepare_cached(self.sql("AddUsage")?)?.execute((
            at,
            &who.model.provider,
            &who.model.id,
            who.agent_id,
            who.agent_name,
            who.source.as_str(),
            n(t.calls),
            n(t.errors),
            n(t.input),
            n(t.output),
            n(t.cache_read),
            n(t.cache_write),
            n(t.first_ms_total),
            n(t.first_ms_count),
        ))?;
        Ok(())
    }

    /// Adds `t` to the chat's totals for `model`. Nothing happens for a chat that is not saved.
    pub fn add_session_usage(&self, session_id: &str, model: &ModelRef, t: &Totals) -> Result<()> {
        let n = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        self.conn
            .prepare_cached(self.sql("AddSessionUsage")?)?
            .execute((
                session_id,
                &model.provider,
                &model.id,
                n(t.input),
                n(t.output),
                n(t.cache_read),
                n(t.cache_write),
            ))?;
        Ok(())
    }

    /// The chat's tokens, by model.
    pub fn session_usage(&self, session_id: &str) -> Result<Vec<(ModelRef, Totals)>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListSessionUsage")?)?;
        let rows = stmt.query_map([session_id], session_usage_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Every chat's tokens by model, by chat id.
    pub fn all_session_usage(&self) -> Result<HashMap<String, Vec<(ModelRef, Totals)>>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListAllSessionUsage")?)?;
        let mut rows = stmt.query([])?;
        let mut out: HashMap<String, Vec<(ModelRef, Totals)>> = HashMap::new();
        while let Some(r) = rows.next()? {
            out.entry(r.get(6)?)
                .or_default()
                .push(session_usage_row(r)?);
        }
        Ok(out)
    }

    /// Whether a schedule opened the chat.
    pub(super) fn is_scheduled_session(&self, session_id: &str) -> Result<bool> {
        Ok(self
            .conn
            .prepare_cached(self.sql("IsScheduledSession")?)?
            .query_row([session_id], |r| r.get(0))?)
    }

    /// The daily totals from `since` (a `YYYY-MM-DD` day) on, or all of them, oldest first.
    pub fn usage(&self, since: Option<&str>) -> Result<Vec<Row>> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListUsage")?)?;
        let rows = stmt.query_map([since.unwrap_or("")], |r| {
            let n = |i: usize| r.get::<_, i64>(i).map(|v| v.max(0) as u64);
            Ok(Row {
                day: r.get(0)?,
                model: ModelRef::new(r.get::<_, String>(1)?, r.get::<_, String>(2)?),
                agent_id: r.get(3)?,
                agent_name: r.get(4)?,
                source: Source::parse(&r.get::<_, String>(5)?),
                totals: Totals {
                    calls: n(6)?,
                    errors: n(7)?,
                    input: n(8)?,
                    output: n(9)?,
                    cache_read: n(10)?,
                    cache_write: n(11)?,
                    first_ms_total: n(12)?,
                    first_ms_count: n(13)?,
                },
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Empties the daily totals. Chat totals are kept.
    pub fn clear_usage(&self) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("ClearUsage")?)?
            .execute([])?;
        Ok(())
    }

    pub fn prices(&self) -> Result<Prices> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListPrices")?)?;
        let mut rows = stmt.query([])?;
        let mut prices = Prices::default();
        while let Some(r) = rows.next()? {
            let model = ModelRef::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?);
            let price = Price {
                input: r.get(3)?,
                output: r.get(4)?,
                cache_read: r.get(5)?,
                cache_write: r.get(6)?,
            };
            match r.get::<_, String>(2)?.as_str() {
                "manual" => prices.manual.insert(model, price),
                _ => prices.catalog.insert(model, price),
            };
        }
        Ok(prices)
    }

    /// Puts the catalog's prices and context limits in place of the last fetch's, all at once.
    pub fn replace_catalog(&self, catalog: &Catalog, now: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.conn
            .prepare_cached(self.sql("DeleteCatalogPrices")?)?
            .execute([])?;
        for (model, p) in &catalog.prices {
            self.put_price(model, "catalog", p, now)?;
        }
        self.conn
            .prepare_cached(self.sql("DeleteLimits")?)?
            .execute([])?;
        let mut put = self.conn.prepare_cached(self.sql("PutLimit")?)?;
        for (model, n) in &catalog.limits {
            put.execute((&model.provider, &model.id, i64::from(*n)))?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The catalog's context limits, as last fetched.
    pub fn limits(&self) -> Result<Limits> {
        let mut stmt = self.conn.prepare_cached(self.sql("ListLimits")?)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                ModelRef::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?),
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Limits::default();
        for row in rows {
            let (model, n) = row?;
            if let Ok(n) = u32::try_from(n) {
                out.catalog.insert(model, n);
            }
        }
        Ok(out)
    }

    /// Your price for `model`, or `None` to go back to the catalog's.
    pub fn set_manual_price(
        &self,
        model: &ModelRef,
        price: Option<&Price>,
        now: i64,
    ) -> Result<()> {
        match price {
            Some(p) => self.put_price(model, "manual", p, now),
            None => {
                self.conn
                    .prepare_cached(self.sql("DeleteManualPrice")?)?
                    .execute((&model.provider, &model.id))?;
                Ok(())
            }
        }
    }

    /// When the catalog was last fetched, if ever.
    pub fn catalog_fetched_at(&self) -> Result<Option<i64>> {
        Ok(self
            .conn
            .prepare_cached(self.sql("CatalogFetchedAt")?)?
            .query_row([], |r| r.get(0))
            .optional()?
            .flatten())
    }

    fn put_price(&self, model: &ModelRef, origin: &str, p: &Price, now: i64) -> Result<()> {
        self.conn.prepare_cached(self.sql("PutPrice")?)?.execute((
            &model.provider,
            &model.id,
            origin,
            p.input,
            p.output,
            p.cache_read,
            p.cache_write,
            now,
        ))?;
        Ok(())
    }
}

/// A model and its tokens, from the first six columns of a chat-usage query.
fn session_usage_row(r: &SqlRow) -> rusqlite::Result<(ModelRef, Totals)> {
    let n = |i: usize| r.get::<_, i64>(i).map(|v| v.max(0) as u64);
    Ok((
        ModelRef::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?),
        Totals {
            input: n(2)?,
            output: n(3)?,
            cache_read: n(4)?,
            cache_write: n(5)?,
            ..Totals::default()
        },
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::agents::Agent;
    use crate::runs::{Run, Status, Step, StepKind};
    use crate::vault::Vault;

    fn store() -> Store {
        Store::in_memory(Vault::fixed([1; 32])).unwrap()
    }

    /// A store with agent `a1` and its first chat, and that chat's id.
    fn with_chat() -> (Store, String) {
        let db = store();
        let mut a = Agent::blank("a1".into());
        a.name = "Atlas".into();
        let session = a.sessions[0].id.clone();
        db.save_roster(&[a]).unwrap();
        (db, session)
    }

    /// 2100-01-01 at noon UTC: the same local day anywhere but the far Pacific.
    const NOON: i64 = 4_102_488_000;

    fn call(seq: u32, input: u32, cache_read: Option<u32>, status: Status) -> Step {
        let mut s = Step::new(seq, StepKind::Llm, NOON);
        (s.input_tokens, s.output_tokens) = (Some(input), Some(10));
        (s.cache_read_tokens, s.first_ms, s.status) = (cache_read, Some(300), status);
        s
    }

    fn run(id: &str, session: &str) -> Run {
        let mut r = Run::new(id, "a1", "Atlas", session, ModelRef::new("openai", "gpt-x"));
        r.started_at = NOON;
        r
    }

    #[test]
    fn each_model_call_is_counted_once_when_saved_under_what_made_it() {
        let (db, chat) = with_chat();
        let lead = run("lead", &chat);
        db.start_run(&lead).unwrap();
        db.save_run(
            &lead,
            &[
                Step::new(1, StepKind::User, NOON),
                call(2, 100, Some(60), Status::Ok),
            ],
        )
        .unwrap();
        db.save_run(&lead, &[call(3, 50, None, Status::Error)])
            .unwrap();
        db.save_run(&lead, &[]).unwrap();

        let mut member = run("member", &chat);
        member.parent = Some("lead".into());
        db.start_run(&member).unwrap();
        db.save_run(&member, &[call(1, 7, None, Status::Ok)])
            .unwrap();

        let rows = db.usage(None).unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        let of = |s: Source| rows.iter().find(|r| r.source == s).unwrap();
        let lead = of(Source::Chat);
        assert_eq!(lead.agent_name, "Atlas");
        assert_eq!(
            lead.totals,
            Totals {
                calls: 2,
                errors: 1,
                input: 150,
                output: 20,
                cache_read: 60,
                cache_write: 0,
                first_ms_total: 600,
                first_ms_count: 2,
            }
        );
        assert_eq!(
            of(Source::Member).totals.input,
            7,
            "its own calls, not its lead's"
        );

        let chat_total = db.session_usage(&chat).unwrap();
        assert_eq!(chat_total.len(), 1, "one model");
        assert_eq!(db.all_session_usage().unwrap()[&chat], chat_total);
        assert_eq!(
            (chat_total[0].1.input, chat_total[0].1.cache_read),
            (157, 60)
        );
    }

    #[test]
    fn a_chat_a_schedule_opened_counts_as_the_schedule_and_chat_totals_go_with_the_chat() {
        let (db, chat) = with_chat();
        db.conn
            .execute_batch(&format!(
                "INSERT INTO schedules (id, agent_id, name, prompt, timing)
                 VALUES ('sc', 'a1', 'Morning', 'Plan', '{{\"once\": 1}}');
                 INSERT INTO schedule_fires (id, schedule_id, due_at, started_at, status, session_id)
                 VALUES ('f', 'sc', 1, 1, 'running', '{chat}');"
            ))
            .unwrap();
        let r = run("r", &chat);
        db.start_run(&r).unwrap();
        db.save_run(&r, &[call(1, 5, None, Status::Ok)]).unwrap();
        assert_eq!(db.usage(None).unwrap()[0].source, Source::Schedule);

        let lost = run("lost", "no-such-chat");
        db.start_run(&lost).unwrap();
        db.save_run(&lost, &[call(1, 5, None, Status::Ok)]).unwrap();
        assert!(db.session_usage("no-such-chat").unwrap().is_empty());

        db.delete_session(&chat).unwrap();
        assert!(
            db.session_usage(&chat).unwrap().is_empty(),
            "gone with its chat"
        );
        assert_eq!(
            db.usage(None)
                .unwrap()
                .iter()
                .map(|r| r.totals.calls)
                .sum::<u64>(),
            2,
            "the daily totals stay"
        );
        db.clear_usage().unwrap();
        assert!(db.usage(None).unwrap().is_empty());
    }

    #[test]
    fn the_totals_are_read_from_a_day_on() {
        let (db, _) = with_chat();
        let model = ModelRef::new("openai", "gpt-x");
        let who = Counted {
            model: &model,
            agent_id: "a1",
            agent_name: "Atlas",
            source: Source::Workflow,
        };
        let t = Totals {
            calls: 3,
            input: 9,
            ..Totals::default()
        };
        let day = 24 * 60 * 60;
        for at in [NOON, NOON + day, NOON + 2 * day] {
            db.add_usage(at, &who, &t).unwrap();
        }
        let all = db.usage(None).unwrap();
        assert_eq!(
            all.iter().map(|r| r.day.as_str()).collect::<Vec<_>>(),
            ["2100-01-01", "2100-01-02", "2100-01-03"]
        );
        assert_eq!(db.usage(Some("2100-01-02")).unwrap().len(), 2);
        assert_eq!(all[0].source, Source::Workflow);
    }

    #[test]
    fn migrating_counts_the_runs_already_kept() {
        let dir = std::env::temp_dir().join(format!("oido-mig12-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("oido.db");
        {
            let conn = Connection::open(&path).unwrap();
            for sql in &super::super::MIGRATIONS[..11] {
                conn.execute_batch(sql).unwrap();
            }
            conn.execute_batch(&format!(
                "INSERT INTO runs (id, agent_id, agent_name, session_id, model_provider, model, status, started_at)
                 VALUES ('r', 'a', 'Atlas', 's', 'openai', 'gpt-x', 'ok', {NOON});
                 INSERT INTO runs (id, agent_id, agent_name, session_id, model_provider, model, status, started_at, parent_run_id)
                 VALUES ('m', 'b', 'Scout', 's', 'openai', 'gpt-x', 'ok', {NOON}, 'r');
                 INSERT INTO run_steps (run_id, seq, kind, input_tokens, output_tokens, first_ms, started_at)
                 VALUES ('r', 1, 'user', NULL, NULL, NULL, {NOON}),
                        ('r', 2, 'llm', 100, 10, 200, {NOON}),
                        ('r', 3, 'llm', 50, 5, NULL, {NOON}),
                        ('m', 1, 'llm', 7, 1, 100, {NOON});"
            ))
            .unwrap();
            conn.pragma_update(None, "user_version", 11).unwrap();
        }
        let db = Store::open(&path, Vault::fixed([1; 32])).unwrap();
        let rows = db.usage(None).unwrap();
        let of = |s: Source| rows.iter().find(|r| r.source == s).unwrap();
        let chat = of(Source::Chat);
        assert_eq!(chat.day, "2100-01-01");
        assert_eq!(
            (chat.totals.calls, chat.totals.input, chat.totals.output),
            (2, 150, 15)
        );
        assert_eq!(
            (chat.totals.first_ms_total, chat.totals.first_ms_count),
            (200, 1)
        );
        assert_eq!(of(Source::Member).agent_name, "Scout");
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn price(input: f64) -> Price {
        Price {
            input,
            output: 2.0 * input,
            cache_read: None,
            cache_write: Some(1.25 * input),
        }
    }

    fn catalog(prices: Vec<(ModelRef, Price)>) -> Catalog {
        Catalog {
            prices,
            limits: Vec::new(),
        }
    }

    #[test]
    fn a_new_fetch_replaces_the_context_limits_too() {
        let db = store();
        let (a, b) = (ModelRef::new("openai", "a"), ModelRef::new("openai", "b"));
        let with = |limits| Catalog {
            prices: vec![(a.clone(), price(1.0))],
            limits,
        };
        db.replace_catalog(&with(vec![(a.clone(), 128_000), (b.clone(), 8_000)]), 10)
            .unwrap();
        db.replace_catalog(&with(vec![(a.clone(), 200_000)]), 20)
            .unwrap();
        let got = db.limits().unwrap();
        assert_eq!(got.catalog.get(&a), Some(&200_000));
        assert_eq!(got.catalog.get(&b), None, "gone from the new catalog");
    }

    #[test]
    fn a_new_fetch_replaces_the_catalog_and_your_price_stays_and_wins() {
        let db = store();
        let (a, b) = (ModelRef::new("openai", "a"), ModelRef::new("openai", "b"));
        assert_eq!(db.catalog_fetched_at().unwrap(), None);
        db.replace_catalog(
            &catalog(vec![(a.clone(), price(1.0)), (b.clone(), price(2.0))]),
            10,
        )
        .unwrap();
        db.set_manual_price(&a, Some(&price(9.0)), 11).unwrap();
        db.replace_catalog(&catalog(vec![(a.clone(), price(3.0))]), 20)
            .unwrap();

        let p = db.prices().unwrap();
        assert_eq!(p.catalog.get(&a), Some(&price(3.0)));
        assert_eq!(p.catalog.get(&b), None, "gone from the new catalog");
        assert_eq!(p.get(&a), Some(&price(9.0)), "yours wins");
        assert_eq!(db.catalog_fetched_at().unwrap(), Some(20));

        db.set_manual_price(&a, None, 30).unwrap();
        assert_eq!(db.prices().unwrap().get(&a), Some(&price(3.0)));
    }
}
