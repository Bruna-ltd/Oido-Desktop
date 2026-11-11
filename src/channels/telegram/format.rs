//! A reply's Markdown as Telegram reads it: its HTML subset (`b`, `i`, `s`, `code`, `pre`, `a`,
//! `blockquote`), which needs only `<`, `>` and `&` escaped. Headings turn bold, lists get
//! bullets, tables go as fixed-width text. Each message holds at most [`MAX_TEXT`] characters, so
//! a long reply is cut between blocks, and a block too long on its own between lines.
//!
//! The same words go unformatted alongside, part for part, for when Telegram refuses the HTML.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use super::api::MAX_TEXT;

/// Room left in each part for what wraps a cut block.
const SLACK: usize = 32;

/// A reply as message parts: HTML ones and the same unformatted, as many of each.
#[derive(Debug, PartialEq)]
pub struct Parts {
    pub html: Vec<String>,
    pub plain: Vec<String>,
}

/// One top-level block, written both ways.
#[derive(Default)]
struct Block {
    html: String,
    plain: String,
    code: bool,
}

pub fn render(markdown: &str) -> Parts {
    pack(blocks(markdown), MAX_TEXT)
}

/// Escapes text for Telegram's HTML.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// What Telegram counts: UTF-16 units.
fn size(s: &str) -> usize {
    s.encode_utf16().count()
}

fn blocks(markdown: &str) -> Vec<Block> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut out = Vec::new();
    let mut b = Block::default();
    let mut depth = 0usize;
    // Each open list: the next number, or `None` for bullets.
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut links: Vec<String> = Vec::new();
    let mut in_table_head = false;
    let mut cell = 0usize;
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => {
                depth += 1;
                match tag {
                    Tag::Heading { .. } | Tag::Strong => b.html.push_str("<b>"),
                    Tag::Emphasis => b.html.push_str("<i>"),
                    Tag::Strikethrough => b.html.push_str("<s>"),
                    Tag::BlockQuote(_) => b.html.push_str("<blockquote>"),
                    Tag::CodeBlock(kind) => {
                        b.code = depth == 1;
                        match kind {
                            CodeBlockKind::Fenced(lang) if !lang.trim().is_empty() => {
                                let lang = lang.split_whitespace().next().unwrap_or_default();
                                b.html.push_str(&format!(
                                    "<pre><code class=\"language-{}\">",
                                    escape(lang)
                                ));
                            }
                            _ => b.html.push_str("<pre><code>"),
                        }
                    }
                    Tag::List(start) => {
                        if !lists.is_empty() {
                            newline(&mut b);
                        }
                        lists.push(start);
                    }
                    Tag::Item => {
                        let indent = "  ".repeat(lists.len().saturating_sub(1));
                        let mark = match lists.last_mut() {
                            Some(Some(n)) => {
                                let mark = format!("{n}. ");
                                *n += 1;
                                mark
                            }
                            _ => "• ".to_owned(),
                        };
                        push_both(&mut b, &format!("{indent}{mark}"));
                    }
                    Tag::Link { dest_url, .. } => {
                        b.html
                            .push_str(&format!("<a href=\"{}\">", escape(&dest_url)));
                        links.push(dest_url.into_string());
                    }
                    Tag::Image { dest_url, .. } => {
                        b.html
                            .push_str(&format!("<a href=\"{}\">🖼 ", escape(&dest_url)));
                        b.plain.push_str("🖼 ");
                        links.push(dest_url.into_string());
                    }
                    Tag::Table(_) => b.html.push_str("<pre>"),
                    Tag::TableHead => in_table_head = true,
                    Tag::TableRow => cell = 0,
                    Tag::TableCell => {
                        if cell > 0 {
                            push_both(&mut b, " | ");
                        }
                        cell += 1;
                    }
                    _ => {}
                }
            }
            Event::End(tag) => {
                depth = depth.saturating_sub(1);
                match tag {
                    TagEnd::Heading(_) | TagEnd::Strong => b.html.push_str("</b>"),
                    TagEnd::Emphasis => b.html.push_str("</i>"),
                    TagEnd::Strikethrough => b.html.push_str("</s>"),
                    TagEnd::BlockQuote(_) => {
                        trim_end(&mut b);
                        b.html.push_str("</blockquote>");
                    }
                    TagEnd::CodeBlock => {
                        trim_end(&mut b);
                        b.html.push_str("</code></pre>");
                    }
                    TagEnd::Paragraph => {
                        if depth > 0 {
                            newline(&mut b);
                        }
                    }
                    TagEnd::Item => newline(&mut b),
                    TagEnd::List(_) => {
                        lists.pop();
                        trim_end(&mut b);
                    }
                    TagEnd::Link | TagEnd::Image => {
                        b.html.push_str("</a>");
                        if let Some(url) = links.pop() {
                            b.plain.push_str(&format!(" ({url})"));
                        }
                    }
                    TagEnd::Table => {
                        trim_end(&mut b);
                        b.html.push_str("</pre>");
                    }
                    TagEnd::TableHead => {
                        in_table_head = false;
                        newline(&mut b);
                    }
                    TagEnd::TableRow => newline(&mut b),
                    _ => {}
                }
                if depth == 0 {
                    let done = std::mem::take(&mut b);
                    if !done.plain.trim().is_empty() {
                        out.push(done);
                    }
                }
            }
            Event::Text(t) => push_both(&mut b, &t),
            Event::Code(t) => {
                b.html.push_str(&format!("<code>{}</code>", escape(&t)));
                b.plain.push_str(&t);
            }
            Event::Html(t) | Event::InlineHtml(t) => push_both(&mut b, &t),
            Event::SoftBreak => push_both(&mut b, if in_table_head { " " } else { "\n" }),
            Event::HardBreak => newline(&mut b),
            Event::Rule => {
                push_both(&mut b, "———");
                if depth == 0 {
                    out.push(std::mem::take(&mut b));
                }
            }
            Event::TaskListMarker(done) => push_both(&mut b, if done { "☑ " } else { "☐ " }),
            Event::FootnoteReference(t) => push_both(&mut b, &format!("[{t}]")),
            Event::InlineMath(t) | Event::DisplayMath(t) => push_both(&mut b, &t),
        }
    }
    if !b.plain.trim().is_empty() {
        out.push(b);
    }
    out
}

/// Adds `text` to both forms, escaped in the HTML one.
fn push_both(b: &mut Block, text: &str) {
    b.html.push_str(&escape(text));
    b.plain.push_str(text);
}

fn newline(b: &mut Block) {
    b.html.push('\n');
    b.plain.push('\n');
}

/// Drops trailing line breaks before a closing tag.
fn trim_end(b: &mut Block) {
    while b.html.ends_with('\n') {
        b.html.pop();
    }
    while b.plain.ends_with('\n') {
        b.plain.pop();
    }
}

/// Puts blocks into parts of at most `limit`, a blank line between blocks. A block too long on
/// its own is cut between lines (within a line if one line is too long), each piece plain text,
/// or fixed-width if it was code.
fn pack(blocks: Vec<Block>, limit: usize) -> Parts {
    let mut parts = Parts {
        html: Vec::new(),
        plain: Vec::new(),
    };
    let (mut html, mut plain) = (String::new(), String::new());
    let flush = |html: &mut String, plain: &mut String, parts: &mut Parts| {
        if !html.is_empty() {
            parts.html.push(std::mem::take(html));
            parts.plain.push(std::mem::take(plain));
        }
    };
    for block in blocks {
        let pieces: Vec<(String, String)> = if size(&block.html) <= limit {
            vec![(block.html, block.plain)]
        } else {
            cut(&block.plain, limit - SLACK)
                .into_iter()
                .map(|piece| {
                    let html = if block.code {
                        format!("<pre>{}</pre>", escape(&piece))
                    } else {
                        escape(&piece)
                    };
                    (html, piece)
                })
                .collect()
        };
        for (h, p) in pieces {
            let joined = if html.is_empty() { 0 } else { 2 };
            if size(&html) + joined + size(&h) > limit {
                flush(&mut html, &mut plain, &mut parts);
            }
            if !html.is_empty() {
                html.push_str("\n\n");
                plain.push_str("\n\n");
            }
            html.push_str(&h);
            plain.push_str(&p);
        }
    }
    flush(&mut html, &mut plain, &mut parts);
    parts
}

/// `text` in pieces whose escaped form fits `limit`, cut between lines where it can.
fn cut(text: &str, limit: usize) -> Vec<String> {
    let fits = |s: &str| size(&escape(s)) <= limit;
    let mut out = Vec::new();
    let mut piece = String::new();
    for line in text.split('\n') {
        let joined = if piece.is_empty() {
            line.to_owned()
        } else {
            format!("{piece}\n{line}")
        };
        if fits(&joined) {
            piece = joined;
            continue;
        }
        if !piece.is_empty() {
            out.push(std::mem::take(&mut piece));
        }
        // One line too long on its own: cut it by characters.
        let mut rest = line;
        while !fits(rest) {
            let mut end = 0;
            for (i, c) in rest.char_indices() {
                if !fits(&rest[..i + c.len_utf8()]) {
                    break;
                }
                end = i + c.len_utf8();
            }
            out.push(rest[..end].to_owned());
            rest = &rest[end..];
        }
        piece = rest.to_owned();
    }
    if !piece.is_empty() {
        out.push(piece);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(md: &str) -> String {
        let parts = render(md);
        assert_eq!(parts.html.len(), 1, "{parts:?}");
        parts.html[0].clone()
    }

    #[test]
    fn markdown_becomes_telegrams_html_with_text_escaped() {
        assert_eq!(
            one(
                "# Plan\n\nUse **bold**, *it*, ~~old~~ and `a<b>`.\n\nSee [docs](https://x.dev/?a=1&b=2)."
            ),
            "<b>Plan</b>\n\nUse <b>bold</b>, <i>it</i>, <s>old</s> and <code>a&lt;b&gt;</code>.\n\nSee <a href=\"https://x.dev/?a=1&amp;b=2\">docs</a>."
        );
        assert_eq!(
            one("```rust\nfn main() {}\n```"),
            "<pre><code class=\"language-rust\">fn main() {}</code></pre>"
        );
        assert_eq!(one("1 < 2 & <script>"), "1 &lt; 2 &amp; &lt;script&gt;");
    }

    #[test]
    fn lists_get_bullets_and_numbers_and_nest() {
        assert_eq!(
            one("- one\n- two\n  1. a\n  2. b\n- [x] done"),
            "• one\n• two\n  1. a\n  2. b\n• ☑ done"
        );
    }

    #[test]
    fn tables_and_quotes_keep_their_shape() {
        assert_eq!(
            one("| a | b |\n|---|---|\n| 1 | 2 |"),
            "<pre>a | b\n1 | 2</pre>"
        );
        assert_eq!(
            one("> said\n> twice"),
            "<blockquote>said\ntwice</blockquote>"
        );
    }

    #[test]
    fn the_plain_form_keeps_the_words_and_links() {
        let parts = render("**Hi** [there](https://a.b)");
        assert_eq!(parts.plain, vec!["Hi there (https://a.b)".to_owned()]);
    }

    #[test]
    fn a_long_reply_is_cut_between_blocks_and_every_part_fits() {
        let para = "word ".repeat(300);
        let md = [para.trim(); 6].join("\n\n");
        let parts = render(&md);
        assert!(parts.html.len() > 1);
        assert_eq!(parts.html.len(), parts.plain.len());
        for p in &parts.html {
            assert!(size(p) <= MAX_TEXT, "{}", size(p));
            assert!(!p.starts_with('\n') && !p.ends_with('\n'));
        }
    }

    #[test]
    fn a_block_too_long_on_its_own_is_cut_between_lines_and_stays_code() {
        let code = (0..2000)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let parts = render(&format!("```\n{code}\n```"));
        assert!(parts.html.len() > 1);
        for p in &parts.html {
            assert!(size(p) <= MAX_TEXT);
            assert!(p.starts_with("<pre>") && p.ends_with("</pre>"), "{p}");
        }
        let one_line = "x".repeat(10_000);
        let parts = render(&one_line);
        assert!(parts.html.iter().all(|p| size(p) <= MAX_TEXT));
        assert_eq!(parts.plain.concat(), one_line);
    }
}
