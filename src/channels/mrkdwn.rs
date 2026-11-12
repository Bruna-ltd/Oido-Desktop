//! Markdown in the shape Slack and WhatsApp share: `*bold*`, `_italic_`, `~struck~`, `` `code` ``,
//! fenced code and `>` quotes. Headings turn bold, lists get bullets, tables go as fixed-width
//! text. Each app escapes text and writes links its own way (its [`Dialect`]). A reply comes out
//! in pieces of at most a limit each, cut between blocks, and a block too long on its own between
//! lines (fenced again if it was code).

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// What one app writes its own way.
pub struct Dialect {
    /// Text as the app reads it.
    pub escape: fn(&str) -> String,
    /// A link: its words, and where it goes.
    pub link: fn(&str, &str) -> String,
}

/// `markdown` in pieces of at most `limit` characters each, a blank line between blocks.
pub fn render(markdown: &str, dialect: &Dialect, limit: usize) -> Vec<String> {
    pack(blocks(markdown, dialect), limit)
}

/// One top-level block, and whether it is code (fixed-width when cut).
struct Block {
    text: String,
    code: bool,
}

fn blocks(markdown: &str, dialect: &Dialect) -> Vec<Block> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut out = Vec::new();
    // What is being written, innermost last: a quote collects its own text, then prefixes it.
    let mut bufs: Vec<String> = vec![String::new()];
    let mut depth = 0usize;
    let mut code = false;
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut links: Vec<String> = Vec::new();
    let mut cell = 0usize;
    let push = |bufs: &mut Vec<String>, s: &str| {
        if let Some(b) = bufs.last_mut() {
            b.push_str(s);
        }
    };
    let trim_end = |bufs: &mut Vec<String>| {
        if let Some(b) = bufs.last_mut() {
            while b.ends_with('\n') {
                b.pop();
            }
        }
    };
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => {
                depth += 1;
                match tag {
                    Tag::Heading { .. } | Tag::Strong => push(&mut bufs, "*"),
                    Tag::Emphasis => push(&mut bufs, "_"),
                    Tag::Strikethrough => push(&mut bufs, "~"),
                    Tag::BlockQuote(_) => bufs.push(String::new()),
                    Tag::CodeBlock(_) => {
                        code = depth == 1;
                        push(&mut bufs, "```\n");
                    }
                    Tag::List(start) => {
                        if !lists.is_empty() {
                            push(&mut bufs, "\n");
                        }
                        lists.push(start);
                    }
                    Tag::Item => {
                        let indent = "    ".repeat(lists.len().saturating_sub(1));
                        let mark = match lists.last_mut() {
                            Some(Some(n)) => {
                                let mark = format!("{n}. ");
                                *n += 1;
                                mark
                            }
                            _ => "• ".to_owned(),
                        };
                        push(&mut bufs, &format!("{indent}{mark}"));
                    }
                    Tag::Link { dest_url, .. } => {
                        links.push(dest_url.into_string());
                        bufs.push(String::new());
                    }
                    Tag::Image { dest_url, .. } => {
                        links.push(dest_url.into_string());
                        bufs.push("🖼 ".to_owned());
                    }
                    Tag::Table(_) => {
                        code = depth == 1;
                        push(&mut bufs, "```\n");
                    }
                    Tag::TableRow | Tag::TableHead => cell = 0,
                    Tag::TableCell => {
                        if cell > 0 {
                            push(&mut bufs, " | ");
                        }
                        cell += 1;
                    }
                    _ => {}
                }
            }
            Event::End(tag) => {
                depth = depth.saturating_sub(1);
                match tag {
                    TagEnd::Heading(_) | TagEnd::Strong => push(&mut bufs, "*"),
                    TagEnd::Emphasis => push(&mut bufs, "_"),
                    TagEnd::Strikethrough => push(&mut bufs, "~"),
                    TagEnd::BlockQuote(_) => {
                        trim_end(&mut bufs);
                        let inner = bufs.pop().unwrap_or_default();
                        let quoted: Vec<String> = inner.lines().map(|l| format!("> {l}")).collect();
                        push(&mut bufs, &quoted.join("\n"));
                        if depth > 0 {
                            push(&mut bufs, "\n");
                        }
                    }
                    TagEnd::CodeBlock => {
                        trim_end(&mut bufs);
                        push(&mut bufs, "\n```");
                    }
                    TagEnd::Paragraph if depth > 0 => push(&mut bufs, "\n"),
                    TagEnd::Item => push(&mut bufs, "\n"),
                    TagEnd::List(_) => {
                        lists.pop();
                        trim_end(&mut bufs);
                    }
                    TagEnd::Link | TagEnd::Image => {
                        let words = bufs.pop().unwrap_or_default();
                        let url = links.pop().unwrap_or_default();
                        push(&mut bufs, &(dialect.link)(&words, &url));
                    }
                    TagEnd::Table => {
                        trim_end(&mut bufs);
                        push(&mut bufs, "\n```");
                    }
                    TagEnd::TableHead | TagEnd::TableRow => push(&mut bufs, "\n"),
                    _ => {}
                }
                if depth == 0 && bufs.len() == 1 {
                    let text = std::mem::take(&mut bufs[0]);
                    if !text.trim().is_empty() {
                        out.push(Block { text, code });
                    }
                    code = false;
                }
            }
            Event::Text(t) => push(&mut bufs, &(dialect.escape)(&t)),
            Event::Code(t) => push(&mut bufs, &format!("`{}`", (dialect.escape)(&t))),
            Event::Html(t) | Event::InlineHtml(t) => push(&mut bufs, &(dialect.escape)(&t)),
            Event::SoftBreak | Event::HardBreak => push(&mut bufs, "\n"),
            Event::Rule => {
                push(&mut bufs, "———");
                if depth == 0 {
                    let text = std::mem::take(&mut bufs[0]);
                    out.push(Block { text, code: false });
                }
            }
            Event::TaskListMarker(done) => push(&mut bufs, if done { "☑ " } else { "☐ " }),
            Event::FootnoteReference(t) => push(&mut bufs, &format!("[{t}]")),
            Event::InlineMath(t) | Event::DisplayMath(t) => push(&mut bufs, &(dialect.escape)(&t)),
        }
    }
    let rest = bufs.concat();
    if !rest.trim().is_empty() {
        out.push(Block { text: rest, code });
    }
    out
}

/// Puts blocks into sections of at most `limit`, a blank line between blocks. A block too long on
/// its own is cut between lines (within a line if one line is too long), fenced again if it was
/// code.
fn pack(blocks: Vec<Block>, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut section = String::new();
    for block in blocks {
        let pieces: Vec<String> = if block.text.chars().count() <= limit {
            vec![block.text]
        } else if block.code {
            let inner = block
                .text
                .trim_start_matches("```\n")
                .trim_end_matches("\n```");
            cut(inner, limit - 8)
                .into_iter()
                .map(|p| format!("```\n{p}\n```"))
                .collect()
        } else {
            cut(&block.text, limit)
        };
        for p in pieces {
            let joined = if section.is_empty() { 0 } else { 2 };
            if section.chars().count() + joined + p.chars().count() > limit {
                out.push(std::mem::take(&mut section));
            }
            if !section.is_empty() {
                section.push_str("\n\n");
            }
            section.push_str(&p);
        }
    }
    if !section.is_empty() {
        out.push(section);
    }
    out
}

/// `text` in pieces of at most `limit` characters, cut between lines where it can.
fn cut(text: &str, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut piece = String::new();
    for line in text.split('\n') {
        let fits = piece.chars().count() + 1 + line.chars().count() <= limit;
        if fits || piece.is_empty() && line.chars().count() <= limit {
            if !piece.is_empty() {
                piece.push('\n');
            }
            piece.push_str(line);
            continue;
        }
        if !piece.is_empty() {
            out.push(std::mem::take(&mut piece));
        }
        let chars: Vec<char> = line.chars().collect();
        let mut chunks = chars.chunks(limit).peekable();
        while let Some(chunk) = chunks.next() {
            let s: String = chunk.iter().collect();
            if chunks.peek().is_some() {
                out.push(s);
            } else {
                piece = s;
            }
        }
    }
    if !piece.is_empty() {
        out.push(piece);
    }
    out
}
