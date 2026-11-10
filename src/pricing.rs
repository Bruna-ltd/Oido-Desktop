//! Model prices and context limits from models.dev, a public catalog: fetched in the background at
//! most once a day and kept in the database, so cost shows offline. Only the built-in providers are
//! looked up; a local or custom server's models have no catalog price and take the one you set.

use std::time::Duration;

use serde_json::Value;

use crate::app::{App, now};
use crate::models::ModelRef;
use crate::update::VERSION;
use crate::usage::Price;

const URL: &str = "https://models.dev/api.json";

/// The catalog is fetched again once it is this old.
const STALE_AFTER_SECS: i64 = 24 * 60 * 60;

/// The whole catalog is one file of a few megabytes; this leaves it room to grow.
const MAX_BYTES: u64 = 64 * 1024 * 1024;

const TIMEOUT: Duration = Duration::from_secs(60);

/// Our provider id and the catalog's, where they differ or exist at all.
const CATALOG_IDS: [(&str, &str); 9] = [
    ("openai", "openai"),
    ("anthropic", "anthropic"),
    ("gemini", "google"),
    ("openrouter", "openrouter"),
    ("nvidia", "nvidia"),
    ("deepseek", "deepseek"),
    ("mistral", "mistral"),
    ("kimi", "moonshotai"),
    ("qwen", "alibaba"),
];

/// What the catalog says about models, under our provider ids.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Catalog {
    pub prices: Vec<(ModelRef, Price)>,
    /// How many tokens each model takes at once.
    pub limits: Vec<(ModelRef, u32)>,
}

/// The prices and context limits in a models.dev catalog. A model with no input or output price
/// has no price, and one with no context limit has no limit. Prices that change with the length of
/// the context are not modelled: the base price is used.
///
/// # Errors
///
/// The text is not a catalog, in words.
pub fn parse(json: &str) -> Result<Catalog, String> {
    let all: Value =
        serde_json::from_str(json).map_err(|e| format!("the price list is not valid: {e}"))?;
    if !all.is_object() {
        return Err("the price list is not a catalog".to_owned());
    }
    let mut out = Catalog::default();
    for (ours, theirs) in CATALOG_IDS {
        let Some(models) = all
            .pointer(&format!("/{theirs}/models"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (id, m) in models {
            let cost = |k: &str| {
                m.pointer(&format!("/cost/{k}"))
                    .and_then(Value::as_f64)
                    .filter(|p| p.is_finite() && *p >= 0.0)
            };
            if let Some(n) = m
                .pointer("/limit/context")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
            {
                out.limits.push((ModelRef::new(ours, id.as_str()), n));
            }
            if let (Some(input), Some(output)) = (cost("input"), cost("output")) {
                out.prices.push((
                    ModelRef::new(ours, id.as_str()),
                    Price {
                        input,
                        output,
                        cache_read: cost("cache_read"),
                        cache_write: cost("cache_write"),
                    },
                ));
            }
        }
    }
    Ok(out)
}

/// The id the catalog most likely lists a model under when it does not list the exact one: without
/// a date (`claude-x-20251101`, `gpt-x-2024-08-06`) or a variant (`vendor/model:free`).
pub fn base_id(id: &str) -> Option<&str> {
    let id_no_variant = id.split_once(':').map_or(id, |(base, _)| base);
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let undated = id_no_variant
        .rsplit_once('-')
        .filter(|(_, d)| d.len() == 8 && digits(d))
        .map(|(base, _)| base)
        .or_else(|| {
            // -YYYY-MM-DD: the last three parts are numbers of four, two and two digits.
            let mut parts = id_no_variant.rsplitn(4, '-');
            let (d, m, y) = (parts.next()?, parts.next()?, parts.next()?);
            let base = parts.next()?;
            (y.len() == 4 && m.len() == 2 && d.len() == 2 && [y, m, d].iter().all(|p| digits(p)))
                .then_some(base)
        })
        .unwrap_or(id_no_variant);
    (undated != id).then_some(undated)
}

/// Fetches the catalog. Blocks, so call it from a worker thread.
fn fetch() -> Result<Catalog, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(format!("oido-desktop/{VERSION}"))
        .build()
        .into();
    let body = agent
        .get(URL)
        .call()
        .map_err(|e| format!("could not reach models.dev: {e}"))?
        .into_body()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_string()
        .map_err(|e| format!("could not read the price list: {e}"))?;
    parse(&body)
}

impl App {
    /// Reads the saved prices and context limits.
    pub(crate) fn load_prices(&mut self) {
        let Some(db) = &self.store else {
            return;
        };
        match db.prices() {
            Ok(p) => self.prices = p,
            Err(e) => eprintln!("oido: could not read model prices: {e}"),
        }
        match db.limits() {
            Ok(l) => self.limits = l,
            Err(e) => eprintln!("oido: could not read model context limits: {e}"),
        }
    }

    /// Fetches the catalog when the saved one is a day old or missing. The last prices stay when the
    /// fetch fails.
    pub fn refresh_prices(&mut self) {
        let Some(db) = &self.store else {
            return;
        };
        let fresh = db
            .catalog_fetched_at()
            .ok()
            .flatten()
            .is_some_and(|at| now() - at < STALE_AFTER_SECS);
        if fresh || self.prices_fetching {
            return;
        }
        self.prices_fetching = true;
        self.spawn(fetch, |app, got| {
            app.prices_fetching = false;
            match got {
                Ok(catalog) if !catalog.prices.is_empty() => {
                    app.save("the model prices", |db| db.replace_catalog(&catalog, now()));
                    app.load_prices();
                }
                Ok(_) => eprintln!("oido: the price list had no prices; keeping the last ones"),
                Err(e) => eprintln!("oido: {e}"),
            }
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_is_read_under_our_provider_ids_and_models_without_a_price_are_left_out() {
        let json = r#"{
            "google": {"models": {
                "gemini-2.5-pro": {"cost": {"input": 1.25, "output": 10, "cache_read": 0.31}},
                "gemini-free": {"cost": {}},
                "gemini-unpriced": {}
            }},
            "anthropic": {"models": {
                "claude-x": {"cost": {"input": 3, "output": 15, "cache_read": 0.3, "cache_write": 3.75}}
            }},
            "someone-else": {"models": {"m": {"cost": {"input": 1, "output": 1}}}}
        }"#;
        let mut got = parse(json).unwrap().prices;
        got.sort_by(|a, b| a.0.id.cmp(&b.0.id));
        assert_eq!(
            got,
            vec![
                (
                    ModelRef::new("anthropic", "claude-x"),
                    Price {
                        input: 3.0,
                        output: 15.0,
                        cache_read: Some(0.3),
                        cache_write: Some(3.75)
                    }
                ),
                (
                    ModelRef::new("gemini", "gemini-2.5-pro"),
                    Price {
                        input: 1.25,
                        output: 10.0,
                        cache_read: Some(0.31),
                        cache_write: None
                    }
                ),
            ]
        );
        assert!(parse("[]").is_err());
        assert!(parse("not json").is_err());
    }

    #[test]
    fn context_limits_are_read_whether_or_not_the_model_has_a_price() {
        let json = r#"{
            "openai": {"models": {
                "gpt-x": {"cost": {"input": 1, "output": 2}, "limit": {"context": 400000, "output": 128000}},
                "gpt-free": {"limit": {"context": 128000}},
                "gpt-zero": {"limit": {"context": 0}},
                "gpt-none": {"cost": {"input": 1, "output": 2}}
            }}
        }"#;
        let mut got = parse(json).unwrap().limits;
        got.sort_by(|a, b| a.0.id.cmp(&b.0.id));
        assert_eq!(
            got,
            vec![
                (ModelRef::new("openai", "gpt-free"), 128_000),
                (ModelRef::new("openai", "gpt-x"), 400_000),
            ]
        );
    }

    #[test]
    fn a_dated_or_variant_id_falls_back_to_its_base() {
        assert_eq!(base_id("claude-opus-4-5-20251101"), Some("claude-opus-4-5"));
        assert_eq!(base_id("gpt-4o-2024-08-06"), Some("gpt-4o"));
        assert_eq!(
            base_id("anthropic/claude-opus-5:batch"),
            Some("anthropic/claude-opus-5")
        );
        assert_eq!(base_id("claude-opus-4-5"), None, "already a base");
        assert_eq!(base_id("gpt-4-turbo"), None);
        assert_eq!(
            base_id("qwen3-235b-a22b-2507"),
            None,
            "four digits is not a date"
        );
    }
}
