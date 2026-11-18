//! When a browser call waits for the person: the one browser setting, the same for every agent, on
//! the Settings screen. Which sites an agent may reach is not decided here: the Oido Browser
//! Connector holds that list (its allowlist, or "Allow all domains"), and pins each tab to its site.

use serde::{Deserialize, Serialize};

use crate::guard::Action;
use crate::guard::actions::Finding;

/// When a kind of browser call waits for the person's go-ahead.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ask {
    Never,
    /// While outside text (a page, say) is in what the model was sent.
    WhileTainted,
    Always,
}

impl Ask {
    pub const ALL: [Ask; 3] = [Ask::Never, Ask::WhileTainted, Ask::Always];

    pub fn label(self) -> &'static str {
        match self {
            Ask::Never => "Never ask",
            Ask::WhileTainted => "Ask after reading a page",
            Ask::Always => "Always ask",
        }
    }
}

/// When browser calls wait for the person.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Approvals {
    /// Opening a page, going back, forward or reloading.
    pub open: Ask,
    /// Clicking, typing, choosing, pressing keys, answering a page's dialog.
    pub act: Ask,
}

impl Default for Approvals {
    /// Pages open freely, since the connector's allowlist decides where; acting asks once a page
    /// has been read, since a page can steer clicks on sites the person is signed in to.
    fn default() -> Approvals {
        Approvals {
            open: Ask::Never,
            act: Ask::WhileTainted,
        }
    }
}

/// What a browser call does, for the approvals.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Reads what is open, scrolls, lists or closes tabs.
    Read,
    /// Opens a page, or goes back, forward, or loads it again.
    Open,
    /// Changes something on the page.
    Act,
}

impl Kind {
    pub fn of(action: &str) -> Option<Kind> {
        Some(match action {
            "read" | "tabs" | "console" | "network" | "scroll" | "close" => Kind::Read,
            "navigate" | "back" | "forward" | "reload" => Kind::Open,
            "click" | "type" | "press" | "submit" | "choose" | "check" | "set" | "hover"
            | "drag" | "dialog" => Kind::Act,
            _ => return None,
        })
    }
}

/// The words the rule ids stand for, on cards and in runs.
pub fn title(rule: &str) -> Option<&'static str> {
    Some(match rule {
        "browser.open" => "Opens a page",
        "browser.act" => "Clicks or types on a page",
        "browser.commit" => "Takes a step that commits",
        _ => return None,
    })
}

impl Approvals {
    /// Whether a step that commits (posting, sending, paying, deleting) waits for the person: as an
    /// acting call would with a page in view, which Jev always has. `taint`: the taint pack is on.
    pub fn commits_ask(&self, taint: bool) -> bool {
        self.check(Kind::Act, taint.then_some("a web page"))
            .is_some()
    }

    /// Whether a call of `kind` waits for the person, given what outside text the model had in
    /// view (`outside`, only when the taint pack is on).
    pub fn check(&self, kind: Kind, outside: Option<&str>) -> Option<Finding> {
        let (ask, rule, what) = match kind {
            Kind::Read => return None,
            Kind::Open => (self.open, "browser.open", "It opens a page in your browser"),
            Kind::Act => (
                self.act,
                "browser.act",
                "It clicks, types or presses keys in your browser",
            ),
        };
        let why = match (ask, outside) {
            (Ask::Never, _) | (Ask::WhileTainted, None) => return None,
            (Ask::Always, _) => format!("{what}, and your settings ask before it does."),
            (Ask::WhileTainted, Some(from)) => format!(
                "{what} after reading {from}, which could be steering it. Check this is what you want."
            ),
        };
        Some(Finding {
            rule,
            why,
            action: Action::Ask,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn approvals_follow_the_setting_and_what_is_in_view() {
        let a = Approvals::default();
        let page = Some("a web page");
        assert!(a.check(Kind::Open, page).is_none());
        assert!(a.check(Kind::Read, page).is_none());
        assert!(a.check(Kind::Act, None).is_none());
        assert_eq!(a.check(Kind::Act, page).unwrap().rule, "browser.act");
        let strict = Approvals {
            open: Ask::Always,
            act: Ask::Always,
        };
        assert_eq!(strict.check(Kind::Open, None).unwrap().action, Action::Ask);
        assert!(
            Approvals {
                act: Ask::Never,
                ..a
            }
            .check(Kind::Act, page)
            .is_none()
        );
        assert_eq!(Kind::of("submit"), Some(Kind::Act));
        assert_eq!(Kind::of("reload"), Some(Kind::Open));
        assert_eq!(Kind::of("fly"), None);
        assert!(a.commits_ask(true) && !a.commits_ask(false));
        assert!(strict.commits_ask(false));
        let free = Approvals {
            act: Ask::Never,
            ..a
        };
        assert!(!free.commits_ask(true));
    }

    #[test]
    fn approvals_saved_by_an_older_version_keep_the_defaults_for_what_is_missing() {
        let a: Approvals = serde_json::from_str(r#"{"act":"always"}"#).unwrap();
        assert_eq!(
            a,
            Approvals {
                act: Ask::Always,
                ..Approvals::default()
            }
        );
        // The first build's setting had more fields; they are ignored.
        let old: Approvals =
            serde_json::from_str(r#"{"open_listed":"never","unlisted":"block"}"#).unwrap();
        assert_eq!(old, Approvals::default());
    }
}
