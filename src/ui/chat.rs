//! The conversation: transcript with tool calls and approval gates, and the composer below it.

use rune::{El, ScrollSpec, TextStyle, col, el, row, text};
use rune_kit::{
    Badge, BadgeVariant, Button, ButtonVariant, Card, ControlSize, Kit, Markdown, MarkdownStyle,
};

use crate::agents::Agent;
use crate::app::App;
use crate::model::{Approval, Decision, Message, Outcome, Stats, ToolCall};
use crate::theme;

const COLUMN: f32 = 760.0;

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let Some(agent) = s.current() else {
        return no_agents(k);
    };
    col()
        .key("chat")
        .w_full()
        .h_full()
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
                        .child(super::composer::build(s, agent, k, layer)),
                ),
        )
}

fn transcript(s: &App, agent: &Agent, k: &Kit) -> El {
    let messages = &agent.chat().messages;
    let body = if messages.is_empty() {
        empty(agent, k)
    } else {
        col()
            .w_full()
            .max_w(COLUMN)
            .px(24.0)
            .py(24.0)
            .gap(20.0)
            .children(
                messages
                    .iter()
                    .enumerate()
                    .map(|(i, m)| message(s, m, i, i + 1 == messages.len(), k)),
            )
            .children(draft(s, &agent.chat().id, k))
    };
    col()
        .key("transcript")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .items_center()
        .scroll(ScrollSpec { stick_bottom: true })
        .child(body)
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
        Message::User(t) => user(t, i, k),
        Message::Divider(label) => divider(label, i, k),
        Message::Assistant {
            text,
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

fn user(t: &str, i: usize, k: &Kit) -> El {
    row().key(format!("m{i}")).w_full().justify_end().child(
        col()
            .max_w(560.0)
            .px(14.0)
            .py(10.0)
            .radius(k.radius.lg)
            .fill(k.c.layer_02)
            .child(
                text(t.to_owned())
                    .font(k.ty.body)
                    .color(k.c.text_base)
                    .wrap()
                    .selectable(true),
            ),
    )
}

/// An assistant message as the transcript shows it.
struct Reply<'a> {
    body: &'a str,
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

fn assistant(s: &App, r: &Reply, i: usize, k: &Kit) -> El {
    let c = &k.c;
    let mut out = col().key(format!("m{i}")).w_full().gap(10.0);
    if let Some(secs) = r.thought {
        out = out.child(
            text(format!("Thought for {secs:.1}s"))
                .font(k.ty.small)
                .color(c.text_faint),
        );
    }
    // Other tools show what they did before the reply; a question comes after it, as its end.
    out = out.children(
        r.tools
            .iter()
            .enumerate()
            .filter(|(_, t)| !crate::tools::needs_you(&t.name))
            .map(|(j, t)| {
                let writing = *r.outcome == Outcome::Working;
                if t.name == crate::tools::DELEGATE {
                    super::flow::delegation(s, t, writing, j, k)
                } else {
                    tool(t, writing, j, k)
                }
            }),
    );
    if *r.outcome == Outcome::Working && r.body.is_empty() {
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
        out = out.child(Markdown::new(r.body).style(reply_style(k)).build(k));
    }
    out = out.children(
        r.tools
            .iter()
            .enumerate()
            .filter(|(_, t)| crate::tools::needs_you(&t.name))
            .map(|(j, t)| {
                if t.name == crate::tools::SHELL {
                    command_card(s, t, j, k)
                } else {
                    question(s, t, r.last, j, k)
                }
            }),
    );
    if let Some(a) = r.approval {
        out = out.child(gate(a, k));
    }
    match r.outcome {
        Outcome::Stopped => out = out.child(ended("Stopped.", c.text_faint, r.last, k)),
        Outcome::Failed(why) => out = out.child(ended(why, theme::amber(), r.last, k)),
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
            theme::green(),
            text(format!("You answered: {answer}"))
                .font(k.ty.small)
                .color(c.text_muted)
                .wrap()
                .selectable(true),
        );
    }
    if !last {
        return card(
            theme::amber(),
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
    card(theme::amber(), body)
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
                    .color(theme::amber())
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
            .accent(theme::violet())
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
    let mut accent = theme::amber();
    match &t.output {
        Some(out) => {
            accent = theme::green();
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
            body = body.child(
                text("Waiting for the command before it.")
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
        theme::amber()
    } else {
        theme::green()
    };
    Card::new()
        .accent(accent)
        .title("Approval needed", None)
        .description(a.action.clone())
        .child(body)
        .build(k)
        .key("gate")
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
