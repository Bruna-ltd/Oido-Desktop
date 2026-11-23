//! A run that stops to wait for you, or for a long delay, and goes on later, even after the app has
//! closed. The engine works through nested steps on its own call stack, so a pause has to unwind
//! it: each level hands back a [`Frame`] saying where it was and what it held, and the frames with
//! the rest of the run's state make a [`Snapshot`]. Resuming walks the frames back down, so the
//! steps before the wait are not run again and the ones after it go on as if nothing had happened.
//! A step in progress when the app closes is never replayed: the wait is the only checkpoint.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::engine::{StepLog, Usage};
use super::template::Scope;

/// One level of the nesting at the moment a run paused. Written innermost first as the run
/// unwinds, and stored outermost first, the order a resume meets them.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "frame", rename_all = "snake_case")]
pub enum Frame {
    /// A list of steps: which was in progress, and what was bound by then.
    Block { at: usize, scope: Scope },
    /// A step: where its line is in the run log.
    Step { slot: usize },
    /// An `if` or `switch`: which of its lists was running (`then`, `else`, `case2`, `default`).
    /// `rest` are the lists still to run after it, when a `switch` runs every case that matches.
    Branch {
        name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rest: Vec<String>,
    },
    /// A `for_each`: the list, the round in progress, and what the earlier rounds gave.
    Each {
        items: Vec<Value>,
        n: usize,
        results: Vec<Value>,
        failed: usize,
        /// What the round in progress bound so far, for the loop's `collect`.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        bag: Scope,
    },
    /// A `while`: the round in progress, and what the earlier rounds gave.
    Loop {
        round: u32,
        results: Vec<Value>,
        /// What the round in progress bound so far, for the loop's `collect`.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        bag: Scope,
    },
    /// A `try`: which attempt, and whether it was already in its catch steps.
    Attempt { attempt: u32, catching: bool },
    /// A `flow` step: the workflow it was running, and what was held for outside text in the one
    /// that called it.
    Call {
        id: String,
        outer: Vec<(String, String)>,
    },
}

/// Everything a paused run needs to go on.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// Outermost first.
    pub frames: Vec<Frame>,
    /// Steps taken so far, against the cap for one run.
    pub taken: usize,
    /// Names holding outside text, and why.
    pub tainted: Vec<(String, String)>,
    pub steered: Option<String>,
    /// The run so far.
    pub output: String,
    pub steps: Vec<StepLog>,
    pub usage: Usage,
}

/// What was answered to a waiting step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    /// You approved, or the delay is over.
    Go,
    /// You said no: the step fails.
    Declined,
}

/// A reason for holding something as outside text, kept for the life of the program. The reasons
/// are few and fixed, and a snapshot read back gives them as text.
pub fn intern(reason: &str) -> &'static str {
    static SEEN: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let Ok(mut seen) = SEEN.lock() else {
        return "outside text";
    };
    if let Some(known) = seen.get(reason) {
        return known;
    }
    let kept: &'static str = Box::leak(reason.to_owned().into_boxed_str());
    seen.insert(kept);
    kept
}

/// Seconds since the epoch.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// A length of time in words: "2 hours", "90 seconds".
pub fn span(seconds: u32) -> String {
    let (n, unit) = match seconds {
        s if s >= 86_400 && s % 86_400 == 0 => (s / 86_400, "day"),
        s if s >= 3_600 && s % 3_600 == 0 => (s / 3_600, "hour"),
        s if s >= 60 && s % 60 == 0 => (s / 60, "minute"),
        s => (s, "second"),
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_paused_scope_is_stored_as_plain_values_and_read_back() {
        let frame = Frame::Block {
            at: 1,
            scope: crate::workflow::template::scope_of([("files".to_owned(), json!({"n": 2}))]),
        };
        let stored = serde_json::to_value(&frame).unwrap();
        assert_eq!(
            stored,
            json!({"frame": "block", "at": 1, "scope": {"files": {"n": 2}}})
        );
        assert_eq!(serde_json::from_value::<Frame>(stored).unwrap(), frame);
    }

    #[test]
    fn a_paused_loop_keeps_what_its_round_bound_and_an_old_frame_without_it_still_reads() {
        let frame = Frame::Each {
            items: vec![json!(1)],
            n: 0,
            results: Vec::new(),
            failed: 0,
            bag: crate::workflow::template::scope_of([("got".to_owned(), json!(3))]),
        };
        let stored = serde_json::to_value(&frame).unwrap();
        assert_eq!(stored["bag"], json!({"got": 3}));
        assert_eq!(serde_json::from_value::<Frame>(stored).unwrap(), frame);
        let old = json!({"frame": "each", "items": [1], "n": 0, "results": [], "failed": 0});
        let read: Frame = serde_json::from_value(old).unwrap();
        assert!(matches!(read, Frame::Each { bag, .. } if bag.is_empty()));
    }
}
