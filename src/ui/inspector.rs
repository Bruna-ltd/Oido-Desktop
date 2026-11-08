//! Right pane of the Runs screen: the selected record's payload.

use rune::{El, col, row, text};
use rune_kit::Kit;

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
    let run = &s.runs[s.run];
    let rec = &run.records[s.record.min(run.records.len().saturating_sub(1))];
    col()
        .key("details")
        .w_full()
        .padding(16.0)
        .gap(14.0)
        .child(
            row()
                .gap(8.0)
                .child(
                    text(rec.kind.label())
                        .font(k.ty.text_xsmall_medium)
                        .color(super::runs::tone(rec.kind, k)),
                )
                .child(text(rec.title).font(k.ty.ui_medium).color(k.c.text_base)),
        )
        .child(field("Run", run.id, k))
        .child(field("Detail", rec.detail, k))
}
