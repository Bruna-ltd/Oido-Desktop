//! The Workflows page: what has been saved, what a workflow does, and a way to run it and see how
//! the runs went. A workflow is made in a chat (Save as workflow) and shown there as a draft first.

use rune::{Color, El, ScrollSpec, col, el, row, text, text_input};
use rune_kit::{Badge, BadgeVariant, Button, ButtonVariant, ControlSize, Kit};

use crate::app::{App, now};
use crate::workflow::definition::outline;
use crate::workflow::{RunRecord, RunStatus, Saved};

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    if s.wf_editor.is_some() {
        return super::workflow_editor::screen(s, k, layer);
    }
    if s.workflows.is_empty() {
        return empty(k);
    }
    row()
        .key("workflows")
        .w_full()
        .h_full()
        .child(list(s, k))
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

fn empty(k: &Kit) -> El {
    col()
        .key("workflows-empty")
        .w_full()
        .h_full()
        .center()
        .gap(6.0)
        .child(text("No workflows yet").font(k.ty.body_medium).color(k.c.text_base))
        .child(
            text("Have an agent do something with its tools, then press Save as workflow above the chat. It becomes steps you can run again.")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .max_w(460.0)
                .wrap(),
        )
        .child(new_button(k))
}

fn list(s: &App, k: &Kit) -> El {
    col()
        .key("workflow-list")
        .w(320.0)
        .h_full()
        .min_h(0.0)
        .padding(12.0)
        .gap(4.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(new_button(k))
        .children(s.workflows.iter().map(|w| item(s, w, k)))
}

fn item(s: &App, w: &Saved, k: &Kit) -> El {
    let c = &k.c;
    let selected = s.wf_open.as_deref() == Some(w.id.as_str());
    let agent = s
        .roster
        .agent_ref(&w.agent_id)
        .map_or("", |a| a.name.as_str());
    let id = w.id.clone();
    let e = col()
        .key(format!("wf-{}", w.id))
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
        .on_press(move |s: &mut App| s.select_workflow(&id))
        .child(
            text(w.name.clone())
                .font(k.ty.ui_medium)
                .color(c.text_base)
                .truncate(),
        )
        .child(
            text(format!(
                "{agent} · {} steps",
                crate::workflow::editor::step_count(&w.definition)
            ))
            .font(k.ty.small)
            .color(c.text_muted)
            .truncate(),
        );
    k.interactive(e)
}

fn badge(status: RunStatus) -> Badge {
    Badge::new(status.as_str()).variant(match status {
        RunStatus::Succeeded => BadgeVariant::Subtle,
        RunStatus::Running => BadgeVariant::Accent,
        RunStatus::Failed | RunStatus::Stopped => BadgeVariant::Neutral,
    })
}

fn ago(secs: i64) -> String {
    match secs {
        ..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86400),
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
    let c = &k.c;
    let mut out = col()
        .key("wf-run-detail")
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(k.radius.md)
        .fill(c.layer_01);
    if !r.error.is_empty() {
        out = out.child(
            text(format!("Failed at {}: {}", r.failed_step, r.error))
                .font(k.ty.small)
                .color(k.c.warning.icon)
                .wrap()
                .selectable(true),
        );
    }
    for (i, st) in r.steps.iter().enumerate() {
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
