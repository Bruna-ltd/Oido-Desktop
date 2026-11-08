//! The conversation: transcript with tool calls and approval gates, and the composer below it.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{Badge, BadgeVariant, Button, ButtonVariant, Card, ControlSize, Kit, Markdown};

use crate::agents::Agent;
use crate::app::App;
use crate::model::{Approval, Decision, Message, Stats, ToolCall};
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
                .child(transcript(agent, k))
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

fn transcript(agent: &Agent, k: &Kit) -> El {
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
            .children(messages.iter().enumerate().map(|(i, m)| message(m, i, k)))
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

fn message(m: &Message, i: usize, k: &Kit) -> El {
    match m {
        Message::User(t) => user(t, i, k),
        Message::Divider(label) => divider(label, i, k),
        Message::Assistant {
            text,
            thought_secs,
            tools,
            approval,
            stats,
        } => assistant(
            text,
            *thought_secs,
            tools,
            approval.as_ref(),
            stats.as_ref(),
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

fn assistant(
    body: &str,
    thought: Option<f32>,
    tools: &[ToolCall],
    approval: Option<&Approval>,
    stats: Option<&Stats>,
    i: usize,
    k: &Kit,
) -> El {
    let c = &k.c;
    let mut out = col().key(format!("m{i}")).w_full().gap(10.0);
    if let Some(secs) = thought {
        out = out.child(
            text(format!("Thought for {secs:.1}s"))
                .font(k.ty.small)
                .color(c.text_faint),
        );
    }
    out = out.children(tools.iter().enumerate().map(|(j, t)| tool(t, j, k)));
    out = out.child(Markdown::new(body).build(k));
    if let Some(a) = approval {
        out = out.child(gate(a, k));
    }
    if let Some(st) = stats {
        out = out.child(
            text(format!(
                "{} · {} tokens · {:.1}s",
                st.model, st.tokens, st.seconds
            ))
            .font(k.ty.small)
            .color(c.text_faint),
        );
    }
    out
}

fn tool(t: &ToolCall, j: usize, k: &Kit) -> El {
    let c = &k.c;
    let done = t.output.is_some();
    let badge = if done {
        Badge::new("done").variant(BadgeVariant::Subtle)
    } else {
        Badge::new("running").variant(BadgeVariant::Accent)
    };
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
    if let Some(out) = t.output.clone() {
        r = r.child(text(out).font(k.ty.small).color(c.text_faint));
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
