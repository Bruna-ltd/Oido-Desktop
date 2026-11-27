//! Approvals on a channel: a call the guard holds for your go-ahead is also sent to you on the
//! channel chosen on the Channels screen, as a card that says what the agent wants to do and why it
//! waits. On Telegram and Slack it has Allow and Refuse buttons; on WhatsApp you reply with its
//! code (`A7` to allow, `R7` to refuse). Whichever answer comes first, here or there, decides it;
//! a later one is told it was already decided.
//!
//! You are the one paired person marked trusted on that channel. With none, or several, no card is
//! sent and the screen says why. What a card shows goes through the chat's secret placeholders, so
//! a hidden value never leaves the computer this way.

use super::press::{Action, clip};
use super::{Kind, Place};
use crate::app::App;
use crate::guard::{Hidden, TripState};
use crate::model::Message;

/// The setting that names the channel approvals go to.
const SETTING: &str = "channels.approvals";
/// How much of what a held call does a card shows, in characters.
const SHOWN: usize = 300;

/// A held call a card's code answers, and whose code it is.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Hold {
    user: String,
    sid: String,
    call: String,
}

/// What a reply means as an answer to a card: allow (`A7`) or refuse (`R7`) card 7.
fn code_in(text: &str) -> Option<(bool, u64)> {
    let text = text.trim();
    let mut chars = text.chars();
    let allow = match chars.next()?.to_ascii_uppercase() {
        'A' => true,
        'R' => false,
        _ => return None,
    };
    chars.as_str().parse().ok().map(|n| (allow, n))
}

impl App {
    /// Reads the channel approvals go to.
    pub(super) fn load_channel_approvals(&mut self) {
        let saved: Option<String> = self
            .store
            .as_ref()
            .and_then(|db| db.setting(SETTING).ok().flatten());
        self.channels.approvals =
            saved.and_then(|s| Kind::ALL.into_iter().find(|k| k.as_str() == s));
    }

    /// Sends approvals to `kind`, or nowhere.
    pub fn set_channel_approvals(&mut self, kind: Option<Kind>) {
        self.channels.approvals = kind;
        let value = kind.map(Kind::as_str);
        self.save("where approvals go", |db| db.set_setting(SETTING, &value));
    }

    /// Where approvals go now, and to whom: the chosen channel's one trusted paired person.
    /// Otherwise why none go, in words for the screen (`Err(None)` when none is chosen).
    pub fn approver(&self) -> Result<(Kind, String), Option<String>> {
        let kind = self.channels.approvals.ok_or(None)?;
        let c = self.channels.get(kind);
        if !c.connected() {
            return Err(Some(format!("{} is not connected.", kind.title())));
        }
        let trusted: Vec<_> = c.users.iter().filter(|u| u.trusted).collect();
        match trusted.as_slice() {
            [you] => Ok((kind, you.id.clone())),
            [] => Err(Some(format!(
                "Mark yourself trusted among the people paired on {}: approvals go to that one person.",
                kind.title()
            ))),
            _ => Err(Some(format!(
                "Several people on {} are trusted; approvals go only where exactly one is.",
                kind.title()
            ))),
        }
    }

    /// Where a held call is approved, in words: "on the computer running Oido", and on the
    /// channel approvals go to.
    pub(super) fn approved_where(&self) -> String {
        match self.approver() {
            Ok((kind, _)) => format!(
                "from the owner, on {} or on the computer running Oido",
                kind.title()
            ),
            Err(_) => "on the computer running Oido".to_owned(),
        }
    }

    /// Sends a card for each call in chat `sid` the guard holds that has none yet, to you, where
    /// approvals go. `hidden` is the chat's secrets: what the card shows is masked with them.
    pub(crate) fn offer_holds(&mut self, sid: &str, hidden: &Hidden) {
        let Ok((kind, user)) = self.approver() else {
            return;
        };
        let Some(agent) = self.roster.owner_of(sid) else {
            return;
        };
        let name = agent.name.clone();
        let held: Vec<(String, String, String, String)> = agent
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .and_then(|chat| match chat.messages.last() {
                Some(Message::Assistant { tools, .. }) => Some(tools),
                _ => None,
            })
            .into_iter()
            .flatten()
            .filter(|t| t.output.is_none())
            .filter_map(|t| {
                let trip = t.trip.as_ref().filter(|t| t.state == TripState::Asking)?;
                Some((
                    t.id.clone(),
                    crate::tools::shown(&t.name, &self.mcp.servers).label,
                    hidden.hide(&t.summary),
                    format!(
                        "{} ({})",
                        hidden.hide(&trip.why),
                        crate::guard::title(&trip.rule)
                    ),
                ))
            })
            .collect();
        for (call, tool, what, why) in held {
            let key = format!("hold:{call}");
            let c = self.channels.get_mut(kind);
            if !c.offered.insert(key) {
                continue;
            }
            c.next_code += 1;
            let n = c.next_code;
            c.holds.insert(
                n,
                Hold {
                    user: user.clone(),
                    sid: sid.to_owned(),
                    call: call.clone(),
                },
            );
            let mut said = format!(
                "🔒 **{name} waits for your go-ahead**\n\n{tool}: {}\n\nWhy it waits: {why}",
                clip(&what, SHOWN)
            );
            let buttons = if kind == Kind::WhatsApp {
                said.push_str(&format!(
                    "\n\nReply **A{n}** to allow it, or **R{n}** to refuse."
                ));
                Vec::new()
            } else {
                c.buttons(
                    &user,
                    vec![
                        (
                            "Allow".to_owned(),
                            Action::Hold {
                                sid: sid.to_owned(),
                                call: call.clone(),
                                allow: true,
                            },
                        ),
                        (
                            "Refuse".to_owned(),
                            Action::Hold {
                                sid: sid.to_owned(),
                                call,
                                allow: false,
                            },
                        ),
                    ],
                )
            };
            c.say_with(&Place::private(&user), &said, buttons);
        }
    }

    /// `user`'s message `text` on `kind` if it answers a card by its code: what to tell them.
    pub(super) fn answer_by_code(&mut self, kind: Kind, user: &str, text: &str) -> Option<String> {
        let (allow, n) = code_in(text)?;
        let c = self.channels.get_mut(kind);
        let hold = c.holds.get(&n).filter(|h| h.user == user)?.clone();
        c.holds.remove(&n);
        Some(self.decide_hold(&hold.sid, &hold.call, allow))
    }

    /// Allows or refuses held call `call` in chat `sid`, if it still waits: what to tell whoever
    /// answered.
    pub(super) fn decide_hold(&mut self, sid: &str, call: &str, allow: bool) -> String {
        let waits = self
            .roster
            .owner_of(sid)
            .and_then(|a| a.sessions.iter().find(|s| s.id == sid))
            .and_then(|chat| match chat.messages.last() {
                Some(Message::Assistant { tools, .. }) => tools.iter().find(|t| t.id == call),
                _ => None,
            })
            .is_some_and(|t| {
                t.output.is_none()
                    && t.trip
                        .as_ref()
                        .is_some_and(|t| t.state == TripState::Asking)
            });
        if !waits {
            return "That was already decided.".to_owned();
        }
        self.decide_trip(sid, call, allow);
        if allow {
            "Allowed. It goes on.".to_owned()
        } else {
            "Refused. The agent is told and goes on without it.".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_reads_as_allow_or_refuse_and_a_number() {
        assert_eq!(code_in("A7"), Some((true, 7)));
        assert_eq!(code_in(" r12 "), Some((false, 12)));
        assert_eq!(code_in("Allow"), None);
        assert_eq!(code_in("A 7"), None);
        assert_eq!(code_in(""), None);
    }
}
