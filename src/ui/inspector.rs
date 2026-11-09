//! Right pane of the Runs screen: the selected step's payload.

use rune::{El, col, el, row, text};
use rune_kit::Kit;

use super::runs::{millis, title, tokens, tone};
use crate::app::App;

fn field(label: &str, value: &str, k: &Kit) -> El {
    col()
        .gap(2.0)
        .child(
            text(label.to_owned())
                .font(k.ty.small)
                .color(k.c.text_faint),
        )
        .child(
            text(value.to_owned())
                .font(k.ty.ui)
                .color(k.c.text_base)
                .wrap()
                .selectable(true),
        )
}

pub fn build(s: &App, k: &Kit) -> El {
    let (Some(run), Some(step)) = (s.runs.get(s.run), s.steps.get(s.record)) else {
        return el();
    };
    let mut out = col()
        .key("details")
        .w_full()
        .padding(16.0)
        .gap(14.0)
        .child(
            row()
                .gap(8.0)
                .child(
                    text(step.kind.label())
                        .font(k.ty.text_xsmall_medium)
                        .color(tone(step.kind, k)),
                )
                .child(
                    text(title(step.kind, &step.name))
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base),
                ),
        )
        .child(field(
            "Model",
            &format!("{} · {}", run.model.provider, run.model.id),
            k,
        ));
    if let (Some(i), Some(o)) = (step.input_tokens, step.output_tokens) {
        out = out.child(field("Tokens", &format!("{i} in · {o} out"), k));
    } else if let Some(t) = tokens(run).filter(|_| step.kind == crate::runs::StepKind::Llm) {
        out = out.child(field("Tokens", &t.to_string(), k));
    }
    if let Some(ms) = step.duration_ms {
        out = out.child(field("Duration", &millis(ms), k));
    }
    for (label, value) in [
        ("Sent", &step.input),
        ("Reply", &step.output),
        ("Thinking", &step.reasoning),
    ] {
        if let Some(v) = value.as_deref().filter(|v| !v.is_empty()) {
            out = out.child(field(label, v, k));
        }
    }
    if step.truncated {
        out = out.child(field("Note", "Long text is cut short in the record.", k));
    }
    out
}
