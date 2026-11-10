//! Usage: what model calls used, in tokens, and what that is likely to have cost. Each model call is
//! added to a day's totals when its step is recorded (see `Store::save_run`), and those totals are
//! kept after the runs themselves are gone. Cost is never stored: it is worked out from the tokens
//! and today's prices whenever it is shown. See `docs/usage.md`.

use std::collections::HashMap;

use chrono::NaiveDate;
use rune::TextEdit;

use crate::app::App;
use crate::models::ModelRef;
use crate::runs::{Run, Status, Step};

/// What made a model call.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    /// You, in a chat.
    Chat,
    /// A schedule, in the chat it opened.
    Schedule,
    /// A workflow's model step.
    Workflow,
    /// A team member, on a task its lead handed over.
    Member,
    /// Summarizing a chat's older messages to make room (see `docs/context.md`).
    Compaction,
}

impl Source {
    pub const ALL: [Source; 5] = [
        Source::Chat,
        Source::Schedule,
        Source::Workflow,
        Source::Member,
        Source::Compaction,
    ];

    /// The stored name back to a source; an unknown one reads as a chat.
    pub fn parse(s: &str) -> Source {
        Source::ALL
            .into_iter()
            .find(|v| v.as_str() == s)
            .unwrap_or(Source::Chat)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Source::Chat => "chat",
            Source::Schedule => "schedule",
            Source::Workflow => "workflow",
            Source::Member => "member",
            Source::Compaction => "compaction",
        }
    }
}

/// Model calls added together. Cache reads and writes are part of `input`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Totals {
    pub calls: u64,
    pub errors: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Time to first token over the calls that reported one: the sum, and how many.
    pub first_ms_total: u64,
    pub first_ms_count: u64,
}

/// One day's totals for one model, agent and source.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Row {
    /// The local day, `YYYY-MM-DD`.
    pub day: String,
    pub model: ModelRef,
    pub agent_id: String,
    /// As it was named when it last made a call.
    pub agent_name: String,
    pub source: Source,
    pub totals: Totals,
}

/// US dollars per million tokens.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    /// When unknown, cached input is charged as input.
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
}

impl Price {
    /// What `t` cost at this price, in US dollars.
    pub fn cost(&self, t: &Totals) -> f64 {
        let fresh = t.input.saturating_sub(t.cache_read + t.cache_write);
        let per = |n: u64, price: f64| n as f64 * price / 1_000_000.0;
        per(fresh, self.input)
            + per(t.cache_read, self.cache_read.unwrap_or(self.input))
            + per(t.cache_write, self.cache_write.unwrap_or(self.input))
            + per(t.output, self.output)
    }
}

/// Every known price: the catalog's, and the ones you set, which win.
#[derive(Clone, Debug, Default)]
pub struct Prices {
    pub catalog: HashMap<ModelRef, Price>,
    pub manual: HashMap<ModelRef, Price>,
}

impl Prices {
    /// Your price for the model, else the catalog's, which may be listed without the id's date or
    /// variant (see [`crate::pricing::base_id`]).
    pub fn get(&self, model: &ModelRef) -> Option<&Price> {
        self.manual
            .get(model)
            .or_else(|| self.catalog.get(model))
            .or_else(|| {
                let base = crate::pricing::base_id(&model.id)?;
                self.catalog.get(&ModelRef::new(&model.provider, base))
            })
    }

    /// What `t` on `model` cost, or `None` when the model has no price.
    pub fn cost(&self, model: &ModelRef, t: &Totals) -> Option<f64> {
        self.get(model).map(|p| p.cost(t))
    }
}

impl Totals {
    /// One model call, as its step recorded it.
    pub fn of_call(s: &Step) -> Totals {
        let n = |v: Option<u32>| u64::from(v.unwrap_or(0));
        Totals {
            calls: 1,
            errors: u64::from(s.status == Status::Error),
            input: n(s.input_tokens),
            output: n(s.output_tokens),
            cache_read: n(s.cache_read_tokens),
            cache_write: n(s.cache_write_tokens),
            first_ms_total: n(s.first_ms),
            first_ms_count: u64::from(s.first_ms.is_some()),
        }
    }

    /// A run's tokens as listed: its own and, for a lead, its team members'.
    fn of_run(r: &Run) -> Totals {
        let n = |v: Option<u32>| u64::from(v.unwrap_or(0));
        Totals {
            calls: u64::from(r.calls),
            input: n(r.input_tokens),
            output: n(r.output_tokens),
            cache_read: n(r.cache_read_tokens),
            cache_write: n(r.cache_write_tokens),
            ..Totals::default()
        }
    }

    /// What is left of `self` once `less` is taken away, never below nothing.
    fn minus(&self, less: &Totals) -> Totals {
        Totals {
            calls: self.calls.saturating_sub(less.calls),
            errors: self.errors.saturating_sub(less.errors),
            input: self.input.saturating_sub(less.input),
            output: self.output.saturating_sub(less.output),
            cache_read: self.cache_read.saturating_sub(less.cache_read),
            cache_write: self.cache_write.saturating_sub(less.cache_write),
            first_ms_total: self.first_ms_total.saturating_sub(less.first_ms_total),
            first_ms_count: self.first_ms_count.saturating_sub(less.first_ms_count),
        }
    }

    pub fn add(&mut self, more: &Totals) {
        self.calls += more.calls;
        self.errors += more.errors;
        self.input += more.input;
        self.output += more.output;
        self.cache_read += more.cache_read;
        self.cache_write += more.cache_write;
        self.first_ms_total += more.first_ms_total;
        self.first_ms_count += more.first_ms_count;
    }

    /// Input and output together.
    pub fn tokens(&self) -> u64 {
        self.input + self.output
    }

    /// The share of input read from the cache, when there was input.
    pub fn cache_hit_rate(&self) -> Option<f64> {
        (self.input > 0).then(|| self.cache_read as f64 / self.input as f64)
    }

    /// The share of calls that failed, when there were calls.
    pub fn error_rate(&self) -> Option<f64> {
        (self.calls > 0).then(|| self.errors as f64 / self.calls as f64)
    }

    /// The average time to first token, in milliseconds, over the calls that reported one.
    pub fn avg_first_ms(&self) -> Option<u64> {
        (self.first_ms_count > 0).then(|| self.first_ms_total / self.first_ms_count)
    }
}

/// An estimated cost: the dollars for what had a price, and whether anything had none.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Cost {
    pub dollars: f64,
    /// Some of the tokens were on a model with no price, so `dollars` falls short.
    pub partial: bool,
}

impl From<f64> for Cost {
    /// A cost that had a price.
    fn from(dollars: f64) -> Cost {
        Cost {
            dollars,
            partial: false,
        }
    }
}

impl Cost {
    fn add(&mut self, cost: Option<f64>, tokens: u64) {
        match cost {
            Some(d) => self.dollars += d,
            None => self.partial |= tokens > 0,
        }
    }

    /// Whether nothing at all had a price.
    pub fn unknown(&self) -> bool {
        self.partial && self.dollars == 0.0
    }
}

/// What `run` likely cost. A lead's listed tokens include its team members' runs (found in `all`),
/// which may be on other models, so each part is priced on its own model.
pub fn run_cost(run: &Run, all: &[Run], prices: &Prices) -> Cost {
    let mut cost = Cost::default();
    let mut own = Totals::of_run(run);
    for member in all
        .iter()
        .filter(|r| r.parent.as_deref() == Some(run.id.as_str()))
    {
        let t = Totals::of_run(member);
        own = own.minus(&t);
        cost.add(prices.cost(&member.model, &t), t.tokens());
    }
    cost.add(prices.cost(&run.model, &own), own.tokens());
    cost
}

/// What a chat's model calls used and likely cost, from its totals by model.
pub fn chat_cost(by_model: &[(ModelRef, Totals)], prices: &Prices) -> (Totals, Cost) {
    let (mut totals, mut cost) = (Totals::default(), Cost::default());
    for (model, t) in by_model {
        totals.add(t);
        cost.add(prices.cost(model, t), t.tokens());
    }
    (totals, cost)
}

/// How far back the Usage screen looks.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Range {
    Week,
    #[default]
    Month,
    Quarter,
    All,
}

impl Range {
    pub const ALL: [Range; 4] = [Range::Week, Range::Month, Range::Quarter, Range::All];

    pub fn label(self) -> &'static str {
        match self {
            Range::Week => "7 days",
            Range::Month => "30 days",
            Range::Quarter => "90 days",
            Range::All => "All",
        }
    }

    /// How many days it covers, today included; `None` for all of them.
    pub fn days(self) -> Option<u32> {
        match self {
            Range::Week => Some(7),
            Range::Month => Some(30),
            Range::Quarter => Some(90),
            Range::All => None,
        }
    }

    /// The first day it covers, as stored (`YYYY-MM-DD`); `None` for all of them.
    pub fn since(self, today: NaiveDate) -> Option<String> {
        let back = self.days()?.saturating_sub(1);
        Some(day_name(today - chrono::Days::new(u64::from(back))))
    }
}

/// What the chart measures.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Metric {
    #[default]
    Cost,
    Tokens,
}

/// What the Usage screen is narrowed to. Each kind is either unset or one value.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Filters {
    pub model: Option<ModelRef>,
    pub agent: Option<String>,
    pub source: Option<Source>,
}

impl Filters {
    pub fn keeps(&self, r: &Row) -> bool {
        self.model.as_ref().is_none_or(|m| *m == r.model)
            && self.agent.as_ref().is_none_or(|a| *a == r.agent_id)
            && self.source.is_none_or(|s| s == r.source)
    }

    pub fn is_empty(&self) -> bool {
        *self == Filters::default()
    }
}

/// One line of a breakdown: a model, an agent or a source, with what it used and cost.
#[derive(Clone, PartialEq, Debug)]
pub struct Group<K> {
    pub key: K,
    pub totals: Totals,
    pub cost: Cost,
}

/// One day of the chart.
#[derive(Clone, PartialEq, Debug)]
pub struct Day {
    /// `YYYY-MM-DD`.
    pub day: String,
    pub totals: Totals,
    pub cost: Cost,
}

/// Everything the Usage screen shows, worked out from the daily totals and today's prices.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Report {
    pub totals: Totals,
    pub cost: Cost,
    /// Every day in the range, oldest first, empty days included.
    pub days: Vec<Day>,
    /// Biggest first: by cost, then by tokens.
    pub models: Vec<Group<ModelRef>>,
    pub agents: Vec<Group<String>>,
    pub sources: Vec<Group<Source>>,
    /// The latest name of each agent in `agents`.
    pub agent_names: HashMap<String, String>,
}

/// Adds up `rows` that `filters` keeps over the days of `range` ending `today`. Rows outside the
/// range are ignored, so `rows` may hold more.
pub fn report(
    rows: &[Row],
    prices: &Prices,
    filters: &Filters,
    range: Range,
    today: NaiveDate,
) -> Report {
    let since = range.since(today);
    let kept: Vec<&Row> = rows
        .iter()
        .filter(|r| since.as_ref().is_none_or(|s| r.day >= *s) && filters.keeps(r))
        .collect();
    let first = since
        .or_else(|| kept.iter().map(|r| r.day.clone()).min())
        .unwrap_or_else(|| day_name(today));
    let mut days: Vec<Day> = day_axis(&first, today)
        .into_iter()
        .map(|day| Day {
            day,
            totals: Totals::default(),
            cost: Cost::default(),
        })
        .collect();
    let mut out = Report::default();
    let mut models: HashMap<ModelRef, Group<ModelRef>> = HashMap::new();
    let mut agents: HashMap<String, Group<String>> = HashMap::new();
    let mut sources: HashMap<Source, Group<Source>> = HashMap::new();
    fn add<K: Clone + Eq + std::hash::Hash>(
        map: &mut HashMap<K, Group<K>>,
        key: &K,
        t: &Totals,
        cost: Option<f64>,
    ) {
        let g = map.entry(key.clone()).or_insert_with(|| Group {
            key: key.clone(),
            totals: Totals::default(),
            cost: Cost::default(),
        });
        g.totals.add(t);
        g.cost.add(cost, t.tokens());
    }
    for r in kept {
        let t = &r.totals;
        let cost = prices.cost(&r.model, t);
        out.totals.add(t);
        out.cost.add(cost, t.tokens());
        // Days are sorted, so a row's day is found by search; one before the axis is impossible.
        if let Ok(i) = days.binary_search_by(|d| d.day.as_str().cmp(&r.day)) {
            days[i].totals.add(t);
            days[i].cost.add(cost, t.tokens());
        }
        add(&mut models, &r.model, t, cost);
        add(&mut agents, &r.agent_id, t, cost);
        add(&mut sources, &r.source, t, cost);
        // Rows come oldest first, so the last name seen is the latest.
        out.agent_names
            .insert(r.agent_id.clone(), r.agent_name.clone());
    }
    out.days = std::mem::take(&mut days);
    out.models = biggest_first(models);
    out.agents = biggest_first(agents);
    out.sources = biggest_first(sources);
    out
}

fn biggest_first<K>(map: HashMap<K, Group<K>>) -> Vec<Group<K>> {
    let mut v: Vec<Group<K>> = map.into_values().collect();
    v.sort_by(|a, b| {
        b.cost
            .dollars
            .total_cmp(&a.cost.dollars)
            .then(b.totals.tokens().cmp(&a.totals.tokens()))
    });
    v
}

/// Every day from `first` to `today`, both included, as stored. An unreadable `first` gives today.
fn day_axis(first: &str, today: NaiveDate) -> Vec<String> {
    let start = NaiveDate::parse_from_str(first, DAY)
        .unwrap_or(today)
        .min(today);
    start
        .iter_days()
        .take_while(|d| *d <= today)
        .map(day_name)
        .collect()
}

/// How days are stored: SQLite's `date()`.
const DAY: &str = "%Y-%m-%d";

pub fn day_name(d: NaiveDate) -> String {
    d.format(DAY).to_string()
}

/// Today, in local time, as usage is counted.
pub fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

/// The Usage screen: what it looks at, what it is narrowed to, every daily total, and the price
/// form when one is open.
#[derive(Debug, Default)]
pub struct Page {
    pub range: Range,
    pub metric: Metric,
    pub filters: Filters,
    /// Every daily total, oldest first; read when the screen opens.
    pub rows: Vec<Row>,
    pub price: Option<PriceForm>,
    /// Settings asks before clearing the history.
    pub confirm_clear: bool,
}

impl Page {
    /// The price form, for its fields to edit. Only drawn while there is one; an empty one stands
    /// in otherwise.
    pub fn price_mut(&mut self) -> &mut PriceForm {
        self.price
            .get_or_insert_with(|| PriceForm::new(ModelRef::new("", ""), None))
    }
}

/// Your price for one model, being typed: US dollars per million tokens.
#[derive(Debug)]
pub struct PriceForm {
    pub model: ModelRef,
    pub input: TextEdit,
    pub output: TextEdit,
    pub cache_read: TextEdit,
    pub cache_write: TextEdit,
    pub error: Option<String>,
}

impl PriceForm {
    /// A form for `model`, filled in with `price` when it has one.
    pub fn new(model: ModelRef, price: Option<&Price>) -> PriceForm {
        let field = |v: Option<f64>| TextEdit::new(v.map(|v| v.to_string()).unwrap_or_default());
        PriceForm {
            model,
            input: field(price.map(|p| p.input)),
            output: field(price.map(|p| p.output)),
            cache_read: field(price.and_then(|p| p.cache_read)),
            cache_write: field(price.and_then(|p| p.cache_write)),
            error: None,
        }
    }

    /// What was typed, as a price.
    ///
    /// # Errors
    ///
    /// A missing input or output price, or anything that is not a number of dollars, in words.
    pub fn price(&self) -> Result<Price, String> {
        let read = |name: &str, t: &TextEdit| -> Result<Option<f64>, String> {
            let t = t.text.trim().trim_start_matches('$');
            if t.is_empty() {
                return Ok(None);
            }
            t.parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map(Some)
                .ok_or_else(|| format!("The {name} price is not an amount of dollars."))
        };
        Ok(Price {
            input: read("input", &self.input)?.ok_or("Give an input price.")?,
            output: read("output", &self.output)?.ok_or("Give an output price.")?,
            cache_read: read("cache read", &self.cache_read)?,
            cache_write: read("cache write", &self.cache_write)?,
        })
    }
}

impl App {
    /// Saves a run and the steps it took since it was last saved; the chat's total follows when a
    /// model call is among them.
    pub(crate) fn save_run(&mut self, run: &Run, steps: &[Step]) {
        self.save("the run", |db| db.save_run(run, steps));
        let counted = steps.iter().any(|s| s.kind == crate::runs::StepKind::Llm);
        if let (true, Some(db)) = (counted, &self.store) {
            match db.session_usage(&run.session_id) {
                Ok(t) => {
                    self.chat_usage.insert(run.session_id.clone(), t);
                }
                Err(e) => eprintln!("oido: could not read the chat's usage: {e}"),
            }
        }
    }

    /// Reads every chat's totals.
    pub(crate) fn load_chat_usage(&mut self) {
        let Some(db) = &self.store else {
            return;
        };
        match db.all_session_usage() {
            Ok(all) => self.chat_usage = all,
            Err(e) => eprintln!("oido: could not read chat usage: {e}"),
        }
    }

    /// What the chat used and likely cost so far, if it used anything.
    pub fn chat_spend(&self, session_id: &str) -> Option<(Totals, Cost)> {
        let by_model = self.chat_usage.get(session_id)?;
        let (t, c) = chat_cost(by_model, &self.prices);
        (t.tokens() > 0).then_some((t, c))
    }

    /// Reads the agent's last 30 days, for its form.
    pub(crate) fn load_agent_spend(&mut self, agent_id: &str) {
        self.agent_spend.clear();
        let Some(db) = &self.store else {
            return;
        };
        match db.usage(Range::Month.since(today()).as_deref()) {
            Ok(rows) => {
                self.agent_spend = rows
                    .into_iter()
                    .filter(|r| r.agent_id == agent_id)
                    .collect();
            }
            Err(e) => eprintln!("oido: could not read the agent's usage: {e}"),
        }
    }

    /// Goes to Usage, narrowed to one agent.
    pub fn usage_of_agent(&mut self, agent_id: &str) {
        self.usage.filters = Filters {
            agent: Some(agent_id.to_owned()),
            ..Filters::default()
        };
        self.go(crate::app::Route::Usage);
    }

    /// Empties the daily totals (Settings). Chat totals and runs are kept.
    pub fn clear_usage_history(&mut self) {
        self.save("the usage history", |db| db.clear_usage());
        self.usage.rows.clear();
        self.agent_spend.clear();
        self.usage.confirm_clear = false;
    }

    /// Reads the daily totals again, for the Usage screen.
    pub fn open_usage(&mut self) {
        let Some(db) = &self.store else {
            return;
        };
        match db.usage(None) {
            Ok(rows) => self.usage.rows = rows,
            Err(e) => eprintln!("oido: could not read usage: {e}"),
        }
    }

    pub fn set_usage_range(&mut self, range: Range) {
        self.usage.range = range;
    }

    pub fn set_usage_metric(&mut self, metric: Metric) {
        self.usage.metric = metric;
    }

    /// Narrows the screen to a model, or widens it again when it already is.
    pub fn filter_usage_model(&mut self, model: &ModelRef) {
        let f = &mut self.usage.filters.model;
        *f = (f.as_ref() != Some(model)).then(|| model.clone());
    }

    pub fn filter_usage_agent(&mut self, agent_id: &str) {
        let f = &mut self.usage.filters.agent;
        *f = (f.as_deref() != Some(agent_id)).then(|| agent_id.to_owned());
    }

    pub fn filter_usage_source(&mut self, source: Source) {
        let f = &mut self.usage.filters.source;
        *f = (*f != Some(source)).then_some(source);
    }

    pub fn clear_usage_filters(&mut self) {
        self.usage.filters = Filters::default();
    }

    /// Opens the price form for `model`, filled in with the price it has now; again closes it.
    pub fn toggle_price_form(&mut self, model: &ModelRef) {
        if self.usage.price.as_ref().is_some_and(|f| f.model == *model) {
            self.usage.price = None;
            return;
        }
        let form = PriceForm::new(model.clone(), self.prices.get(model));
        self.usage.price = Some(form);
    }

    /// Saves the price being typed as yours for its model.
    pub fn save_price(&mut self) {
        let Some(form) = &mut self.usage.price else {
            return;
        };
        let price = match form.price() {
            Ok(p) => p,
            Err(why) => {
                form.error = Some(why);
                return;
            }
        };
        let model = form.model.clone();
        let saved = match &self.store {
            Some(db) => db
                .set_manual_price(&model, Some(&price), crate::app::now())
                .map_err(|e| e.to_string()),
            None => Err("There is no saved data to keep it in.".to_owned()),
        };
        match saved {
            Ok(()) => {
                self.usage.price = None;
                self.load_prices();
            }
            Err(why) => self.usage.price_mut().error = Some(format!("Could not save it: {why}")),
        }
    }

    /// Drops your price for `model`, so the catalog's (if any) counts again.
    pub fn reset_price(&mut self, model: &ModelRef) {
        self.save("the price", |db| {
            db.set_manual_price(model, None, crate::app::now())
        });
        self.usage.price = None;
        self.load_prices();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn price() -> Price {
        Price {
            input: 3.0,
            output: 15.0,
            cache_read: Some(0.3),
            cache_write: None,
        }
    }

    #[test]
    fn cached_input_is_charged_at_its_own_price_and_input_price_when_it_has_none() {
        let t = Totals {
            input: 1_000_000,
            cache_read: 500_000,
            cache_write: 100_000,
            output: 200_000,
            ..Totals::default()
        };
        // 400k fresh at $3, 500k read at $0.30, 100k written at $3 (no price), 200k out at $15.
        let want = 1.2 + 0.15 + 0.3 + 3.0;
        assert!(
            (price().cost(&t) - want).abs() < 1e-9,
            "{}",
            price().cost(&t)
        );
    }

    #[test]
    fn a_provider_that_reports_more_cache_than_input_costs_nothing_for_fresh_input() {
        let t = Totals {
            input: 10,
            cache_read: 20,
            ..Totals::default()
        };
        assert!((price().cost(&t) - 20.0 * 0.3 / 1e6).abs() < 1e-12);
    }

    #[test]
    fn your_price_wins_over_the_catalog_and_a_model_with_neither_has_no_cost() {
        let m = ModelRef::new("openai", "gpt-x");
        let mut prices = Prices::default();
        let t = Totals {
            output: 1_000_000,
            ..Totals::default()
        };
        assert_eq!(prices.cost(&m, &t), None);
        prices.catalog.insert(m.clone(), price());
        assert_eq!(prices.cost(&m, &t), Some(15.0));
        prices.manual.insert(
            m.clone(),
            Price {
                output: 1.0,
                ..price()
            },
        );
        assert_eq!(prices.cost(&m, &t), Some(1.0));

        let dated = ModelRef::new("openai", "gpt-x-2024-08-06");
        assert_eq!(prices.cost(&dated, &t), Some(15.0), "the catalog's base id");
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, DAY).unwrap()
    }

    fn row(day: &str, model: &str, agent: &str, source: Source, input: u64) -> Row {
        Row {
            day: day.into(),
            model: ModelRef::new("openai", model),
            agent_id: agent.into(),
            agent_name: format!("{agent} on {day}"),
            source,
            totals: Totals {
                calls: 1,
                input,
                output: input / 10,
                ..Totals::default()
            },
        }
    }

    fn priced() -> Prices {
        let mut p = Prices::default();
        p.catalog.insert(
            ModelRef::new("openai", "big"),
            Price {
                input: 10.0,
                output: 10.0,
                cache_read: None,
                cache_write: None,
            },
        );
        p
    }

    #[test]
    fn a_report_fills_every_day_of_its_range_and_leaves_out_what_is_before_it() {
        let rows = [
            row("2100-01-01", "big", "a", Source::Chat, 1_000_000),
            row("2100-01-28", "big", "a", Source::Chat, 100_000),
            row("2100-01-30", "free", "b", Source::Workflow, 50_000),
        ];
        let r = report(
            &rows,
            &priced(),
            &Filters::default(),
            Range::Week,
            date("2100-01-30"),
        );
        assert_eq!(r.days.len(), 7);
        assert_eq!(
            (r.days[0].day.as_str(), r.days[6].day.as_str()),
            ("2100-01-24", "2100-01-30")
        );
        assert_eq!(r.days[4].totals.input, 100_000, "the 28th");
        assert_eq!(
            r.days[5].totals,
            Totals::default(),
            "an empty day is still there"
        );
        assert_eq!(r.totals.input, 150_000, "the 1st is out of range");
        assert!((r.cost.dollars - 1.1).abs() < 1e-9, "{:?}", r.cost);
        assert!(r.cost.partial, "the free model has no price");
        assert_eq!(
            r.days[6].cost,
            Cost {
                dollars: 0.0,
                partial: true
            }
        );
        assert!(r.days[6].cost.unknown());
        assert_eq!(
            r.models
                .iter()
                .map(|g| g.key.id.as_str())
                .collect::<Vec<_>>(),
            ["big", "free"],
            "costliest first"
        );

        let all = report(
            &rows,
            &priced(),
            &Filters::default(),
            Range::All,
            date("2100-01-30"),
        );
        assert_eq!(all.days.len(), 30, "from the first day there is");
        assert_eq!(all.totals.calls, 3);
    }

    #[test]
    fn a_filter_narrows_every_part_and_the_latest_agent_name_is_kept() {
        let rows = [
            row("2100-01-29", "big", "a", Source::Chat, 10),
            row("2100-01-30", "big", "a", Source::Schedule, 20),
            row("2100-01-30", "free", "b", Source::Chat, 40),
        ];
        let f = Filters {
            agent: Some("a".into()),
            ..Filters::default()
        };
        let r = report(&rows, &priced(), &f, Range::Month, date("2100-01-30"));
        assert_eq!(r.totals.input, 30);
        assert_eq!(r.agents.len(), 1);
        assert_eq!(r.sources.len(), 2);
        assert_eq!(r.agent_names["a"], "a on 2100-01-30");
        assert!(!r.cost.partial);

        let f = Filters {
            source: Some(Source::Chat),
            model: Some(ModelRef::new("openai", "free")),
            ..Filters::default()
        };
        let r = report(&rows, &priced(), &f, Range::Month, date("2100-01-30"));
        assert_eq!(r.totals.input, 40);
        assert!(r.cost.unknown());
    }

    #[test]
    fn an_empty_report_still_has_its_days() {
        let r = report(
            &[],
            &Prices::default(),
            &Filters::default(),
            Range::All,
            date("2100-01-30"),
        );
        assert_eq!(r.days.len(), 1, "today");
        assert_eq!(r.totals, Totals::default());
        assert_eq!(r.cost, Cost::default());
        assert_eq!(
            Range::Week.since(date("2100-03-01")).as_deref(),
            Some("2100-02-23")
        );
    }

    #[test]
    fn pressing_a_line_narrows_to_it_and_pressing_it_again_widens() {
        let mut app = App::new(None);
        let m = ModelRef::new("openai", "big");
        app.filter_usage_model(&m);
        app.filter_usage_agent("a");
        app.filter_usage_source(Source::Chat);
        assert_eq!(app.usage.filters.model.as_ref(), Some(&m));
        app.filter_usage_model(&m);
        app.filter_usage_source(Source::Chat);
        assert_eq!(
            app.usage.filters,
            Filters {
                agent: Some("a".into()),
                ..Filters::default()
            }
        );
        app.clear_usage_filters();
        assert!(app.usage.filters.is_empty());
    }

    #[test]
    fn your_price_is_checked_saved_and_dropped_again() {
        let db = crate::testing::TempDb::new();
        let mut app = db.open();
        let m = ModelRef::new("llama-server:box", "qwen");
        app.toggle_price_form(&m);
        let f = app.usage.price_mut();
        f.output = TextEdit::new("2");
        app.save_price();
        assert_eq!(
            app.usage.price.as_ref().and_then(|f| f.error.as_deref()),
            Some("Give an input price.")
        );
        let f = app.usage.price_mut();
        (f.input, f.cache_read) = (TextEdit::new("$0.5"), TextEdit::new("-1"));
        app.save_price();
        assert!(
            app.usage
                .price
                .as_ref()
                .unwrap()
                .error
                .as_deref()
                .unwrap()
                .contains("cache read")
        );
        app.usage.price_mut().cache_read = TextEdit::new("");
        app.save_price();
        assert!(app.usage.price.is_none(), "closed once saved");
        let want = Price {
            input: 0.5,
            output: 2.0,
            cache_read: None,
            cache_write: None,
        };
        assert_eq!(app.prices.get(&m), Some(&want));
        drop(app);

        let mut app = db.open();
        assert_eq!(app.prices.get(&m), Some(&want), "kept");
        app.toggle_price_form(&m);
        assert_eq!(
            app.usage.price.as_ref().unwrap().input.text,
            "0.5",
            "filled in"
        );
        app.toggle_price_form(&m);
        assert!(app.usage.price.is_none(), "pressed again, it closes");
        app.reset_price(&m);
        assert_eq!(app.prices.get(&m), None);
    }

    #[test]
    fn a_leads_cost_prices_its_team_members_on_their_own_models() {
        let run = |id: &str, model: &str, parent: Option<&str>, input: u32| {
            let mut r = Run::new(id, "a", "A", "s", ModelRef::new("openai", model));
            (r.input_tokens, r.output_tokens, r.parent) =
                (Some(input), Some(0), parent.map(Into::into));
            r
        };
        // The lead's listed 1.5M include its member's 0.5M on a model with no price.
        let lead = run("lead", "big", None, 1_500_000);
        let member = run("m", "free", Some("lead"), 500_000);
        let all = [lead.clone(), member.clone()];
        let c = run_cost(&lead, &all, &priced());
        assert!((c.dollars - 10.0).abs() < 1e-9, "{c:?}");
        assert!(c.partial, "the member's part has no price");
        assert!(run_cost(&member, &all, &priced()).unknown());

        let (t, c) = chat_cost(
            &[
                (
                    ModelRef::new("openai", "big"),
                    Totals {
                        input: 100_000,
                        ..Totals::default()
                    },
                ),
                (
                    ModelRef::new("openai", "free"),
                    Totals {
                        output: 5,
                        ..Totals::default()
                    },
                ),
            ],
            &priced(),
        );
        assert_eq!(t.tokens(), 100_005);
        assert_eq!(
            c,
            Cost {
                dollars: 1.0,
                partial: true
            }
        );
    }

    #[test]
    fn clearing_the_history_empties_usage_and_keeps_chat_totals() {
        let db = crate::testing::TempDb::new();
        let mut app = db.open();
        let m = ModelRef::new("openai", "big");
        let who = crate::store::Counted {
            model: &m,
            agent_id: "a",
            agent_name: "A",
            source: Source::Chat,
        };
        let t = Totals {
            calls: 1,
            input: 5,
            ..Totals::default()
        };
        app.store
            .as_ref()
            .unwrap()
            .add_usage(crate::app::now(), &who, &t)
            .unwrap();
        app.edit_agent(&crate::agents::Target::agent("a"));
        assert_eq!(
            app.agent_spend.len(),
            1,
            "the form reads the agent's last 30 days"
        );
        app.go(crate::app::Route::Usage);
        assert_eq!(app.usage.rows.len(), 1);

        app.usage.confirm_clear = true;
        app.clear_usage_history();
        assert!(app.usage.rows.is_empty() && app.agent_spend.is_empty());
        assert!(!app.usage.confirm_clear);
        app.go(crate::app::Route::Usage);
        assert!(app.usage.rows.is_empty(), "gone from the database too");

        app.usage_of_agent("a");
        assert_eq!(app.route, crate::app::Route::Usage);
        assert_eq!(app.usage.filters.agent.as_deref(), Some("a"));
    }

    #[test]
    fn rates_are_unknown_without_calls_or_input() {
        let t = Totals {
            calls: 4,
            errors: 1,
            input: 200,
            cache_read: 50,
            first_ms_total: 900,
            first_ms_count: 3,
            ..Totals::default()
        };
        assert_eq!(t.cache_hit_rate(), Some(0.25));
        assert_eq!(t.error_rate(), Some(0.25));
        assert_eq!(t.avg_first_ms(), Some(300));
        let none = Totals::default();
        assert_eq!(
            (
                none.cache_hit_rate(),
                none.error_rate(),
                none.avg_first_ms()
            ),
            (None, None, None)
        );
    }
}
