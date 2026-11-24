//! Whether a browser action commits something (posts, sends, pays, deletes, submits) and so must
//! not run on Jev's pick alone where the person's settings ask first. Ported from oido-core's
//! approval gate (`internal/browseragent/gate.go`, `judge`).
//!
//! Deterministic on purpose: an advisory prompt was ignored by a real model, and TypeSafe's own
//! needs-approval answer is not calibrated. It fails closed: a click on a control the page does not
//! show, or on one with no label, counts as committing.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use super::render::Page;

/// A control caption that commits something. Word-bounded, so "Posts" and "Following feed" (tabs)
/// don't match while "Post", "Follow" and "Following" (the unfollow toggle) do. Not "change":
/// "Change language" or "change number of passengers" open a picker, and the click that commits in
/// it is its own Save or Confirm. "clear", "reset" and "discard" throw away answers already given:
/// live, Jev cleared a Google Forms row the goal said to keep.
static CONSEQUENTIAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(follow|following|unfollow|send|post|tweet|reply|repost|retweet|pay|buy|purchase|order|checkout|delete|remove|clear|reset|discard|submit|subscribe|unsubscribe|confirm|publish|book|donate|transfer|apply|approve|save|update|place|complete|finish|sign ?up|register|create|invite|connect|add to (cart|basket))\b")
        .unwrap_or_else(|e| panic!("bad consequential-label pattern: {e}"))
});

/// X's reply and repost icons ("92 Replies. Reply"): they open the composer or a menu, and the
/// click that posts is a second one, which stays held.
static COUNTER_ICON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*[\d.,]+\s*[KM]?\s+(replies|reply|reposts|repost)\.\s*(reply|repost)\s*$")
        .unwrap_or_else(|e| panic!("bad counter-icon pattern: {e}"))
});

/// A box where Enter runs a query rather than submitting a form that does something.
static SEARCH_BOX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)search|find|query|filter|look ?up")
        .unwrap_or_else(|e| panic!("bad search-box pattern: {e}"))
});

/// Why the browser call `args` commits something on `page`, or `None` when it does not. `typed` is
/// the label of the box last typed into on this page, for Enter.
pub fn reason(args: &Map<String, Value>, page: &Page, typed: Option<&str>) -> Option<String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default();
    match s("action") {
        "click" if s("button") == "right" => None,
        "click" => {
            let r = s("ref");
            let label = r
                .strip_prefix('c')
                .and_then(|n| n.parse::<usize>().ok())
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| page.controls.get(i))
                .map(|c| c.label.trim());
            match label {
                None => Some(format!(
                    "a click on {r:?}, which the page as read does not show"
                )),
                Some("") => Some(format!("a click on an unlabelled control ({r})")),
                Some(l) if COUNTER_ICON.is_match(l) => None,
                Some(l) if CONSEQUENTIAL.is_match(l) => Some(format!("a click on {l:?}")),
                Some(_) => None,
            }
        }
        "submit" => Some("submitting the form in front of it".to_owned()),
        "press" => {
            // Ctrl+Enter is the send shortcut on most composers: judged by the key.
            let key = s("key").rsplit('+').next().unwrap_or_default().trim();
            let searching = typed.is_some_and(|l| SEARCH_BOX.is_match(l));
            (key.eq_ignore_ascii_case("Enter") && !searching)
                .then(|| "pressing Enter, which may submit a form".to_owned())
        }
        "drag" => Some(
            "dragging one item onto another, which may move or file something for real".to_owned(),
        ),
        "dialog" if s("answer") == "accept" => Some(match &page.dialog {
            Some(d) => format!("accepting the page's question {:?}", d.message),
            None => "accepting a page dialog".to_owned(),
        }),
        // navigate, read, type, choose, check, scroll, back…: the allowlist and the tab's site lock
        // cover where it goes, and none of these commits by itself.
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    fn page() -> Page {
        serde_json::from_value(json!({
            "url": "https://x.com",
            "controls": [
                {"tag": "input", "type": "text", "label": "Search query"},
                {"tag": "input", "type": "text", "label": "Post text"},
                {"tag": "button", "label": "Follow @ada"},
                {"tag": "button", "label": "Following"},
                {"tag": "button", "label": "Posts"},
                {"tag": "button", "label": "More options"},
                {"tag": "button", "label": ""},
                {"tag": "button", "label": "92 Replies. Reply"},
                {"tag": "button", "label": "1.2K Reposts. Repost"},
                {"tag": "button", "label": "Reply"},
                {"tag": "button", "label": "Change number of passengers"},
                {"tag": "button", "label": "Clear selection"},
                {"tag": "button", "label": "Clearance sale"}
            ]
        }))
        .unwrap()
    }

    fn held(v: Value, typed: Option<&str>) -> bool {
        let Value::Object(a) = v else { panic!() };
        reason(&a, &page(), typed).is_some()
    }

    #[test]
    fn clicks_that_commit_are_told_from_ones_that_do_not() {
        for (args, want) in [
            (json!({"action": "navigate", "url": "https://x.com"}), false),
            (json!({"action": "read"}), false),
            (json!({"action": "scroll", "direction": "down"}), false),
            (json!({"action": "type", "ref": "c2", "text": "hi"}), false),
            (json!({"action": "back"}), false),
            (
                json!({"action": "choose", "ref": "c1", "option": "Yes"}),
                false,
            ),
            (json!({"action": "click", "ref": "c3"}), true),
            (json!({"action": "click", "ref": "c4"}), true),
            (json!({"action": "click", "ref": "c5"}), false),
            (json!({"action": "click", "ref": "c6"}), false),
            (json!({"action": "click", "ref": "c1"}), false),
            (json!({"action": "click", "ref": "c7"}), true),
            (json!({"action": "click", "ref": "c99"}), true),
            (json!({"action": "click", "ref": "l1"}), true),
            (json!({"action": "click", "ref": "c8"}), false),
            (json!({"action": "click", "ref": "c9"}), false),
            (json!({"action": "click", "ref": "c10"}), true),
            (json!({"action": "click", "ref": "c11"}), false),
            (json!({"action": "click", "ref": "c12"}), true),
            (json!({"action": "click", "ref": "c13"}), false),
            (
                json!({"action": "click", "ref": "c3", "button": "right"}),
                false,
            ),
            (json!({"action": "press", "key": "Escape"}), false),
            (json!({"action": "press", "key": "Shift+Tab"}), false),
            (json!({"action": "press", "key": "Enter"}), true),
            (json!({"action": "press", "key": "Ctrl+Enter"}), true),
            (json!({"action": "submit"}), true),
            (json!({"action": "drag", "ref": "c3", "to": "c1"}), true),
            (json!({"action": "dialog", "answer": "accept"}), true),
            (json!({"action": "dialog", "answer": "dismiss"}), false),
        ] {
            assert_eq!(held(args.clone(), None), want, "{args}");
        }
    }

    #[test]
    fn enter_in_a_search_box_runs_a_query_but_elsewhere_may_submit() {
        let enter = json!({"action": "press", "key": "Enter"});
        assert!(!held(enter.clone(), Some("Search query")));
        assert!(held(enter, Some("Post text")));
    }

    #[test]
    fn accepting_a_dialog_names_its_question() {
        let mut p = page();
        p.dialog = Some(super::super::render::Dialog {
            kind: "confirm".into(),
            message: "Delete this item?".into(),
        });
        let Value::Object(a) = json!({"action": "dialog", "answer": "accept"}) else {
            panic!()
        };
        assert!(reason(&a, &p, None).unwrap().contains("Delete this item?"));
    }
}
