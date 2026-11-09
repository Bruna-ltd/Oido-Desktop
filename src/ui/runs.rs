//! Runs: the list of agent runs and the ledger of steps inside the selected one.

use rune::{Color, El, ScrollSpec, col, el, row, text};
use rune_kit::{Badge, BadgeVariant, Kit};

use crate::app::{App, now};
use crate::runs::{Run, Status, StepKind};

pub fn screen(s: &App, k: &Kit) -> El {
    if s.runs.is_empty() {
        return empty(k);
    }
    row()
        .key("runs")
        .w_full()
        .h_full()
        .child(list(s, k))
        .child(ledger(s, k))
}

fn empty(k: &Kit) -> El {
    col()
        .key("runs-empty")
        .w_full()
        .h_full()
        .center()
        .gap(6.0)
        .child(
            text("No runs yet")
                .font(k.ty.body_medium)
                .color(k.c.text_base),
        )
        .child(
            text("Send a message to an agent. Each reply is recorded here for 30 days.")
                .font(k.ty.small)
                .color(k.c.text_muted),
        )
}

fn list(s: &App, k: &Kit) -> El {
    col()
        .key("run-list")
        .w(320.0)
        .h_full()
        .min_h(0.0)
        .padding(12.0)
        .gap(4.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .children(s.runs.iter().enumerate().map(|(i, r)| run_row(s, i, r, k)))
}

fn run_row(s: &App, i: usize, r: &Run, k: &Kit) -> El {
    let c = &k.c;
    let selected = s.run == i;
    let e = col()
        .key(format!("run-{}", r.id))
        .w_full()
        .px(12.0)
        .py(10.0)
        .pl(if r.parent.is_some() { 28.0 } else { 12.0 })
        .gap(4.0)
        .radius(k.radius.md)
        .fill(if selected {
            c.layer_02
        } else {
            Color::TRANSPARENT
        })
        .on_press(move |s: &mut App| s.select_run(i))
        .child(
            row()
                .w_full()
                .center_y()
                .child(
                    text(r.agent_name.clone())
                        .font(k.ty.ui_medium)
                        .color(c.text_base)
                        .truncate(),
                )
                .child(el().grow(1.0))
                .child(status_badge(r.status).build(k)),
        )
        .child(
            text(summary(r))
                .font(k.ty.small)
                .color(c.text_muted)
                .truncate(),
        );
    k.interactive(e)
}

pub fn status_badge(s: Status) -> Badge {
    Badge::new(s.label()).variant(match s {
        Status::Ok => BadgeVariant::Subtle,
        Status::Running => BadgeVariant::Accent,
        Status::Error | Status::Cancelled => BadgeVariant::Neutral,
    })
}

/// "5 min ago · 1.2s · 340 tokens".
fn summary(r: &Run) -> String {
    let mut parts = vec![ago(now() - r.started_at)];
    if let Some(ms) = r.duration_ms {
        parts.push(millis(ms));
    }
    if let Some(t) = tokens(r) {
        parts.push(format!("{t} tokens"));
    }
    parts.join(" · ")
}

pub fn tokens(r: &Run) -> Option<u32> {
    match (r.input_tokens, r.output_tokens) {
        (None, None) => None,
        (i, o) => Some(i.unwrap_or(0) + o.unwrap_or(0)),
    }
}

fn ago(secs: i64) -> String {
    match secs {
        ..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86400),
    }
}

fn ledger(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let Some(run) = s.runs.get(s.run) else {
        return el().grow(1.0);
    };
    col()
        .key("ledger")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(12.0)
        .gap(2.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            text(format!("{} · {}", run.agent_name, run.model.id))
                .font(k.ty.ui_medium)
                .color(c.text_base),
        )
        .children(s.steps.iter().enumerate().map(|(i, step)| {
            let selected = s.record == i;
            let e = row()
                .key(format!("step{i}"))
                .w_full()
                .px(12.0)
                .py(8.0)
                .gap(12.0)
                .center_y()
                .radius(k.radius.sm)
                .fill(if selected {
                    c.layer_02
                } else {
                    Color::TRANSPARENT
                })
                .on_press(move |s: &mut App| s.record = i)
                .child(
                    text(step.kind.label())
                        .font(k.ty.text_xsmall_medium)
                        .color(tone(step.kind, k))
                        .w(84.0),
                )
                .child(
                    text(title(step.kind, &step.name))
                        .font(k.ty.small)
                        .color(c.text_base)
                        .truncate(),
                )
                .child(el().grow(1.0))
                .child(
                    text(step.duration_ms.map(millis).unwrap_or_default())
                        .font(k.ty.small)
                        .color(c.text_faint),
                );
            k.interactive(e)
        }))
}

/// What a step is called: its own name, else what its kind means.
pub fn title(kind: StepKind, name: &str) -> String {
    if !name.is_empty() {
        return name.to_owned();
    }
    match kind {
        StepKind::System => "Agent brief",
        StepKind::User => "Your message",
        StepKind::Llm => "Model reply",
        StepKind::Error => "It failed",
    }
    .to_owned()
}

/// One hue per step kind, wherever it appears (Studio's trace ledger).
pub fn tone(kind: StepKind, k: &Kit) -> Color {
    match kind {
        StepKind::System => k.c.text_faint,
        StepKind::User => k.c.text_base,
        StepKind::Llm => k.c.info.icon,
        StepKind::Error => k.c.warning.icon,
    }
}

pub fn millis(ms: u32) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f32 / 1000.0)
    }
}
