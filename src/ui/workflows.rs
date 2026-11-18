//! The Workflows page: what has been saved, what a workflow does, and a way to run it and see how
//! the runs went. A workflow is made in a chat (Save as workflow) and shown there as a draft first.

use rune::{Color, El, ScrollSpec, col, el, row, text, text_input};
use rune_kit::{Badge, BadgeVariant, Button, ButtonVariant, ControlSize, Kit, SegmentedControl};

use crate::workflow::Sort;

use crate::app::{App, now};
use crate::workflow::definition::outline;
use crate::workflow::pause::Decision;
use crate::workflow::{Pause, RunRecord, RunStatus, Saved};

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    if s.wf_editor.is_some() {
        return super::workflow_editor::screen(s, k, layer);
    }
    if s.workflows.is_empty() {
        return empty(s, k);
    }
    if s.wf_open.is_none() {
        return index(s, k);
    }
    col()
        .key("workflows")
        .w_full()
        .h_full()
        .child(
            row().px(20.0).pt(12.0).child(
                Button::new("← Workflows")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut App| s.close_workflow())
                    .build(k)
                    .key("wf-back"),
            ),
        )
        .child(detail(s, k))
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
        .on_press(move |s: &mut App| s.select_workflow(&id))
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
        .child(tags);
    k.interactive(e)
}

fn badge(status: RunStatus) -> Badge {
    Badge::new(status.as_str()).variant(match status {
        RunStatus::Succeeded => BadgeVariant::Subtle,
        RunStatus::Running | RunStatus::Waiting => BadgeVariant::Accent,
        RunStatus::Failed | RunStatus::Stopped => BadgeVariant::Neutral,
    })
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

fn detail(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let Some(w) = s
        .wf_open
        .as_ref()
        .and_then(|id| s.workflows.iter().find(|w| &w.id == id))
    else {
        return el().grow(1.0);
    };
    let running = s.wf_jobs.contains_key(&w.id);
    let mut out = col()
        .key("workflow-detail")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(20.0)
        .gap(14.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            text(w.name.clone())
                .font(k.ty.body_medium)
                .color(c.text_base),
        );
    // Only the published copy runs for real; the page's Run button runs that one.
    let state = match (w.live(), w.has_unpublished_changes()) {
        (None, _) => {
            "Draft: not published, so agents and schedules cannot run it. Publish it in the editor."
        }
        (Some(_), true) => "Published. The draft has changes that are not published yet.",
        (Some(_), false) => "Published.",
    };
    out = out.child(text(state).font(k.ty.small).color(c.text_muted).wrap());
    if !w.goal.is_empty() {
        out = out.child(
            text(w.goal.clone())
                .font(k.ty.small)
                .color(c.text_muted)
                .wrap(),
        );
    }
    let mut n = 0;
    out = out.children(outline(&w.definition.steps).into_iter().map(|r| {
        // Steps in the first level are numbered; what is inside them is indented under them.
        let label = if r.path.contains('.') {
            r.label
        } else {
            n += 1;
            format!("{n}. {}", r.label)
        };
        text(label)
            .font(k.ty.small)
            .color(c.text_base)
            .wrap()
            .pl(r.depth as f32 * 16.0)
    }));
    if !w.definition.success.is_empty() {
        out = out.child(
            text(format!("Done when: {}", w.definition.success))
                .font(k.ty.small)
                .color(c.text_faint)
                .wrap(),
        );
    }

    // What to give it, then run.
    for (name, edit) in &s.wf_inputs {
        let input = w.definition.inputs.get(name);
        let hint = input.map_or(String::new(), |i| {
            let req = if i.default.is_none() { ", needed" } else { "" };
            if i.description.is_empty() {
                format!("{}{req}", i.kind)
            } else {
                format!("{} ({}{req})", i.description, i.kind)
            }
        });
        let style = rune::TextStyle {
            line_height: 22.0,
            ..k.ty.body
        };
        let n = name.clone();
        out = out.child(
            col()
                .w_full()
                .gap(4.0)
                .child(
                    text(format!("{name}  {hint}"))
                        .font(k.ty.small)
                        .color(c.text_muted),
                )
                .child(
                    col()
                        .w_full()
                        .px(12.0)
                        .py(8.0)
                        .radius(k.radius.md)
                        .border(
                            1.0,
                            if edit.focused {
                                c.border_strong
                            } else {
                                c.border_base
                            },
                        )
                        .child(
                            text_input(edit, style)
                                .key(format!("wf-input-{name}"))
                                .w_full()
                                .color(c.text_base)
                                .caret(c.text_base, k.selection())
                                .label(name.clone())
                                .on_edit(move |s: &mut App| {
                                    let i =
                                        s.wf_inputs.iter().position(|(m, _)| *m == n).unwrap_or(0);
                                    &mut s.wf_inputs[i].1
                                }),
                        ),
                ),
        );
    }
    let mut actions = row().gap(8.0).center_y();
    actions = if running {
        actions.child(
            Button::new("Stop")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.stop_open_workflow())
                .build(k)
                .key("wf-stop"),
        )
    } else {
        actions.child(
            Button::new("Run")
                .variant(ButtonVariant::Submit)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.run_open_workflow())
                .build(k)
                .key("wf-run"),
        )
    };
    actions = actions.child(
        Button::new("Edit")
            .variant(ButtonVariant::Outline)
            .control_size(ControlSize::Small)
            .disabled(running)
            .on_press(|s: &mut App| s.edit_open_workflow())
            .build(k)
            .key("wf-edit"),
    );
    actions = if s.wf_confirm {
        actions
            .child(
                Button::new("Delete it for good")
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut App| s.delete_open_workflow())
                    .build(k)
                    .key("wf-delete-confirm"),
            )
            .child(
                Button::new("Keep it")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut App| s.cancel_delete_workflow())
                    .build(k)
                    .key("wf-delete-cancel"),
            )
    } else {
        actions.child(
            Button::new("Delete")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .disabled(running)
                .on_press(|s: &mut App| s.delete_open_workflow())
                .build(k)
                .key("wf-delete"),
        )
    };
    out = out.child(actions);
    out = out.children(super::guard::waiting(
        s,
        &crate::guard::ask::Where::Workflow(w.id.clone()),
        k,
    ));
    if let Some(why) = &s.wf_notice {
        out = out.child(
            text(why.clone())
                .font(k.ty.small)
                .color(k.c.warning.icon)
                .wrap(),
        );
    }

    if !s.wf_runs.is_empty() {
        out = out.child(text("Runs").font(k.ty.ui_medium).color(c.text_base));
        out = out.children(
            s.wf_runs
                .iter()
                .enumerate()
                .map(|(i, r)| run_row(s, i, r, k)),
        );
        if let Some(r) = s.wf_runs.get(s.wf_run) {
            out = out.child(run_detail(r, k));
        }
    }
    out
}

fn run_row(s: &App, i: usize, r: &RunRecord, k: &Kit) -> El {
    let c = &k.c;
    let tokens = r.input_tokens + r.output_tokens;
    let mut line = format!("{} · by {}", ago(now() - r.started_at), r.started_by);
    if tokens > 0 {
        line.push_str(&format!(" · {tokens} tokens"));
    }
    let e = row()
        .key(format!("wf-run-{}", r.id))
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.sm)
        .fill(if s.wf_run == i {
            c.layer_02
        } else {
            Color::TRANSPARENT
        })
        .on_press(move |s: &mut App| s.select_wf_run(i))
        .child(badge(r.status).build(k))
        .child(text(line).font(k.ty.small).color(c.text_muted).truncate());
    k.interactive(e)
}

fn run_detail(r: &RunRecord, k: &Kit) -> El {
    let failure =
        (!r.error.is_empty()).then(|| format!("Failed at {}: {}", r.failed_step, r.error));
    let log = step_log(&r.steps, failure, "wf-run-detail", k);
    match &r.pause {
        Some(pause) => col()
            .w_full()
            .gap(8.0)
            .child(waiting(r, pause, k))
            .child(log),
        None => log,
    }
}

/// What a waiting run waits for, and the buttons that answer it.
fn waiting(r: &RunRecord, pause: &Pause, k: &Kit) -> El {
    let c = &k.c;
    let asks = pause.until.is_none();
    let line = match pause.until {
        Some(at) => format!(
            "{}. It goes on by itself in {}.",
            pause.prompt,
            until(at - now())
        ),
        None => pause.prompt.clone(),
    };
    let (wf, run) = (r.workflow_id.clone(), r.id.clone());
    let button = |label: &str, key: &str, variant: ButtonVariant| {
        Button::new(label.to_owned())
            .variant(variant)
            .control_size(ControlSize::Small)
            .build(k)
            .key(format!("{key}-{}", r.id))
    };
    let (w1, r1) = (wf.clone(), run.clone());
    let (w2, r2) = (wf.clone(), run.clone());
    let (w3, r3) = (wf, run);
    let mut buttons = row().gap(8.0).center_y();
    buttons = buttons.child(
        button(
            if asks { "Approve" } else { "Go on now" },
            "wf-approve",
            ButtonVariant::Submit,
        )
        .on_press(move |s: &mut App| s.answer_wait(&w1, &r1, Decision::Go, true)),
    );
    if asks {
        buttons = buttons.child(
            button("Decline", "wf-decline", ButtonVariant::Outline)
                .on_press(move |s: &mut App| s.answer_wait(&w2, &r2, Decision::Declined, true)),
        );
    }
    buttons = buttons.child(
        button("Stop it", "wf-stop-wait", ButtonVariant::Ghost)
            .on_press(move |s: &mut App| s.stop_wait(&w3, &r3)),
    );
    col()
        .key("wf-waiting")
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(k.radius.md)
        .border(1.0, c.border_strong)
        .child(
            text(if asks { "Waiting for you" } else { "Waiting" })
                .font(k.ty.ui_medium)
                .color(c.text_base),
        )
        .child(
            text(line)
                .font(k.ty.body)
                .color(c.text_base)
                .wrap()
                .selectable(true),
        )
        .child(buttons)
}

/// How long until `secs` from now, in words.
fn until(secs: i64) -> String {
    match secs {
        ..60 => "under a minute".to_owned(),
        60..3600 => format!("{} min", secs / 60),
        3600..86400 => format!("{} h", secs / 3600),
        _ => format!("{} d", secs / 86400),
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
