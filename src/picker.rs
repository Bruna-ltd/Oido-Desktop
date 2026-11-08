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
fn score(query: &str, text: &str) -> Option<i32> {
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
