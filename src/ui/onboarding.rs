//! The first-run screens: pick a look, then connect a model. Once one is connected the chat opens on the Setup agent
//! (see `crate::onboarding`).

use rune::{El, ScrollSpec, col, text};
use rune_kit::{Button, ButtonVariant, Kit};

use super::{providers, settings};
use crate::app::App;

const PAGE: f32 = 768.0;

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let title = crate::theme::ty::title(k);
    let (heading, lead) = if s.setup_look_done {
        (
            "Connect a model",
            "Then an assistant asks a few questions and helps you set up your first agents.",
        )
    } else {
        (
            "Welcome to Oido",
            "First, pick how it looks. You can change it later in Settings.",
        )
    };
    let mut page = col()
        .w_full()
        .max_w(PAGE)
        .px(24.0)
        .py(32.0)
        .gap(28.0)
        .child(
            col()
                .gap(10.0)
                .child(text(heading).font(title).color(k.c.text_base))
                .child(
                    text(lead)
                        .font(k.ty.body)
                        .color(k.c.text_muted)
                        .wrap()
                        .max_w(520.0),
                ),
        );
    if !s.setup_look_done {
        page = page
            .child(settings::section(
                "Appearance",
                vec![settings::theme_row(s, k, layer)],
                k,
            ))
            .child(
                Button::new("Continue")
                    .variant(ButtonVariant::Submit)
                    .on_press(|s: &mut App| s.setup_look_done = true)
                    .build(k)
                    .key("onboarding-continue"),
            );
        return frame(page);
    }
    page = page.child(
        Button::new("Skip setup")
            .variant(ButtonVariant::Outline)
            .on_press(|s: &mut App| s.finish_onboarding())
            .build(k)
            .key("onboarding-skip"),
    );
    for section in providers::unconnected(s, k) {
        page = page.child(section);
    }
    frame(page)
}

fn frame(page: El) -> El {
    col()
        .key("onboarding")
        .w_full()
        .h_full()
        .min_h(0.0)
        .items_center()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}
