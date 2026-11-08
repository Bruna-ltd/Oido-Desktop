//! Runs: the list of agent runs and the ledger of records inside the selected one.

use rune::{Color, El, ScrollSpec, col, el, row, text};
use rune_kit::{Badge, BadgeVariant, Kit};

use crate::app::App;
use crate::model::{RecordKind, Run, RunStatus};
use crate::theme;

pub fn screen(s: &App, k: &Kit) -> El {
    row()
        .key("runs")
        .w_full()
        .h_full()
        .child(list(s, k))
        .child(ledger(s, k))
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
    let status = match r.status {
        RunStatus::Done => Badge::new("done").variant(BadgeVariant::Subtle),
        RunStatus::Running => Badge::new("running").variant(BadgeVariant::Accent),
        RunStatus::Waiting => Badge::new("waiting").variant(BadgeVariant::Accent),
        RunStatus::Failed => Badge::new("failed").variant(BadgeVariant::Neutral),
    };
    let e = col()
        .key(r.id)
        .w_full()
        .px(12.0)
        .py(10.0)
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
                .child(text(r.agent).font(k.ty.ui_medium).color(c.text_base))
                .child(el().grow(1.0))
                .child(status.build(k)),
        )
        .child(
            text(format!(
                "{} · {:.1}s · {} tokens",
                r.started, r.seconds, r.tokens
            ))
            .font(k.ty.small)
            .color(c.text_muted),
        );
    k.interactive(e)
}

fn ledger(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let run = &s.runs[s.run];
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
            text(format!("{} · {}", run.id, run.agent))
                .font(k.ty.ui_medium)
                .color(c.text_base),
        )
        .children(run.records.iter().enumerate().map(|(i, rec)| {
            let selected = s.record == i;
            let e = row()
                .key(format!("rec{i}"))
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
                    text(rec.kind.label())
                        .font(k.ty.text_xsmall_medium)
                        .color(tone(rec.kind, k))
                        .w(84.0),
                )
                .child(
                    text(rec.title)
                        .font(k.ty.small)
                        .color(c.text_base)
                        .truncate(),
                )
                .child(el().grow(1.0))
                .child(
                    text(millis(rec.millis))
                        .font(k.ty.small)
                        .color(c.text_faint),
                );
            k.interactive(e)
        }))
}

/// One hue per record kind, wherever it appears (Studio's trace ledger).
pub fn tone(kind: RecordKind, k: &Kit) -> Color {
    match kind {
        RecordKind::System => k.c.text_faint,
        RecordKind::User => k.c.text_base,
        RecordKind::Context => theme::green(),
        RecordKind::Assistant => theme::violet(),
        RecordKind::Tool | RecordKind::Approval => theme::amber(),
        RecordKind::Subagent => theme::sky(),
    }
}

fn millis(ms: u32) -> String {
    if ms == 0 {
        String::new()
    } else if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f32 / 1000.0)
    }
}
