//! Right pane of the Runs screen: everything about the selected step. A model call shows its reply,
//! its thinking and the messages it was sent; a tool its arguments and result; the setup steps the
//! brief, skill, context or tools exactly as the model got them.

use rune::{El, col, el, row, text};
use rune_kit::{Kit, Markdown, MarkdownStyle};

use super::runs::{count, millis, status_badge, title, tone};
use super::usage::money;
use crate::app::App;
use crate::runs::{Run, Status, Step, StepKind};

pub fn build(s: &App, k: &Kit) -> El {
    let (Some(run), Some(step)) = (s.runs.get(s.run), s.steps.get(s.record)) else {
        return el();
    };
    let mut head = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            text(step.kind.label())
                .font(k.ty.text_xsmall_medium)
                .color(tone(step, k)),
        )
        .child(
            text(title(step))
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .truncate(),
        );
    if step.status != Status::Ok {
        head = head
            .child(el().grow(1.0))
            .child(status_badge(step.status).build(k));
    }
    let mut out = col()
        .key("details")
        .w_full()
        .padding(16.0)
        .gap(16.0)
        .child(head)
        .child(facts(s, run, step, k));
    for part in parts(step, k) {
        out = out.child(part);
    }
    if step.truncated {
        out = out.child(
            text("Long text is cut short in the record.")
                .font(k.ty.small)
                .color(k.c.text_faint),
        );
    }
    out
}

/// The step's numbers, as label and value rows.
fn facts(s: &App, run: &Run, step: &Step, k: &Kit) -> El {
    let mut rows: Vec<(&str, String)> = Vec::new();
    match step.kind {
        StepKind::Llm => {
            rows.push((
                "Model",
                format!("{} · {}", run.model.id, run.model.provider),
            ));
            if let (Some(i), Some(o)) = (step.input_tokens, step.output_tokens) {
                rows.push(("Tokens", format!("{} in · {} out", count(i), count(o))));
                if let Some(ms) = step.duration_ms.filter(|ms| *ms > 0 && o > 0) {
                    rows.push((
                        "Speed",
                        format!("{:.0} tokens/s", o as f32 * 1000.0 / ms as f32),
                    ));
                }
            }
            if let (Some(_), Some(p)) = (step.input_tokens, s.prices.get(&run.model)) {
                let cost = p.cost(&crate::usage::Totals::of_call(step));
                rows.push(("Cost", format!("about {}", money(&cost.into()))));
            }
            if let Some(ms) = step.first_ms {
                rows.push(("First token", millis(ms)));
            }
        }
        StepKind::Tool => rows.push(("Tool", step.name.clone())),
        StepKind::System | StepKind::Skill | StepKind::Context | StepKind::Tools => {
            let bytes = step.output.as_deref().map_or(0, str::len);
            rows.push((
                "Adds",
                format!(
                    "about {} tokens to every model call",
                    count((bytes / 4) as u32)
                ),
            ));
        }
        StepKind::User | StepKind::Guard | StepKind::Error => {}
    }
    if let Some(ms) = step.duration_ms {
        rows.push(("Took", millis(ms)));
    }
    if let Some(at) = step.at_ms.filter(|_| !step.kind.is_setup()) {
        rows.push(("Began", format!("{} into the run", millis(at))));
    }
    col()
        .w_full()
        .gap(6.0)
        .children(rows.into_iter().map(|(label, value)| {
            row()
                .w_full()
                .gap(12.0)
                .child(
                    text(label.to_owned())
                        .font(k.ty.small)
                        .color(k.c.text_faint)
                        .w(84.0),
                )
                .child(
                    text(value)
                        .font(k.ty.small)
                        .color(k.c.text_base)
                        .wrap()
                        .selectable(true),
                )
        }))
}

/// How a part's text is shown.
enum Look {
    /// Written as Markdown: replies, briefs, skills.
    Prose,
    /// Exactly as it was: messages sent, arguments, results.
    Code,
    Plain,
}

/// The step's content, each part under its own label.
fn parts(step: &Step, k: &Kit) -> Vec<El> {
    let output = step.output.as_deref().unwrap_or_default();
    let input = step.input.as_deref().unwrap_or_default();
    let reasoning = step.reasoning.as_deref().unwrap_or_default();
    let list: Vec<(&str, String, Look)> = match step.kind {
        StepKind::System => vec![("Brief", output.to_owned(), Look::Prose)],
        StepKind::Skill => vec![("Instructions", output.to_owned(), Look::Prose)],
        StepKind::Context => vec![("Told to the model", output.to_owned(), Look::Prose)],
        StepKind::Tools => vec![("Offered", output.to_owned(), Look::Code)],
        StepKind::User => vec![("Message", output.to_owned(), Look::Plain)],
        StepKind::Llm => vec![
            ("Reply", output.to_owned(), Look::Prose),
            ("Thinking", reasoning.to_owned(), Look::Plain),
            ("Messages sent", input.to_owned(), Look::Code),
        ],
        StepKind::Tool => vec![
            ("Arguments", pretty(input), Look::Code),
            ("Result", output.to_owned(), Look::Code),
        ],
        StepKind::Guard => vec![("What it did", output.to_owned(), Look::Plain)],
        StepKind::Error => vec![("What went wrong", output.to_owned(), Look::Plain)],
    };
    list.into_iter()
        .filter(|(_, body, _)| !body.trim().is_empty())
        .map(|(label, body, look)| part(label, body, look, step, k))
        .collect()
}

fn part(label: &str, body: String, look: Look, step: &Step, k: &Kit) -> El {
    let c = &k.c;
    let content = match look {
        Look::Prose => Markdown::new(body)
            .style(MarkdownStyle::from_kit(k))
            .build(k),
        Look::Code => col()
            .w_full()
            .px(12.0)
            .py(10.0)
            .radius(k.radius.md)
            .fill(c.layer_02)
            .child(
                text(body)
                    .font(k.ty.mono)
                    .color(c.text_base)
                    .wrap()
                    .selectable(true),
            ),
        Look::Plain => text(body)
            .font(k.ty.ui)
            .color(match step.kind {
                StepKind::Error => c.warning.icon,
                StepKind::Llm => c.text_muted,
                _ => c.text_base,
            })
            .wrap()
            .selectable(true),
    };
    col()
        .w_full()
        .gap(6.0)
        .child(
            text(label.to_owned())
                .font(k.ty.small_medium)
                .color(c.text_faint),
        )
        .child(content)
}

/// Arguments laid out to read, when they are JSON.
fn pretty(args: &str) -> String {
    serde_json::from_str::<serde_json::Value>(args)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| args.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_arguments_are_laid_out_and_anything_else_is_kept_as_it_was() {
        assert_eq!(pretty(r#"{"a":1}"#), "{\n  \"a\": 1\n}");
        assert_eq!(pretty("{cut off"), "{cut off");
    }
}
