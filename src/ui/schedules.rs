//! The Schedules page: every schedule with when it fires next, one open with what it does and how
//! its fires went, and the form that makes or changes one (or an agent's draft, which saving
//! approves).

use chrono::Local;
use rune::{Color, El, ScrollSpec, TextEdit, col, el, row, text, text_input};
use rune_kit::{
    Badge, BadgeVariant, Button, ButtonVariant, ControlSize, Kit, SegmentedControl, Select,
    SelectAppearance,
};

use crate::app::{App, now};
use crate::schedule::timing::{Days, when};
use crate::schedule::{Fire, FireStatus, Schedule, Target};
use crate::schedules::form::{Form, Kind, Source, When};

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let page = &s.schedule_page;
    if s.schedules.is_empty() && page.form.is_none() {
        return empty(k);
    }
    let right = match &page.form {
        Some(f) => form(s, f, k, layer),
        None => detail(s, k),
    };
    row()
        .key("schedules")
        .w_full()
        .h_full()
        .child(list(s, k))
        .child(right)
}

fn new_button(k: &Kit) -> El {
    Button::new("New schedule")
        .variant(ButtonVariant::Outline)
        .control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.new_schedule())
        .build(k)
        .key("schedule-new")
}

fn empty(k: &Kit) -> El {
    col()
        .key("schedules-empty")
        .w_full()
        .h_full()
        .center()
        .gap(8.0)
        .child(text("No schedules yet").font(k.ty.body_medium).color(k.c.text_base))
        .child(
            text("A schedule sends an agent a prompt, or runs one of its workflows, at set times while Oido is open. Make one here, or ask an agent that has the schedule tool.")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .max_w(460.0)
                .wrap(),
        )
        .child(new_button(k))
}

fn list(s: &App, k: &Kit) -> El {
    col()
        .key("schedule-list")
        .w(320.0)
        .h_full()
        .min_h(0.0)
        .padding(12.0)
        .gap(4.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(row().w_full().pb(6.0).child(new_button(k)))
        .children(s.schedules.iter().map(|x| item(s, x, k)))
}

/// "next Tue 11 Nov 2026 09:00", "paused" or "finished".
fn state(x: &Schedule) -> String {
    match (x.enabled, x.next_at) {
        (false, _) => "paused".to_owned(),
        (true, Some(t)) => format!("next {}", when(t, &Local)),
        (true, None) => "finished".to_owned(),
    }
}

fn item(s: &App, x: &Schedule, k: &Kit) -> El {
    let c = &k.c;
    let page = &s.schedule_page;
    let selected = page.form.is_none() && page.open.as_deref() == Some(x.id.as_str());
    let agent = s
        .roster
        .agent_ref(&x.agent_id)
        .map_or("", |a| a.name.as_str());
    let id = x.id.clone();
    let e = col()
        .key(format!("schedule-{}", x.id))
        .w_full()
        .px(12.0)
        .py(10.0)
        .gap(3.0)
        .radius(k.radius.md)
        .fill(if selected {
            c.layer_02
        } else {
            Color::TRANSPARENT
        })
        .on_press(move |s: &mut App| s.select_schedule(&id))
        .child(
            text(x.name.clone())
                .font(k.ty.ui_medium)
                .color(c.text_base)
                .truncate(),
        )
        .child(
            text(format!("{agent} · {}", x.timing.describe(&Local)))
                .font(k.ty.small)
                .color(c.text_muted)
                .truncate(),
        )
        .child(
            text(state(x))
                .font(k.ty.small)
                .color(c.text_faint)
                .truncate(),
        );
    k.interactive(e)
}

/// What a schedule does, in words.
pub fn target_words(s: &App, t: &Target) -> String {
    match t {
        Target::Prompt(p) => format!("Sends: {}", p.trim()),
        Target::Workflow { id, .. } => {
            let name = s
                .workflows
                .iter()
                .find(|w| &w.id == id)
                .map_or("?", |w| w.name.as_str());
            format!("Runs the workflow {name}")
        }
    }
}

fn small_button(label: &str, variant: ButtonVariant) -> Button {
    Button::new(label)
        .variant(variant)
        .control_size(ControlSize::Small)
}

fn detail(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let page = &s.schedule_page;
    let Some(x) = page
        .open
        .as_ref()
        .and_then(|id| s.schedules.iter().find(|x| &x.id == id))
    else {
        return el().grow(1.0);
    };
    let agent = s
        .roster
        .agent_ref(&x.agent_id)
        .map_or("", |a| a.name.as_str());
    let line = |t: String| text(t).font(k.ty.small).color(c.text_base).wrap();
    let mut out = col()
        .key("schedule-detail")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(20.0)
        .gap(10.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            text(x.name.clone())
                .font(k.ty.body_medium)
                .color(c.text_base),
        )
        .child(line(format!("Agent: {agent}")))
        .child(line(target_words(s, &x.target)))
        .child(line(format!("When: {}", x.timing.describe(&Local))))
        .child(text(state(x)).font(k.ty.small).color(c.text_muted));

    let mut actions = row().gap(8.0).center_y().pt(6.0);
    actions = actions
        .child(
            small_button("Edit", ButtonVariant::Outline)
                .on_press(|s: &mut App| s.edit_schedule())
                .build(k)
                .key("schedule-edit"),
        )
        .child(
            small_button(
                if x.enabled { "Pause" } else { "Resume" },
                ButtonVariant::Outline,
            )
            .on_press(|s: &mut App| s.toggle_open_schedule())
            .build(k)
            .key("schedule-toggle"),
        )
        .child(
            small_button("Run now", ButtonVariant::Submit)
                .disabled(s.firing.contains_key(&x.id))
                .on_press(|s: &mut App| s.run_open_schedule())
                .build(k)
                .key("schedule-run"),
        );
    actions = if page.confirm {
        actions
            .child(
                small_button("Delete it for good", ButtonVariant::Outline)
                    .on_press(|s: &mut App| s.delete_open_schedule())
                    .build(k)
                    .key("schedule-delete-confirm"),
            )
            .child(
                small_button("Keep it", ButtonVariant::Ghost)
                    .on_press(|s: &mut App| s.cancel_delete_schedule())
                    .build(k)
                    .key("schedule-delete-cancel"),
            )
    } else {
        actions.child(
            small_button("Delete", ButtonVariant::Ghost)
                .on_press(|s: &mut App| s.delete_open_schedule())
                .build(k)
                .key("schedule-delete"),
        )
    };
    out = out.child(actions);
    if !s.run_schedules {
        out = out.child(
            text("Schedules are turned off in Settings: nothing fires on its own.")
                .font(k.ty.small)
                .color(k.c.warning.icon)
                .wrap(),
        );
    }
    if let Some(why) = &page.notice {
        out = out.child(
            text(why.clone())
                .font(k.ty.small)
                .color(k.c.warning.icon)
                .wrap(),
        );
    }
    if !page.fires.is_empty() {
        out = out
            .child(
                text("Fires")
                    .font(k.ty.ui_medium)
                    .color(c.text_base)
                    .pt(8.0),
            )
            .children(page.fires.iter().map(|f| fire_row(f, k)));
    }
    out
}

fn badge(status: FireStatus) -> Badge {
    Badge::new(status.as_str()).variant(match status {
        FireStatus::Succeeded => BadgeVariant::Subtle,
        FireStatus::Running => BadgeVariant::Accent,
        FireStatus::Failed | FireStatus::Skipped => BadgeVariant::Neutral,
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

/// One fire: how it went, when, and why if it did not; pressing it opens its chat.
fn fire_row(f: &Fire, k: &Kit) -> El {
    let c = &k.c;
    let mut words = ago(now() - f.started_at);
    if !f.note.is_empty() {
        words.push_str(" · ");
        words.push_str(&f.note);
    }
    let mut e = row()
        .key(format!("fire-{}", f.id))
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.sm)
        .child(badge(f.status).build(k))
        .child(text(words).font(k.ty.small).color(c.text_muted).truncate());
    if let Some(sid) = f.session_id.clone() {
        e = e.on_press(move |s: &mut App| s.open_chat(&sid));
        return k.interactive(e);
    }
    e
}

// ---- the form

/// A labelled one-line text field.
fn input(
    label: &str,
    edit: &TextEdit,
    key: &str,
    placeholder: &str,
    k: &Kit,
    f: impl for<'a> Fn(&'a mut App) -> &'a mut TextEdit + 'static,
) -> El {
    let c = &k.c;
    let style = rune::TextStyle {
        line_height: 22.0,
        ..k.ty.body
    };
    col()
        .w_full()
        .gap(4.0)
        .child(text(label.to_owned()).font(k.ty.small).color(c.text_muted))
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
                        .key(format!("schedule-form-{key}"))
                        .w_full()
                        .color(c.text_base)
                        .placeholder(placeholder.to_owned(), c.text_faint)
                        .caret(c.text_base, k.selection())
                        .label(label.to_owned())
                        .on_edit(f),
                ),
        )
}

fn labelled(label: &str, control: El, k: &Kit) -> El {
    col()
        .w_full()
        .gap(4.0)
        .child(
            text(label.to_owned())
                .font(k.ty.small)
                .color(k.c.text_muted),
        )
        .child(control)
}

/// Every agent and team member, by name, with its id.
fn agents(s: &App) -> Vec<(String, String)> {
    s.roster
        .list
        .iter()
        .flat_map(|a| std::iter::once(a).chain(a.subagents.iter()))
        .map(|a| (a.name.clone(), a.id.clone()))
        .collect()
}

fn form(s: &App, f: &Form, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let title = match &f.source {
        Source::New => "New schedule",
        Source::Edit(_) => "Change schedule",
        Source::Draft(_) => "Agent's draft",
    };
    let mut out = col()
        .key("schedule-form")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .max_w(620.0)
        .padding(20.0)
        .gap(14.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(text(title).font(k.ty.body_medium).color(c.text_base))
        .child(input(
            "Name",
            &f.name,
            "name",
            "Morning digest",
            k,
            |s: &mut App| &mut s.schedule_page.form_mut().name,
        ));

    // The agent.
    let people = agents(s);
    let names: Vec<&str> = people.iter().map(|(n, _)| n.as_str()).collect();
    let current = people
        .iter()
        .find(|(_, id)| f.agent_id.as_ref() == Some(id))
        .map(|(n, _)| n.as_str());
    let agent_select = Select::new("schedule-agent", &names)
        .current(current)
        .placeholder("Choose an agent")
        .label("Agent")
        .appearance(SelectAppearance::Field)
        .open(f.agent_open)
        .on_toggle(|s: &mut App| {
            let f = s.schedule_page.form_mut();
            f.agent_open ^= true;
        })
        .on_close(|s: &mut App| s.schedule_page.form_mut().agent_open = false)
        .on_choose(move |s: &mut App, name: &str, _| {
            let id = agents(s)
                .into_iter()
                .find(|(n, _)| n == name)
                .map(|(_, id)| id);
            let f = s.schedule_page.form_mut();
            f.agent_open = false;
            if f.agent_id != id {
                f.agent_id = id;
                f.workflow_id = None;
                f.inputs.clear();
            }
        })
        .build(k, layer);
    out = out.child(labelled("Agent", row().child(agent_select), k));

    // What it does.
    let kinds = ["Send a prompt", "Run a workflow"];
    let picked = match f.kind {
        Kind::Prompt => 0,
        Kind::Workflow => 1,
    };
    out = out.child(labelled(
        "What it does",
        SegmentedControl::new(&kinds, Some(picked))
            .label("What it does")
            .fit()
            .on_pick(|s: &mut App, i: usize, _| {
                s.schedule_page.form_mut().kind =
                    if i == 0 { Kind::Prompt } else { Kind::Workflow };
            })
            .build(k)
            .key("schedule-kind"),
        k,
    ));
    out = match f.kind {
        Kind::Prompt => out.child(input(
            "Prompt",
            &f.prompt,
            "prompt",
            "What the agent is told each time",
            k,
            |s: &mut App| &mut s.schedule_page.form_mut().prompt,
        )),
        Kind::Workflow => {
            let theirs: Vec<(String, String)> = s
                .workflows
                .iter()
                .filter(|w| f.agent_id.as_ref() == Some(&w.agent_id))
                .map(|w| (w.name.clone(), w.id.clone()))
                .collect();
            let names: Vec<&str> = theirs.iter().map(|(n, _)| n.as_str()).collect();
            let current = theirs
                .iter()
                .find(|(_, id)| f.workflow_id.as_ref() == Some(id))
                .map(|(n, _)| n.as_str());
            let pick = Select::new("schedule-workflow", &names)
                .current(current)
                .placeholder(if names.is_empty() {
                    "This agent has no workflows"
                } else {
                    "Choose a workflow"
                })
                .label("Workflow")
                .appearance(SelectAppearance::Field)
                .disabled(names.is_empty())
                .open(f.workflow_open)
                .on_toggle(|s: &mut App| {
                    let f = s.schedule_page.form_mut();
                    f.workflow_open ^= true;
                })
                .on_close(|s: &mut App| s.schedule_page.form_mut().workflow_open = false)
                .on_choose(|s: &mut App, name: &str, _| {
                    let agent = s.schedule_page.form_mut().agent_id.clone();
                    let id = s
                        .workflows
                        .iter()
                        .find(|w| w.name == name && Some(&w.agent_id) == agent.as_ref())
                        .map(|w| w.id.clone());
                    let workflows = s.workflows.clone();
                    let f = s.schedule_page.form_mut();
                    f.workflow_open = false;
                    if let Some(id) = id {
                        f.pick_workflow(&id, &workflows);
                    }
                })
                .build(k, layer);
            let mut out = out.child(labelled("Workflow", row().child(pick), k));
            for (name, edit) in &f.inputs {
                let n = name.clone();
                out = out.child(input(
                    name,
                    edit,
                    &format!("input-{name}"),
                    "",
                    k,
                    move |s: &mut App| {
                        let f = s.schedule_page.form_mut();
                        if !f.inputs.iter().any(|(m, _)| *m == n) {
                            f.inputs.push((n.clone(), TextEdit::new("")));
                        }
                        let i = f.inputs.iter().position(|(m, _)| *m == n).unwrap_or(0);
                        &mut f.inputs[i].1
                    },
                ));
            }
            out
        }
    };

    // When it fires.
    let labels: Vec<&str> = When::ALL.iter().map(|w| w.label()).collect();
    let at = When::ALL.iter().position(|w| *w == f.when);
    out = out.child(labelled(
        "When",
        SegmentedControl::new(&labels, at)
            .label("When")
            .fit()
            .on_pick(|s: &mut App, i: usize, _| {
                if let Some(w) = When::ALL.get(i) {
                    s.schedule_page.form_mut().when = *w;
                }
            })
            .build(k)
            .key("schedule-when"),
        k,
    ));
    out = match f.when {
        When::Days => out
            .child(input("At", &f.at, "at", "09:00", k, |s: &mut App| {
                &mut s.schedule_page.form_mut().at
            }))
            .child(labelled("On", days(f.days, k), k)),
        When::Every => out.child(input(
            "Every (minutes)",
            &f.every,
            "every",
            "60",
            k,
            |s: &mut App| &mut s.schedule_page.form_mut().every,
        )),
        When::Once => out.child(input(
            "On",
            &f.once,
            "once",
            "2026-11-12 09:00",
            k,
            |s: &mut App| &mut s.schedule_page.form_mut().once,
        )),
        When::Cron => out.child(input(
            "Cron (minute hour day month weekday)",
            &f.cron,
            "cron",
            "0 9 * * 1-5",
            k,
            |s: &mut App| &mut s.schedule_page.form_mut().cron,
        )),
    };

    // What it would do, as read so far.
    if let Ok((_, _, _, timing)) = f.read(&s.workflows, now(), &Local) {
        let next = timing.next_after(now(), &Local).map_or_else(
            || "its time has passed".to_owned(),
            |t| format!("next {}", when(t, &Local)),
        );
        out = out.child(
            text(format!("{} · {next}", timing.describe(&Local)))
                .font(k.ty.small)
                .color(c.text_muted)
                .wrap(),
        );
    }
    if let Some(why) = &f.notice {
        out = out.child(
            text(why.clone())
                .font(k.ty.small)
                .color(k.c.warning.icon)
                .wrap(),
        );
    }
    let save = match f.source {
        Source::Draft(_) => "Approve",
        _ => "Save",
    };
    out.child(
        row()
            .gap(8.0)
            .child(
                Button::new(save)
                    .variant(ButtonVariant::Submit)
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut App| s.save_schedule_form())
                    .build(k)
                    .key("schedule-save"),
            )
            .child(
                Button::new("Cancel")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut App| s.cancel_schedule_form())
                    .build(k)
                    .key("schedule-cancel"),
            ),
    )
}

/// A toggle per weekday, Monday first.
fn days(chosen: Days, k: &Kit) -> El {
    row().gap(6.0).children(Days::week().map(|(d, name)| {
        Button::new(name)
            .variant(if chosen.contains(d) {
                ButtonVariant::Submit
            } else {
                ButtonVariant::Outline
            })
            .control_size(ControlSize::Small)
            .on_press(move |s: &mut App| s.schedule_page.form_mut().days.toggle(d))
            .build(k)
            .key(format!("schedule-day-{d}"))
    }))
}
