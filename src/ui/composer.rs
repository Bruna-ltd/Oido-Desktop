//! Studio's message box: a bare auto-growing field in a rounded container, with attach, think
//! effort and the model on the left, voice and send (or stop, while the agent works) on the right.
//! When the agent has no model that can answer, it says so and does not let you send.

use rune::{El, TextStyle, auto_grow, col, el, icon, row, text, text_input};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, glyphs};

use super::icons;
use super::model_picker::ModelPicker;
use crate::agents::Agent;
use crate::app::{App, Effort};
use crate::models::can_think;
use crate::providers::ModelStatus;

const IDLE: &str = "Write a message.";
const BUSY: &str = "Waiting for the reply. Press Stop to cut it short.";
const ASKED: &str = "Answer or decide on the request above first.";

pub fn build(s: &App, agent: &Agent, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let running = s.busy();
    let asked = s.awaiting_answer();
    let status = s.providers.status(agent.model.as_ref());
    let ready = matches!(status, ModelStatus::Ready(_));
    let has_text = !s.composer.text.trim().is_empty();
    // Unknown abilities stay enabled: only a model known not to reason greys the button out.
    let thinks = match &status {
        ModelStatus::Ready(m) => s.providers.model_info(m).is_none_or(|i| can_think(&i.caps)),
        _ => true,
    };

    let style = TextStyle {
        line_height: 22.0,
        ..k.ty.body
    };
    let field = text_input(&s.composer, style)
        .key("composer-input")
        .w_full()
        .color(c.text_base)
        .placeholder(
            if running {
                BUSY
            } else if asked {
                ASKED
            } else {
                IDLE
            },
            c.text_faint,
        )
        .caret(c.text_base, k.selection())
        .label("Message")
        .on_edit(|s: &mut App| &mut s.composer)
        .on_submit(|s: &mut App, _| s.send());
    let input = col().w_full().px(8.0).py(8.0).child(auto_grow(field, 1, 8));

    let attach = IconButton::new(&icons::PAPERCLIP)
        .variant(IconButtonVariant::GhostMuted)
        .help("Attach a file")
        .label("Attach a file")
        .disabled(running)
        .build(k)
        .key("attach");
    let think = Button::new(s.effort.label())
        .variant(if s.effort == Effort::Off {
            ButtonVariant::Ghost
        } else {
            ButtonVariant::Contrast
        })
        .control_size(ControlSize::Small)
        .help(if thinks {
            "How hard the agent thinks before answering. Click to cycle: off, low, medium, high."
        } else {
            "This model has no thinking setting."
        })
        .disabled(running || !thinks)
        .on_press(|s: &mut App| s.effort = s.effort.next())
        .build(k)
        .key("think");
    let voice = IconButton::new(&glyphs::MIC)
        .variant(IconButtonVariant::GhostMuted)
        .help("Hold to talk")
        .label("Hold to talk")
        .build(k)
        .key("voice");
    let action = if running {
        IconButton::new(&icons::STOP)
            .variant(IconButtonVariant::Contrast)
            .label("Stop")
            .on_press(|s: &mut App| s.stop())
            .build(k)
            .key("stop")
    } else {
        let can_send = has_text && ready && !asked;
        let mut send = IconButton::new(&glyphs::ARROW_RIGHT)
            .variant(if can_send {
                IconButtonVariant::Submit
            } else {
                IconButtonVariant::Ghost
            })
            .label("Send")
            .disabled(!can_send)
            .on_press(|s: &mut App| s.send());
        if let Some(why) = reason(&status) {
            send = send.help(why);
        }
        send.build(k).key("send")
    };

    let mut card = col()
        .key("composer")
        .w_full()
        .radius(24.0)
        .fill(c.layer_01)
        .border(
            1.0,
            if s.composer.focused {
                c.border_strong
            } else {
                c.border_base
            },
        );
    if let Some(why) = reason(&status) {
        card = card.child(
            row()
                .key("model-notice")
                .w_full()
                .px(16.0)
                .pt(10.0)
                .child(text(why).font(k.ty.small).color(k.c.warning.icon).wrap()),
        );
    }
    card.child(input).child(
        row()
            .w_full()
            .px(10.0)
            .pb(8.0)
            .gap(4.0)
            .center_y()
            .child(attach)
            .child(think)
            .child(model_pill(s, agent, &status, k, layer))
            .child(el().grow(1.0))
            .child(voice)
            .child(action),
    )
}

/// Why this agent cannot answer, in words, when it cannot.
fn reason(status: &ModelStatus) -> Option<String> {
    match status {
        ModelStatus::Ready(_) => None,
        ModelStatus::Choose => Some("Choose a model to start chatting.".to_owned()),
        ModelStatus::Unavailable { why, .. } => {
            Some(format!("{why} Choose another model, or connect it again."))
        }
    }
}

/// The agent's model: a quiet pill that opens the grouped list. A model that cannot answer is kept
/// and shown in amber, not swapped for another.
fn model_pill(s: &App, agent: &Agent, status: &ModelStatus, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let (name, ink) = match status {
        ModelStatus::Ready(m) => (s.providers.model_name(m), c.text_muted),
        ModelStatus::Choose => ("Choose a model".to_owned(), k.c.warning.icon),
        ModelStatus::Unavailable { model, .. } => (s.providers.model_name(model), k.c.warning.icon),
    };
    let pill = k.interactive(
        row()
            .key("model-pill")
            .h(28.0)
            .px(10.0)
            .gap(4.0)
            .center_y()
            .radius(14.0)
            .role("button")
            .label(format!("Model: {name}"))
            .child(text(name).font(k.ty.small).color(ink))
            .child(icon(&glyphs::CHEVRON_DOWN, 10.0).color(c.text_faint)),
    );
    let current = agent.model.clone();
    let current_name = current.as_ref().map(|m| s.providers.model_name(m));
    ModelPicker::new("composer-model", s.providers.groups())
        .current(current, current_name)
        .label("Model")
        .trigger(pill)
        .on_open(|s: &mut App| s.models_opened())
        .on_connect(|s: &mut App| {
            s.model_picker.close();
            s.route = crate::app::Route::Providers;
        })
        .build(
            k,
            layer,
            &s.model_picker,
            |s: &mut App| &mut s.model_picker,
            |s: &mut App, v| {
                // The picker is not clearable, so a choice always carries a model.
                if v.is_some() {
                    s.set_model(v);
                }
            },
        )
}
