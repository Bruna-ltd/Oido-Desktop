//! Guardrails on screen: the card under a tool call a rule refused or held for your go-ahead, and
//! the Guardrails screen, where each pack of rules is switched on or off.

use rune::{El, ScrollSpec, col, text};
use rune_kit::{Button, ButtonVariant, Card, ControlSize, Kit, Switch};

use super::settings::{section, setting};
use crate::app::App;
use crate::guard::{Pack, TripState, title};
use crate::model::ToolCall;

const COLUMN: f32 = 760.0;

/// The card for the guardrail call `t` tripped in chat `sid`, if it tripped one. It sits under the
/// call's own card; a command's card already shows the command, so it is not repeated.
pub fn trip(s: &App, sid: &str, t: &ToolCall, j: usize, k: &Kit) -> Option<El> {
    let trip = t.trip.as_ref()?;
    let c = &k.c;
    // A command's card above already shows the command.
    let what = (t.name != crate::tools::SHELL).then(|| mono(shown(s, &t.name, &t.summary), k));
    let line = |words: String, ink| text(words).font(k.ty.small).color(ink).wrap();
    let (accent, heading, body) = match trip.state {
        TripState::Asking => {
            let (sid, call) = (sid.to_owned(), t.id.clone());
            let (allow, deny) = ((sid.clone(), call.clone()), (sid, call));
            let body = col().w_full().gap(10.0).children(what).child(
                rune::row()
                    .gap(8.0)
                    .child(
                        Button::new("Allow")
                            .variant(ButtonVariant::Submit)
                            .control_size(ControlSize::Small)
                            .on_press(move |s: &mut App| s.decide_trip(&allow.0, &allow.1, true))
                            .build(k)
                            .key(format!("trip-allow{j}")),
                    )
                    .child(
                        Button::new("Don't allow")
                            .variant(ButtonVariant::Outline)
                            .control_size(ControlSize::Small)
                            .on_press(move |s: &mut App| s.decide_trip(&deny.0, &deny.1, false))
                            .build(k)
                            .key(format!("trip-deny{j}")),
                    ),
            );
            let body = body.child(line(
                format!(
                    "If nobody answers within {} minutes, it does not run.",
                    s.guard.wait()
                ),
                c.text_faint,
            ));
            (c.warning.icon, "Needs your go-ahead", body)
        }
        TripState::Allowed => (
            c.success.icon,
            "Allowed by you",
            col().w_full().gap(8.0).children(what),
        ),
        TripState::TimedOut => (
            c.text_faint,
            "No answer in time",
            col().w_full().gap(8.0).children(what).child(line(
                "Nobody gave the go-ahead in time, so it did not run, and the agent was told."
                    .to_owned(),
                c.text_muted,
            )),
        ),
        TripState::Denied => (
            c.text_faint,
            "Not allowed",
            col().w_full().gap(8.0).children(what).child(line(
                "You said no, and the agent was told.".to_owned(),
                c.text_muted,
            )),
        ),
        TripState::Blocked => (
            c.warning.icon,
            "Refused by a guardrail",
            col().w_full().gap(8.0).children(what).child(line(
                "It did not run, and the agent was told.".to_owned(),
                c.text_muted,
            )),
        ),
    };
    Some(
        Card::new()
            .accent(accent)
            .title(heading, None)
            .description(format!("{}: {}", title(&trip.rule), trip.why))
            .child(body)
            .build(k)
            .key(format!("trip{j}")),
    )
}

/// The cards of calls from work nobody watches (a team member, a workflow) waiting at `at`.
pub fn waiting(s: &App, at: &crate::guard::ask::Where, k: &Kit) -> Vec<El> {
    let c = &k.c;
    s.waiting_at(at)
        .map(|w| {
            let (allow, deny) = (w.id.clone(), w.id.clone());
            let body = col()
                .w_full()
                .gap(10.0)
                .child(mono(shown(s, &w.tool, &w.summary), k))
                .child(
                    rune::row()
                        .gap(8.0)
                        .child(
                            Button::new("Allow")
                                .variant(ButtonVariant::Submit)
                                .control_size(ControlSize::Small)
                                .on_press(move |s: &mut App| s.decide_waiting(&allow, true))
                                .build(k)
                                .key(format!("waiting-allow-{}", w.id)),
                        )
                        .child(
                            Button::new("Don't allow")
                                .variant(ButtonVariant::Outline)
                                .control_size(ControlSize::Small)
                                .on_press(move |s: &mut App| s.decide_waiting(&deny, false))
                                .build(k)
                                .key(format!("waiting-deny-{}", w.id)),
                        ),
                )
                .child(
                    text(format!(
                        "{} waits for this. If nobody answers within {} minutes, it does not run.",
                        w.from,
                        s.guard.wait()
                    ))
                    .font(k.ty.small)
                    .color(c.text_faint)
                    .wrap(),
                );
            Card::new()
                .accent(c.warning.icon)
                .title(format!("{} needs your go-ahead", w.from), None)
                .description(format!("{}: {}", title(&w.trip.rule), w.trip.why))
                .child(body)
                .build(k)
                .key(format!("waiting-{}", w.id))
        })
        .collect()
}

/// A line of code or a command, on its own background.
fn mono(shown: String, k: &Kit) -> El {
    let c = &k.c;
    col()
        .w_full()
        .px(12.0)
        .py(8.0)
        .radius(k.radius.md)
        .fill(c.layer_02)
        .child(
            text(shown)
                .font(k.ty.mono)
                .color(c.text_base)
                .wrap()
                .selectable(true),
        )
}

/// What the call would do, in a line: the command, or the tool and what it is given.
fn shown(s: &App, tool: &str, summary: &str) -> String {
    if tool == crate::tools::SHELL {
        return summary.to_owned();
    }
    let label = crate::tools::shown(tool, &s.mcp.servers).label;
    match summary.trim() {
        "" => label,
        about => format!("{label}: {about}"),
    }
}

/// The Guardrails screen: each pack of rules, with its switch.
pub fn screen(s: &App, k: &Kit) -> El {
    let rows = Pack::ALL
        .iter()
        .map(|&pack| {
            let on = s.guard.on(pack);
            setting(
                &format!("guard-{pack:?}").to_lowercase(),
                pack.title(),
                pack.about(),
                Switch::new(on)
                    .label(pack.title())
                    .hide_label()
                    .on_toggle(move |s: &mut App| s.set_pack(pack, !on))
                    .build(k),
                k,
            )
        })
        .collect();
    col()
        .key("guardrails")
        .w_full()
        .h_full()
        .min_h(0.0)
        .items_center()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .max_w(COLUMN)
                .px(24.0)
                .py(24.0)
                .gap(16.0)
                .child(
                    text("Checks made on this computer, before anything reaches a model and before an agent's action runs. They apply to every agent, in every chat, schedule, workflow and channel.")
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .wrap(),
                )
                .child(section("Rules", rows, k))
                .child(section("Go-ahead", vec![wait_row(s, k)], k)),
        )
}

/// How long a call held for your go-ahead waits before it is not run.
fn wait_row(s: &App, k: &Kit) -> El {
    let chosen = s.guard.wait();
    let choices = rune::row()
        .gap(6.0)
        .children(crate::guard::WAITS.iter().map(|&m| {
            Button::new(format!("{m} min"))
                .variant(if m == chosen {
                    ButtonVariant::Contrast
                } else {
                    ButtonVariant::Outline
                })
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.set_guard_wait(m))
                .build(k)
                .key(format!("guard-wait-{m}"))
        }));
    setting(
        "guard-wait",
        "Wait for your answer",
        "How long a call held for your go-ahead waits. Then it does not run, and the agent, or the person on a channel, is told.",
        choices,
        k,
    )
}
