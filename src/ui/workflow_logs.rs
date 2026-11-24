//! The Logs panel under the graph, as n8n has one: every step that ran as a list in the order it
//! ran (nested the way the workflow is), what each did and how long it took, and beside it the
//! selected one in full: where it is in the workflow, and what it made or why it failed. Pressing a
//! row selects that step on the graph.

use rune::{El, ScrollSpec, col, el, icon, row, text};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit, glyphs};

use super::workflow_graph::picture;
use crate::app::App;
use crate::workflow::editor::{Editor, Loc};
use crate::workflow::engine::StepLog;
use crate::workflow::layout::trail;
use crate::workflow::trace::loc_of;

/// How tall the panel is when open.
const HEIGHT: f32 = 230.0;
/// How wide the list of steps is; the selected step takes the rest.
const LIST_W: f32 = 300.0;
/// The most of a step's output the panel shows.
const OUTPUT_MAX: usize = 4000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Running,
    Waiting,
    Done,
    Failed,
}

fn status(line: &StepLog, waiting: Option<&str>) -> Status {
    if line.error.is_some() {
        Status::Failed
    } else if line.running {
        Status::Running
    } else if waiting == Some(line.id.as_str()) {
        Status::Waiting
    } else {
        Status::Done
    }
}

/// A time as a person reads it: milliseconds under a second, then seconds, then minutes.
pub fn took(ms: u64) -> String {
    match ms {
        0..1000 => format!("{ms} ms"),
        1000..60_000 => format!("{:.1} s", ms as f64 / 1000.0),
        _ => format!("{} min {} s", ms / 60_000, ms % 60_000 / 1000),
    }
}

/// The row the panel shows in full: the one pressed, else the first that failed, else the last.
fn chosen(s: &App, lines: &[StepLog]) -> Option<usize> {
    s.wf_log_pick
        .filter(|i| *i < lines.len())
        .or_else(|| lines.iter().position(|l| l.error.is_some()))
        .or_else(|| lines.len().checked_sub(1))
}

/// Where a step is, in words: "Step 3 › Loop over each name › Run mv <name> docs/".
pub fn place(ed: &Editor, line: &StepLog) -> String {
    let Some(at) = loc_of(ed, &line.path) else {
        return line.what.clone();
    };
    let number = at.place.first().map_or(at.index, |(step, _)| *step) + 1;
    let names: Vec<String> = trail(ed, &at)
        .iter()
        .map(|w| {
            let (cut, more) = crate::runs::clip(w, 36);
            if more { format!("{cut}…") } else { cut }
        })
        .collect();
    format!("Step {number} › {}", names.join(" › "))
}

/// The panel, when there is a run to show or something that stopped one from starting.
pub fn panel(s: &App, ed: &Editor, k: &Kit) -> Option<El> {
    let c = &k.c;
    let shown = s.shown_run();
    let notice = s.wf_test.notice.as_deref();
    if shown.is_none() && notice.is_none() {
        return None;
    }
    let (label, lines, waiting) = match &shown {
        Some(r) => (r.label, r.lines, r.waiting.as_deref()),
        None => ("Test run", &[][..], None),
    };
    let failed = lines.iter().filter(|l| l.error.is_some()).count();
    let total: u64 = lines
        .iter()
        .filter(|l| l.path.is_empty() || !l.path.contains('.'))
        .map(|l| l.ms)
        .sum();
    let summary = match (lines.len(), failed) {
        (0, _) => String::new(),
        (n, 0) => format!("{n} steps · {}", took(total)),
        (n, f) => format!("{n} steps, {f} failed · {}", took(total)),
    };
    let open = s.wf_logs_open;
    let head = row()
        .key("wfl-head")
        .w_full()
        .center_y()
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .child(
            IconButton::new(if open {
                &glyphs::CHEVRON_DOWN
            } else {
                &glyphs::CHEVRON_UP
            })
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label(if open { "Hide logs" } else { "Show logs" })
            .on_press(|s: &mut App| s.wf_logs_open ^= true)
            .build(k)
            .key("wfl-toggle"),
        )
        .child(text("Logs").font(k.ty.ui_medium).color(c.text_base))
        .child(text(label).font(k.ty.small).color(c.text_muted))
        .child(text(summary).font(k.ty.small).color(if failed > 0 {
            c.warning.icon
        } else {
            c.text_muted
        }));
    let mut out = col()
        .key("wfl")
        .w_full()
        .radius(k.radius.md)
        .border(1.0, c.border_base)
        .fill(c.layer_01)
        .child(head);
    if !open {
        return Some(out);
    }
    let pick = chosen(s, lines);
    let mut list = col().w_full().gap(1.0).padding(6.0);
    for (i, line) in lines.iter().enumerate() {
        list = list.child(item(
            s,
            ed,
            i,
            line,
            status(line, waiting),
            pick == Some(i),
            k,
        ));
    }
    let detail = match (pick.and_then(|i| lines.get(i)), notice) {
        (Some(line), _) => detail(ed, line, status(line, waiting), k),
        (None, Some(why)) => col().gap(6.0).padding(12.0).child(
            text(why.to_owned())
                .font(k.ty.body)
                .color(c.warning.icon)
                .wrap()
                .selectable(true),
        ),
        (None, None) => col().padding(12.0).child(
            text("Nothing has run yet.")
                .font(k.ty.small)
                .color(c.text_muted),
        ),
    };
    out = out.child(el().w_full().h(1.0).fill(c.border_muted)).child(
        row()
            .w_full()
            .h(HEIGHT)
            .child(
                col()
                    .w(LIST_W)
                    .h_full()
                    .min_h(0.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(list),
            )
            .child(el().w(1.0).h_full().fill(c.border_muted))
            .child(
                col()
                    .grow(1.0)
                    .h_full()
                    .min_w(0.0)
                    .min_h(0.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(detail),
            ),
    );
    Some(out)
}

/// One step as a row of the list: how deep it is, how it ended, its picture, what it did.
fn item(s: &App, ed: &Editor, i: usize, line: &StepLog, st: Status, picked: bool, k: &Kit) -> El {
    let c = &k.c;
    let mark = match st {
        Status::Done => icon(&glyphs::CHECK_SMALL, 14.0).color(c.text_muted),
        Status::Failed => icon(&glyphs::CIRCLE_X, 14.0).color(c.warning.icon),
        Status::Running => icon(&glyphs::TRIANGLE_RIGHT, 14.0).color(c.bg_accent),
        Status::Waiting => icon(&glyphs::DASH, 14.0).color(c.bg_accent),
    };
    let at: Option<Loc> = loc_of(ed, &line.path);
    let selected_on_graph = at.is_some() && s.selected_step() == at;
    let mut note = row().gap(8.0).center_y();
    if let Some(n) = line.round_note() {
        note = note.child(text(n).font(k.ty.small).color(c.text_faint));
    }
    let e = row()
        .key(format!("wfl-row-{i}"))
        .w_full()
        .gap(8.0)
        .pl(8.0 + line.depth() as f32 * 14.0)
        .pr(8.0)
        .py(5.0)
        .center_y()
        .radius(k.radius.sm)
        .fill(if picked || selected_on_graph {
            c.layer_03
        } else {
            rune::Color::TRANSPARENT
        })
        .role("button")
        .label(format!("{}, {}", line.what, status_word(st)))
        .on_press(move |s: &mut App| {
            s.wf_log_pick = Some(i);
            if let Some(at) = at.clone() {
                s.select_step(Some(at));
            }
        })
        .child(mark)
        .child(icon(picture(&line.kind), 14.0).color(c.text_muted))
        .child(
            text(line.what.clone())
                .font(k.ty.small)
                .color(if st == Status::Failed {
                    c.warning.icon
                } else {
                    c.text_base
                })
                .truncate()
                .grow(1.0),
        )
        .child(note)
        .child(
            text(if line.running {
                "…".to_owned()
            } else {
                took(line.ms)
            })
            .font(k.ty.small)
            .color(c.text_muted),
        );
    k.interactive(e)
}

fn status_word(st: Status) -> &'static str {
    match st {
        Status::Done => "done",
        Status::Failed => "failed",
        Status::Running => "running",
        Status::Waiting => "waiting",
    }
}

/// The selected step in full: where it is, how it ended, and what it made or why it failed.
fn detail(ed: &Editor, line: &StepLog, st: Status, k: &Kit) -> El {
    let c = &k.c;
    let mut how = vec![match st {
        Status::Failed => format!("Failed after {}", took(line.ms)),
        Status::Running => "Still running".to_owned(),
        Status::Waiting => "Waiting here".to_owned(),
        Status::Done => format!("Done in {}", took(line.ms)),
    }];
    if let Some(n) = line.round_note() {
        how.push(n);
    }
    let mut out = col()
        .key("wfl-detail")
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .child(
            text(line.what.clone())
                .font(k.ty.ui_medium)
                .color(c.text_base)
                .wrap(),
        )
        .child(
            text(place(ed, line))
                .font(k.ty.caption())
                .color(c.text_muted)
                .wrap()
                .selectable(true),
        )
        .child(
            text(how.join(" · "))
                .font(k.ty.caption())
                .color(if st == Status::Failed {
                    c.warning.icon
                } else {
                    c.text_muted
                })
                .wrap(),
        );
    match (&line.error, line.running) {
        (Some(why), _) => {
            out = out.child(
                col()
                    .w_full()
                    .gap(4.0)
                    .padding(10.0)
                    .radius(k.radius.md)
                    .border(1.0, c.warning.icon)
                    .child(text("Error").font(k.ty.ui_medium).color(c.warning.icon))
                    .child(
                        text(why.clone())
                            .font(k.ty.mono)
                            .color(c.text_base)
                            .wrap()
                            .selectable(true),
                    ),
            );
        }
        (None, true) => {}
        (None, false) => {
            let (cut, more) = crate::runs::clip(&line.output, OUTPUT_MAX);
            out = out
                .child(text("Output").font(k.ty.small).color(c.text_muted))
                .child(
                    text(if line.output.is_empty() {
                        "Nothing came back.".to_owned()
                    } else if more {
                        format!("{cut}…")
                    } else {
                        cut
                    })
                    .font(k.ty.mono)
                    .color(c.text_base)
                    .wrap()
                    .selectable(true),
                );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_in_the_unit_a_person_would_use() {
        assert_eq!(took(0), "0 ms");
        assert_eq!(took(999), "999 ms");
        assert_eq!(took(1500), "1.5 s");
        assert_eq!(took(61_000), "1 min 1 s");
    }
}
