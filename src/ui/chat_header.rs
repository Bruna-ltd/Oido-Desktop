//! Above the transcript, as in Studio's ChatBox top bar: who this conversation is with, "Start
//! fresh", and the agent's open chats as pills.

use rune::{Color, El, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Divider, IconButton, IconButtonVariant, Kit, glyphs,
};

use super::{icons, sidebar};
use crate::agents::Agent;
use crate::app::App;
use crate::model::AgentStatus;
use crate::providers::ModelStatus;
use crate::theme;

pub fn build(s: &App, a: &Agent, k: &Kit) -> El {
    col()
        .key("chat-header")
        .w_full()
        .child(title(s, a, k))
        .child(tabs(a, k))
        .child(Divider::horizontal().build(k))
}

fn title(s: &App, a: &Agent, k: &Kit) -> El {
    let (subtitle, ink) = match a.status {
        AgentStatus::Running => ("Working…".to_owned(), theme::green()),
        AgentStatus::Waiting => ("Waiting for your approval".to_owned(), theme::amber()),
        // Idle: which model it will answer with, or why it cannot.
        AgentStatus::Idle => match s.providers.status(a.model.as_ref()) {
            ModelStatus::Ready(m) => (s.providers.model_name(&m), k.c.text_muted),
            ModelStatus::Choose => ("Choose a model".to_owned(), theme::amber()),
            ModelStatus::Unavailable { why, .. } => (why, theme::amber()),
        },
    };
    row()
        .w_full()
        .px(20.0)
        .pt(12.0)
        .pb(4.0)
        .gap(12.0)
        .center_y()
        .child(sidebar::avatar(a, k, 28.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(text(a.name.clone()).font(k.ty.body_medium).color(k.c.text_base))
                .child(text(subtitle).font(k.ty.small).color(ink)),
        )
        .child(
            Button::new("Start fresh")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .help("The agent forgets everything said so far in this chat and begins with a clean slate. What you have already read stays on screen.")
                .on_press(|s: &mut App| s.start_fresh())
                .build(k)
                .key("start-fresh"),
        )
}

fn tabs(a: &Agent, k: &Kit) -> El {
    let many = a.sessions.len() > 1;
    row()
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(4.0)
        .center_y()
        .children(a.sessions.iter().enumerate().map(|(i, t)| {
            tab(
                i,
                &t.title,
                i == a.active,
                a.status == AgentStatus::Running,
                many,
                k,
            )
        }))
        .child(
            IconButton::new(&icons::PLUS)
                .variant(IconButtonVariant::GhostMuted)
                .label("New chat")
                .on_press(|s: &mut App| s.new_tab())
                .build(k)
                .key("new-tab"),
        )
}

fn tab(i: usize, title: &str, active: bool, running: bool, closable: bool, k: &Kit) -> El {
    let c = &k.c;
    let ink = if active { c.text_base } else { c.text_muted };
    let mut e = row()
        .key(format!("tab{i}"))
        .h(32.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .radius(16.0)
        .fill(if active {
            c.layer_02
        } else {
            Color::TRANSPARENT
        })
        .on_press(move |s: &mut App| s.select_tab(i));
    if running && active {
        e = e.child(el().size(6.0, 6.0).radius(3.0).fill(theme::green()));
    }
    e = e.child(
        text(title.to_owned())
            .font(k.ty.ui)
            .color(ink)
            .truncate()
            .max_w(140.0),
    );
    if active && closable {
        e = e.child(
            row()
                .key(format!("close{i}"))
                .size(16.0, 16.0)
                .radius(8.0)
                .center()
                .on_press(move |s: &mut App| s.close_tab(i))
                .role("button")
                .label("Close chat")
                .child(icon(&glyphs::CLOSE, 10.0).color(c.text_muted)),
        );
    }
    k.interactive(e)
}
