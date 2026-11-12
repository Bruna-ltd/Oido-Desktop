//! The conversation: transcript with tool calls and approval gates, and the composer below it.

use std::borrow::Cow;

use rune::{El, GrowthSpec, ScrollPos, ScrollSpec, TextStyle, col, el, row, text};
use rune_kit::{
    Badge, BadgeVariant, Button, ButtonVariant, Card, Collapsible, ControlSize, Dialog, IconButton,
    IconButtonVariant, Kit, Markdown, MarkdownStyle, glyphs,
};

use super::picture::Web;
use crate::agents::Agent;
use crate::app::App;
use crate::model::{Approval, Decision, Message, Outcome, Stats, ToolCall};

const COLUMN: f32 = 760.0;
/// How far above the newest message, in points, the transcript is scrolled before the jump button
/// shows.
const SCROLLED_UP: f32 = 200.0;
/// A message's height before it has been seen, in points: about a short reply.
const ROW_ESTIMATE: f32 = 140.0;

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let Some(agent) = s.current() else {
        return no_agents(k);
    };
    viewer(s, k, layer);
    col()
        .key("chat")
        .w_full()
        .h_full()
        // Files dropped anywhere on the chat are attached to the message being written.
        .on_drop_files(|s: &mut App, files, _| s.attach_paths(files.to_vec()))
        .child(super::chat_header::build(s, agent, k))
        .child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .items_center()
                .child(transcript(s, agent, k))
                .child(
                    col()
                        .w_full()
                        .max_w(COLUMN)
                        .px(24.0)
                        .pb(20.0)
                        .children(s.scrolled_up.then(|| jump_to_latest(k)))
                        .child(super::composer::build(s, agent, k, layer)),
                ),
        )
}

/// A round button over the top of the composer, while you have scrolled up from the newest
/// message: it glides back down.
fn jump_to_latest(k: &Kit) -> El {
    row()
        .key("jump-to-latest")
        .pin(Some(0.0), Some(-48.0), Some(0.0), None)
        .justify_center()
        .child(
            el().radius(18.0)
                .fill(k.c.layer_01)
                .border(1.0, k.c.border_base)
                .child(
                    IconButton::new(&glyphs::CHEVRON_DOWN)
                        .variant(IconButtonVariant::GhostMuted)
                        .label("Jump to the latest message")
                        .help("Jump to the latest message")
                        .on_press_tx(|_: &mut App, tx| tx.scroll_to_bottom("transcript"))
                        .build(k),
                ),
        )
}

fn transcript(s: &App, agent: &Agent, k: &Kit) -> El {
    let body = if agent.chat().messages.is_empty() {
        empty(agent, k)
    } else {
        rows(s, agent, k)
    };
    col()
        .key("transcript")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .items_center()
        .scroll(ScrollSpec { stick_bottom: true })
        .on_scroll_offset({
            let up = s.scrolled_up;
            move |at: ScrollPos| {
                let away = at.max[1] - at.offset[1] > SCROLLED_UP;
                // This runs as the frame is drawn and cannot change state; it asks for the change.
                if away != up
                    && let Some(app) = crate::app::handle()
                {
                    app.update(move |s: &mut App, _| s.scrolled_up = away);
                }
            }
        })
        .child(body)
}

/// The messages, then whatever waits under them, as rows that exist only near the window. A long
/// chat then costs what its visible part costs, in layout and in what is drawn each frame.
fn rows(s: &App, agent: &Agent, k: &Kit) -> El {
    let chat = agent.chat();
    let messages = chat.messages.len();
    // Cards for what the agent is asking, drafting or holding. A few at most, usually none.
    let tail: Vec<El> = draft(s, &chat.id, k)
        .into_iter()
        .chain(schedule_drafts(s, &chat.id, k))
        .chain(mcp_drafts(s, &chat.id, k))
        .chain(agent_drafts(s, &chat.id, k))
        .chain(super::guard::waiting(
            s,
            &crate::guard::ask::Where::Chat(chat.id.clone()),
            k,
        ))
        .collect();
    let kit = k.clone();
    col()
        .key("rows")
        .w_full()
        .max_w(COLUMN)
        .px(24.0)
        .py(24.0)
        .gap(20.0)
        .growth(GrowthSpec::default())
        .virtual_rows(
            messages + tail.len(),
            ROW_ESTIMATE,
            move |_: &App, i| {
                if i < messages {
                    Cow::Owned(format!("m{i}"))
                } else {
                    Cow::Owned(format!("tail{}", i - messages))
                }
            },
            move |s: &App, i| match i.checked_sub(messages) {
                Some(j) => tail.get(j).cloned().unwrap_or_else(el),
                None => s
                    .current()
                    .and_then(|a| a.chat().messages.get(i).map(|m| (a, m)))
                    .map_or_else(el, |(a, m)| {
                        message(s, m, i, i + 1 == a.chat().messages.len(), &kit)
                    }),
            },
        )
}

fn empty(a: &Agent, k: &Kit) -> El {
    col()
        .w_full()
        .h_full()
        .min_h(360.0)
        .center()
        .gap(6.0)
        .child(
            text(a.name.clone())
                .font(k.ty.body_medium)
                .color(k.c.text_base),
        )
        .child(
            text("Say something to get started.")
                .font(k.ty.small)
                .color(k.c.text_muted),
        )
}

/// Every agent is deleted: the one way forward is to make another.
fn no_agents(k: &Kit) -> El {
    col()
        .key("no-agents")
        .w_full()
        .h_full()
        .center()
        .gap(12.0)
        .child(
            text("No agents yet")
                .font(k.ty.body_medium)
                .color(k.c.text_base),
        )
        .child(
            text("An agent is a name and a brief. Make one to start working.")
                .font(k.ty.small)
                .color(k.c.text_muted),
        )
        .child(
            Button::new("New agent")
                .variant(ButtonVariant::Contrast)
                .on_press(|s: &mut App| s.new_agent())
                .build(k)
                .key("new-agent"),
        )
}

fn message(s: &App, m: &Message, i: usize, last: bool, k: &Kit) -> El {
    match m {
        Message::User(p) => user(p, i, k),
        Message::Divider(label) => divider(label, i, k),
        Message::Summary { text, .. } => summary(s, text, i, k),
        Message::Assistant {
            text,
            thinking,
            thought_secs,
            tools,
            approval,
            stats,
            run,
            outcome,
        } => assistant(
            s,
            &Reply {
                body: text,
                thinking,
                thought: *thought_secs,
                tools,
                approval: approval.as_ref(),
                stats: stats.as_ref(),
                run: run.as_deref(),
                outcome,
                last,
            },
            i,
            k,
        ),
    }
}

fn user(p: &crate::prompt::Prompt, i: usize, k: &Kit) -> El {
    let mut bubble = col()
        .max_w(560.0)
        .px(14.0)
        .py(10.0)
        .gap(8.0)
        .radius(k.radius.lg)
        .fill(k.c.layer_02);
    let pictures: Vec<El> = p
        .images()
        .map(|img| {
            let (w, h) = super::picture::fit(img.width, img.height, 320.0, 240.0);
            let mut shown = el()
                .size(w, h)
                .radius(k.radius.md)
                .fill(k.c.layer_01)
                .help(format!("{} · {}×{}", img.name, img.width, img.height));
            if let Some(px) = super::picture::pixels(img) {
                shown = shown.pixels(&px);
            }
            shown
        })
        .collect();
    if !pictures.is_empty() {
        bubble = bubble.child(row().gap(6.0).wrap().justify_end().children(pictures));
    }
    let files: Vec<El> = p
        .attachments
        .iter()
        .filter(|a| !a.is_image())
        .map(|a| {
            row()
                .gap(4.0)
                .center_y()
                .child(rune::icon(&rune_kit::glyphs::FILE_TEXT, 12.0).color(k.c.text_muted))
                .child(
                    text(a.name())
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .max_w(240.0)
                        .truncate(),
                )
        })
        .collect();
    if !files.is_empty() {
        bubble = bubble.child(row().gap(6.0).wrap().children(files));
    }
    if !p.text.is_empty() {
        bubble = bubble.child(
            text(p.text.clone())
                .font(k.ty.body)
                .color(k.c.text_base)
                .wrap()
                .selectable(true),
        );
    }
    row()
        .key(format!("m{i}"))
        .w_full()
        .justify_end()
        .child(bubble)
}

/// An assistant message as the transcript shows it.
struct Reply<'a> {
    body: &'a str,
    /// What the model said while thinking; empty when it said nothing.
    thinking: &'a str,
    thought: Option<f32>,
    tools: &'a [ToolCall],
    approval: Option<&'a Approval>,
    stats: Option<&'a Stats>,
    run: Option<&'a str>,
    outcome: &'a Outcome,
    /// Only the newest reply can be retried.
    last: bool,
}

/// Reply text a step above the kit's body size; headings keep their lead over it.
fn reply_style(k: &Kit) -> MarkdownStyle {
    let body = TextStyle {
        size: 16.0,
        line_height: 25.6,
        ..k.ty.body
    };
    let heading = |size, lh| TextStyle {
        size,
        weight: 600.0,
        line_height: lh,
        ..body
    };
    MarkdownStyle::from_kit(k)
        .body_font(body)
        .heading_font(1, heading(20.0, 28.0))
        .heading_font(2, heading(18.0, 26.0))
        .heading_font(3, heading(16.0, 24.0))
        .heading_font(4, heading(16.0, 24.0))
}

/// Reply text as Markdown: math written out in Unicode, diagrams and charts drawn, links opened in
/// the browser, pictures from the agent's folder, and web pictures once you let them load (see
/// [`super::picture::markdown`]).
fn markdown(s: &App, body: &str, style: MarkdownStyle, k: &Kit) -> Markdown {
    let folder = s
        .current()
        .map(|a| s.workdir_of(&a.chat().id))
        .unwrap_or_default();
    let kit = k.clone();
    Markdown::new(super::math::unicode(body))
        .style(style)
        .on_link(|_: &mut App, url| crate::browser::open_link(url))
        .images(move |src| super::picture::markdown(src, &folder))
        .image_frame(move |shown, f| {
            if super::picture::is_web(f.src) && super::picture::web(f.src) != Web::Shown {
                return web_picture(f.src, f.alt, &kit);
            }
            let opened = (f.src.to_owned(), f.alt.to_owned());
            shown.on_press(move |s: &mut App| s.viewing = Some(opened.clone()))
        })
}

/// Largest a picture opened large draws, in points; smaller ones draw at their own size.
const VIEWER: (f32, f32) = (960.0, 600.0);

/// The picture you pressed in a reply, large, over the chat. Esc or a press outside closes it.
fn viewer(s: &App, k: &Kit, layer: &mut Vec<El>) {
    let Some((src, alt)) = &s.viewing else {
        return;
    };
    let folder = s
        .current()
        .map(|a| s.workdir_of(&a.chat().id))
        .unwrap_or_default();
    let Some(image) = super::picture::markdown(src, &folder) else {
        return;
    };
    let Some([w, h]) = image.size() else {
        return;
    };
    let (w, h) = super::picture::fit(w, h, VIEWER.0, VIEWER.1);
    let shown = el()
        .role("image")
        .label(if alt.is_empty() {
            "Picture"
        } else {
            alt.as_str()
        })
        .size(w, h)
        .radius(k.radius.md)
        .pixels(image.pixels());
    Dialog::new(
        "picture",
        if alt.is_empty() {
            "Picture"
        } else {
            alt.as_str()
        },
    )
    .open(true)
    .width(w.max(240.0) + 32.0)
    .fit()
    .child(
        row()
            .w_full()
            .px(16.0)
            .pb(16.0)
            .justify_center()
            .child(shown),
    )
    .on_close(|s: &mut App| s.viewing = None)
    .build(k, layer);
}

/// A web picture in a reply until it is shown: an offer to load it, naming the site it comes
/// from. Loading it tells that site you are reading the reply, and whatever the address
/// was written to carry.
fn web_picture(url: &str, alt: &str, k: &Kit) -> El {
    let state = super::picture::web(url);
    let host = url
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(rest))
        .unwrap_or(url);
    let what = if alt.is_empty() { "Picture" } else { alt };
    let (note, ink) = match &state {
        Web::Failed(why) => (format!("Could not load: {why}"), k.c.warning.icon),
        _ => (format!("From {host}"), k.c.text_faint),
    };
    let address = url.to_owned();
    let button = Button::new(match state {
        Web::Loading => "Loading…",
        Web::Failed(_) => "Try again",
        _ => "Load",
    })
    .variant(ButtonVariant::Outline)
    .control_size(ControlSize::Small)
    .disabled(state == Web::Loading)
    .on_press(move |s: &mut App| super::picture::load(s, &address))
    .build(k);
    row()
        .w_full()
        .px(12.0)
        .py(10.0)
        .gap(10.0)
        .center_y()
        .radius(k.radius.md)
        .border(1.0, k.c.border_muted)
        .help(url.to_owned())
        .child(rune::icon(&glyphs::FILE_IMAGE, 16.0).color(k.c.text_muted))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(what.to_owned())
                        .font(k.ty.body)
                        .color(k.c.text_base)
                        .truncate(),
                )
                .child(text(note).font(k.ty.small).color(ink).wrap()),
        )
        .child(button)
}

/// Thinking set a step back from the reply: smaller and muted.
fn thinking_style(k: &Kit) -> MarkdownStyle {
    let body = TextStyle {
        size: 14.0,
        line_height: 22.0,
        ..k.ty.body
    };
    MarkdownStyle::from_kit(k)
        .text(k.c.text_muted)
        .body_font(body)
}

/// What the model said while thinking, folded under one line. Open while it thinks, closed once
/// the reply starts, unless you opened or closed it yourself.
fn thinking(s: &App, r: &Reply, i: usize, k: &Kit) -> Option<El> {
    if r.thinking.is_empty() {
        return None;
    }
    let sid = s.current()?.chat().id.clone();
    let now = *r.outcome == Outcome::Working && r.body.is_empty();
    let title = if now {
        let waited = s
            .writing_secs()
            .map(|n| format!(" {}", super::flow::clock(std::time::Duration::from_secs(n))))
            .unwrap_or_default();
        format!("Thinking…{waited}")
    } else if let Some(secs) = r.thought {
        format!("Thought for {secs:.1}s")
    } else {
        "Thought".to_owned()
    };
    let open = s
        .thinking_open
        .get(&(sid.clone(), i))
        .copied()
        .unwrap_or(now);
    let mut c = Collapsible::new(title, open)
        .ghost()
        .on_toggle(move |s: &mut App| s.toggle_thinking(&sid, i, open));
    if open {
        c = c.child(
            row()
                .w_full()
                .gap(12.0)
                .child(el().w(2.0).h_full().radius(1.0).fill(k.c.border_muted))
                .child(
                    col().grow(1.0).min_w(0.0).child(
                        markdown(s, r.thinking, thinking_style(k), k)
                            .streaming(now)
                            .build(k)
                            .key("thinking"),
                    ),
                ),
        );
    }
    Some(c.build(k))
}

fn assistant(s: &App, r: &Reply, i: usize, k: &Kit) -> El {
    let c = &k.c;
    let mut out = col().key(format!("m{i}")).w_full().gap(10.0);
    match thinking(s, r, i, k) {
        Some(folded) => out = out.child(folded),
        None => {
            if let Some(secs) = r.thought {
                out = out.child(
                    text(format!("Thought for {secs:.1}s"))
                        .font(k.ty.small)
                        .color(c.text_faint),
                );
            }
        }
    }
    // Calls held for your go-ahead wait, all of them, until it is given.
    let held = r.tools.iter().any(|t| {
        t.output.is_none()
            && t.trip
                .as_ref()
                .is_some_and(|t| t.state == crate::guard::TripState::Asking)
    });
    // A guardrail's card goes right under the call it stopped.
    let sid = s.current().map(|a| a.chat().id.clone()).unwrap_or_default();
    let tripped = |j: usize, t: &ToolCall| super::guard::trip(s, &sid, t, j, k);
    // Other tools show what they did before the reply; a question comes after it, as its end.
    out = out.children(
        r.tools
            .iter()
            .enumerate()
            .filter(|(_, t)| !crate::tools::needs_you(&t.name))
            .flat_map(|(j, t)| {
                let writing = *r.outcome == Outcome::Working || held;
                let shown = if t.name == crate::tools::DELEGATE {
                    super::flow::delegation(s, t, writing, j, k)
                } else {
                    tool(t, writing, j, k)
                };
                std::iter::once(shown).chain(tripped(j, t))
            }),
    );
    if *r.outcome == Outcome::Working && r.body.is_empty() && r.thinking.is_empty() {
        let waited = s
            .writing_secs()
            .map(|n| format!(" {}", super::flow::clock(std::time::Duration::from_secs(n))))
            .unwrap_or_default();
        out = out.child(
            text(format!("Thinking…{waited}"))
                .font(k.ty.body)
                .color(c.text_faint),
        );
    } else if !r.body.is_empty() {
        out = out.child(
            markdown(s, r.body, reply_style(k), k)
                .streaming(*r.outcome == Outcome::Working)
                .build(k)
                .key("reply"),
        );
    }
    out = out.children(
        r.tools
            .iter()
            .enumerate()
            .filter(|(_, t)| crate::tools::needs_you(&t.name))
            .flat_map(|(j, t)| {
                let shown = if t.name == crate::tools::SHELL {
                    command_card(s, t, j, k)
                } else {
                    question(s, t, r.last, j, k)
                };
                std::iter::once(shown).chain(tripped(j, t))
            }),
    );
    if let Some(a) = r.approval {
        out = out.child(gate(a, k));
    }
    match r.outcome {
        Outcome::Stopped => out = out.child(ended("Stopped.", c.text_faint, r.last, k)),
        Outcome::Failed(why) => out = out.child(ended(why, k.c.warning.icon, r.last, k)),
        Outcome::Working | Outcome::Done => {}
    }
    let mut foot = row().w_full().center_y().gap(8.0);
    if let Some(st) = r.stats {
        foot = foot.child(
            text(format!(
                "{} · {} tokens · {:.1}s",
                st.model, st.tokens, st.seconds
            ))
            .font(k.ty.small)
            .color(c.text_faint),
        );
    }
    if let (Some(id), false) = (r.run, *r.outcome == Outcome::Working) {
        let id = id.to_owned();
        foot = foot.child(
            Button::new("View run")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.open_run(&id))
                .build(k)
                .key(format!("run{i}")),
        );
    }
    if r.stats.is_some() || (r.run.is_some() && *r.outcome != Outcome::Working) {
        out = out.child(foot);
    }
    out
}

/// How a reply ended when it did not finish, with a way to ask again.
fn ended(note: &str, ink: rune::Color, retry: bool, k: &Kit) -> El {
    let mut r = row().w_full().center_y().gap(8.0).child(
        text(note.to_owned())
            .font(k.ty.small)
            .color(ink)
            .wrap()
            .selectable(true),
    );
    if retry {
        r = r.child(
            Button::new("Retry")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.retry())
                .build(k)
                .key("retry"),
        );
    }
    r
}

/// A question the agent asked: its options and a box for your own answer while it waits, then
/// your answer once given.
fn question(s: &App, t: &ToolCall, last: bool, j: usize, k: &Kit) -> El {
    let c = &k.c;
    let q = crate::tools::question(&t.args);
    let card = |accent, body: El| {
        Card::new()
            .accent(accent)
            .title("Question", None)
            .description(q.text.clone())
            .child(body)
            .build(k)
            .key(format!("ask{j}"))
    };
    if let Some(answer) = &t.output {
        return card(
            k.c.success.icon,
            text(format!("You answered: {answer}"))
                .font(k.ty.small)
                .color(c.text_muted)
                .wrap()
                .selectable(true),
        );
    }
    if !last {
        return card(
            k.c.warning.icon,
            text("Not answered.").font(k.ty.small).color(c.text_faint),
        );
    }
    let mut body = col().w_full().gap(10.0);
    if !q.options.is_empty() {
        body = body.child(row().w_full().gap(6.0).children(q.options.iter().map(|o| {
            let picked = s.picks.contains(o);
            let (multiple, option) = (q.multiple, o.clone());
            Button::new(o.clone())
                .variant(if picked {
                    ButtonVariant::Contrast
                } else {
                    ButtonVariant::Outline
                })
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| {
                    if multiple {
                        s.toggle_pick(&option);
                    } else {
                        s.answer(&option);
                    }
                })
                .build(k)
                .key(format!("ask-option-{o}"))
        })));
    }
    let style = rune::TextStyle {
        line_height: 22.0,
        ..k.ty.body
    };
    body = body
        .child(
            col()
                .w_full()
                .px(12.0)
                .py(8.0)
                .radius(k.radius.md)
                .border(
                    1.0,
                    if s.answer_text.focused {
                        c.border_strong
                    } else {
                        c.border_base
                    },
                )
                .child(
                    rune::text_input(&s.answer_text, style)
                        .key("ask-input")
                        .w_full()
                        .color(c.text_base)
                        .placeholder(
                            if q.options.is_empty() {
                                "Type your answer"
                            } else {
                                "Or write your own answer"
                            },
                            c.text_faint,
                        )
                        .caret(c.text_base, k.selection())
                        .label("Your answer")
                        .on_edit(|s: &mut App| &mut s.answer_text)
                        .on_submit(|s: &mut App, _| s.send_answer()),
                ),
        )
        .child(
            row()
                .gap(8.0)
                .child(
                    Button::new("Send answer")
                        .variant(ButtonVariant::Submit)
                        .control_size(ControlSize::Small)
                        .disabled(crate::tools::answer(&s.picks, &s.answer_text.text).is_empty())
                        .on_press(|s: &mut App| s.send_answer())
                        .build(k)
                        .key("ask-send"),
                )
                .child(
                    Button::new("Skip")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press(|s: &mut App| s.skip_question())
                        .build(k)
                        .key("ask-skip"),
                ),
        );
    card(k.c.warning.icon, body)
}

/// The workflow being learned from this chat, or the draft of it: the steps as lines, and what you
/// can do with it. Nothing runs from a draft.
fn draft(s: &App, sid: &str, k: &Kit) -> Option<El> {
    use crate::flows::Teach;
    let c = &k.c;
    let state = s.teaching.get(sid);
    let draft = s.drafts.get(sid);
    if state.is_none() && draft.is_none() {
        return None;
    }
    let mut body = col().w_full().gap(10.0);
    if let Some(d) = draft {
        if !d.goal.is_empty() {
            body = body.child(
                text(d.goal.clone())
                    .font(k.ty.small)
                    .color(c.text_muted)
                    .wrap(),
            );
        }
        body = body.children(d.definition.steps.iter().enumerate().map(|(i, st)| {
            text(format!(
                "{}. {}",
                i + 1,
                crate::workflow::definition::describe(st)
            ))
            .font(k.ty.small)
            .color(c.text_base)
            .wrap()
        }));
        if !d.definition.inputs.is_empty() {
            let inputs: Vec<String> = d
                .definition
                .inputs
                .iter()
                .map(|(n, i)| match &i.default {
                    Some(v) => format!(
                        "{n} ({}, usually {})",
                        i.kind,
                        crate::workflow::template::as_text(v)
                    ),
                    None => format!("{n} ({}, asked each time)", i.kind),
                })
                .collect();
            body = body.child(
                text(format!("Takes: {}", inputs.join(", ")))
                    .font(k.ty.small)
                    .color(c.text_faint)
                    .wrap(),
            );
        }
        if !d.definition.success.is_empty() {
            body = body.child(
                text(format!("Done when: {}", d.definition.success))
                    .font(k.ty.small)
                    .color(c.text_faint)
                    .wrap(),
            );
        }
    }
    match state {
        Some(Teach::Working(what)) => {
            body = body.child(text(*what).font(k.ty.small).color(c.text_muted));
        }
        Some(Teach::Failed(why)) => {
            body = body.child(
                text(why.clone())
                    .font(k.ty.small)
                    .color(k.c.warning.icon)
                    .wrap()
                    .selectable(true),
            );
        }
        None => {}
    }
    let working = matches!(state, Some(Teach::Working(_)));
    if draft.is_some() {
        let style = rune::TextStyle {
            line_height: 22.0,
            ..k.ty.body
        };
        body = body
            .child(
                col()
                    .w_full()
                    .px(12.0)
                    .py(8.0)
                    .radius(k.radius.md)
                    .border(
                        1.0,
                        if s.refine.focused {
                            c.border_strong
                        } else {
                            c.border_base
                        },
                    )
                    .child(
                        rune::text_input(&s.refine, style)
                            .key("refine-input")
                            .w_full()
                            .color(c.text_base)
                            .placeholder(
                                "Change it in your own words, e.g. also list hidden files",
                                c.text_faint,
                            )
                            .caret(c.text_base, k.selection())
                            .label("Change the workflow")
                            .on_edit(|s: &mut App| &mut s.refine)
                            .on_submit(|s: &mut App, _| s.refine_open()),
                    ),
            )
            .child(
                row()
                    .gap(8.0)
                    .child(
                        Button::new("Save workflow")
                            .variant(ButtonVariant::Submit)
                            .control_size(ControlSize::Small)
                            .disabled(working)
                            .on_press(|s: &mut App| s.save_draft())
                            .build(k)
                            .key("draft-save"),
                    )
                    .child(
                        Button::new("Change it")
                            .variant(ButtonVariant::Outline)
                            .control_size(ControlSize::Small)
                            .disabled(working || s.refine.text.trim().is_empty())
                            .on_press(|s: &mut App| s.refine_open())
                            .build(k)
                            .key("draft-change"),
                    )
                    .child(
                        Button::new("Discard")
                            .variant(ButtonVariant::Ghost)
                            .control_size(ControlSize::Small)
                            .on_press(|s: &mut App| s.discard_draft())
                            .build(k)
                            .key("draft-discard"),
                    ),
            );
    } else if matches!(state, Some(Teach::Failed(_))) {
        body = body.child(
            Button::new("Dismiss")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.discard_draft())
                .build(k)
                .key("draft-dismiss"),
        );
    }
    Some(
        Card::new()
            .accent(k.c.info.icon)
            .title(
                match draft {
                    Some(d) => format!("Workflow draft: {}", d.name),
                    None => "Workflow".to_owned(),
                },
                None,
            )
            .description("Steps it would run again. Nothing runs until you save it.")
            .child(body)
            .build(k)
            .key("draft"),
    )
}

/// Schedules the agent proposed in this chat, each waiting for you: what it would do, when, the
/// next few times, and Approve, Edit (on the Schedules page) or Reject. Nothing runs until approved.
fn schedule_drafts(s: &App, sid: &str, k: &Kit) -> Vec<El> {
    use chrono::Local;
    let c = &k.c;
    s.proposals
        .iter()
        .filter(|p| p.session_id == sid)
        .map(|p| {
            let x = &p.schedule;
            let line = |t: String| text(t).font(k.ty.small).color(c.text_base).wrap();
            let mut body = col()
                .w_full()
                .gap(8.0)
                .child(line(super::schedules::target_words(s, &x.target)))
                .child(line(format!("When: {}", x.timing.describe(&Local))));
            let next = s.upcoming(x, 3);
            if !next.is_empty() {
                body = body.child(
                    text(format!("Next: {}", next.join(" · ")))
                        .font(k.ty.small)
                        .color(c.text_faint)
                        .wrap(),
                );
            }
            if let Some((_, why)) = s
                .schedule_page
                .card_notice
                .as_ref()
                .filter(|(call, _)| *call == p.call_id)
            {
                body = body.child(
                    text(why.clone())
                        .font(k.ty.small)
                        .color(k.c.warning.icon)
                        .wrap(),
                );
            }
            let (approve, edit, reject) = (p.call_id.clone(), p.call_id.clone(), p.call_id.clone());
            body = body.child(
                row()
                    .gap(8.0)
                    .child(
                        Button::new("Approve")
                            .variant(ButtonVariant::Submit)
                            .control_size(ControlSize::Small)
                            .on_press(move |s: &mut App| s.approve_from_card(&approve))
                            .build(k)
                            .key(format!("schedule-draft-approve-{}", p.call_id)),
                    )
                    .child(
                        Button::new("Edit")
                            .variant(ButtonVariant::Outline)
                            .control_size(ControlSize::Small)
                            .on_press(move |s: &mut App| s.edit_proposal(&edit))
                            .build(k)
                            .key(format!("schedule-draft-edit-{}", p.call_id)),
                    )
                    .child(
                        Button::new("Reject")
                            .variant(ButtonVariant::Ghost)
                            .control_size(ControlSize::Small)
                            .on_press(move |s: &mut App| s.reject_proposal(&reject))
                            .build(k)
                            .key(format!("schedule-draft-reject-{}", p.call_id)),
                    ),
            );
            let title = if p.updates {
                format!("Change to schedule: {}", x.name)
            } else {
                format!("Schedule draft: {}", x.name)
            };
            Card::new()
                .accent(k.c.info.icon)
                .title(title, None)
                .description("It runs on its own, with this agent's tools, once you approve it.")
                .child(body)
                .build(k)
                .key(format!("schedule-draft-{}", p.call_id))
        })
        .collect()
}

/// MCP server changes the agent asked for in this chat, each waiting for you: what would be added,
/// changed or removed, and Approve, Edit (an add, in the MCP page's form) or Reject. A local server
/// shows the exact command it would run. Keys and variable values are never shown.
fn mcp_drafts(s: &App, sid: &str, k: &Kit) -> Vec<El> {
    use crate::mcp::{Proposal, Server, Transport};
    let c = &k.c;
    let line = |t: String| text(t).font(k.ty.small).color(c.text_base).wrap();
    let target = |x: &Server| {
        match &x.transport {
        Transport::Http { .. } => line(format!("Address: {}", x.target())),
        Transport::Stdio { .. } => col()
            .gap(4.0)
            .child(line("Runs this program on your computer, with your permissions, whenever an agent uses it:".to_owned()))
            .child(text(x.target()).font(k.ty.mono).color(c.text_base).wrap()),
    }
    };
    s.mcp
        .drafts
        .iter()
        .filter(|d| d.session_id == sid)
        .map(|d| {
            let id = d.call_id.clone();
            let mut body = col().w_full().gap(8.0);
            let mut edit = false;
            let (title, approve) = match &d.proposal {
                Proposal::Add { server, secrets } => {
                    body = body.child(target(server));
                    if !secrets.env.is_empty() {
                        let names: Vec<&str> = secrets.env.keys().map(String::as_str).collect();
                        body = body.child(line(format!("Variables: {}", names.join(", "))));
                    }
                    match server.auth {
                        crate::mcp::Auth::Key => {
                            body = body.child(line("Signs in with a key.".to_owned()))
                        }
                        crate::mcp::Auth::OAuth => {
                            body = body
                                .child(line("Signs in in your browser when you add it.".to_owned()))
                        }
                        crate::mcp::Auth::None => {}
                    }
                    edit = true;
                    let approve = match server.transport {
                        Transport::Stdio { .. } => "Run it and add",
                        Transport::Http { .. } => "Add",
                    };
                    (format!("Add MCP server: {}", server.name), approve)
                }
                Proposal::Update {
                    server, touched, ..
                } => {
                    if let Some(now) = s.mcp.servers.iter().find(|x| x.id == server.id)
                        && now.target() != server.target()
                    {
                        body = body.child(line(format!("Was: {}", now.target())));
                    }
                    body = body.child(target(server));
                    if !touched.is_empty() {
                        body = body.child(line(format!("Changes: {}", touched.join(", "))));
                    }
                    (format!("Change MCP server: {}", server.name), "Approve")
                }
                Proposal::Remove { name, .. } => {
                    body = body.child(line("Agents given its tools lose them.".to_owned()));
                    (format!("Remove MCP server: {name}"), "Remove")
                }
            };
            if let Some((_, why)) = s.mcp.draft_notice.as_ref().filter(|(call, _)| *call == id) {
                body = body.child(
                    text(why.clone())
                        .font(k.ty.small)
                        .color(c.warning.icon)
                        .wrap(),
                );
            }
            let mut buttons = row().gap(8.0).child({
                let press = id.clone();
                Button::new(approve)
                    .variant(ButtonVariant::Submit)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut App| s.mcp_approve_draft(&press))
                    .build(k)
                    .key(format!("mcp-draft-approve-{id}"))
            });
            if edit {
                let press = id.clone();
                buttons = buttons.child(
                    Button::new("Edit")
                        .variant(ButtonVariant::Outline)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut App| s.mcp_edit_draft(&press))
                        .build(k)
                        .key(format!("mcp-draft-edit-{id}")),
                );
            }
            buttons = buttons.child({
                let press = id.clone();
                Button::new("Reject")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut App| s.mcp_reject_draft(&press))
                    .build(k)
                    .key(format!("mcp-draft-reject-{id}"))
            });
            Card::new()
                .accent(k.c.info.icon)
                .title(title, None)
                .description(
                    "MCP servers are shared by all your agents. Nothing changes until you approve.",
                )
                .child(body.child(buttons))
                .build(k)
                .key(format!("mcp-draft-{id}"))
        })
        .collect()
}

/// How much of a drafted agent's instructions its card shows.
const DRAFT_BRIEF: usize = 400;

/// Agent changes the agent asked for in this chat, each waiting for you: the agent as it would be
/// (or, for a change, only what changes), and Approve, Edit (in the agent form) or Reject.
fn agent_drafts(s: &App, sid: &str, k: &Kit) -> Vec<El> {
    use crate::agent_tool::Proposal;
    let c = &k.c;
    let line = |t: String| text(t).font(k.ty.small).color(c.text_base).wrap();
    // One line per part of the agent; `only` limits it to the parts that change.
    let parts = |a: &Agent, only: Option<&[&str]>| {
        let wanted = |p: &str| only.is_none_or(|o| o.contains(&p));
        let mut body = col().w_full().gap(6.0);
        if only.is_some() && wanted("name") {
            body = body.child(line(format!("Name: {}", a.name)));
        }
        if wanted("model") {
            let m = a.model.as_ref().map_or_else(
                || "the default model".to_owned(),
                |m| s.providers.model_name(m),
            );
            body = body.child(line(format!("Model: {m}")));
        }
        if wanted("tools") {
            let tools: Vec<String> = a
                .tools
                .iter()
                .map(|t| crate::tools::shown(t, &s.mcp.servers).label)
                .collect();
            body = body.child(line(if tools.is_empty() {
                "Tools: the app's own".to_owned()
            } else {
                format!("Tools: the app's own, and {}", tools.join(", "))
            }));
        }
        if wanted("skills") && (only.is_some() || !a.skills.is_empty()) {
            body = body.child(line(format!("Skills: {}", a.skills.join(", "))));
        }
        if wanted("folder") {
            body = body.child(line(format!(
                "Folder: {}",
                a.workdir.as_deref().unwrap_or("home folder")
            )));
        }
        if wanted("description") && (only.is_some() || !a.description.is_empty()) {
            body = body.child(line(format!("Gets tasks for: {}", a.description)));
        }
        if wanted("instructions") {
            let mut brief: String = a.prompt.chars().take(DRAFT_BRIEF).collect();
            if a.prompt.chars().count() > DRAFT_BRIEF {
                brief.push('…');
            }
            body = body.child(
                text(if brief.is_empty() {
                    "No instructions.".to_owned()
                } else {
                    brief
                })
                .font(k.ty.small)
                .color(c.text_muted)
                .wrap(),
            );
        }
        body
    };
    s.agent_drafts
        .iter()
        .filter(|d| d.session_id == sid)
        .map(|d| {
            let id = d.call_id.clone();
            let (title, approve, body, edit) = match &d.proposal {
                Proposal::Create { agent, lead: None } => (
                    format!("New agent: {}", agent.name),
                    "Create",
                    parts(agent, None),
                    true,
                ),
                Proposal::Create {
                    agent,
                    lead: Some(lead),
                } => {
                    let lead = s.roster.agent_ref(lead).map_or("?", |a| a.name.as_str());
                    (
                        format!("New team member of {lead}: {}", agent.name),
                        "Add to team",
                        parts(agent, None),
                        true,
                    )
                }
                Proposal::Update { agent, changed, .. } => (
                    format!("Change agent: {}", agent.name),
                    "Save changes",
                    parts(agent, Some(changed)),
                    true,
                ),
                Proposal::Delete { target, name } => {
                    let team = s.roster.find(target).map_or(0, |a| a.subagents.len());
                    let also = match team {
                        0 => String::new(),
                        1 => ", and its team member".to_owned(),
                        n => format!(", and its {n} team members"),
                    };
                    (
                        format!("Delete agent: {name}"),
                        "Delete",
                        col().child(line(format!(
                            "Deletes {name} with all its chats{also}. This cannot be undone."
                        ))),
                        false,
                    )
                }
            };
            let mut body = body;
            if let Some((_, why)) = s
                .agent_draft_notice
                .as_ref()
                .filter(|(call, _)| *call == id)
            {
                body = body.child(
                    text(why.clone())
                        .font(k.ty.small)
                        .color(c.warning.icon)
                        .wrap(),
                );
            }
            let mut buttons = row().gap(8.0).child({
                let press = id.clone();
                Button::new(approve)
                    .variant(ButtonVariant::Submit)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut App| s.approve_agent_draft(&press))
                    .build(k)
                    .key(format!("agent-draft-approve-{id}"))
            });
            if edit {
                let press = id.clone();
                buttons = buttons.child(
                    Button::new("Edit")
                        .variant(ButtonVariant::Outline)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut App| s.edit_agent_draft(&press))
                        .build(k)
                        .key(format!("agent-draft-edit-{id}")),
                );
            }
            buttons = buttons.child({
                let press = id.clone();
                Button::new("Reject")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut App| s.reject_agent_draft(&press))
                    .build(k)
                    .key(format!("agent-draft-reject-{id}"))
            });
            Card::new()
                .accent(k.c.info.icon)
                .title(title, None)
                .description("Nothing changes until you approve.")
                .child(body.gap(8.0).child(buttons))
                .build(k)
                .key(format!("agent-draft-{id}"))
        })
        .collect()
}

/// A command the agent ran: what it ran, where, and what it printed. It runs as soon as the agent
/// asks; Stop in the composer ends it.
fn command_card(s: &App, t: &ToolCall, j: usize, k: &Kit) -> El {
    let c = &k.c;
    let spec = crate::command::parse(&t.args);
    let command = spec
        .as_ref()
        .map_or_else(|| t.args.clone(), |c| c.command.clone());
    let why = spec
        .as_ref()
        .map(|c| c.reason.clone())
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "The agent wants to run this on your computer.".to_owned());
    let mono = |body: String, ink| {
        col()
            .w_full()
            .px(12.0)
            .py(8.0)
            .radius(k.radius.md)
            .fill(c.layer_02)
            .child(
                text(body)
                    .font(k.ty.mono)
                    .color(ink)
                    .wrap()
                    .selectable(true),
            )
    };
    let running = s
        .current()
        .is_some_and(|a| s.commands.contains_key(&a.chat().id));
    let folder = s
        .current()
        .map(|a| s.workdir_of(&a.chat().id))
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let mut body = col()
        .w_full()
        .gap(10.0)
        .child(mono(command, c.text_base))
        .child(
            text(format!("in {folder}"))
                .font(k.ty.small)
                .color(c.text_faint)
                .truncate(),
        );
    let mut accent = k.c.warning.icon;
    let stopped = t.trip.as_ref().is_some_and(|t| t.state.stopped());
    match &t.output {
        Some(out) => {
            accent = if stopped {
                k.c.text_faint
            } else {
                k.c.success.icon
            };
            let (shown, cut) = crate::runs::clip(out, 2000);
            body = body.child(mono(
                if cut { format!("{shown}\n…") } else { shown },
                c.text_muted,
            ));
        }
        None if running => {
            body = body.child(
                text("Running… press Stop to end it.")
                    .font(k.ty.small)
                    .color(c.text_muted),
            );
        }
        None => {
            let held = t.trip.is_some() || {
                // Held with another call that waits for your go-ahead.
                s.current().is_some_and(|a| s.holding_in(&a.chat().id))
            };
            body = body.child(
                text(if held {
                    "Waits for your go-ahead."
                } else {
                    "Waiting for the command before it."
                })
                .font(k.ty.small)
                .color(c.text_faint),
            );
        }
    }
    Card::new()
        .accent(accent)
        .title("Command", None)
        .description(why)
        .child(body)
        .build(k)
        .key(format!("shell{j}"))
}

fn tool(t: &ToolCall, writing: bool, j: usize, k: &Kit) -> El {
    let c = &k.c;
    let state = super::flow::state_of(t, writing);
    let badge = super::flow::badge(state);
    let mut r = col()
        .key(format!("tool{j}"))
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(2.0)
        .radius(k.radius.md)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .child(
            row()
                .w_full()
                .center_y()
                .gap(8.0)
                .child(super::flow::mark(state, k))
                .child(text(t.name.clone()).font(k.ty.mono).color(c.text_base))
                .child(
                    text(t.summary.clone())
                        .font(k.ty.small)
                        .color(c.text_muted)
                        .truncate(),
                )
                .child(el().grow(1.0))
                .child(badge.build(k)),
        );
    if let Some(out) = &t.output {
        // A file read can be long; the full text is in the run's record.
        let (shown, cut) = crate::runs::clip(out, 600);
        let shown = if cut { format!("{shown}…") } else { shown };
        r = r.child(text(shown).font(k.ty.small).color(c.text_faint).wrap());
    }
    r
}

/// The human-in-the-loop gate: the run is suspended until it is decided here.
fn gate(a: &Approval, k: &Kit) -> El {
    let body = match a.decision {
        None => row()
            .gap(8.0)
            .child(
                Button::new("Approve")
                    .variant(ButtonVariant::Submit)
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut App| s.decide(Decision::Approve))
                    .build(k)
                    .key("approve"),
            )
            .child(
                Button::new("Reject")
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut App| s.decide(Decision::Reject))
                    .build(k)
                    .key("reject"),
            ),
        Some(Decision::Approve) => row().child(
            Badge::new("Approved")
                .variant(BadgeVariant::Accent)
                .build(k),
        ),
        Some(Decision::Reject) => row().child(
            Badge::new("Rejected")
                .variant(BadgeVariant::Subtle)
                .build(k),
        ),
    };
    let accent = if a.decision.is_none() {
        k.c.warning.icon
    } else {
        k.c.success.icon
    };
    Card::new()
        .accent(accent)
        .title("Approval needed", None)
        .description(a.action.clone())
        .child(body)
        .build(k)
        .key("gate")
}

/// Where older messages were summarized to make room: a marker that opens to the summary. The
/// messages above it stay on screen.
fn summary(s: &App, body: &str, i: usize, k: &Kit) -> El {
    let Some(sid) = s.current().map(|a| a.chat().id.clone()) else {
        return el();
    };
    let open = s.summaries_open.contains(&(sid.clone(), i));
    let mut c = Collapsible::new("Earlier messages summarized to make room", open)
        .ghost()
        .on_toggle(move |s: &mut App| s.toggle_summary(&sid, i));
    if open {
        // Set apart from the replies: a note about the chat, not something the agent said.
        c = c.child(
            col()
                .w_full()
                .px(14.0)
                .py(10.0)
                .radius(k.radius.md)
                .fill(k.c.layer_01)
                .border(1.0, k.c.border_muted)
                .child(markdown(s, body, reply_style(k), k).build(k)),
        );
    }
    col().key(format!("m{i}")).w_full().child(c.build(k))
}

fn divider(label: &str, i: usize, k: &Kit) -> El {
    let rule = || el().grow(1.0).h(1.0).fill(k.c.border_muted);
    row()
        .key(format!("m{i}"))
        .w_full()
        .center_y()
        .gap(12.0)
        .child(rule())
        .child(
            text(label.to_owned())
                .font(k.ty.small)
                .color(k.c.text_faint),
        )
        .child(rule())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use rune::{Env, Runtime};

    use crate::agents::Target;
    use crate::model::{Message, Outcome};

    /// A chat of hundreds of messages builds and draws the ones near its window, so scrolling it
    /// costs what the window shows and not what the chat holds.
    #[test]
    fn a_long_chat_builds_only_the_messages_near_the_window() {
        let mut app = crate::testing::samples();
        app.roster.select(Target::agent("scout"));
        app.roster.current_mut().unwrap().chat_mut().messages = (0..300)
            .flat_map(|i| {
                [
                    Message::User(format!("Question {i}").into()),
                    Message::reply("An answer.", None, Outcome::Done),
                ]
            })
            .collect();
        let mut rt = Runtime::new(
            app,
            crate::ui::view,
            (1280.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.find("m599").is_some(), "the newest message is on screen");
        assert!(rt.find("m0").is_none(), "the oldest message was built");
        let built = rt.elements().len();
        assert!(built < 800, "{built} elements for a 600-message chat");
    }
}
