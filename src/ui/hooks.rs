//! The Hooks screen: the commands set to run around agents' tool calls (see [`crate::hooks`]),
//! and the form to add or change one.

use rune::{El, ScrollSpec, col, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Switch, TextInput};

use super::guard::choices;
use super::settings::{section, setting};
use crate::app::App;
use crate::hooks::{Event, Fail, Hook, MAX_SECS};

const COLUMN: f32 = 760.0;

/// The time limits offered, in seconds.
const LIMITS: [u64; 4] = [5, 10, 30, MAX_SECS];

pub fn screen(s: &App, k: &Kit) -> El {
    let hooks = s.hooks.list();
    let listed = if hooks.is_empty() {
        vec![note(
            "No hooks yet. Add one below: a command that looks at a tool call before it runs, or at its result before the model sees it.",
            k,
        )]
    } else {
        hooks.iter().map(|h| row(h, k)).collect()
    };
    col()
        .key("hooks")
        .w_full()
        .h_full()
        .min_h(0.0)
        .items_center()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .max_w(COLUMN)
                .px(24.0)
                .py(24.0)
                .gap(16.0)
                .child(note(
                    "Commands that run on this computer around an agent's tool calls, in every chat, schedule and channel. The guardrails look at a call first; a hook only sees what they let through, and it can refuse the call, hold it for your go-ahead or change it, but never allow what a guardrail stopped.",
                    k,
                ))
                .child(section("Your hooks", listed, k))
                .child(section(
                    if s.hook_form.id.is_some() {
                        "Change hook"
                    } else {
                        "Add a hook"
                    },
                    form(s, k),
                    k,
                ))
                .child(section("How a hook answers", vec![help(k)], k)),
        )
}

fn note(words: &str, k: &Kit) -> El {
    text(words.to_owned())
        .font(k.ty.small)
        .color(k.c.text_muted)
        .wrap()
        .padding(16.0)
}

/// One hook: what it is for and what it runs, with its switch.
fn row(h: &Hook, k: &Kit) -> El {
    let c = &k.c;
    let tools = if h.tools.trim().is_empty() {
        "every tool".to_owned()
    } else {
        h.tools.trim().to_owned()
    };
    let fail = match h.fail {
        Fail::Open => "if it fails, the call goes on",
        Fail::Closed => "if it fails, the call is stopped",
    };
    let (toggle, edit, delete) = (h.id.clone(), h.id.clone(), h.id.clone());
    rune::row()
        .key(format!("hook-{}", h.id))
        .w_full()
        .padding(16.0)
        .gap(12.0)
        .center_y()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(h.label().to_owned())
                        .font(k.ty.ui_medium)
                        .color(c.text_base),
                )
                .child(
                    text(format!(
                        "{} · {tools} · {}s · {fail}",
                        h.event.title(),
                        h.secs
                    ))
                    .font(k.ty.small)
                    .color(c.text_muted)
                    .wrap(),
                )
                .child(
                    text(h.command.clone())
                        .font(k.ty.mono)
                        .color(c.text_faint)
                        .wrap(),
                ),
        )
        .child(
            Button::new("Change")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.edit_hook(&edit))
                .build(k)
                .key(format!("hook-edit-{}", h.id)),
        )
        .child(
            Button::new("Remove")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.delete_hook(&delete))
                .build(k)
                .key(format!("hook-remove-{}", h.id)),
        )
        .child(
            Switch::new(h.on)
                .label(h.label().to_owned())
                .hide_label()
                .on_toggle(move |s: &mut App| s.toggle_hook(&toggle))
                .build(k),
        )
}

/// The fields of a hook, and the buttons that save it.
fn form(s: &App, k: &Kit) -> Vec<El> {
    let f = &s.hook_form;
    let field = |key: &str,
                 name: &str,
                 help: &str,
                 edit: &rune::TextEdit,
                 holder: fn(&mut App) -> &mut rune::TextEdit,
                 hint: &str| {
        setting(
            key,
            name,
            help,
            TextInput::new(edit)
                .placeholder(hint)
                .label(name.to_owned())
                .on_edit(holder)
                .build(k)
                .w(300.0)
                .key(format!("{key}-input")),
            k,
        )
    };
    vec![
        setting(
            "hook-event",
            "Runs",
            "Before a tool runs, once the guardrails let it through; or when it has its result, before the model sees it.",
            choices(
                "hook-event",
                Event::ALL.map(|e| (e, e.title().to_owned())),
                f.event,
                |s, e| s.hook_form.event = e,
                k,
            ),
            k,
        ),
        field(
            "hook-name",
            "Name",
            "Optional: what the hook is called in the run.",
            &f.name,
            |s| &mut s.hook_form.name,
            "e.g. No deploys on Friday",
        ),
        field(
            "hook-tools",
            "Tools",
            "Which tools it is for, split by |, with * for any characters. Empty: every tool.",
            &f.tools,
            |s| &mut s.hook_form.tools,
            "e.g. shell|write_file or mcp__*",
        ),
        field(
            "hook-command",
            "Command",
            "Run with sh, in the agent's folder. It is given the event as JSON on its input.",
            &f.command,
            |s| &mut s.hook_form.command,
            "e.g. ~/bin/check-call.sh",
        ),
        setting(
            "hook-secs",
            "Time limit",
            "A hook that runs longer is ended and counts as failed.",
            choices(
                "hook-secs",
                LIMITS.map(|n| (n, format!("{n} s"))),
                f.secs,
                |s, n| s.hook_form.secs = n,
                k,
            ),
            k,
        ),
        setting(
            "hook-fail",
            "If it fails",
            "Failed means it did not start, ran past its limit, exited with an error or printed something that is not an answer.",
            choices(
                "hook-fail",
                [
                    (Fail::Open, "Go on".to_owned()),
                    (Fail::Closed, "Stop it".to_owned()),
                ],
                f.fail,
                |s, fail| s.hook_form.fail = fail,
                k,
            ),
            k,
        ),
        rune::row()
            .key("setting-hook-save")
            .w_full()
            .padding(16.0)
            .gap(8.0)
            .child(
                Button::new(if f.id.is_some() {
                    "Save changes"
                } else {
                    "Add hook"
                })
                .variant(ButtonVariant::Contrast)
                .control_size(ControlSize::Small)
                .disabled(!f.ready())
                .on_press(App::save_hook_form)
                .build(k)
                .key("hook-save"),
            )
            .children((f.id.is_some() || f.ready()).then(|| {
                Button::new("Clear")
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press(|s: &mut App| s.hook_form = crate::hooks::Form::default())
                    .build(k)
                    .key("hook-clear")
            })),
    ]
}

/// What a hook is given and how it answers.
fn help(k: &Kit) -> El {
    let c = &k.c;
    col()
        .key("setting-hook-help")
        .w_full()
        .padding(16.0)
        .gap(8.0)
        .child(
            text("The command reads the event from its input: {\"event\": \"before_tool\", \"tool\": \"shell\", \"args\": {…}, \"agent\": …, \"cwd\": …}; after a tool it also has \"result\".")
                .font(k.ty.small)
                .color(c.text_muted)
                .wrap(),
        )
        .child(
            text("Print nothing, or exit 0, to let things go on. Exit 2 refuses (what it wrote to stderr is the reason). Or print {\"action\": \"block\" | \"ask\" | \"allow\", \"reason\": …}, or {\"action\": \"replace\", \"args\": {…}} before a tool, {\"action\": \"replace\", \"result\": \"…\"} after one. \"ask\" holds the call for your go-ahead.")
                .font(k.ty.small)
                .color(c.text_muted)
                .wrap(),
        )
}
