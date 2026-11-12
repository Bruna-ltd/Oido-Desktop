//! What a channel person's message must pass before it reaches an agent: the limits on how long
//! and how often (Guardrails screen, off until set), and the agent's topics (its form, answers
//! anything until set). A message that fails gets a short reply on the spot; no model is asked.

use crate::app::App;
use crate::channels::Kind;

/// How far back the hourly limit looks, in seconds.
const HOUR: i64 = 60 * 60;

impl App {
    /// Why `text`, from `user` on `kind` at `at`, is turned away by the limits, or `None` when it
    /// may go on (and then it counts towards the hourly limit).
    pub(crate) fn intake_refusal(
        &mut self,
        kind: Kind,
        user: &str,
        text: &str,
        at: i64,
    ) -> Option<String> {
        let limits = self.guard.limits();
        let length = text.chars().count();
        if limits.length > 0 && length > limits.length as usize {
            return Some(format!(
                "That message is too long for me: {length} characters, and I take up to {}. Please send a shorter one.",
                limits.length
            ));
        }
        if limits.per_hour == 0 {
            return None;
        }
        let recent = self
            .intake
            .entry(format!("{}:{user}", kind.as_str()))
            .or_default();
        recent.retain(|t| at - t < HOUR);
        if recent.len() >= limits.per_hour as usize {
            return Some(format!(
                "You have sent {} messages in the last hour, the most I take. Please try again later.",
                limits.per_hour
            ));
        }
        recent.push(at);
        None
    }

    /// Why `text`, from `user` on `kind` for agent `agent`, is turned away for being off the
    /// agent's topics, or `None` when the agent has none, the text names one, or it answers a
    /// question the agent asked them.
    pub(crate) fn off_topic(
        &self,
        kind: Kind,
        user: &str,
        agent: &str,
        text: &str,
    ) -> Option<String> {
        let a = self.roster.agent_ref(agent)?;
        if a.channel_topics.is_empty() {
            return None;
        }
        let answering = self
            .channels
            .get(kind)
            .chats
            .iter()
            .find(|b| b.user == user && b.agent == agent)
            .is_some_and(|b| {
                self.pending_call_in(&b.session, crate::tools::ASK)
                    .is_some()
            });
        let said = text.to_lowercase();
        let on_topic = a
            .channel_topics
            .iter()
            .any(|t| said.contains(&t.to_lowercase()));
        (!answering && !on_topic).then(|| {
            format!(
                "{} only helps with {} here. Ask about one of those.",
                a.name,
                list(&a.channel_topics)
            )
        })
    }
}

/// "a", "a and b", "a, b and c".
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// An agent's topics as typed in its form: split on commas and lines, trimmed, empty ones left out.
pub fn topics(text: &str) -> Vec<String> {
    text.split([',', '\n'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::guard::Limits;

    #[test]
    fn a_message_too_long_or_one_too_many_in_an_hour_is_turned_away() {
        let mut app = App::new(None);
        assert_eq!(
            app.intake_refusal(Kind::Telegram, "42", &"x".repeat(9000), 0),
            None,
            "no limits yet"
        );
        app.set_channel_limits(Limits {
            length: 10,
            per_hour: 2,
        });
        assert!(
            app.intake_refusal(Kind::Telegram, "42", "far too long a message", 0)
                .is_some()
        );
        assert_eq!(app.intake_refusal(Kind::Telegram, "42", "one", 0), None);
        assert_eq!(app.intake_refusal(Kind::Telegram, "42", "two", 10), None);
        assert!(
            app.intake_refusal(Kind::Telegram, "42", "three", 20)
                .is_some()
        );
        assert_eq!(
            app.intake_refusal(Kind::Telegram, "7", "hi there", 20),
            None
        );
        assert_eq!(
            app.intake_refusal(Kind::Telegram, "42", "later", HOUR + 1),
            None,
            "an hour on"
        );
    }

    #[test]
    fn topics_are_read_from_the_form_and_listed_in_words() {
        assert_eq!(
            topics(" billing, refunds\n\n,orders "),
            ["billing", "refunds", "orders"]
        );
        assert_eq!(
            list(&topics("billing, refunds, orders")),
            "billing, refunds and orders"
        );
        assert_eq!(list(&topics("billing")), "billing");
    }
}
