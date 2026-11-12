//! A reply's Markdown as WhatsApp writes it: the shape it shares with Slack (see [`mrkdwn`]),
//! nothing escaped, and links as their words with the address after them. A reply goes as
//! messages of at most [`MAX_TEXT`] characters.

use crate::channels::mrkdwn::{self, Dialect};

/// The most characters in one message. WhatsApp takes far more, but folds a long one away
/// behind "Read more"; replies are cut between blocks well before that.
const MAX_TEXT: usize = 4000;

const WHATSAPP: Dialect = Dialect {
    escape: str::to_owned,
    link: |words, url| {
        let words = words.trim();
        if words.is_empty() || words == url {
            url.to_owned()
        } else {
            format!("{words} ({url})")
        }
    },
};

/// The reply as messages.
pub fn render(markdown: &str) -> Vec<String> {
    mrkdwn::render(markdown, &WHATSAPP, MAX_TEXT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(md: &str) -> String {
        let messages = render(md);
        assert_eq!(messages.len(), 1, "{messages:?}");
        messages[0].clone()
    }

    #[test]
    fn markdown_becomes_whatsapp_markup_with_nothing_escaped() {
        assert_eq!(
            one(
                "# Plan\n\nUse **bold**, *it*, ~~old~~ and `a<b>`.\n\nSee [the docs](https://x.dev/?a=1&b=2)."
            ),
            "*Plan*\n\nUse *bold*, _it_, ~old~ and `a<b>`.\n\nSee the docs (https://x.dev/?a=1&b=2)."
        );
        assert_eq!(one("<https://x.dev>"), "https://x.dev");
        assert_eq!(one("1 < 2 & 3"), "1 < 2 & 3");
        assert_eq!(one("```rust\nfn main() {}\n```"), "```\nfn main() {}\n```");
    }

    #[test]
    fn a_long_reply_goes_as_several_messages() {
        let para = "word ".repeat(500);
        let messages = render(&[para.trim(); 4].join("\n\n"));
        assert!(messages.len() > 1);
        assert!(messages.iter().all(|m| m.chars().count() <= MAX_TEXT));
    }
}
