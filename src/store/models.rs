//! The models each provider offered when last asked.

use std::collections::HashMap;

use super::{Result, Store};
use crate::models::{Caps, ModelInfo, ProviderModels};

impl Store {
    /// Every provider's saved models, in the order they were listed.
    pub fn load_models(&self) -> Result<HashMap<String, ProviderModels>> {
        let mut out: HashMap<String, ProviderModels> = HashMap::new();
        {
            let mut stmt = self.conn.prepare_cached(self.sql("ListModels")?)?;
            let flag = |v: Option<i64>| v.map(|v| v != 0);
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    ModelInfo {
                        id: r.get(1)?,
                        name: r.get(2)?,
                        caps: Caps {
                            vision: flag(r.get(3)?),
                            reasoning: flag(r.get(4)?),
                            tools: flag(r.get(5)?),
                        },
                        context: r
                            .get::<_, Option<i64>>(6)?
                            .and_then(|n| u32::try_from(n).ok()),
                    },
                ))
            })?;
            for row in rows {
                let (provider, model) = row?;
                out.entry(provider).or_default().models.push(model);
            }
        }
        let mut stmt = self.conn.prepare_cached(self.sql("ListModelFetchTimes")?)?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (provider, at) = row?;
            out.entry(provider).or_default().fetched_at = Some(at);
        }
        Ok(out)
    }

    /// Replaces a provider's saved models (a refresh) in one step.
    pub fn save_models(&self, provider: &str, models: &[ModelInfo], fetched_at: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.write_models(provider, models, fetched_at)?;
        tx.commit()?;
        Ok(())
    }

    /// The rows, inside whatever transaction the caller opened.
    pub(super) fn write_models(
        &self,
        provider: &str,
        models: &[ModelInfo],
        fetched_at: i64,
    ) -> Result<()> {
        self.conn
            .prepare_cached(self.sql("DeleteModels")?)?
            .execute([provider])?;
        let mut insert = self.conn.prepare_cached(self.sql("InsertModel")?)?;
        for (i, m) in models.iter().enumerate() {
            insert.execute((
                provider,
                &m.id,
                &m.name,
                m.caps.vision.map(i64::from),
                m.caps.reasoning.map(i64::from),
                m.caps.tools.map(i64::from),
                i as i64,
                m.context.map(i64::from),
            ))?;
        }
        self.conn
            .prepare_cached(self.sql("SetModelsFetchedAt")?)?
            .execute((provider, fetched_at))?;
        Ok(())
    }
}
