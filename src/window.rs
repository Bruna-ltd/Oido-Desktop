//! The context window: how many tokens a model takes at once, and how much of that a chat fills.
//! See `docs/context.md`.

use std::collections::HashMap;

use crate::agents::Agent;
use crate::app::App;
use crate::completion::{Request, Role, Turn};
use crate::model::{Message, Outcome};
use crate::models::ModelRef;
use crate::providers::ModelStatus;

/// The share of the limit from which a chat counts as nearly full.
pub const NEAR: f64 = 0.8;
/// Past this share of the limit, what a call sends is trimmed...
const TRIM_ABOVE: f64 = 0.9;
/// ...down to this share, so the next calls send the same and the provider's cache keeps working.
const TRIM_TO: f64 = 0.75;
/// The latest turns always go whole: they are what the model is working on.
const KEEP_TURNS: usize = 4;
/// Tool results shorter than this are left alone; trimming them saves next to nothing.
const SMALL_RESULT: usize = 1024;
/// About this many characters make a token, for text no provider has counted yet.
const CHARS_PER_TOKEN: usize = 4;
/// What a picture is taken to cost: about a megapixel on the main providers.
const PICTURE_TOKENS: u32 = 1_600;
/// Each message is cut to this many bytes in what the summary is written from: enough to keep
/// what it says, not a whole file it read.
const SUMMARY_TURN_BYTES: usize = 4_000;
/// What the summary is written from, at most, as a share of the limit; the rest is room for it.
const SUMMARY_INPUT: f64 = 0.6;
/// What the summary is written from when the limit is not known, at most, in bytes.
const SUMMARY_INPUT_BYTES: usize = 200_000;
/// The name of a summary's model call in the run, which Usage counts as `compaction`.
pub const SUMMARY_STEP: &str = "summary";

const SUMMARY_PROMPT: &str = "You condense a conversation between a person and an AI agent, so the agent can carry on from your summary without the conversation itself.\n\nKeep:\n- what the person asked for, and every instruction or preference they gave\n- decisions made, and why\n- facts, names, numbers, file paths, commands and results that matter\n- what was done with tools, and what came of it\n- what is still open or in progress\n\nIf the conversation starts with an earlier summary, fold it in. Leave out greetings and repetition. Write plain prose and short lists, under 800 words, in the language of the conversation. Write only the summary.";

/// Context limits from the models.dev catalog, as last fetched.
#[derive(Clone, Default, Debug)]
pub struct Limits {
    pub catalog: HashMap<ModelRef, u32>,
}

impl Limits {
    /// The model's limit: what its provider declared, else the catalog's, which may list it without
    /// the id's date or variant (see [`crate::pricing::base_id`]). `None` when nothing says.
    pub fn get(&self, model: &ModelRef, declared: Option<u32>) -> Option<u32> {
        declared
            .or_else(|| self.catalog.get(model).copied())
            .or_else(|| {
                let base = crate::pricing::base_id(&model.id)?;
                self.catalog
                    .get(&ModelRef::new(&model.provider, base))
                    .copied()
            })
    }
}

/// How full a chat is: the tokens it holds, and the model's limit when known.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fill {
    pub used: u32,
    pub limit: Option<u32>,
}

impl Fill {
    /// The share of the limit in use, when the limit is known.
    pub fn share(self) -> Option<f64> {
        self.limit
            .filter(|l| *l > 0)
            .map(|l| f64::from(self.used) / f64::from(l))
    }

    pub fn near(self) -> bool {
        self.share().is_some_and(|s| s >= NEAR)
    }
}

/// Whether what the model is sent starts again after `m`: "Start fresh", or a summary. A count
/// from before it no longer says how big the chat is.
fn starts_over(m: &Message) -> bool {
    matches!(m, Message::Divider(_) | Message::Summary { .. })
}

/// The tokens a chat holds: the provider's count at its last finished reply since the last divider
/// or summary. `None` before one has finished, and right after "Start fresh" or a summary.
pub fn used(messages: &[Message]) -> Option<u32> {
    messages
        .iter()
        .rev()
        .take_while(|m| !starts_over(m))
        .find_map(|m| match m {
            Message::Assistant {
                stats: Some(s),
                outcome: Outcome::Done,
                ..
            } => s.context,
            _ => None,
        })
}

/// What the provider counted for a chat's last call, and what trimming took out of it.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Measured {
    /// Everything that call sent and wrote, in tokens (see [`crate::model::Stats::context`]).
    pub context: u32,
    /// The estimated tokens trimmed from what it sent.
    pub trimmed: u32,
}

/// The chat's last finished reply since the last divider or summary, if the provider counted it. A
/// later reply that went uncounted leaves nothing measured, as does "Start fresh" or a summary.
pub fn measured(messages: &[Message]) -> Option<Measured> {
    messages
        .iter()
        .rev()
        .take_while(|m| !starts_over(m))
        .find_map(|m| match m {
            Message::Assistant {
                stats,
                outcome: Outcome::Done,
                ..
            } => Some(stats.as_ref().and_then(|s| {
                Some(Measured {
                    context: s.context?,
                    trimmed: s.trimmed.unwrap_or(0),
                })
            })),
            _ => None,
        })
        .flatten()
}

/// Tokens in text no provider has counted yet, roughly.
fn tokens_of(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(CHARS_PER_TOKEN)).unwrap_or(u32::MAX)
}

fn turn_tokens(t: &Turn) -> u32 {
    let calls: u32 = t
        .calls
        .iter()
        .map(|c| tokens_of(&c.name).saturating_add(tokens_of(&c.args)))
        .sum();
    let pictures = u32::try_from(t.images.len()).unwrap_or(u32::MAX);
    tokens_of(&t.text)
        .saturating_add(calls)
        .saturating_add(pictures.saturating_mul(PICTURE_TOKENS))
}

/// How big `req` would be sent whole: the last call's count, what was trimmed from it, and what
/// came after it. With nothing measured, everything is estimated: the brief, skills, context and
/// tools as well as the turns.
fn full_size(req: &Request, measured: Option<Measured>) -> u32 {
    let sum = |turns: &[Turn]| turns.iter().map(turn_tokens).fold(0, u32::saturating_add);
    match measured {
        Some(m) => {
            let after = req
                .turns
                .iter()
                .rposition(|t| t.role == Role::Assistant)
                .map_or(0, |i| i + 1);
            m.context
                .saturating_add(m.trimmed)
                .saturating_add(sum(&req.turns[after..]))
        }
        None => {
            let tools = serde_json::to_string(
                &req.tools
                    .iter()
                    .map(|t| (&t.name, &t.description, &t.schema))
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default();
            [req.system.as_deref(), req.context.as_deref(), Some(&tools)]
                .into_iter()
                .flatten()
                .map(tokens_of)
                .fold(sum(&req.turns), u32::saturating_add)
        }
    }
}

/// How many tokens to trim from a call of `full` tokens, when the last call trimmed `kept`. The
/// same as last time while that still fits under [`TRIM_ABOVE`], so what is sent stays the same;
/// past it, enough to come down to [`TRIM_TO`]. With no known limit, only what was trimmed before.
fn goal(limit: Option<u32>, full: u32, kept: u32) -> u32 {
    let Some(limit) = limit.filter(|l| *l > 0) else {
        return kept;
    };
    let share = |s: f64| (f64::from(limit) * s) as u32;
    if full.saturating_sub(kept) <= share(TRIM_ABOVE) {
        kept
    } else {
        full.saturating_sub(share(TRIM_TO)).max(kept)
    }
}

/// Trims at least `goal` tokens from `turns`, oldest first, leaving the last [`KEEP_TURNS`] whole:
/// a long tool result becomes a note of its size, and pictures a note that they were there. The
/// calls stay, so the model knows what it did. Returns the tokens trimmed, which fall short of
/// `goal` when there is not that much to trim.
fn trim(turns: &mut [Turn], goal: u32) -> u32 {
    let end = turns.len().saturating_sub(KEEP_TURNS);
    let mut trimmed = 0u32;
    for t in &mut turns[..end] {
        if trimmed >= goal {
            break;
        }
        match t.role {
            Role::Tool if t.text.len() > SMALL_RESULT => {
                let note = format!(
                    "(output removed to save room: {} KB)",
                    t.text.len().div_ceil(1024)
                );
                trimmed = trimmed.saturating_add(tokens_of(&t.text) - tokens_of(&note));
                t.text = note;
            }
            Role::User if !t.images.is_empty() => {
                let n = t.images.len();
                let note = match n {
                    1 => "[picture removed to save room]".to_owned(),
                    n => format!("[{n} pictures removed to save room]"),
                };
                let pictures = u32::try_from(n).unwrap_or(u32::MAX);
                trimmed = trimmed.saturating_add(pictures.saturating_mul(PICTURE_TOKENS));
                t.images.clear();
                if !t.text.is_empty() {
                    t.text.push_str("\n\n");
                }
                t.text.push_str(&note);
            }
            _ => {}
        }
    }
    trimmed
}

/// Whether trimming would take anything out of `turns`.
pub fn trimmable(turns: &[Turn]) -> bool {
    trim(&mut turns.to_vec(), u32::MAX) > 0
}

/// What [`fit`] did to a call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fit {
    /// The tokens trimmed, estimated; 0 when it goes whole.
    pub trimmed: u32,
    /// How big what is sent is now, estimated.
    pub size: u32,
}

impl Fit {
    /// Whether it is still past [`TRIM_ABOVE`] of a known limit after trimming: time to summarize.
    pub fn over(self, limit: Option<u32>) -> bool {
        limit.is_some_and(|l| f64::from(self.size) > f64::from(l) * TRIM_ABOVE)
    }

    /// Whether it cannot fit a known limit at all.
    pub fn too_big(self, limit: Option<u32>) -> bool {
        limit.is_some_and(|l| self.size > l)
    }
}

/// Makes `req` fit the model before it is sent, trimming its turns when they come near `limit`
/// (see `docs/context.md`). `measured` is the last call's count; `force` trims all it can, for a
/// call the provider refused as too long.
pub fn fit(req: &mut Request, limit: Option<u32>, measured: Option<Measured>, force: bool) -> Fit {
    let full = full_size(req, measured);
    let kept = measured.map_or(0, |m| m.trimmed);
    let goal = if force {
        u32::MAX
    } else {
        goal(limit, full, kept)
    };
    let trimmed = if goal == 0 {
        0
    } else {
        trim(&mut req.turns, goal)
    };
    Fit {
        trimmed,
        size: full.saturating_sub(trimmed),
    }
}

/// The turns a message gives the model: one for yours, one for a finished reply and one for each
/// tool result that came back to it.
fn turns_in(m: &Message) -> usize {
    match m {
        Message::User(_) => 1,
        Message::Assistant {
            tools,
            outcome: Outcome::Done,
            ..
        } => 1 + tools.iter().filter(|t| !t.id.is_empty()).count(),
        _ => 0,
    }
}

/// Where a summary written now would start keeping messages word for word: the first message it
/// keeps, so the last [`KEEP_TURNS`] turns go whole. `None` when nothing since the last divider or
/// summary would be left to summarize.
pub fn summary_cut(messages: &[Message]) -> Option<usize> {
    let floor = messages.iter().rposition(starts_over).map_or(0, |i| i + 1);
    let mut kept = 0;
    let cut = (floor..messages.len()).rev().find(|&i| {
        kept += turns_in(&messages[i]);
        kept >= KEEP_TURNS
    })?;
    messages[floor..cut]
        .iter()
        .any(|m| turns_in(m) > 0)
        .then_some(cut)
}

/// What the model is sent in place of the messages a summary covers.
pub fn summary_turn(text: &str) -> String {
    format!("A summary of the earlier part of this conversation, written to save room:\n\n{text}")
}

/// The call that writes a summary of `covered`, the chat up to where the summary starts keeping
/// messages (see [`summary_cut`]). Each message is cut short and pictures are named, not sent; if
/// that is still more than `limit` leaves room for, the most recent part is kept.
pub fn summary_request(model: &str, covered: &[Message], limit: Option<u32>) -> Request {
    let mut turns = crate::run::build_turns(covered);
    for t in &mut turns {
        let (short, cut) = crate::runs::clip(&t.text, SUMMARY_TURN_BYTES);
        if cut {
            t.text = format!("{short}…");
        }
    }
    let room = limit.map_or(SUMMARY_INPUT_BYTES, |l| {
        (f64::from(l) * SUMMARY_INPUT) as usize * CHARS_PER_TOKEN
    });
    let (said, _) = crate::runs::clip_start(&crate::run::transcript(&turns), room);
    Request {
        model: model.to_owned(),
        system: Some(SUMMARY_PROMPT.to_owned()),
        context: None,
        turns: vec![Turn::user(format!(
            "<conversation>\n{said}\n</conversation>"
        ))],
        think: None,
        tools: Vec::new(),
    }
}

impl App {
    /// Opens or closes the summary at position `i` of chat `sid`.
    pub fn toggle_summary(&mut self, sid: &str, i: usize) {
        let key = (sid.to_owned(), i);
        if !self.summaries_open.remove(&key) {
            self.summaries_open.insert(key);
        }
    }

    /// Opens or closes a reply's thinking, which was `open`; from then on it stays as you left it.
    pub fn toggle_thinking(&mut self, sid: &str, i: usize, open: bool) {
        self.thinking_open.insert((sid.to_owned(), i), !open);
    }

    /// The model's context limit, when anything states it.
    pub fn context_limit(&self, model: &ModelRef) -> Option<u32> {
        let declared = self.providers.model_info(model).and_then(|i| i.context);
        self.limits.get(model, declared)
    }

    /// How full the agent's open chat is, against the model it answers with now.
    pub fn chat_fill(&self, agent: &Agent) -> Option<Fill> {
        let used = used(&agent.chat().messages)?;
        let limit = match self.providers.status(agent.model.as_ref()) {
            ModelStatus::Ready(m) => self.context_limit(&m),
            _ => None,
        };
        Some(Fill { used, limit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Stats;

    fn reply(context: Option<u32>, outcome: Outcome) -> Message {
        let mut m = Message::reply("ok", None, outcome);
        if let Message::Assistant { stats, .. } = &mut m {
            *stats = Some(Stats {
                model: "m".into(),
                tokens: 10,
                seconds: 1.0,
                context,
                trimmed: None,
            });
        }
        m
    }

    fn request(turns: Vec<Turn>) -> Request {
        Request {
            model: "m".into(),
            system: Some("brief".into()),
            context: None,
            turns,
            think: None,
            tools: Vec::new(),
        }
    }

    /// user, assistant calling a tool, its result of `size` bytes: one exchange.
    fn exchange(n: usize, size: usize) -> Vec<Turn> {
        let id = format!("c{n}");
        let call = crate::completion::Call {
            id: id.clone(),
            name: "shell".into(),
            args: "{}".into(),
        };
        vec![
            Turn::user(format!("q{n}")),
            Turn::assistant("", vec![call]),
            Turn::tool(id, "r".repeat(size)),
        ]
    }

    fn texts(req: &Request) -> Vec<String> {
        req.turns.iter().map(|t| t.text.clone()).collect()
    }

    #[test]
    fn trimming_starts_past_ninety_percent_and_comes_down_to_three_quarters() {
        assert_eq!(goal(Some(1_000), 900, 0), 0);
        assert_eq!(goal(Some(1_000), 901, 0), 151);
        assert_eq!(goal(None, 1_000_000, 0), 0, "no limit, no guessing");
    }

    #[test]
    fn what_was_trimmed_stays_trimmed_until_it_no_longer_fits() {
        assert_eq!(goal(Some(1_000), 950, 200), 200, "750 sent: the same again");
        assert_eq!(goal(Some(1_000), 1_150, 200), 400, "950 sent: down to 750");
        assert_eq!(goal(None, 5_000, 300), 300);
    }

    #[test]
    fn old_long_results_go_first_and_the_last_four_turns_stay_whole() {
        let turns: Vec<Turn> = (1..=4).flat_map(|n| exchange(n, 8_000)).collect();
        let mut req = request(turns);
        // About 8k tokens against 8.5k: past 90%, so down to 75% takes one result.
        let trimmed = fit(&mut req, Some(8_500), None, false).trimmed;
        let t = texts(&req);
        assert_eq!(t[2], "(output removed to save room: 8 KB)");
        assert_eq!(t[5], "r".repeat(8_000), "one was enough");
        assert!(trimmed >= 1_900, "{trimmed}");

        let mut all = request(req.turns.clone());
        fit(&mut all, None, None, true);
        let t = texts(&all);
        assert!(t[5].starts_with("(output removed"), "forced: all it can");
        assert_eq!(t[8].len(), 8_000, "but never the last four turns");
    }

    #[test]
    fn short_results_and_pictures_are_handled_in_order_and_noted() {
        let picture = crate::completion::Image {
            media_type: "image/png".into(),
            data: "AAAA".into(),
        };
        let mut turns = vec![Turn::user_with("look", vec![picture.clone(), picture])];
        turns.extend(exchange(1, 100));
        turns.extend(exchange(2, 10));
        turns.push(Turn::user("now"));
        let mut req = request(turns);
        assert_eq!(fit(&mut req, None, None, true).trimmed, 2 * PICTURE_TOKENS);
        assert!(req.turns[0].images.is_empty());
        assert_eq!(
            req.turns[0].text,
            "look\n\n[2 pictures removed to save room]"
        );
        assert_eq!(req.turns[3].text, "r".repeat(100), "too small to bother");
        assert!(!trimmable(&req.turns), "nothing left");
    }

    #[test]
    fn the_size_builds_on_the_last_count_and_estimates_only_what_came_after() {
        let mut turns = exchange(1, 40);
        turns.push(Turn::user("x".repeat(400)));
        let req = request(turns);
        let last = Measured {
            context: 10_000,
            trimmed: 2_000,
        };
        assert_eq!(full_size(&req, Some(last)), 12_000 + 10 + 100);
        let unmeasured = full_size(&req, None);
        assert!(
            (110..130).contains(&unmeasured),
            "brief and every turn: {unmeasured}"
        );
    }

    #[test]
    fn only_a_counted_last_reply_measures_the_chat() {
        let mut trimmed = reply(Some(5_000), Outcome::Done);
        if let Message::Assistant { stats: Some(s), .. } = &mut trimmed {
            s.trimmed = Some(700);
        }
        let user = || Message::User("hi".into());
        assert_eq!(
            measured(&[user(), trimmed.clone(), user()]),
            Some(Measured {
                context: 5_000,
                trimmed: 700
            })
        );
        assert_eq!(
            measured(&[user(), trimmed, reply(None, Outcome::Done)]),
            None,
            "a later uncounted reply"
        );
    }

    fn talk(n: usize) -> Vec<Message> {
        (1..=n)
            .flat_map(|i| {
                [
                    Message::User(format!("q{i}").as_str().into()),
                    reply(Some(100), Outcome::Done),
                ]
            })
            .collect()
    }

    #[test]
    fn a_summary_keeps_the_last_four_turns_and_needs_something_to_cover() {
        assert_eq!(summary_cut(&talk(2)), None, "four turns: all kept");
        assert_eq!(summary_cut(&talk(3)), Some(2));
        let mut chat = talk(3);
        chat.push(Message::Summary {
            text: "s".into(),
            keeps: 4,
        });
        assert_eq!(summary_cut(&chat), None, "nothing new since");
        chat.extend(talk(2));
        assert_eq!(summary_cut(&chat), None, "the new turns are the ones kept");
        chat.extend(talk(1));
        assert_eq!(summary_cut(&chat), Some(9), "past the last summary");
    }

    #[test]
    fn a_summary_or_divider_leaves_the_chat_unmeasured() {
        let mut chat = talk(1);
        assert!(measured(&chat).is_some());
        chat.push(Message::Summary {
            text: "s".into(),
            keeps: 2,
        });
        assert_eq!(measured(&chat), None);
        assert_eq!(used(&chat), None);
    }

    #[test]
    fn a_summary_is_written_from_the_chat_cut_short_with_an_earlier_summary_in_it() {
        let mut chat = vec![Message::Summary {
            text: "before".into(),
            keeps: 0,
        }];
        chat.push(Message::User("x".repeat(10_000).as_str().into()));
        let req = summary_request("m", &chat, None);
        assert!(
            req.system
                .as_deref()
                .is_some_and(|p| p.starts_with("You condense"))
        );
        assert!(req.tools.is_empty() && req.turns.len() == 1);
        let said = &req.turns[0].text;
        assert!(said.contains("written to save room:\n\nbefore"));
        assert!(
            said.contains(&format!("{}…", "x".repeat(10))) && said.len() < SUMMARY_TURN_BYTES + 400
        );
        let small = summary_request("m", &chat, Some(1_000));
        assert!(
            small.turns[0].text.len() <= 2_400 + 40,
            "room for the summary"
        );
    }

    #[test]
    fn the_declared_limit_wins_then_the_catalogs_then_the_catalogs_base_id() {
        let mut limits = Limits::default();
        limits
            .catalog
            .insert(ModelRef::new("anthropic", "claude-x"), 200_000);
        let dated = ModelRef::new("anthropic", "claude-x-20261101");
        assert_eq!(limits.get(&dated, None), Some(200_000));
        assert_eq!(limits.get(&dated, Some(1_000_000)), Some(1_000_000));
        assert_eq!(limits.get(&ModelRef::new("openai", "claude-x"), None), None);
    }

    #[test]
    fn a_chat_holds_what_its_last_finished_reply_counted_until_it_starts_fresh() {
        let user = || Message::User("hi".into());
        assert_eq!(used(&[user()]), None, "nothing answered yet");
        let chat = vec![
            user(),
            reply(Some(1_200), Outcome::Done),
            user(),
            reply(Some(9_999), Outcome::Failed("no".into())),
            reply(None, Outcome::Done),
        ];
        assert_eq!(
            used(&chat),
            Some(1_200),
            "failed and uncounted replies skipped"
        );
        let mut fresh = chat.clone();
        fresh.push(Message::Divider("Context cleared".into()));
        assert_eq!(used(&fresh), None);
        fresh.push(user());
        fresh.push(reply(Some(300), Outcome::Done));
        assert_eq!(used(&fresh), Some(300));
    }

    #[test]
    fn a_chat_is_near_full_from_eighty_percent_of_a_known_limit() {
        let fill = |used, limit| Fill { used, limit };
        assert!(!fill(79_999, Some(100_000)).near());
        assert!(fill(80_000, Some(100_000)).near());
        assert!(!fill(1_000_000, None).near(), "unknown limit never warns");
        assert_eq!(fill(50, Some(200)).share(), Some(0.25));
    }
}
