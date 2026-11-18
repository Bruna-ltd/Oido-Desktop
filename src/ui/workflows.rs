//! The Workflows page: what has been saved, what a workflow does, and a way to run it and see how
//! the runs went. A workflow is made in a chat (Save as workflow) and shown there as a draft first.

use rune::{El, ScrollSpec, col, el, row, text, text_input};
use rune_kit::{Badge, BadgeVariant, Button, ButtonVariant, ControlSize, Kit, SegmentedControl};

use crate::workflow::Sort;

use crate::app::{App, now};
use crate::workflow::Saved;

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    if s.wf_editor.is_some() {
        return super::workflow_editor::screen(s, k, layer);
    }
    if s.workflows.is_empty() {
        return empty(s, k);
    }
    index(s, k)
}

fn new_button(k: &Kit) -> El {
    Button::new("New workflow")
        .variant(ButtonVariant::Outline)
        .control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.new_workflow())
        .build(k)
        .key("wf-new")
}

fn examples_button(k: &Kit) -> El {
    Button::new("Add examples")
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.add_examples())
        .build(k)
        .key("wf-examples")
}

/// What the last press said, if anything.
fn notice(s: &App, k: &Kit) -> Option<El> {
    s.wf_notice.as_ref().map(|why| {
        text(why.clone())
            .font(k.ty.small)
            .color(k.c.text_muted)
            .max_w(460.0)
            .wrap()
    })
}

fn empty(s: &App, k: &Kit) -> El {
    col()
        .key("workflows-empty")
        .w_full()
        .h_full()
        .center()
        .gap(6.0)
        .child(text("No workflows yet").font(k.ty.body_medium).color(k.c.text_base))
        .child(
            text("Have an agent do something with its tools, then press Save as workflow above the chat. It becomes steps you can run again. Or add a few examples to try the builder on.")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .max_w(460.0)
                .wrap(),
        )
        .child(
            row()
                .gap(8.0)
                .child(new_button(k))
                .child(examples_button(k)),
        )
        .children(notice(s, k))
}

/// The page of workflows: a title and the way to make one, a search and an order, then a card
/// for each, in a column of its own width.
fn index(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let shown = s.wf_sort.of(&s.workflows, &s.wf_search.text);
    let style = rune::TextStyle {
        line_height: 20.0,
        ..k.ty.body
    };
    let search = row()
        .w(220.0)
        .px(12.0)
        .py(6.0)
        .radius(k.radius.md)
        .border(
            1.0,
            if s.wf_search.focused {
                c.border_strong
            } else {
                c.border_base
            },
        )
        .child(
            text_input(&s.wf_search, style)
                .key("wf-search")
                .w_full()
                .color(c.text_base)
                .placeholder("Search".to_owned(), c.text_faint)
                .caret(c.text_base, k.selection())
                .label("Search workflows".to_owned())
                .on_edit(|s: &mut App| &mut s.wf_search),
        );
    let labels: Vec<&str> = Sort::ALL.iter().map(|o| o.label()).collect();
    let order = SegmentedControl::new(&labels, Sort::ALL.iter().position(|o| *o == s.wf_sort))
        .label("Sort by")
        .on_pick(|s: &mut App, i, _| s.wf_sort = Sort::ALL[i.min(Sort::ALL.len() - 1)])
        .build(k)
        .key("wf-sort");
    let mut cards = col().w_full().gap(8.0);
    if shown.is_empty() {
        cards = cards.child(
            text("No workflows match.")
                .font(k.ty.small)
                .color(c.text_muted),
        );
    }
    cards = cards.children(shown.iter().map(|w| item(s, w, k)));
    let total = if shown.len() == s.workflows.len() {
        format!("Total {}", s.workflows.len())
    } else {
        format!("{} of {}", shown.len(), s.workflows.len())
    };
    col()
        .key("workflows")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .key("workflow-list")
                .w_full()
                .max_w(820.0)
                .self_center()
                .padding(24.0)
                .gap(16.0)
                .child(
                    row()
                        .w_full()
                        .center_y()
                        .gap(8.0)
                        .child(
                            col()
                                .gap(2.0)
                                .child(
                                    text("Workflows")
                                        .font(k.ty.body_medium)
                                        .color(c.text_base),
                                )
                                .child(
                                    text("Steps an agent runs again, by hand, on a schedule or when asked")
                                        .font(k.ty.small)
                                        .color(c.text_muted),
                                ),
                        )
                        .child(el().grow(1.0))
                        .child(examples_button(k))
                        .child(
                            Button::new("New workflow")
                                .variant(ButtonVariant::Submit)
                                .control_size(ControlSize::Small)
                                .on_press(|s: &mut App| s.new_workflow())
                                .build(k)
                                .key("wf-new"),
                        ),
                )
                .child(
                    row()
                        .w_full()
                        .center_y()
                        .gap(8.0)
                        .child(search)
                        .child(el().grow(1.0))
                        .child(order),
                )
                .children(notice(s, k))
                .child(cards)
                .child(
                    row()
                        .w_full()
                        .justify_end()
                        .child(text(total).font(k.ty.small).color(c.text_muted)),
                ),
        )
}

/// One workflow as a card: its name, how long ago it was made and published, who runs it and how
/// many steps it has, and whether it is live.
fn item(s: &App, w: &Saved, k: &Kit) -> El {
    let c = &k.c;
    let agent = s
        .roster
        .agent_ref(&w.agent_id)
        .map_or("", |a| a.name.as_str());
    let id = w.id.clone();
    let mut meta = Vec::new();
    if let Some(p) = &w.published {
        meta.push(format!("Published {}", ago(now() - p.at)));
    }
    meta.push(format!("Created {}", ago(now() - w.created_at)));
    if !agent.is_empty() {
        meta.push(agent.to_owned());
    }
    meta.push(format!(
        "{} steps",
        crate::workflow::editor::step_count(&w.definition)
    ));
    let state = match (w.live(), w.has_unpublished_changes()) {
        (None, _) => ("Draft", BadgeVariant::Neutral),
        (Some(_), true) => ("Unpublished changes", BadgeVariant::Accent),
        (Some(_), false) => ("Published", BadgeVariant::Subtle),
    };
    let mut tags = row().center_y().gap(8.0);
    if w.live().is_some_and(|d| !d.trigger.is_manual()) {
        tags = tags.child(
            Badge::new("Schedule")
                .variant(BadgeVariant::Neutral)
                .build(k),
        );
    }
    tags = tags.child(Badge::new(state.0).variant(state.1).build(k));
    let e = row()
        .key(format!("wf-{}", w.id))
        .w_full()
        .px(16.0)
        .py(12.0)
        .gap(12.0)
        .center_y()
        .radius(k.radius.md)
        .border(1.0, c.border_base)
        .fill(c.layer_01)
        .on_press(move |s: &mut App| s.open_in_builder(&id))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(
                    text(w.name.clone())
                        .font(k.ty.ui_medium)
                        .color(c.text_base)
                        .truncate(),
                )
                .child(
                    text(meta.join(" · "))
                        .font(k.ty.small)
                        .color(c.text_muted)
                        .truncate(),
                ),
        )
        .child(tags)
        .child(actions(s, w, k));
    k.interactive(e)
}

/// Run (or Stop), Edit and Delete for one card. Delete asks a second time on the card it was
/// pressed on.
fn actions(s: &App, w: &Saved, k: &Kit) -> El {
    let running = s.wf_jobs.contains_key(&w.id);
    let asking = s.wf_confirm && s.wf_open.as_ref() == Some(&w.id);
    let small = |label: &str, variant: ButtonVariant, key: &str| {
        Button::new(label.to_owned())
            .variant(variant)
            .control_size(ControlSize::Small)
            .build(k)
            .key(format!("{key}-{}", w.id))
    };
    let id = w.id.clone();
    let mut out = row().gap(4.0).center_y();
    let stop_id = id.clone();
    out = out.child(if running {
        small("Stop", ButtonVariant::Outline, "wf-stop").on_press(move |s: &mut App| {
            s.select_workflow(&stop_id);
            s.stop_open_workflow();
        })
    } else {
        let run_id = id.clone();
        small("Run", ButtonVariant::Outline, "wf-run").on_press(move |s: &mut App| {
            s.select_workflow(&run_id);
            s.run_open_workflow();
        })
    });
    let edit_id = id.clone();
    out = out.child(
        small("Edit", ButtonVariant::Ghost, "wf-edit")
            .disabled(running)
            .on_press(move |s: &mut App| s.open_in_builder(&edit_id)),
    );
    if asking {
        out.child(
            small(
                "Delete it for good",
                ButtonVariant::Outline,
                "wf-delete-confirm",
            )
            .on_press(|s: &mut App| s.delete_open_workflow()),
        )
        .child(
            small("Keep it", ButtonVariant::Ghost, "wf-delete-cancel")
                .on_press(|s: &mut App| s.cancel_delete_workflow()),
        )
    } else {
        out.child(
            small("Delete", ButtonVariant::Ghost, "wf-delete")
                .disabled(running)
                .on_press(move |s: &mut App| {
                    s.wf_open = Some(id.clone());
                    s.delete_open_workflow();
                }),
        )
    }
}

fn ago(secs: i64) -> String {
    match secs {
        ..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        86400..5184000 => format!("{} d ago", secs / 86400),
        _ => format!("{} mo ago", secs / 2592000),
    }
}

/// The steps of a run as they ran, nested the way the workflow is, with each one's output.
pub(super) fn step_log(
    steps: &[crate::workflow::engine::StepLog],
    failure: Option<String>,
    key: &str,
    k: &Kit,
) -> El {
    let c = &k.c;
    let mut out = col()
        .key(key.to_owned())
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(k.radius.md)
        .fill(c.layer_01);
    if let Some(failure) = failure {
        out = out.child(
            text(failure)
                .font(k.ty.small)
                .color(k.c.warning.icon)
                .wrap()
                .selectable(true),
        );
    }
    for (i, st) in steps.iter().enumerate() {
        let note = st
            .round_note()
            .map_or(String::new(), |n| format!("  ({n})"));
        let indent = st.depth() as f32 * 16.0;
        out = out.child(
            text(format!("{}. {}{note}", i + 1, st.what))
                .font(k.ty.small)
                .color(c.text_base)
                .wrap()
                .pl(indent),
        );
        let shown = st.error.clone().unwrap_or_else(|| st.output.clone());
        if !shown.is_empty() {
            let (cut, more) = crate::runs::clip(&shown, 800);
            out = out.child(
                text(if more { format!("{cut}…") } else { cut })
                    .font(k.ty.mono)
                    .color(if st.error.is_some() {
                        k.c.warning.icon
                    } else {
                        c.text_faint
                    })
                    .wrap()
                    .pl(indent)
                    .selectable(true),
            );
        }
    }
    out
}
