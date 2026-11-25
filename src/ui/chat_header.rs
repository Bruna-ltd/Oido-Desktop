//! Above the transcript, as in Studio's ChatBox top bar: who this conversation is with in the
//! window's toolbar, and the agent's open chats as pills under it. Start fresh and Save as
//! workflow are `/clear` and `/save-workflow` in the composer.

use rune::{Color, El, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Divider, IconButton, IconButtonVariant, Kit, Toolbar,
    glyphs,
};

use super::{icons, sidebar, usage};
use crate::agents::Agent;
use crate::app::App;
use crate::model::AgentStatus;
use crate::providers::ModelStatus;

/// Fills the window's toolbar on the chat screen: who the conversation is with on the left, what
/// it has used on the right. The band would otherwise sit empty above
/// a header of its own.
pub fn toolbar(s: &App, a: &Agent, k: &Kit, bar: Toolbar) -> Toolbar {
    // The figures read as one group, set apart from the buttons after them.
    let figures: Vec<El> = fill(s, a, k).into_iter().chain(spend(s, a, k)).collect();
    let mut bar = bar.leading(identity(s, a, k));
    if !figures.is_empty() {
        bar = bar.trailing(row().gap(12.0).pr(8.0).center_y().children(figures));
    }
    stop_background(s, a, k)
        .into_iter()
        .fold(bar, Toolbar::trailing)
}

/// Under the toolbar: the agent's open chats as pills.
pub fn build(s: &App, a: &Agent, k: &Kit) -> El {
    col()
        .key("chat-header")
        .w_full()
        .child(tabs(s, a, k))
        .child(Divider::horizontal().build(k))
}

fn identity(s: &App, a: &Agent, k: &Kit) -> El {
    let background = s.background_running(a);
    let (subtitle, ink) = match a.status {
        AgentStatus::Running => ("Working…".to_owned(), k.c.success.icon),
        AgentStatus::Waiting => ("Waiting for your approval".to_owned(), k.c.warning.icon),
        // Not in a turn, but something it started is still going.
        AgentStatus::Idle if background > 0 => (
            format!(
                "{background} background process{} running",
                if background == 1 { "" } else { "es" }
            ),
            k.c.success.icon,
        ),
        // Idle: which model it will answer with, or why it cannot.
        AgentStatus::Idle => match s.providers.status(a.model.as_ref()) {
            ModelStatus::Ready(m) => (s.providers.model_name(&m), k.c.text_muted),
            ModelStatus::Choose => ("Choose a model".to_owned(), k.c.warning.icon),
            ModelStatus::Unavailable { why, .. } => (why, k.c.warning.icon),
        },
    };
    row()
        .key("chat-identity")
        .shrink()
        .min_w(0.0)
        .gap(10.0)
        .center_y()
        .child(sidebar::avatar(a, k, 28.0))
        .child(
            col()
                .shrink()
                .min_w(0.0)
                .child(
                    text(a.name.clone())
                        .font(k.ty.body_medium)
                        .color(k.c.text_base)
                        .truncate(),
                )
                .child(text(subtitle).font(k.ty.small).color(ink).truncate()),
        )
}

/// A way to end what the agent left running in the background, which is not "busy" and so has no
/// Stop of its own. The tooltip lists the commands.
fn stop_background(s: &App, a: &Agent, k: &Kit) -> Option<El> {
    let running = s.background_commands(a);
    if running.is_empty() {
        return None;
    }
    let id = a.id.clone();
    Some(
        Button::new("Stop background")
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .help(format!(
                "Ends these and everything they started:\n{}",
                running.join("\n")
            ))
            .on_press(move |s: &mut App| s.stop_background(&id))
            .build(k)
            .key("stop-background"),
    )
}

/// How full the chat is against the model's context limit; amber when nearly full. Nothing before
/// its first reply or after "Start fresh".
fn fill(s: &App, a: &Agent, k: &Kit) -> Option<El> {
    let f = s.chat_fill(a)?;
    let used = usage::tokens(u64::from(f.used));
    let (said, help) = match (f.share(), f.limit) {
        (Some(share), Some(limit)) => (
            format!("{:.0}% context", (share * 100.0).min(999.0)),
            format!(
                "This chat fills {used} of the {} tokens the model takes at once. Past 90%, older tool results and pictures are left out of what it is sent; the chat keeps them.",
                usage::tokens(u64::from(limit))
            ),
        ),
        _ => (
            format!("{used} context"),
            format!(
                "This chat holds about {used} tokens. How many the model takes at once is not known."
            ),
        ),
    };
    let ink = if f.near() {
        k.c.warning.icon
    } else {
        k.c.text_faint
    };
    Some(
        text(said)
            .key("chat-fill")
            .font(k.ty.small)
            .color(ink)
            .help(help),
    )
}

/// What this chat has used so far, in tokens and likely cost; nothing before its first reply.
fn spend(s: &App, a: &Agent, k: &Kit) -> Option<El> {
    let (t, cost) = s.chat_spend(&a.chat().id)?;
    let mut said = format!("{} tokens", usage::tokens(t.tokens()));
    if !cost.unknown() {
        said = format!("{said} · {}", usage::money(&cost));
    }
    Some(
        text(said)
            .key("chat-spend")
            .font(k.ty.small)
            .color(k.c.text_faint)
            .help("What this chat has used so far, team members included. The cost is an estimate from current prices; the Usage screen has more."),
    )
}

/// How far one press of an arrow moves the pills.
const TABS_STEP: f32 = 200.0;
const TABS: &str = "chat-tabs";

/// The chats as pills in a strip that scrolls sideways when they overflow, with an arrow at each
/// end that has more to show. "New chat" stays put at the right.
fn tabs(s: &App, a: &Agent, k: &Kit) -> El {
    let many = a.sessions.len() > 1;
    let [at, max] = s.tabs_scroll.get();
    let seen = s.tabs_scroll.clone();
    // The scroll viewport stretches its one child to its own width, so the pills sit in a row of
    // their own and keep their natural size.
    let strip = row()
        .key(TABS)
        .grow(1.0)
        .min_w(0.0)
        .scroll_x()
        .scroll_edge_fade(16.0)
        .on_scroll_offset(move |p| seen.set([p.offset[0], p.max[0]]))
        .child(
            row()
                .gap(4.0)
                .center_y()
                .children(a.sessions.iter().enumerate().map(|(i, t)| {
                    tab(
                        i,
                        &t.title,
                        i == a.active,
                        a.status == AgentStatus::Running,
                        many,
                        s.on_channel(&t.id),
                        k,
                    )
                })),
        );
    let arrow = |glyph, label: &str, key: &str, by: f32, shown: bool| {
        shown.then(|| {
            IconButton::new(glyph)
                .variant(IconButtonVariant::GhostMuted)
                .label(label.to_owned())
                .build(k)
                .key(key.to_owned())
                .on_press_tx(move |s: &mut App, tx| {
                    let [at, _] = s.tabs_scroll.get();
                    tx.scroll_to(TABS, (at + by).max(0.0), 0.0);
                })
        })
    };
    row()
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(4.0)
        .center_y()
        .children(arrow(
            &icons::CHEVRON_LEFT,
            "Earlier chats",
            "tabs-left",
            -TABS_STEP,
            at > 0.5,
        ))
        .child(strip)
        .children(arrow(
            &icons::CHEVRON_RIGHT,
            "Later chats",
            "tabs-right",
            TABS_STEP,
            at < max - 0.5,
        ))
        .child(
            IconButton::new(&icons::PLUS)
                .variant(IconButtonVariant::GhostMuted)
                .label("New chat")
                .on_press(|s: &mut App| s.new_tab())
                .build(k)
                .key("new-tab"),
        )
}

fn tab(
    i: usize,
    title: &str,
    active: bool,
    running: bool,
    closable: bool,
    on_channel: Option<crate::channels::Kind>,
    k: &Kit,
) -> El {
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
        e = e.child(el().size(6.0, 6.0).radius(3.0).fill(k.c.success.icon));
    }
    // A chat a paired person carries on from a channel: what is written here goes to them too.
    if let Some(kind) = on_channel {
        e = e.child(super::logo::channel(kind, 14.0).help(format!(
            "Carried on from {}: replies here go there too",
            kind.title()
        )));
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
