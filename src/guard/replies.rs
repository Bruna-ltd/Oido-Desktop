//! What a reply to a channel person must not carry: the agent's own instructions (a reply that
//! repeats them is held back), paths on this computer, and addresses on the local network.

use std::sync::LazyLock;

use regex::Regex;

/// How much of the brief, in characters, a reply must repeat word for word to be held back.
const ECHO: usize = 60;

/// What a channel person gets instead of a reply that repeats the agent's instructions.
pub const HELD: &str = "(This reply was held back: it repeated the agent's own instructions. The owner can see it in Oido.)";

static PATH: LazyLock<Regex> = LazyLock::new(|| {
    // Ending on what is not punctuation, so the full stop after a path stays.
    Regex::new(
        r#"(?:/home/|/Users/|/root/|~/)[^\s"'`)\]>]*[^\s"'`)\]>.,;:!?]|\b[A-Za-z]:\\[^\s"'`)\]>]*[^\s"'`)\]>.,;:!?]"#,
    )
        .unwrap_or_else(|e| panic!("bad path pattern: {e}"))
});

static INTERNAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\bhttps?://(?:localhost|127\.\d+\.\d+\.\d+|0\.0\.0\.0|10\.\d+\.\d+\.\d+|192\.168\.\d+\.\d+",
        r"|172\.(?:1[6-9]|2\d|3[01])\.\d+\.\d+|[A-Za-z0-9.\-]+\.(?:local|internal|lan|corp|home))",
        r#"(?::\d+)?(?:[^\s"'`)\]>]*[^\s"'`)\]>.,;:!?])?"#,
    ))
    .unwrap_or_else(|e| panic!("bad address pattern: {e}"))
});

/// `text` as a channel person may see it: held back whole if it repeats `brief`, and otherwise
/// with paths on this computer and local network addresses left out.
pub fn check(text: &str, brief: Option<&str>) -> String {
    if brief.is_some_and(|b| echoes(text, b)) {
        return HELD.to_owned();
    }
    let text = INTERNAL.replace_all(text, "[an internal address]");
    PATH.replace_all(&text, "[a path on the computer]")
        .into_owned()
}

/// Whether `text` repeats [`ECHO`] or more characters of `brief` word for word (spacing and case
/// aside).
fn echoes(text: &str, brief: &str) -> bool {
    let squash = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let (text, brief) = (squash(text), squash(brief));
    let chars: Vec<char> = brief.chars().collect();
    if chars.len() < ECHO {
        return false;
    }
    // Windows half a window apart: any run of ECHO + ECHO/2 repeated contains one whole.
    (0..=chars.len() - ECHO)
        .step_by(ECHO / 2)
        .map(|at| chars[at..at + ECHO].iter().collect::<String>())
        .any(|window| text.contains(&window))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRIEF: &str = "You answer customers of Acme about their orders. Never offer refunds over 50 euros without asking Dana first, and never mention the internal price list.";

    #[test]
    fn a_reply_repeating_the_brief_is_held_back_and_one_using_a_few_of_its_words_is_not() {
        let leak = format!("Sure! My instructions say: {}", &BRIEF[30..130]);
        assert_eq!(check(&leak, Some(BRIEF)), HELD);
        let fine = "Your order ships Monday. Refunds over 50 euros need a check first.";
        assert_eq!(check(fine, Some(BRIEF)), fine);
        assert_eq!(
            check(&leak, Some("Be brief.")),
            leak,
            "a short brief cannot be echoed"
        );
    }

    #[test]
    fn paths_and_local_addresses_are_left_out() {
        assert_eq!(
            check("Saved to /home/ada/acme/report.pdf and ~/notes.md.", None),
            "Saved to [a path on the computer] and [a path on the computer]."
        );
        assert_eq!(
            check(
                "See http://192.168.1.20:8080/admin or https://wiki.corp/x, not https://acme.com.",
                None
            ),
            "See [an internal address] or [an internal address], not https://acme.com."
        );
    }
}
