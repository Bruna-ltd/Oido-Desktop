//! An agent's own approvals: tools it may use only with your go-ahead, chosen on its page, and
//! where that go-ahead is asked for. Each call to one is held like any the guard questions (rule
//! `agent.ask`), so it gets the same card here and, where approvals go, on a channel. They only
//! add to the guard: nothing here lets through what a rule holds.

use serde::{Deserialize, Serialize};

use super::Action;
use super::actions::Finding;
use crate::channels::Kind;

/// Where an agent's held calls are sent for your go-ahead, beside the card here.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendTo {
    /// Where the Channels screen sends approvals.
    #[default]
    Global,
    /// Nowhere: the card here only.
    Desktop,
    Telegram,
    Slack,
    #[serde(rename = "whatsapp")]
    WhatsApp,
}

impl SendTo {
    pub const ALL: [SendTo; 5] = [
        SendTo::Global,
        SendTo::Desktop,
        SendTo::Telegram,
        SendTo::Slack,
        SendTo::WhatsApp,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SendTo::Global => "As Channels",
            SendTo::Desktop => "Only here",
            SendTo::Telegram => "Telegram",
            SendTo::Slack => "Slack",
            SendTo::WhatsApp => "WhatsApp",
        }
    }

    /// The channel it names itself, if it names one.
    pub fn kind(self) -> Option<Kind> {
        match self {
            SendTo::Telegram => Some(Kind::Telegram),
            SendTo::Slack => Some(Kind::Slack),
            SendTo::WhatsApp => Some(Kind::WhatsApp),
            SendTo::Global | SendTo::Desktop => None,
        }
    }
}

/// An agent's approvals, as saved with it.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Approvals {
    /// Whether they hold anything; off, the tools stay chosen for when it is on again.
    pub on: bool,
    pub to: SendTo,
    /// The tools each call to which waits for your go-ahead.
    pub tools: Vec<String>,
}

impl Approvals {
    /// The tools that wait now: none while it is off.
    pub fn asking(&self) -> Vec<String> {
        if self.on {
            self.tools.clone()
        } else {
            Vec::new()
        }
    }
}

/// The hold for a call to `name` when it is one of `asking`.
pub(super) fn check(name: &str, asking: &[String]) -> Option<Finding> {
    (name != crate::tools::ASK && asking.iter().any(|t| t == name)).then(|| Finding {
        rule: "agent.ask",
        why: "You chose to approve every use of this tool by this agent.".to_owned(),
        action: Action::Ask,
    })
}

/// What holds a `run_workflow` or `script` call once, before it starts: its steps call `picked`,
/// tools the agent chose to approve, named as people know them. Its steps then run without asking
/// again for those.
pub(crate) fn steps_ask(picked: &[String]) -> Option<Finding> {
    (!picked.is_empty()).then(|| Finding {
        rule: "agent.ask",
        why: format!(
            "Its steps use tools you chose to approve for this agent: {}. Allowing it lets them run.",
            picked.join(", ")
        ),
        action: Action::Ask,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_tools_chosen_while_it_is_on_wait_and_never_the_question_tool() {
        let mut a = Approvals {
            on: false,
            to: SendTo::Desktop,
            tools: vec!["shell".into(), crate::tools::ASK.into()],
        };
        assert!(a.asking().is_empty());
        a.on = true;
        let asking = a.asking();
        assert!(check("shell", &asking).is_some_and(|f| f.action == Action::Ask));
        assert!(check("read_file", &asking).is_none());
        assert!(check(crate::tools::ASK, &asking).is_none());
    }

    #[test]
    fn steps_that_use_a_chosen_tool_wait_once_and_name_it() {
        assert!(steps_ask(&[]).is_none());
        let f = steps_ask(&["Shell".into(), "Send message (Telegram)".into()]);
        assert!(
            f.as_ref()
                .is_some_and(|f| f.action == Action::Ask && f.rule == "agent.ask")
        );
        assert!(f.is_some_and(|f| f.why.contains("Shell, Send message (Telegram)")));
    }

    #[test]
    fn it_reads_back_as_saved_and_an_empty_one_is_off() {
        let a = Approvals {
            on: true,
            to: SendTo::WhatsApp,
            tools: vec!["shell".into()],
        };
        let json = serde_json::to_string(&a).unwrap_or_default();
        assert!(json.contains("\"whatsapp\""), "{json}");
        assert_eq!(serde_json::from_str::<Approvals>(&json).ok(), Some(a));
        assert_eq!(
            serde_json::from_str::<Approvals>("{}").ok(),
            Some(Approvals::default())
        );
    }
}
