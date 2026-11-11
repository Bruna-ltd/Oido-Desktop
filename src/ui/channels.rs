//! The Channels screen: connecting a Telegram bot and a Slack app, approving the people who ask to
//! pair with them, and the people paired. See `src/channels`.

use rune::{El, ScrollSpec, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Switch, TextInput};

use super::settings::{section, setting};
use crate::app::App;
use crate::channels::{Kind, Paired, Pairing, Status, slack};
use crate::picker::{Item, MultiPicker};

const TOKEN_WIDTH: f32 = 280.0;
const COLUMN: f32 = 760.0;

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &s.channels;
    let mut page = col().w_full().max_w(COLUMN).px(24.0).py(24.0).gap(24.0);
    for kind in Kind::ALL {
        let mut rows = match kind {
            Kind::Telegram => vec![telegram_row(s, k)],
            Kind::Slack => slack_rows(s, k),
        };
        if c.get(kind).connected() {
            rows.push(agents_row(s, kind, k, layer));
        }
        page = page.child(section(kind.title(), rows, k));
    }
    let pending: Vec<El> = Kind::ALL
        .into_iter()
        .flat_map(|kind| c.get(kind).pending.iter().map(move |p| (kind, p)))
        .map(|(kind, p)| pending_row(kind, p, k))
        .collect();
    if !pending.is_empty() {
        page = page.child(section("Waiting to pair", pending, k));
    }
    let paired: Vec<El> = Kind::ALL
        .into_iter()
        .flat_map(|kind| c.get(kind).users.iter().map(move |p| (kind, p)))
        .map(|(kind, p)| paired_row(s, kind, p, k))
        .collect();
    if !paired.is_empty() {
        page = page.child(section("Paired", paired, k));
    }
    col()
        .key("channels")
        .w_full()
        .h_full()
        .min_h(0.0)
        .items_center()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

/// The agents people on `kind` may pick, chosen from the agents of their own.
fn agents_row(s: &App, kind: Kind, k: &Kit, layer: &mut Vec<El>) -> El {
    let chosen = s.channel_agents(kind);
    let help = match chosen.len() {
        0 => "None chosen yet: paired people have no agent to talk to.".to_owned(),
        1 => "Paired people talk to it without choosing.".to_owned(),
        _ => format!(
            "Paired people choose among them with {}.",
            kind.command("agent")
        ),
    };
    let items = s
        .roster
        .list
        .iter()
        .map(|a| Item {
            name: a.id.clone(),
            label: a.name.clone(),
            description: a.prompt.lines().next().unwrap_or_default().to_owned(),
            group: String::new(),
        })
        .collect();
    let spec =
        |key, get: fn(&App) -> &MultiPicker, edit: fn(&mut App) -> &mut MultiPicker, toggle| {
            super::multi_picker::Spec {
                key,
                title: "Agents",
                placeholder: "Choose agents",
                noun: ("agent", "agents"),
                none: "No agents yet.",
                items,
                chosen: chosen.iter().map(|(id, _)| id.clone()).collect(),
                get,
                edit,
                toggle,
            }
        };
    let spec = match kind {
        Kind::Telegram => spec(
            "telegram-agents",
            |s| &s.channels.telegram.picker,
            |s| &mut s.channels.telegram.picker,
            |s, id| s.toggle_channel_agent(Kind::Telegram, id),
        ),
        Kind::Slack => spec(
            "slack-agents",
            |s| &s.channels.slack.picker,
            |s| &mut s.channels.slack.picker,
            |s, id| s.toggle_channel_agent(Kind::Slack, id),
        ),
    };
    // The picker under its words, full width: beside them it would squeeze them.
    col()
        .key(format!("setting-{}-agents", kind.as_str()))
        .w_full()
        .padding(16.0)
        .gap(10.0)
        .child(
            col()
                .gap(2.0)
                .child(text("Agents").font(k.ty.ui_medium).color(k.c.text_base))
                .child(text(help).font(k.ty.small).color(k.c.text_muted).wrap()),
        )
        .child(super::multi_picker::build(s, k, layer, spec))
}

fn button(label: &str) -> Button {
    Button::new(label).control_size(ControlSize::Small)
}

/// A secret field for a token.
fn token_field(
    k: &Kit,
    key: &str,
    label: &str,
    placeholder: &str,
    edit: &rune::TextEdit,
    disabled: bool,
    on_edit: impl for<'a> Fn(&'a mut App) -> &'a mut rune::TextEdit + 'static,
) -> El {
    TextInput::new(edit)
        .placeholder(placeholder)
        .label(label)
        .secure()
        .width(Some(TOKEN_WIDTH))
        .disabled(disabled)
        .on_edit(on_edit)
        .build(k)
        .key(key.to_owned())
}

/// Telegram: a token to connect with, or the connected bot.
fn telegram_row(s: &App, k: &Kit) -> El {
    let t = &s.channels.telegram;
    if t.connected() {
        return connected_row(s, Kind::Telegram, k);
    }
    let connecting = t.status == Status::Connecting;
    let help = match &t.status {
        Status::Failed(why) => why.clone(),
        Status::Connecting => "Checking the token with Telegram…".to_owned(),
        _ => "Talk to your agents from Telegram. Make a bot with @BotFather, then paste its token here.".to_owned(),
    };
    let control = row()
        .gap(8.0)
        .center_y()
        .child(token_field(
            k,
            "telegram-token",
            "Bot token",
            "123456789:AAH…",
            &s.channels.telegram_token,
            connecting,
            |s: &mut App| &mut s.channels.telegram_token,
        ))
        .child(
            button(if connecting {
                "Connecting…"
            } else {
                "Connect"
            })
            .variant(ButtonVariant::Submit)
            .disabled(connecting || s.channels.telegram_token.text.trim().is_empty())
            .on_press(|s: &mut App| s.connect_telegram())
            .build(k)
            .key("telegram-connect"),
        );
    setting("telegram", "Connect a bot", &help, control, k)
}

/// Slack: making the app from its manifest and pasting its two tokens, or the connected app.
fn slack_rows(s: &App, k: &Kit) -> Vec<El> {
    let c = &s.channels;
    if c.slack.connected() {
        return vec![connected_row(s, Kind::Slack, k)];
    }
    let connecting = c.slack.status == Status::Connecting;
    let make = setting(
        "slack-manifest",
        "1. Make the Slack app",
        "At api.slack.com/apps choose Create New App, then From a manifest, pick your workspace, and paste this manifest. Then install the app to the workspace.",
        button("Copy manifest")
            .variant(ButtonVariant::Outline)
            .on_press_tx(|_: &mut App, tx: &mut Tx| tx.copy(slack::MANIFEST))
            .build(k)
            .key("slack-copy-manifest"),
        k,
    );
    let help = match &c.slack.status {
        Status::Failed(why) => why.clone(),
        Status::Connecting => "Checking the tokens with Slack…".to_owned(),
        _ => "The bot token is under OAuth & Permissions (xoxb-…). The app-level token is made under Basic Information → App-Level Tokens, with the connections:write scope (xapp-…).".to_owned(),
    };
    let blank =
        c.slack_app_token.text.trim().is_empty() || c.slack_bot_token.text.trim().is_empty();
    let tokens = setting(
        "slack-tokens",
        "2. Paste its tokens",
        &help,
        col()
            .gap(8.0)
            .child(token_field(
                k,
                "slack-bot-token",
                "Bot token",
                "xoxb-…",
                &c.slack_bot_token,
                connecting,
                |s: &mut App| &mut s.channels.slack_bot_token,
            ))
            .child(token_field(
                k,
                "slack-app-token",
                "App-level token",
                "xapp-…",
                &c.slack_app_token,
                connecting,
                |s: &mut App| &mut s.channels.slack_app_token,
            ))
            .child(
                button(if connecting {
                    "Connecting…"
                } else {
                    "Connect"
                })
                .variant(ButtonVariant::Submit)
                .disabled(connecting || blank)
                .on_press(|s: &mut App| s.connect_slack())
                .build(k)
                .key("slack-connect"),
            ),
        k,
    );
    vec![make, tokens]
}

/// A connected channel: its bot, where it stands, its switch, and disconnecting.
fn connected_row(s: &App, kind: Kind, k: &Kit) -> El {
    let c = s.channels.get(kind);
    let id = kind.as_str();
    let help = match &c.status {
        Status::Listening => "Listening. Paired people can talk to agents while Oido is open.".to_owned(),
        Status::Connecting => "Connecting…".to_owned(),
        Status::Retrying(why) => format!("Cannot reach {}, trying again: {why}.", kind.title()),
        Status::Failed(why) => why.clone(),
        Status::Off => "Off. Messages sent now are answered if they are under 10 minutes old when it is turned on.".to_owned(),
    };
    let disconnect = if c.confirm_disconnect {
        row()
            .gap(8.0)
            .child(
                button("Keep it")
                    .variant(ButtonVariant::Ghost)
                    .on_press(move |s: &mut App| {
                        s.channels.get_mut(kind).confirm_disconnect = false
                    })
                    .build(k)
                    .key(format!("{id}-keep")),
            )
            .child(
                button("Disconnect")
                    .variant(ButtonVariant::Danger)
                    .on_press(move |s: &mut App| s.disconnect_channel(kind))
                    .build(k)
                    .key(format!("{id}-disconnect-confirm")),
            )
    } else {
        button("Disconnect…")
            .variant(ButtonVariant::Outline)
            .on_press(move |s: &mut App| s.channels.get_mut(kind).confirm_disconnect = true)
            .build(k)
            .key(format!("{id}-disconnect"))
    };
    let control = row()
        .gap(12.0)
        .center_y()
        .child(
            Switch::new(c.enabled)
                .label("Listen")
                .hide_label()
                .on_toggle(move |s: &mut App| {
                    let on = !s.channels.get(kind).enabled;
                    s.set_channel_enabled(kind, on);
                })
                .build(k),
        )
        .child(disconnect);
    let name = c.bot.as_deref().unwrap_or_default();
    setting(id, &format!("@{name}"), &help, control, k)
}

fn who(name: &str, handle: Option<&str>) -> String {
    match handle {
        Some(h) => format!("{name} (@{h})"),
        None => name.to_owned(),
    }
}

/// Someone asking to pair, with their code to check against what they were sent.
fn pending_row(kind: Kind, p: &Pairing, k: &Kit) -> El {
    let (deny, approve) = (p.user.clone(), p.user.clone());
    setting(
        &format!("pairing-{}-{}", kind.as_str(), p.user),
        &who(&p.name, p.handle.as_deref()),
        &format!(
            "On {}, asks to talk to your agents, with code {}. Approve only if they showed you that code: they can then use the agents' tools on this computer.",
            kind.title(),
            p.code
        ),
        row()
            .gap(8.0)
            .child(
                button("Deny")
                    .variant(ButtonVariant::Ghost)
                    .on_press(move |s: &mut App| s.deny_pairing(kind, &deny))
                    .build(k),
            )
            .child(
                button("Approve")
                    .variant(ButtonVariant::Submit)
                    .on_press(move |s: &mut App| s.approve_pairing(kind, &approve))
                    .build(k),
            ),
        k,
    )
}

fn paired_row(s: &App, kind: Kind, p: &Paired, k: &Kit) -> El {
    let agent = p
        .agent
        .as_deref()
        .and_then(|id| s.roster.agent_ref(id))
        .map_or_else(
            || "No agent picked yet".to_owned(),
            |a| format!("Talks to {}", a.name),
        );
    let id = p.id.clone();
    setting(
        &format!("paired-{}-{}", kind.as_str(), p.id),
        &who(&p.name, p.handle.as_deref()),
        &format!("On {}. {agent}.", kind.title()),
        button("Unpair")
            .variant(ButtonVariant::Outline)
            .on_press(move |s: &mut App| s.unpair(kind, &id))
            .build(k),
        k,
    )
}
