//! The model picker's logic, apart from drawing: what is listed (grouped by provider, filtered and
//! ranked by the search) and how the keyboard moves through it.

use rune::TextEdit;

use crate::models::{Caps, ModelInfo, ModelRef};

/// The picker's search text, highlighted model and open flag, kept in app state.
#[derive(Clone, Debug, Default)]
pub struct PickerState {
    pub open: bool,
    pub search: TextEdit,
    /// The model the keyboard is on; the chosen one until an arrow moves it.
    pub active: Option<ModelRef>,
}

impl PickerState {
    pub fn close(&mut self) {
        self.open = false;
        self.search.focused = false;
        self.search.preedit = None;
    }

    /// Opens with a clean search and the highlight on `current`.
    pub fn show(&mut self, current: Option<&ModelRef>) {
        self.open = true;
        self.search = TextEdit::new("");
        self.search.focused = true;
        self.active = current.cloned();
    }
}

/// One connected provider and the models it offers.
#[derive(Clone, Debug)]
pub struct Group {
    pub provider: String,
    pub label: String,
    pub models: Vec<ModelInfo>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A provider's name over its models. Not choosable.
    Heading { provider: String, label: String },
    Model {
        model: ModelRef,
        name: String,
        caps: Caps,
    },
}

/// The rows for `query`: each provider's heading and the models that match, best match first.
/// A provider with no match disappears with its heading. The provider's name is searched too, so
/// typing "anthropic" finds all of that provider's models.
pub fn rows(groups: &[Group], query: &str) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    let mut out = Vec::new();
    for g in groups {
        let mut hits: Vec<(i32, usize)> = g
            .models
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                if query.is_empty() {
                    return Some((0, i));
                }
                [&m.name, &m.id, &g.label]
                    .iter()
                    .filter_map(|text| score(&query, text))
                    .max()
                    .map(|s| (s, i))
            })
            .collect();
        if hits.is_empty() {
            continue;
        }
        hits.sort_by_key(|&(s, i)| (std::cmp::Reverse(s), i));
        out.push(Row::Heading {
            provider: g.provider.clone(),
            label: g.label.clone(),
        });
        for (_, i) in hits {
            let m = &g.models[i];
            out.push(Row::Model {
                model: ModelRef::new(&g.provider, &m.id),
                name: m.name.clone(),
                caps: m.caps,
            });
        }
    }
    out
}

/// Case-insensitive subsequence match; runs of consecutive letters and word starts score higher.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let needle: Vec<char> = query.chars().collect();
    let (mut n, mut score, mut previous, mut boundary) = (0, 0, false, true);
    for c in text.to_lowercase().chars() {
        if n < needle.len() && c == needle[n] {
            score += 10 + if previous { 12 } else { 0 } + if boundary { 8 } else { 0 };
            previous = true;
            n += 1;
        } else {
            previous = false;
            score -= 1;
        }
        boundary = !c.is_alphanumeric();
    }
    (n == needle.len()).then_some(score)
}

/// Where the highlight is in `rows`: `active` if it is shown, else the first model.
pub fn active_index(rows: &[Row], active: Option<&ModelRef>) -> Option<usize> {
    let models = || {
        rows.iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, Row::Model { .. }))
    };
    models()
        .find(|(_, r)| matches!(r, Row::Model { model, .. } if Some(model) == active))
        .or_else(|| models().next())
        .map(|(i, _)| i)
}

/// The model `by` places from the highlight, skipping headings. An arrow wraps around the ends;
/// a page, Home or End stops at them.
pub fn shifted(rows: &[Row], active: Option<&ModelRef>, by: isize, wrap: bool) -> Option<ModelRef> {
    let models: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r, Row::Model { .. }))
        .map(|(i, _)| i)
        .collect();
    if models.is_empty() {
        return None;
    }
    let at = active_index(rows, active)
        .and_then(|i| models.iter().position(|&m| m == i))
        .unwrap_or(0) as isize;
    let last = models.len() as isize - 1;
    let to = if wrap {
        (at + by).rem_euclid(last + 1)
    } else {
        at.saturating_add(by).clamp(0, last)
    };
    match &rows[models[to as usize]] {
        Row::Model { model, .. } => Some(model.clone()),
        Row::Heading { .. } => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn m(id: &str, name: &str) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            name: name.into(),
            caps: Caps::default(),
        }
    }

    fn groups() -> Vec<Group> {
        vec![
            Group {
                provider: "anthropic".into(),
                label: "Anthropic".into(),
                models: vec![
                    m("claude-opus-4-7", "Claude Opus 4.7"),
                    m("claude-sonnet-4-6", "Claude Sonnet 4.6"),
                ],
            },
            Group {
                provider: "openai".into(),
                label: "OpenAI".into(),
                models: vec![m("gpt-4o", "GPT-4o"), m("gpt-4o-mini", "GPT-4o Mini")],
            },
        ]
    }

    fn describe(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                Row::Heading { label, .. } => format!("# {label}"),
                Row::Model { model, .. } => format!("{}/{}", model.provider, model.id),
            })
            .collect()
    }

    #[test]
    fn with_no_search_every_provider_shows_under_its_heading_in_order() {
        assert_eq!(
            describe(&rows(&groups(), "")),
            vec![
                "# Anthropic",
                "anthropic/claude-opus-4-7",
                "anthropic/claude-sonnet-4-6",
                "# OpenAI",
                "openai/gpt-4o",
                "openai/gpt-4o-mini"
            ]
        );
    }

    #[test]
    fn a_search_keeps_only_matches_and_drops_a_provider_with_none() {
        assert_eq!(
            describe(&rows(&groups(), "opus")),
            vec!["# Anthropic", "anthropic/claude-opus-4-7"]
        );
        assert!(rows(&groups(), "zzzz").is_empty());
    }

    #[test]
    fn the_provider_name_is_searched_so_it_finds_all_its_models() {
        let hits = describe(&rows(&groups(), "openai"));
        assert_eq!(
            hits,
            vec!["# OpenAI", "openai/gpt-4o", "openai/gpt-4o-mini"]
        );
    }

    #[test]
    fn the_best_match_comes_first_within_a_provider() {
        let r = rows(&groups(), "mini");
        assert_eq!(describe(&r), vec!["# OpenAI", "openai/gpt-4o-mini"]);
        let g = vec![Group {
            provider: "p".into(),
            label: "P".into(),
            models: vec![m("a-xmx", "A xmx"), m("mx", "Mx")],
        }];
        assert_eq!(
            describe(&rows(&g, "mx"))[1],
            "p/mx",
            "a word that starts with the letters beats one that contains them"
        );
    }

    #[test]
    fn the_highlight_starts_on_the_chosen_model_or_the_first() {
        let r = rows(&groups(), "");
        assert_eq!(
            active_index(&r, Some(&ModelRef::new("openai", "gpt-4o"))),
            Some(4)
        );
        assert_eq!(
            active_index(&r, Some(&ModelRef::new("openai", "gone"))),
            Some(1),
            "a model not shown falls back to the first"
        );
        assert_eq!(active_index(&r, None), Some(1), "never on a heading");
        assert_eq!(active_index(&[], None), None);
    }

    #[test]
    fn arrows_skip_headings_and_wrap_but_pages_and_ends_stop() {
        let r = rows(&groups(), "");
        let sonnet = ModelRef::new("anthropic", "claude-sonnet-4-6");
        assert_eq!(
            shifted(&r, Some(&sonnet), 1, true),
            Some(ModelRef::new("openai", "gpt-4o")),
            "down steps over the OpenAI heading"
        );
        let last = ModelRef::new("openai", "gpt-4o-mini");
        assert_eq!(
            shifted(&r, Some(&last), 1, true),
            Some(ModelRef::new("anthropic", "claude-opus-4-7")),
            "past the end wraps to the start"
        );
        assert_eq!(
            shifted(
                &r,
                Some(&ModelRef::new("anthropic", "claude-opus-4-7")),
                -1,
                true
            ),
            Some(last.clone())
        );
        assert_eq!(
            shifted(&r, Some(&last), 5, false),
            Some(last),
            "a page stops at the end"
        );
        assert_eq!(
            shifted(&r, Some(&sonnet), isize::MIN, false),
            Some(ModelRef::new("anthropic", "claude-opus-4-7")),
            "Home"
        );
        assert_eq!(shifted(&[], None, 1, true), None);
    }
}

/// A multi-select picker's search, highlighted row and open flag (skills, tools). Unlike the
/// model picker it stays open as you choose, since several can be picked.
#[derive(Clone, Debug, Default)]
pub struct MultiPicker {
    pub open: bool,
    pub search: TextEdit,
    /// Place in the shown list the keyboard is on.
    pub active: usize,
}

impl MultiPicker {
    pub fn close(&mut self) {
        self.open = false;
        self.search.focused = false;
        self.search.preedit = None;
    }

    pub fn show(&mut self) {
        self.open = true;
        self.search = TextEdit::new("");
        self.search.focused = true;
        self.active = 0;
    }
}

/// One choice in a multi-select: a skill or a tool.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
    /// What is chosen and saved.
    pub name: String,
    /// What the person sees; often the name itself.
    pub label: String,
    pub description: String,
    /// The heading it is listed under, or empty for a list without headings. Items of one group
    /// come together.
    pub group: String,
}

impl Item {
    /// An item shown by its own name, under no heading.
    pub fn plain(name: String, description: String) -> Self {
        Self {
            label: name.clone(),
            name,
            description,
            group: String::new(),
        }
    }
}

/// The items that match `query`, best first within each group, groups in their order: a label or
/// name that starts with it, then one that contains it, then a description that does. All, in
/// order, when the query is empty. Plain text, not scattered letters: with a long list those match
/// nearly everything.
pub fn matching<'a>(items: &'a [Item], query: &str) -> Vec<&'a Item> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return items.iter().collect();
    }
    let group_at = |group: &str| items.iter().position(|it| it.group == group);
    let mut hits: Vec<(Option<usize>, u8, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let names = [s.label.to_lowercase(), s.name.to_lowercase()];
            let rank = if names.iter().any(|n| n.starts_with(&query)) {
                0
            } else if names.iter().any(|n| n.contains(&query)) {
                1
            } else if s.description.to_lowercase().contains(&query) {
                2
            } else {
                return None;
            };
            Some((group_at(&s.group), rank, i))
        })
        .collect();
    hits.sort();
    hits.into_iter().map(|(_, _, i)| &items[i]).collect()
}

#[cfg(test)]
mod multi_tests {
    use super::*;

    #[test]
    fn searching_finds_by_name_first_then_by_description() {
        let item = |n: &str, d: &str| Item::plain(n.into(), d.into());
        let all = [
            item("copy", "Helps with seo copy"),
            item("ai-seo", "Search answers"),
            item("seo-audit", "Checks pages"),
            item("serverless", "Bring any source"),
        ];
        let names = |q| {
            matching(&all, q)
                .iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(""), ["copy", "ai-seo", "seo-audit", "serverless"]);
        assert_eq!(
            names("seo"),
            ["seo-audit", "ai-seo", "copy"],
            "starts with, then contains, then the description; no scattered letters"
        );
        assert!(names("zzz").is_empty());
    }

    #[test]
    fn a_search_keeps_groups_together_and_finds_by_label() {
        let item = |n: &str, l: &str, g: &str| Item {
            name: n.into(),
            label: l.into(),
            description: String::new(),
            group: g.into(),
        };
        let all = [
            item("read_file", "Read files", "Defaults"),
            item("mcp__resend__send_email", "Send email", "Resend"),
            item("shell", "Run commands", "Defaults"),
            item("mcp__resend__list_emails", "List emails", "Resend"),
        ];
        let names = |q| {
            matching(&all, q)
                .iter()
                .map(|s| s.label.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names("send e"), ["Send email"], "the label is searched");
        assert_eq!(
            names("e"),
            ["Read files", "Run commands", "Send email", "List emails"],
            "matches stay under their group, groups in list order"
        );
    }
}
