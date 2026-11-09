//! The tools register: every tool an agent can be given, what it does, what it takes, and which
//! agents have it. An agent is offered only the tools picked in its own form.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{Badge, BadgeVariant, Kit};

use crate::app::App;
use crate::completion::ToolSpec;

const COLUMN: f32 = 760.0;

pub fn screen(s: &App, k: &Kit) -> El {
    let tools = crate::tools::registered();
    col()
        .key("tools")
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
                .gap(12.0)
                .child(
                    text("Tools are what an agent can do besides talk. Give one to an agent in its form, under More options.")
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .wrap(),
                )
                .children(tools.iter().map(|t| card(s, t, k))),
        )
}

fn card(s: &App, t: &ToolSpec, k: &Kit) -> El {
    let c = &k.c;
    let needs = crate::tools::needs_typesafe(t.name);
    col()
        .key(format!("tool-{}", t.name))
        .w_full()
        .padding(16.0)
        .gap(8.0)
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .child(
            row()
                .w_full()
                .center_y()
                .gap(8.0)
                .child(text(t.name).font(k.ty.mono).color(c.text_base))
                .child(el().grow(1.0))
                .child(
                    Badge::new(if needs { "Needs TypeSafe" } else { "Built in" })
                        .variant(BadgeVariant::Subtle)
                        .build(k),
                ),
        )
        .child(
            text(t.description)
                .font(k.ty.small)
                .color(c.text_muted)
                .wrap(),
        )
        .child(
            text(format!("Takes: {}", params(t)))
                .font(k.ty.small)
                .color(c.text_faint)
                .wrap(),
        )
        .children((needs && !s.typesafe_connected()).then(|| {
            text("Off until TypeSafe is connected on the Providers page.")
                .font(k.ty.small)
                .color(c.text_faint)
                .wrap()
        }))
        .child(
            text(used_by(s, t.name))
                .font(k.ty.small)
                .color(c.text_faint)
                .wrap(),
        )
}

/// "question (required), options, multiple".
fn params(t: &ToolSpec) -> String {
    let required: Vec<&str> = t.schema["required"]
        .as_array()
        .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let names: Vec<String> = t.schema["properties"]
        .as_object()
        .map(|p| {
            p.keys()
                .map(|n| {
                    if required.contains(&n.as_str()) {
                        format!("{n} (required)")
                    } else {
                        n.clone()
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        "nothing".to_owned()
    } else {
        names.join(", ")
    }
}

/// "Used by Atlas, Forge." or that no agent has it yet.
fn used_by(s: &App, name: &str) -> String {
    let mut who: Vec<&str> = Vec::new();
    for a in &s.roster.list {
        for agent in std::iter::once(a).chain(a.subagents.iter()) {
            if agent.tools.iter().any(|t| t == name) {
                who.push(&agent.name);
            }
        }
    }
    if who.is_empty() {
        "No agent has it yet.".to_owned()
    } else {
        format!("Used by {}.", who.join(", "))
    }
}
