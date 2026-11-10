//! Runs: the list of agent runs, and the selected run's steps. Its setup (the brief, each skill, the
//! context, the tools on offer) comes first, then a timeline of what you said, every model call and
//! every tool, each with a bar showing when it ran and for how long within the run.

use rune::{Color, El, ScrollSpec, col, el, row, text};
use rune_kit::{Badge, BadgeVariant, Kit};

use crate::app::{App, now};
use crate::runs::{Run, Status, Step, StepKind};

/// The width of a step's timing bar.
const TRACK: f32 = 120.0;

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
            text("Send a message to an agent. What it does with it is recorded here for 30 days.")
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
    let mut e = col()
        .key(format!("run-{}", r.id))
        .w_full()
        .px(12.0)
        .py(10.0)
        .pl(if r.parent.is_some() { 28.0 } else { 12.0 })
        .gap(3.0)
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
                .gap(8.0)
                .child(
                    text(r.agent_name.clone())
                        .font(k.ty.ui_medium)
                        .color(c.text_base)
                        .truncate(),
                )
                .child(el().grow(1.0))
                .child(status_badge(r.status).build(k)),
        );
    if !r.title.is_empty() {
        e = e.child(
            text(r.title.clone())
                .font(k.ty.small)
                .color(c.text_base)
                .truncate(),
        );
    }
    e = e.child(
        text(summary(r))
            .font(k.ty.small)
            .color(c.text_faint)
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

/// "5 min ago · 1.2s · 1.3k tokens · 2 calls · 3 tools".
fn summary(r: &Run) -> String {
    let mut parts = vec![ago(now() - r.started_at)];
    if let Some(ms) = r.duration_ms {
        parts.push(millis(ms));
    }
    if let Some(t) = tokens(r) {
        parts.push(format!("{} tokens", count(t)));
    }
    if r.calls > 1 {
        parts.push(format!("{} calls", r.calls));
    }
    match r.tools {
        0 => {}
        1 => parts.push("1 tool".to_owned()),
        n => parts.push(format!("{n} tools")),
    }
    parts.join(" · ")
}

pub fn tokens(r: &Run) -> Option<u32> {
    match (r.input_tokens, r.output_tokens) {
        (None, None) => None,
        (i, o) => Some(i.unwrap_or(0) + o.unwrap_or(0)),
    }
}

/// "812", "12.4k".
pub fn count(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f32 / 1000.0)
    } else {
        n.to_string()
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
    let Some(run) = s.runs.get(s.run) else {
        return el().grow(1.0);
    };
    let (setup, timeline): (Vec<_>, Vec<_>) = s
        .steps
        .iter()
        .enumerate()
        .partition(|(_, st)| st.kind.is_setup());
    let mut out = col()
        .key("ledger")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(12.0)
        .gap(2.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(header(run, k));
    if !setup.is_empty() {
        out = out.child(section("Setup", "sent with every model call", k));
        out = out.children(setup.into_iter().map(|(i, st)| step_row(s, run, i, st, k)));
    }
    if !timeline.is_empty() {
        out = out.child(section("Timeline", "", k));
        out = out.children(
            timeline
                .into_iter()
                .map(|(i, st)| step_row(s, run, i, st, k)),
        );
    }
    out
}

/// What the run was, and its totals.
fn header(run: &Run, k: &Kit) -> El {
    let c = &k.c;
    let title = if run.title.is_empty() {
        run.agent_name.clone()
    } else {
        run.title.clone()
    };
    let mut stats = row()
        .w_full()
        .gap(6.0)
        .center_y()
        .child(status_badge(run.status).build(k));
    let mut chips = Vec::new();
    if let Some(ms) = run.duration_ms {
        chips.push(millis(ms));
    }
    if let (Some(i), Some(o)) = (run.input_tokens, run.output_tokens) {
        chips.push(format!("{} in · {} out", count(i), count(o)));
    }
    chips.push(match run.calls {
        1 => "1 model call".to_owned(),
        n => format!("{n} model calls"),
    });
    if run.tools > 0 {
        chips.push(match run.tools {
            1 => "1 tool call".to_owned(),
            n => format!("{n} tool calls"),
        });
    }
    for chip in chips {
        stats = stats.child(
            el().px(8.0)
                .py(2.0)
                .radius(k.radius.sm)
                .fill(c.layer_02)
                .child(text(chip).font(k.ty.small).color(c.text_muted)),
        );
    }
    let mut out = col()
        .key("run-header")
        .w_full()
        .px(12.0)
        .pt(4.0)
        .pb(8.0)
        .gap(6.0)
        .child(text(title).font(k.ty.body_medium).color(c.text_base).wrap())
        .child(
            text(format!(
                "{} · {} · {}",
                run.agent_name, run.model.id, run.model.provider
            ))
            .font(k.ty.small)
            .color(c.text_muted)
            .truncate(),
        )
        .child(stats);
    if let Some(why) = &run.error {
        out = out.child(
            el().w_full()
                .px(10.0)
                .py(8.0)
                .radius(k.radius.md)
                .border(1.0, c.border_muted)
                .child(
                    text(why.clone())
                        .font(k.ty.small)
                        .color(c.warning.icon)
                        .wrap()
                        .selectable(true),
                ),
        );
    }
    out
}

fn section(name: &str, note: &str, k: &Kit) -> El {
    row()
        .w_full()
        .px(12.0)
        .pt(12.0)
        .pb(4.0)
        .gap(8.0)
        .center_y()
        .child(
            text(name.to_owned())
                .font(k.ty.small_medium)
                .color(k.c.text_muted),
        )
        .child(text(note.to_owned()).font(k.ty.small).color(k.c.text_faint))
}

fn step_row(s: &App, run: &Run, i: usize, step: &Step, k: &Kit) -> El {
    let c = &k.c;
    let selected = s.record == i;
    let mut e = row()
        .key(format!("step{i}"))
        .w_full()
        .px(12.0)
        .pl(if step.kind == StepKind::Tool {
            28.0
        } else {
            12.0
        })
        .py(7.0)
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
                .color(tone(step, k))
                .w(64.0),
        )
        .child(
            // A message or a model call reads best as what it said; anything else by its title.
            text(preview(step).unwrap_or_else(|| title(step)))
                .font(k.ty.small)
                .color(c.text_base)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        );
    if !step.kind.is_setup() {
        e = e.child(bar(run, step, k));
    }
    e = e.child(
        text(meta(step))
            .font(k.ty.small)
            .color(c.text_faint)
            .w(96.0)
            .justify_end(),
    );
    k.interactive(e)
}

/// When the step ran within the run, and for how long, drawn to scale.
fn bar(run: &Run, step: &Step, k: &Kit) -> El {
    let total = run.duration_ms.unwrap_or(0).max(1) as f32;
    let at = step.at_ms.unwrap_or(0) as f32;
    let length = step.duration_ms.unwrap_or(0) as f32;
    let left = (at / total * TRACK).clamp(0.0, TRACK - 2.0);
    let width = (length / total * TRACK).clamp(2.0, TRACK - left);
    row()
        .w(TRACK)
        .h(6.0)
        .radius(3.0)
        .fill(k.c.layer_02)
        .child(el().w(left).h(6.0))
        .child(el().w(width).h(6.0).radius(3.0).fill(tone(step, k)))
}

/// What a step is called: its own name, else what its kind means.
pub fn title(step: &Step) -> String {
    let name = step.name.as_str();
    match step.kind {
        StepKind::Tool => {
            let args = step.input.as_deref().unwrap_or_default();
            match crate::tools::summary(name, args).trim() {
                "" => name.to_owned(),
                about => format!("{name} · {about}"),
            }
        }
        StepKind::Tools => match name {
            "1" => "1 tool offered".to_owned(),
            n => format!("{n} tools offered"),
        },
        _ if !name.is_empty() => name.to_owned(),
        StepKind::System => "Agent brief".to_owned(),
        StepKind::Skill => "Skill".to_owned(),
        StepKind::Context => "Date and time".to_owned(),
        StepKind::User => "Your message".to_owned(),
        StepKind::Llm => "Model call".to_owned(),
        StepKind::Error => "It failed".to_owned(),
    }
}

/// A glimpse of what a message or a model call said, beside its title.
fn preview(step: &Step) -> Option<String> {
    let text = step.output.as_deref()?;
    match step.kind {
        StepKind::User | StepKind::Llm | StepKind::Error => {}
        _ => return None,
    }
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(120).collect())
}

/// Beside the bar: how long it took, and for a model call the tokens in and out; for setup, about
/// how many tokens it adds to every call.
fn meta(step: &Step) -> String {
    if step.kind.is_setup() {
        let bytes = step.output.as_deref().map_or(0, str::len);
        return format!("≈{} tok", count((bytes / 4) as u32));
    }
    let mut parts = Vec::new();
    if let (Some(i), Some(o)) = (step.input_tokens, step.output_tokens) {
        parts.push(format!("{}→{}", count(i), count(o)));
    }
    if let Some(ms) = step.duration_ms {
        parts.push(millis(ms));
    }
    parts.join(" ")
}

/// One hue per step kind, wherever it appears (Studio's trace ledger); a step that failed is
/// marked as such.
pub fn tone(step: &Step, k: &Kit) -> Color {
    if step.status == Status::Error {
        return k.c.warning.icon;
    }
    match step.kind {
        StepKind::System | StepKind::Skill | StepKind::Context | StepKind::Tools => k.c.text_faint,
        StepKind::User => k.c.text_base,
        StepKind::Llm => k.c.info.icon,
        StepKind::Tool => k.c.success.icon,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_titled_by_what_they_are_and_tools_by_what_they_did() {
        let mut tool = Step::new(1, StepKind::Tool, 0);
        tool.name = "read_file".into();
        tool.input = Some(r#"{"path":"src/main.rs"}"#.into());
        assert_eq!(title(&tool), "read_file · src/main.rs");
        let mut offered = Step::new(2, StepKind::Tools, 0);
        offered.name = "3".into();
        assert_eq!(title(&offered), "3 tools offered");
        let mut skill = Step::new(3, StepKind::Skill, 0);
        skill.name = "pdf".into();
        assert_eq!(title(&skill), "pdf");
        assert_eq!(title(&Step::new(4, StepKind::Llm, 0)), "Model call");
    }

    #[test]
    fn a_model_call_shows_its_tokens_and_time_and_setup_its_size() {
        let mut call = Step::new(1, StepKind::Llm, 0);
        (call.input_tokens, call.output_tokens, call.duration_ms) =
            (Some(1200), Some(40), Some(820));
        assert_eq!(meta(&call), "1.2k→40 820ms");
        let mut brief = Step::new(2, StepKind::System, 0);
        brief.output = Some("x".repeat(4000));
        assert_eq!(meta(&brief), "≈1.0k tok");
    }
}
