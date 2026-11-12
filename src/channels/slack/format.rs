//! A reply's Markdown as Slack reads it: mrkdwn (see [`mrkdwn`]) with `&`, `<` and `>` escaped and
//! links as `<url|text>`. A message is a list of section blocks of at most [`SECTION`] characters
//! each, at most [`SECTIONS`] to a message.

use crate::channels::mrkdwn::{self, Dialect};

/// The most characters a section's text holds (Slack takes 3000; room is left for what wraps a
/// cut block).
const SECTION: usize = 2900;
/// The most sections in one message (Slack takes 50 blocks; room is left for buttons).
const SECTIONS: usize = 45;

const SLACK: Dialect = Dialect {
    escape,
    link: |words, url| {
        // `|` and `>` would end the link early.
        let words = words.replace('|', "¦").replace('>', "&gt;");
        format!("<{url}|{words}>")
    },
};

/// The reply as messages, each a list of section texts.
pub fn render(markdown: &str) -> Vec<Vec<String>> {
    let sections = mrkdwn::render(markdown, &SLACK, SECTION);
    sections.chunks(SECTIONS).map(<[String]>::to_vec).collect()
}

/// Escapes text for mrkdwn.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(md: &str) -> String {
        let messages = render(md);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].len(), 1, "{messages:?}");
        messages[0][0].clone()
    }

    #[test]
    fn markdown_becomes_mrkdwn_with_text_escaped() {
        assert_eq!(
            one(
                "# Plan\n\nUse **bold**, *it*, ~~old~~ and `a<b>`.\n\nSee [the docs](https://x.dev/?a=1&b=2)."
            ),
            "*Plan*\n\nUse *bold*, _it_, ~old~ and `a&lt;b&gt;`.\n\nSee <https://x.dev/?a=1&b=2|the docs>."
        );
        assert_eq!(one("```rust\nfn main() {}\n```"), "```\nfn main() {}\n```");
        assert_eq!(one("1 < 2 & <script>"), "1 &lt; 2 &amp; &lt;script&gt;");
    }

    #[test]
    fn lists_quotes_and_tables_keep_their_shape() {
        assert_eq!(
            one("- one\n- two\n  1. a\n- [x] done"),
            "• one\n• two\n    1. a\n• ☑ done"
        );
        assert_eq!(one("> said\n> twice"), "> said\n> twice");
        assert_eq!(
            one("| a | b |\n|---|---|\n| 1 | 2 |"),
            "```\na | b\n1 | 2\n```"
        );
    }

    #[test]
    fn a_long_reply_is_cut_into_sections_that_fit_and_code_stays_fenced() {
        let para = "word ".repeat(400);
        let md = [para.trim(); 4].join("\n\n");
        let sections = &render(&md)[0];
        assert!(sections.len() > 1);
        assert!(sections.iter().all(|s| s.chars().count() <= SECTION));
        let code = (0..1500)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let sections = &render(&format!("```\n{code}\n```"))[0];
        assert!(sections.len() > 1);
        for s in sections {
            assert!(s.chars().count() <= 3000);
            assert!(s.starts_with("```\n") && s.ends_with("\n```"), "{s}");
        }
        let one_line = "x".repeat(7000);
        let sections = &render(&one_line)[0];
        assert_eq!(sections.concat(), one_line);
    }
}
