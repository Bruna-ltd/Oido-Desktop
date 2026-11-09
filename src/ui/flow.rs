//! What an agent is doing, as it does it: the state of every tool call, and a card for each task
//! handed to a team member with its steps, its words as they come, its clock and its tokens.

use std::time::{Duration, Instant};

use rune::{El, col, el, icon, row, text};
use rune_kit::{Badge, BadgeVariant, Button, ButtonVariant, ControlSize, Kit, Spinner, glyphs};

use crate::app::App;
use crate::delegate::{Delegated, Stage};
use crate::model::ToolCall;
use crate::theme;

/// Steps shown on a card; older ones are counted.
const SHOWN: usize = 6;
/// How much of what a member is writing is shown: the end of it.
const TAIL: usize = 280;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Asked for, not started: the reply that asked is still being written.
    Pending,
    Running,
    Done,
    Failed,
    Stopped,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "waiting",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// How a call stands, from its result: results are words, and the ones that did not work say so
/// at the start.
pub fn state_of(t: &ToolCall, writing: bool) -> State {
    match t.output.as_deref() {
        None if writing => State::Pending,
        None => State::Running,
        Some(o) if o.starts_with("Error") => State::Failed,
        Some(o)
            if ["The person stopped", "Not run", "Stopped"]
                .iter()
                .any(|p| o.starts_with(p)) =>
        {
            State::Stopped
        }
        Some(_) => State::Done,
    }
}

/// The icon for a state: dots sweeping while it runs, a tick, a cross, a dash.
pub fn mark(state: State, k: &Kit) -> El {
    let c = &k.c;
    match state {
        State::Running => Spinner::new().size(14.0).build(k),
        State::Done => icon(&glyphs::CHECK_SMALL, 14.0).color(theme::green()),
        State::Failed => icon(&glyphs::CIRCLE_X, 14.0).color(theme::amber()),
        State::Pending | State::Stopped => icon(&glyphs::DASH, 14.0).color(c.text_faint),
    }
}

pub fn badge(state: State) -> Badge {
    Badge::new(state.label()).variant(match state {
        State::Running => BadgeVariant::Accent,
        State::Done => BadgeVariant::Subtle,
        State::Pending | State::Failed | State::Stopped => BadgeVariant::Neutral,
    })
}

/// "7s", "1m 05s".
pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

fn tokens(n: u32) -> String {
    if n < 1000 {
        format!("{n} tokens")
    } else {
        format!("{:.1}k tokens", f64::from(n) / 1000.0)
    }
}

fn since(began: Instant, ended: Option<Instant>) -> Duration {
    ended.unwrap_or_else(Instant::now).duration_since(began)
}

/// The card for one task handed to a team member.
pub fn delegation(s: &App, t: &ToolCall, writing: bool, j: usize, k: &Kit) -> El {
    let c = &k.c;
    let live = s.delegated.get(&t.id);
    let running = live.is_some_and(|d| d.ended.is_none());
    let state = if running {
        State::Running
    } else {
        state_of(t, writing)
    };
    let mut head = row()
        .w_full()
        .center_y()
        .gap(8.0)
        .child(mark(state, k))
        .child(
            text(t.summary.clone())
                .font(k.ty.ui_medium)
                .color(c.text_base)
                .truncate(),
        )
        .child(el().grow(1.0));
    if let Some(d) = live {
        let mut facts = clock(since(d.began, d.ended));
        if d.input + d.output > 0 {
            facts = format!("{facts} · {}", tokens(d.input + d.output));
        }
        head = head.child(text(facts).font(k.ty.small).color(c.text_faint));
    }
    if running {
        let id = t.id.clone();
        head = head.child(
            Button::new("Stop")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.stop_delegation(&id))
                .build(k)
                .key(format!("stop-{}", t.id)),
        );
    } else {
        head = head.child(badge(state).build(k));
    }
    let mut card = col()
        .key(format!("tool{j}"))
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(6.0)
        .radius(k.radius.md)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .child(head);
    if let Some(d) = live {
        card = card.child(steps(d, k));
        if running && !d.text.is_empty() {
            card = card.child(
                text(tail(&d.text))
                    .font(k.ty.small)
                    .color(c.text_muted)
                    .wrap(),
            );
        }
    }
    if let Some(out) = &t.output {
        let (shown, cut) = crate::runs::clip(out, 600);
        let shown = if cut { format!("{shown}…") } else { shown };
        card = card.child(text(shown).font(k.ty.small).color(c.text_faint).wrap());
    }
    card
}

/// The end of what is being written, from a word boundary where there is one.
fn tail(written: &str) -> String {
    let count = written.chars().count();
    if count <= TAIL {
        return written.to_owned();
    }
    let cut: String = written.chars().skip(count - TAIL).collect();
    format!("…{}", cut.trim_start())
}

fn steps(d: &Delegated, k: &Kit) -> El {
    let c = &k.c;
    let skipped = d.stages.len().saturating_sub(SHOWN);
    let mut list = col().w_full().gap(3.0).pl(22.0);
    if skipped > 0 {
        list = list.child(
            text(format!("{skipped} earlier steps"))
                .font(k.ty.small)
                .color(c.text_faint),
        );
    }
    list.children(d.stages.iter().skip(skipped).map(|st| step(st, k)))
}

fn step(st: &Stage, k: &Kit) -> El {
    let c = &k.c;
    let state = match (st.ended, st.ok) {
        (None, _) => State::Running,
        (Some(_), true) => State::Done,
        (Some(_), false) => State::Failed,
    };
    row()
        .w_full()
        .center_y()
        .gap(8.0)
        .child(mark(state, k))
        .child(text(st.label.clone()).font(k.ty.mono).color(c.text_base))
        .child(
            text(st.detail.clone())
                .font(k.ty.small)
                .color(c.text_muted)
                .truncate(),
        )
        .child(el().grow(1.0))
        .child(
            text(clock(since(st.began, st.ended)))
                .font(k.ty.small)
                .color(c.text_faint),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(output: Option<&str>) -> ToolCall {
        ToolCall {
            output: output.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn a_call_stands_where_its_result_says() {
        assert_eq!(state_of(&call(None), true), State::Pending);
        assert_eq!(state_of(&call(None), false), State::Running);
        assert_eq!(state_of(&call(Some("fine")), false), State::Done);
        assert_eq!(state_of(&call(Some("Error: no")), false), State::Failed);
        assert_eq!(
            state_of(
                &call(Some("The person stopped x before it finished.")),
                false
            ),
            State::Stopped
        );
    }

    #[test]
    fn clocks_and_tails_read_short() {
        assert_eq!(clock(Duration::from_secs(7)), "7s");
        assert_eq!(clock(Duration::from_secs(65)), "1m 05s");
        assert_eq!(tail("short"), "short");
        let long = "é".repeat(TAIL + 50);
        assert!(tail(&long).chars().count() <= TAIL + 1);
    }
}
