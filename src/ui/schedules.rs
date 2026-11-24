//! The Schedules page: every schedule with when it fires next, one open with what it does and how
//! its fires went, and the form that makes or changes one (or an agent's draft, which saving
//! approves).

use chrono::{Datelike, Local, TimeZone};
use rune::{Color, El, ScrollSpec, TextEdit, col, el, icon, row, text, text_input};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Divider, Kit, SegmentedControl, Select, SelectAppearance,
    glyphs,
};

use super::icons;

use crate::app::{App, now};
use crate::schedule::timing::Days;
use crate::schedule::{Fire, FireStatus, Schedule, Target};
use crate::schedules::form::{Form, Kind, Source, TimingForm, When};

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
        .variant(ButtonVariant::Submit)
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
        .gap(12.0)
        .child(
            col()
                .w(56.0)
                .h(56.0)
                .center()
                .radius(k.radius.lg)
                .fill(k.c.layer_01)
                .border(1.0, k.c.border_muted)
                .child(icon(&icons::SCHEDULED, 26.0).color(k.c.text_muted)),
        )
        .child(text("Nothing scheduled yet").font(crate::theme::ty::heading(k)).color(k.c.text_base))
        .child(
            text("A schedule gives an agent a job at set times: send it a prompt, or run one of its workflows. Oido has to be open for it to run. You can make one here, or ask an agent to set it up.")
                .font(k.ty.body)
                .color(k.c.text_muted)
                .max_w(440.0)
                .align(rune::Align::Center)
                .wrap(),
        )
        .child(el().h(4.0))
        .child(new_button(k))
}

fn list(s: &App, k: &Kit) -> El {
    let n = s.schedules.len();
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
        .child(
            row()
                .w_full()
                .center_y()
                .px(4.0)
                .pb(8.0)
                .child(
                    text(match n {
                        1 => "1 schedule".to_owned(),
                        n => format!("{n} schedules"),
                    })
                    .font(k.ty.small)
                    .color(k.c.text_muted),
                )
                .child(el().grow(1.0))
                .child(new_button(k)),
        )
        .children(s.schedules.iter().map(|x| item(s, x, k)))
}

/// A prompt longer than this many characters or lines is cut short until asked for.
const PROMPT_CHARS: usize = 240;
const PROMPT_LINES: usize = 4;

fn dot(color: Color) -> El {
    el().w(8.0).h(8.0).radius(4.0).fill(color)
}

/// "Today 08:30", "Tomorrow 08:30", "Fri 20 Nov 08:30"; the year only when it is not this one.
fn soon(t: i64) -> String {
    let Some(at) = Local.timestamp_opt(t, 0).earliest() else {
        return t.to_string();
    };
    let clock = at.format("%H:%M");
    let Some(today) = Local
        .timestamp_opt(now(), 0)
        .earliest()
        .map(|d| d.date_naive())
    else {
        return format!("{} {clock}", at.format("%a %-d %b %Y"));
    };
    let day = at.date_naive();
    if day == today {
        format!("Today {clock}")
    } else if today.succ_opt() == Some(day) {
        format!("Tomorrow {clock}")
    } else if day.year() == today.year() {
        format!("{} {clock}", at.format("%a %-d %b"))
    } else {
        format!("{} {clock}", at.format("%a %-d %b %Y"))
    }
}

/// Where a schedule stands: a word, the line under it, and the colour of its dot.
fn state(x: &Schedule, k: &Kit) -> (&'static str, String, Color) {
    match (x.enabled, x.next_at) {
        (false, _) => (
            "Paused",
            "Won't run until you resume it".to_owned(),
            k.c.text_faint,
        ),
        (true, Some(t)) => ("Active", format!("Next run {}", soon(t)), k.c.success.icon),
        (true, None) => ("Finished", "Has no more runs".to_owned(), k.c.text_faint),
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
    let (word, next, ink) = state(x, k);
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
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(dot(ink).label(word))
                .child(
                    text(x.name.clone())
                        .font(k.ty.ui_medium)
                        .color(c.text_base)
                        .truncate(),
                ),
        )
        .child(
            text(format!("{agent} · {}", x.timing.describe(&Local)))
                .font(k.ty.small)
                .color(c.text_muted)
                .truncate()
                .pl(16.0),
        )
        .child(
            text(next)
                .font(k.ty.small)
                .color(c.text_faint)
                .truncate()
                .pl(16.0),
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

/// A bordered group on the page's surface.
fn card(k: &Kit) -> El {
    col()
        .w_full()
        .radius(k.radius.lg)
        .border(1.0, k.c.border_muted)
        .fill(k.c.layer_01)
}

/// A note in the page's warning colour, with a rule to its left.
fn notice(why: &str, k: &Kit) -> El {
    row()
        .w_full()
        .gap(10.0)
        .px(14.0)
        .py(10.0)
        .radius(k.radius.md)
        .fill(k.c.warning.bg)
        .border(1.0, k.c.warning.border)
        .child(icon(&glyphs::CIRCLE_X, 14.0).color(k.c.warning.icon))
        .child(
            text(why.to_owned())
                .font(k.ty.caption())
                .color(k.c.warning.fg)
                .wrap()
                .grow(1.0),
        )
}

/// One fact: what it is, then its value.
fn fact(label: &str, value: El, k: &Kit) -> El {
    row()
        .w_full()
        .gap(16.0)
        .px(16.0)
        .py(12.0)
        .child(
            text(label.to_owned())
                .font(k.ty.caption())
                .color(k.c.text_muted)
                .pt(3.0)
                .w(88.0),
        )
        .child(value.grow(1.0).min_w(0.0))
}

fn value(t: impl Into<String>, k: &Kit) -> El {
    text(t.into()).font(k.ty.body).color(k.c.text_base).wrap()
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
    let (word, next, ink) = state(x, k);

    let title = col()
        .w_full()
        .gap(6.0)
        .child(
            text(x.name.clone())
                .font(crate::theme::ty::heading(k))
                .color(c.text_base)
                .wrap(),
        )
        .child(
            row().gap(8.0).center_y().child(dot(ink)).child(
                text(format!("{word} · {}", next.to_lowercase()))
                    .font(k.ty.small)
                    .color(c.text_muted),
            ),
        );

    let mut actions = row().w_full().gap(8.0).center_y().wrap();
    actions = actions.child(
        small_button("Run now", ButtonVariant::Submit)
            .disabled(s.firing.contains_key(&x.id))
            .on_press(|s: &mut App| s.run_open_schedule())
            .build(k)
            .key("schedule-run"),
    );
    actions = actions.child(
        small_button(
            if x.enabled { "Pause" } else { "Resume" },
            ButtonVariant::Outline,
        )
        .on_press(|s: &mut App| s.toggle_open_schedule())
        .build(k)
        .key("schedule-toggle"),
    );
    actions = actions.children((!x.from_trigger).then(|| {
        small_button("Edit", ButtonVariant::Outline)
            .on_press(|s: &mut App| s.edit_schedule())
            .build(k)
            .key("schedule-edit")
    }));
    actions = if x.from_trigger {
        actions
    } else if page.confirm {
        actions
            .child(el().grow(1.0))
            .child(
                small_button("Yes, delete it", ButtonVariant::Outline)
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
        actions.child(el().grow(1.0)).child(
            small_button("Delete", ButtonVariant::Ghost)
                .on_press(|s: &mut App| s.delete_open_schedule())
                .build(k)
                .key("schedule-delete"),
        )
    };

    let does = match &x.target {
        Target::Prompt(p) => {
            let p = p.trim();
            let long = p.chars().count() > PROMPT_CHARS || p.lines().count() > PROMPT_LINES;
            let open = page.prompt_open;
            let mut body = text(p.to_owned()).font(k.ty.body).color(c.text_base).wrap();
            if long && !open {
                body = body.max_lines(PROMPT_LINES);
            }
            let mut quote = col().w_full().gap(6.0).child(body);
            if long {
                quote = quote.child(
                    small_button(
                        if open { "Show less" } else { "Show all" },
                        ButtonVariant::Ghost,
                    )
                    .on_press(|s: &mut App| s.schedule_page.prompt_open ^= true)
                    .build(k)
                    .key("schedule-prompt-more"),
                );
            }
            ("Sends this prompt", quote)
        }
        Target::Workflow { .. } => (
            "Runs a workflow",
            col().w_full().child(value(
                target_words(s, &x.target)
                    .trim_start_matches("Runs the workflow ")
                    .to_owned(),
                k,
            )),
        ),
    };
    let facts = card(k)
        .child(fact("Agent", value(agent, k), k))
        .child(Divider::horizontal().build(k))
        .child(fact(does.0, does.1, k))
        .child(Divider::horizontal().build(k))
        .child(fact("Repeats", value(x.timing.describe(&Local), k), k))
        .child(Divider::horizontal().build(k))
        .child(fact(
            "Next run",
            value(
                match (x.enabled, x.next_at) {
                    (_, None) => "None".to_owned(),
                    (true, Some(t)) => soon(t),
                    (false, Some(t)) => format!("{} (paused)", soon(t)),
                },
                k,
            ),
            k,
        ));

    let mut out = col()
        .key("schedule-detail")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(28.0)
        .gap(20.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(title)
        .child(actions);
    if !s.run_schedules {
        out = out.child(notice(
            "Schedules are turned off in Settings, so nothing runs on its own. Run now still works.",
            k,
        ));
    }
    if let Some(why) = &page.notice {
        out = out.child(notice(why, k));
    }
    if x.from_trigger {
        out = out.child(
            text("This schedule belongs to a workflow's schedule trigger. To change it, edit the workflow and publish it again.")
                .font(k.ty.caption())
                .color(c.text_muted)
                .wrap(),
        );
    }
    out = out.child(facts);
    out.child(history(page.fires.as_slice(), k))
}

/// How the recent fires went, newest first.
fn history(fires: &[Fire], k: &Kit) -> El {
    let head = text("Recent runs")
        .font(k.ty.ui_medium)
        .color(k.c.text_base);
    if fires.is_empty() {
        return col().w_full().gap(10.0).child(head).child(
            text("It hasn't run yet. Use Run now to try it.")
                .font(k.ty.caption())
                .color(k.c.text_muted)
                .wrap(),
        );
    }
    let mut list = card(k);
    let last = fires.len() - 1;
    for (i, f) in fires.iter().enumerate() {
        list = list.child(fire_row(f, k));
        if i != last {
            list = list.child(Divider::horizontal().build(k));
        }
    }
    col().w_full().gap(10.0).child(head).child(list)
}

fn ago(secs: i64) -> String {
    match secs {
        ..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        86400..172800 => "yesterday".to_owned(),
        _ => format!("{} days ago", secs / 86400),
    }
}

/// One fire: how it went, when, and why if it did not; pressing it opens its chat.
fn fire_row(f: &Fire, k: &Kit) -> El {
    let c = &k.c;
    let (mark, word) = match f.status {
        FireStatus::Succeeded => (
            icon(&glyphs::CHECK_SMALL, 16.0).color(c.success.icon),
            "Succeeded",
        ),
        FireStatus::Failed => (icon(&glyphs::CIRCLE_X, 16.0).color(c.danger.icon), "Failed"),
        FireStatus::Skipped => (icon(&glyphs::DASH, 16.0).color(c.text_faint), "Skipped"),
        FireStatus::Running => (
            icon(&glyphs::CHECK_SMALL, 16.0).color(c.text_faint),
            "Running",
        ),
    };
    let mut body = col().grow(1.0).min_w(0.0).gap(2.0).child(
        row()
            .gap(8.0)
            .center_y()
            .child(text(word).font(k.ty.ui_medium).color(c.text_base))
            .child(
                text(ago(now() - f.started_at))
                    .font(k.ty.small)
                    .color(c.text_muted),
            ),
    );
    if !f.note.is_empty() {
        body = body.child(
            text(f.note.clone())
                .font(k.ty.caption())
                .color(if f.status == FireStatus::Failed {
                    c.danger.fg
                } else {
                    c.text_muted
                })
                .wrap(),
        );
    }
    let mut e = row()
        .key(format!("fire-{}", f.id))
        .w_full()
        .px(16.0)
        .py(12.0)
        .gap(12.0)
        .child(mark)
        .child(body);
    if let Some(sid) = f.session_id.clone() {
        e = e
            .child(text("Open chat").font(k.ty.small).color(c.text_muted))
            .help("See what the agent did in this run")
            .on_press(move |s: &mut App| s.open_chat(&sid));
        return k.interactive(e);
    }
    e
}

// ---- the form

/// How wide the form's column grows.
const FORM_WIDTH: f32 = 640.0;

/// A labelled one-line text field.
pub(super) fn input(
    label: &str,
    edit: &TextEdit,
    key: &str,
    placeholder: &str,
    k: &Kit,
    f: impl for<'a> Fn(&'a mut App) -> &'a mut TextEdit + 'static,
) -> El {
    let c = &k.c;
    let style = crate::theme::ty::field(k);
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

pub(super) fn labelled(label: &str, control: El, k: &Kit) -> El {
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
    let (title, about) = match &f.source {
        Source::New => (
            "New schedule",
            "Pick who does the job, what it is, and when it happens.",
        ),
        Source::Edit(_) => ("Change schedule", "Your changes apply to its next run."),
        Source::Draft(_) => (
            "Agent's draft",
            "An agent suggested this. Check it, change what you like, then approve it to start it.",
        ),
    };
    let head = col()
        .w_full()
        .gap(4.0)
        .child(
            text(title)
                .font(crate::theme::ty::heading(k))
                .color(c.text_base),
        )
        .child(text(about).font(k.ty.caption()).color(c.text_muted).wrap());
    let mut out = col().w_full().gap(14.0).child(input(
        "Name",
        &f.name,
        "name",
        "e.g. Morning briefing",
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
            "What the agent should do each time, e.g. Summarise overnight news about our competitors",
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

    let what = card(k).padding(16.0).gap(14.0).child(out);

    // When it fires.
    let when_card = card(k).padding(16.0).gap(14.0).child(timing_fields(
        &f.timing,
        "schedule-",
        k,
        |s: &mut App| &mut s.schedule_page.form_mut().timing,
    ));

    // What it would do, as read so far.
    let preview = f
        .read(&s.workflows, now(), &Local)
        .ok()
        .map(|(_, _, _, timing)| {
            let next = timing.next_after(now(), &Local).map_or_else(
                || "its time has passed".to_owned(),
                |t| format!("next run {}", soon(t)),
            );
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(10.0)
                .center_y()
                .child(icon(&icons::SCHEDULED, 16.0).color(c.text_muted))
                .child(
                    text(format!("Runs {}, {next}", timing.describe(&Local)))
                        .font(k.ty.caption())
                        .color(c.text_base)
                        .wrap()
                        .grow(1.0),
                )
        });
    let save = match f.source {
        Source::Draft(_) => "Approve and start",
        _ => "Save schedule",
    };
    let buttons = row()
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
        );
    let fields = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .padding(28.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .max_w(FORM_WIDTH)
                .gap(20.0)
                .child(head)
                .child(section("Task", what, k))
                .child(section("When it runs", when_card, k)),
        );
    // What it would do and the buttons that start it stay in sight however long the form is.
    let footer = col().w_full().px(28.0).py(14.0).child(
        col()
            .w_full()
            .max_w(FORM_WIDTH)
            .gap(12.0)
            .children(f.notice.as_ref().map(|why| notice(why, k)))
            .child(
                row()
                    .w_full()
                    .gap(16.0)
                    .center_y()
                    .child(preview.unwrap_or_else(|| el().grow(1.0)))
                    .child(buttons),
            ),
    );
    col()
        .key("schedule-form")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .child(fields)
        .child(Divider::horizontal().build(k))
        .child(footer)
}

fn section(title: &str, body: El, k: &Kit) -> El {
    col()
        .w_full()
        .gap(8.0)
        .child(
            text(title.to_owned())
                .font(k.ty.ui_medium)
                .color(k.c.text_base),
        )
        .child(body)
}

/// A field with its line of help just under it, so the two read as one.
fn hinted(field: El, t: &str, k: &Kit) -> El {
    col().w_full().gap(6.0).child(field).child(
        text(t.to_owned())
            .font(k.ty.caption())
            .color(k.c.text_faint)
            .wrap(),
    )
}

/// How wide a box for a time or a number is: its value is a few characters.
const SHORT_FIELD: f32 = 160.0;
/// How wide a box for a date and time, or a cron line, is.
const MEDIUM_FIELD: f32 = 280.0;

/// What a timing is made of, as boxes: how often, then what that takes. `form` finds the
/// [`TimingForm`] being edited; `prefix` keeps the boxes' keys apart from another form's.
pub(super) fn timing_fields(
    f: &TimingForm,
    prefix: &str,
    k: &Kit,
    form: fn(&mut App) -> &mut TimingForm,
) -> El {
    let key = |name: &str| format!("{prefix}{name}");
    let labels: Vec<&str> = When::ALL.iter().map(|w| w.label()).collect();
    let at = When::ALL.iter().position(|w| *w == f.when);
    let out = col().w_full().gap(14.0).child(labelled(
        "When",
        SegmentedControl::new(&labels, at)
            .label("When")
            .fit()
            .on_pick(move |s: &mut App, i: usize, _| {
                if let Some(w) = When::ALL.get(i) {
                    form(s).when = *w;
                }
            })
            .build(k)
            .key(key("when")),
        k,
    ));
    match f.when {
        When::Days => out
            .child(hinted(
                input("At", &f.at, &key("at"), "09:00", k, move |s: &mut App| {
                    &mut form(s).at
                })
                .max_w(SHORT_FIELD),
                "Uses this computer's time.",
                k,
            ))
            .child(labelled("On", days(f.days, prefix, k, form), k)),
        When::Every => out.child(hinted(
            input(
                "Repeat every (minutes)",
                &f.every,
                &key("every"),
                "60",
                k,
                move |s: &mut App| &mut form(s).every,
            )
            .max_w(SHORT_FIELD),
            "60 is every hour, 1440 is every day.",
            k,
        )),
        When::Once => out.child(hinted(
            input(
                "On",
                &f.once,
                &key("once"),
                "2026-11-12 09:00",
                k,
                move |s: &mut App| &mut form(s).once,
            )
            .max_w(MEDIUM_FIELD),
            "Year-month-day, then hour:minute (24-hour), in this computer's time.",
            k,
        )),
        When::Cron => out.child(hinted(
            input(
                "Cron expression",
                &f.cron,
                &key("cron"),
                "0 9 * * 1-5",
                k,
                move |s: &mut App| &mut form(s).cron,
            )
            .max_w(MEDIUM_FIELD),
            "Five parts: minute hour day month weekday. 0 9 * * 1-5 is 09:00 on weekdays.",
            k,
        )),
    }
}

/// A toggle per weekday, Monday first.
fn days(chosen: Days, prefix: &str, k: &Kit, form: fn(&mut App) -> &mut TimingForm) -> El {
    row().gap(6.0).children(Days::week().map(|(d, name)| {
        Button::new(name)
            .variant(if chosen.contains(d) {
                ButtonVariant::Submit
            } else {
                ButtonVariant::Outline
            })
            .control_size(ControlSize::Small)
            .on_press(move |s: &mut App| form(s).days.toggle(d))
            .build(k)
            .key(format!("{prefix}day-{d}"))
    }))
}
