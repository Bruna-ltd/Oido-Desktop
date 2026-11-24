//! The workflow editor: the steps as a stack of cards, each with the boxes of its kind, and a
//! list of steps inside a card for each branch or loop it holds. What is wrong with what is
//! typed is listed above the buttons.

use rune::{Align, El, ScrollSpec, Side, TextEdit, col, el, row, text, text_input};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Kit, PopupItem, PopupList, SegmentedControl, Select,
    SelectAppearance, Switch,
};

use super::workflow_graph;
use crate::app::App;
use crate::flow_editor::Complete;
use crate::workflow::cond;
use crate::workflow::definition::{FOR_EACH, FieldKind, GENERATE, PARALLEL, SWITCH, TOOL, WHILE};
use crate::workflow::editor::{
    ArgKind, Block, Editor, INPUT_KINDS, KINDS, Loc, Menu, Node, ParamSpec, Part, Place, Refs,
    Target, params_of, specs,
};

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let Some(ed) = &s.wf_editor else {
        return el();
    };
    graph(s, ed, k, layer)
}

/// The bar along the top, as n8n has it: back to the list and the workflow's name on the left;
/// where it stands, then Check, Save and Publish on the right. Publish makes what is saved the
/// live copy; Unpublish and Revert show when there is a live copy to take down or go back to.
fn top_bar(s: &App, ed: &Editor, k: &Kit) -> El {
    let c = &k.c;
    let saved = ed
        .workflow
        .as_ref()
        .and_then(|id| s.workflows.iter().find(|w| &w.id == id));
    let live = saved.is_some_and(|w| w.live().is_some());
    let changed = saved.is_none_or(|w| w.has_unpublished_changes());
    let (state, quiet) = match (live, changed) {
        (false, _) => ("Draft · not published", false),
        (true, true) => ("Published · changes not published", false),
        (true, false) => ("Published", true),
    };
    let button = |label: &str, variant: ButtonVariant, key: &str| {
        Button::new(label.to_owned())
            .variant(variant)
            .control_size(ControlSize::Regular)
            .build(k)
            .key(key.to_owned())
    };
    let style = rune::TextStyle {
        line_height: 22.0,
        ..k.ty.body_medium
    };
    let name = row()
        .w(280.0)
        .px(10.0)
        .py(5.0)
        .radius(k.radius.md)
        .border(
            1.0,
            if ed.name.focused {
                c.border_strong
            } else {
                rune::Color::TRANSPARENT
            },
        )
        .child(
            text_input(&ed.name, style)
                .key("wfe-name")
                .w_full()
                .color(c.text_base)
                .placeholder("Name this workflow".to_owned(), c.text_faint)
                .caret(c.text_base, k.selection())
                .label("Workflow name".to_owned())
                .on_edit(|s: &mut App| s.editor_text(&Target::Name)),
        );
    let bar = row()
        .key("wfe-bar")
        .w_full()
        .center_y()
        .gap(10.0)
        .px(16.0)
        .py(10.0)
        .child(
            button("← Workflows", ButtonVariant::Ghost, "wfe-cancel")
                .help("Back to the list. Your changes are already kept.")
                .on_press(|s: &mut App| s.cancel_editor()),
        )
        .child(el().w(1.0).h(20.0).fill(c.border_muted))
        .child(name)
        .child(el().grow(1.0))
        .children(s.has_last_run().then(|| {
            let on = s.wf_graph.show_last;
            button(
                if on { "Hide last run" } else { "Show last run" },
                ButtonVariant::Ghost,
                "wfg-last-run",
            )
            .on_press(move |s: &mut App| {
                s.hide_run();
                s.wf_graph.show_last = !on;
            })
        }))
        .child(
            text(if s.editor_saved() {
                "Saved"
            } else {
                "Unsaved changes"
            })
            .font(k.ty.small)
            .color(c.text_faint),
        )
        .child(text(state).font(k.ty.small).color(if quiet {
            c.text_muted
        } else {
            c.warning.icon
        }))
        .children((live && changed).then(|| {
            button("Revert", ButtonVariant::Ghost, "wfe-revert")
                .help("Drop the changes made since it was published")
                .on_press(|s: &mut App| s.revert_editor())
        }))
        .children(live.then(|| {
            button("Unpublish", ButtonVariant::Ghost, "wfe-unpublish")
                .help("Nothing runs it, and its schedule stops, until it is published again")
                .on_press(|s: &mut App| s.unpublish_editor())
        }))
        .child(
            button("Check", ButtonVariant::Ghost, "wfe-check")
                .on_press(|s: &mut App| s.check_editor()),
        )
        .child(
            button("Save", ButtonVariant::Outline, "wfe-save")
                .on_press(|s: &mut App| s.save_editor()),
        )
        .child(
            button("Publish", ButtonVariant::Submit, "wfe-publish")
                .help("Save, then make this the version that agents and schedules run")
                .on_press(|s: &mut App| s.publish_editor()),
        );
    col()
        .w_full()
        .child(bar)
        .child(el().w_full().h(1.0).fill(c.border_muted))
}

/// Name, what it is for, who owns it, and what a run is given.
fn basics(s: &App, ed: &Editor, k: &Kit, layer: &mut Vec<El>) -> El {
    col()
        .w_full()
        .gap(14.0)
        .child(field(
            "What it is for",
            &ed.goal,
            "wfe-goal",
            "One line, shown on the page",
            1,
            None,
            k,
            |s: &mut App| s.editor_text(&Target::Goal),
        ))
        .child(agent_row(s, ed, k, layer))
        .child(inputs(s, ed, k, layer))
}

/// When it is done, what is wrong with it, and a test run.
fn tail(s: &App, ed: &Editor, k: &Kit) -> El {
    let mut out = col().w_full().gap(14.0).child(field(
        "Done when",
        &ed.success,
        "wfe-success",
        "What shows a run did its job",
        1,
        None,
        k,
        |s: &mut App| s.editor_text(&Target::Success),
    ));
    if !ed.problems.is_empty() {
        out = out.child(col().key("wfe-problems").w_full().gap(4.0).children(
            ed.problems.iter().map(|p| {
                text(p.clone())
                    .font(k.ty.caption())
                    .color(k.c.warning.icon)
                    .wrap()
                    .selectable(true)
            }),
        ));
    }
    out.child(test_run(s, ed, k))
}

/// The steps as boxes and wires, with the form of the selected one beside them.
fn graph(s: &App, ed: &Editor, k: &Kit, layer: &mut Vec<El>) -> El {
    let page = col()
        .key("workflow-editor")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .child(top_bar(s, ed, k))
        .child(
            col()
                .grow(1.0)
                .min_h(0.0)
                .padding(16.0)
                .gap(12.0)
                .children(workflow_graph::banner(s, ed, k))
                .child(
                    row()
                        .grow(1.0)
                        .min_h(0.0)
                        .gap(16.0)
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .min_h(0.0)
                                .gap(12.0)
                                .child(workflow_graph::view(s, ed, k, layer))
                                .children(super::workflow_logs::panel(s, ed, k)),
                        )
                        .children(side(s, ed, k, layer)),
                ),
        );
    super::workflow_node::window(s, ed, k, layer);
    page
}

/// The panel at the right: the workflow's own settings, the list of steps to add, or the trigger.
/// A selected step has its own window instead.
fn side(s: &App, ed: &Editor, k: &Kit, layer: &mut Vec<El>) -> Option<El> {
    (s.picker_place().is_some() || s.selected_step().is_none()).then(|| inspector(s, ed, k, layer))
}

/// The form of the selected step, or the workflow's own when nothing is selected.
fn inspector(s: &App, ed: &Editor, k: &Kit, layer: &mut Vec<El>) -> El {
    if s.picker_place().is_some() {
        return super::workflow_picker::panel(s, k);
    }
    let mut out = col()
        .key("wfg-inspector")
        .w(workflow_graph::INSPECTOR_W)
        .h_full()
        .min_h(0.0)
        .gap(14.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if s.wf_graph.on_trigger {
        out = out.child(trigger_form(ed, k));
    } else {
        let c = &k.c;
        out = out
                .child(
                    col()
                        .gap(2.0)
                        .child(
                            text("Workflow settings")
                                .font(k.ty.body_medium)
                                .color(c.text_base),
                        )
                        .child(
                            text("Name, who runs it, what it takes, and when it counts as done. Double-press a step to open it.")
                                .font(k.ty.caption())
                                .color(c.text_muted)
                                .wrap(),
                        ),
                )
                .child(basics(s, ed, k, layer))
                .child(el().w_full().h(1.0).fill(c.border_muted))
                .child(tail(s, ed, k));
    }
    out
}

/// Runs the selected step for real, after the results of the ones before it, so what it made can
/// be seen under it and the next step can use it.
pub(super) fn step_actions(
    s: &App,
    ed: &Editor,
    at: &Loc,
    node: &Node,
    k: &Kit,
    layer: &mut Vec<El>,
) -> El {
    let t = &s.wf_test;
    let nested = !at.place.is_empty();
    let go = at.clone();
    let button = if t.running.is_some() {
        Button::new("Stop")
            .variant(ButtonVariant::Outline)
            .control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.stop_test())
            .build(k)
            .key("wfg-step-stop")
    } else {
        Button::new("Execute step")
            .variant(ButtonVariant::Submit)
            .control_size(ControlSize::Small)
            .on_press(move |s: &mut App| s.run_step(&go))
            .build(k)
            .key("wfg-step-run")
            .help(if nested {
                "Runs the steps before this one and the step that holds it as far as this one, for real. Steps after it are not run, so ones not finished yet do not stop it."
            } else {
                "Runs this step for real after the results of the steps before it. Ones that have not run yet run first; ones that did are not run again."
            })
    };
    let before = at.clone();
    let mut out = col()
        .w_full()
        .gap(6.0)
        .child(row().gap(8.0).center_y().child(button).children(
            (t.running.is_none()).then(|| {
                small("Run what comes before", ButtonVariant::Outline)
                    .on_press(move |s: &mut App| s.run_before(&before))
                    .build(k)
                    .key("wfg-step-before")
                    .help("Runs the steps before this one, for real, so what they made is there to use. This step is not run, so it can still be unfinished.")
            }),
        ))
        .child(row().gap(8.0).center_y().children(
            t.memos.as_ref().filter(|m| m.len() > 0).map(|_| {
                small("Forget results", ButtonVariant::Ghost)
                    .on_press(|s: &mut App| s.forget_step_results())
                    .build(k)
                    .key("wfg-step-forget")
                    .help("The next step run starts again from the first step")
            }),
        ));
    if let Some(why) = &t.notice {
        out = out.child(
            text(why.clone())
                .font(k.ty.caption())
                .color(k.c.warning.icon)
                .wrap(),
        );
    }
    out.child(discover(s, ed, at, node, k, layer))
}

/// Finding the fields of a step's result: a model of the person's choice looks at the result the
/// step made when it was tried, and what it finds is offered by name after the step.
fn discover(s: &App, _ed: &Editor, at: &Loc, node: &Node, k: &Kit, layer: &mut Vec<El>) -> El {
    use super::model_picker::ModelPicker;
    let c = &k.c;
    let m = &s.wf_mapper;
    let go = at.clone();
    let picker = ModelPicker::new("wf-mapper-model", s.providers.groups())
        .current(
            m.model.clone(),
            m.model.as_ref().map(|x| s.providers.model_name(x)),
        )
        .placeholder("Same model as the agent".to_owned())
        .label("Model that looks at the result")
        .clearable(true)
        .on_open(|s: &mut App| s.models_opened())
        .build(
            k,
            layer,
            &m.picker,
            |s: &mut App| &mut s.wf_mapper.picker,
            |s: &mut App, v| s.wf_mapper.model = v,
        )
        .key("wf-mapper-model");
    let mut out = col()
        .w_full()
        .gap(6.0)
        .child(
            text("Fields of the result")
                .font(k.ty.small_medium)
                .color(c.text_muted),
        )
        .child(picker)
        .child(
            row().child(
                small(
                    if m.running {
                        "Looking…"
                    } else {
                        "Discover fields"
                    },
                    ButtonVariant::Outline,
                )
                .disabled(m.running)
                .on_press(move |s: &mut App| s.discover_fields(&go))
                .build(k)
                .key("wfg-step-discover")
                .help("Runs this step first if it has not been run, then a model looks at what it made and lists its fields, which later boxes offer by name"),
            ),
        );
    if !node.found.is_empty() {
        let list: Vec<String> = node
            .found
            .iter()
            .map(|f| format!("{} ({})", f.name, f.kind.label()))
            .collect();
        out = out.child(
            text(format!("Found: {}", list.join(", ")))
                .font(k.ty.caption())
                .color(c.text_faint)
                .wrap(),
        );
    }
    if let Some(why) = &m.notice {
        out = out.child(
            text(why.clone())
                .font(k.ty.caption())
                .color(c.text_muted)
                .wrap(),
        );
    }
    out
}

/// What starts the workflow: you or an agent, or a schedule. It takes effect when published.
fn trigger_form(ed: &Editor, k: &Kit) -> El {
    let c = &k.c;
    let t = &ed.trigger;
    let mut out = col()
        .w_full()
        .gap(14.0)
        .child(
            row().child(
                small("Workflow settings", ButtonVariant::Ghost)
                    .on_press(|s: &mut App| s.select_step(None))
                    .build(k)
                    .key("wfg-settings"),
            ),
        )
        .child(text("Trigger").font(k.ty.ui_medium).color(c.text_base))
        .child(
            SegmentedControl::new(&["Manual", "Schedule"], Some(usize::from(t.schedule)))
                .label("Trigger")
                .fit()
                .on_pick(|s: &mut App, i, _| s.editor_set_schedule(i == 1))
                .build(k)
                .key("wfe-trigger"),
        )
        .child(
            text(if t.schedule {
                "Runs by itself while Oido is open, once the workflow is published. A test run ignores the schedule."
            } else {
                "Starts when you or an agent run the published workflow."
            })
            .font(k.ty.caption())
            .color(c.text_muted)
            .wrap(),
        );
    if t.schedule {
        out = out.child(super::schedules::timing_fields(
            &t.timing,
            "trigger-",
            k,
            |s: &mut App| s.trigger_timing(),
        ));
        let names: Vec<String> = ed
            .inputs
            .iter()
            .map(|r| r.name.text.trim().to_owned())
            .filter(|n| !n.is_empty())
            .collect();
        if !names.is_empty() {
            out = out.child(
                text("What each run is given")
                    .font(k.ty.small)
                    .color(c.text_muted),
            );
        }
        for name in names {
            let blank = TextEdit::new("");
            let edit = t.inputs.get(&name).unwrap_or(&blank);
            let n = name.clone();
            out = out.child(super::schedules::input(
                &name,
                edit,
                &format!("trigger-input-{name}"),
                "",
                k,
                move |s: &mut App| s.trigger_input(&n),
            ));
        }
    }
    out
}

/// Runs the workflow as it stands, for real, and shows each step as it went.
fn test_run(s: &App, ed: &Editor, k: &Kit) -> El {
    let c = &k.c;
    let t = &s.wf_test;
    let running = t.running.is_some();
    let mut out = col()
        .key("wfe-test")
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(k.radius.md)
        .border(1.0, c.border_muted)
        .child(text("Test run").font(k.ty.ui_medium).color(c.text_base))
        .child(
            text("A test runs the steps for real: commands run, files change, messages go out. Nothing is saved.")
                .font(k.ty.caption())
                .color(c.text_muted)
                .wrap(),
        );
    if ed.inputs.iter().any(|r| !r.name.text.trim().is_empty()) {
        out = out.child(field(
            "What to give it, one per line: name = value",
            &t.inputs,
            "wfe-test-inputs",
            "folder = notes",
            2,
            None,
            k,
            |s: &mut App| &mut s.wf_test.inputs,
        ));
    }
    let button = if running {
        Button::new("Stop")
            .variant(ButtonVariant::Outline)
            .control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.stop_test())
            .build(k)
            .key("wfe-test-stop")
    } else {
        Button::new("Run the test")
            .variant(ButtonVariant::Outline)
            .control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.test_editor())
            .build(k)
            .key("wfe-test-run")
    };
    out = out.child(row().child(button));
    let at = crate::guard::ask::Where::Workflow(
        ed.workflow
            .clone()
            .unwrap_or_else(|| crate::flow_editor::TEST_CARD.to_owned()),
    );
    out = out.children(super::guard::waiting(s, &at, k));
    if let Some(why) = &t.notice {
        out = out.child(
            text(why.clone())
                .font(k.ty.caption())
                .color(c.warning.icon)
                .wrap(),
        );
    }
    if let Some(r) = &t.result {
        use crate::workflow::engine::Ending;
        let (words, failure) = match &r.ending {
            Ending::Succeeded => ("It worked.".to_owned(), None),
            Ending::Stopped => ("You stopped it.".to_owned(), None),
            Ending::Waiting { prompt, .. } => (
                format!(
                    "It reached a wait ({prompt}). A test does not wait for an answer, so it stops here."
                ),
                None,
            ),
            Ending::Failed { step, error } => (
                "It failed.".to_owned(),
                Some(format!("Failed at {step}: {error}")),
            ),
        };
        out = out
            .child(text(words).font(k.ty.small).color(c.text_base))
            .child(super::workflows::step_log(
                &r.steps,
                failure,
                "wfe-test-log",
                k,
            ));
    }
    out
}

fn small(label: &str, variant: ButtonVariant) -> Button {
    Button::new(label.to_owned())
        .variant(variant)
        .control_size(ControlSize::Small)
}

/// A labelled text box, `lines` tall, with a list of names to put into it when there are some.
#[allow(clippy::too_many_arguments)]
fn field(
    label: &str,
    edit: &TextEdit,
    key: &str,
    hint: &str,
    lines: usize,
    names: Option<El>,
    k: &Kit,
    f: impl for<'a> Fn(&'a mut App) -> &'a mut TextEdit + 'static,
) -> El {
    field_with(label, edit, key, hint, lines, names, None, k, f)
}

/// The list that finishes the name being typed in a box, under it, while the box has the caret.
fn finishing(s: &App, c: &Complete, edit: &TextEdit, k: &Kit, layer: &mut Vec<El>) {
    if !edit.focused {
        return;
    }
    let Some(found) = s.wf_completion(c, edit) else {
        return;
    };
    let active = s.wf_suggest.active.min(found.options.len() - 1);
    let c = c.clone();
    PopupList::new(format!("{}-finish", c.anchor), c.anchor.clone())
        .anchored(c.anchor.clone(), Side::Bottom, Align::Start, 4.0)
        .width(320.0)
        .max_height(240.0)
        .items(
            found
                .options
                .iter()
                .map(|o| PopupItem::new(o.clone(), o.clone())),
        )
        .active(Some(active))
        .label("Names".to_owned())
        .on_choose(move |s: &mut App, id, _| s.editor_complete(&c, id))
        .on_close(|_: &mut App| {})
        .build(k, layer);
}

/// [`field`] that also finishes the names typed in it, when it is given what it may name.
#[allow(clippy::too_many_arguments)]
fn field_with(
    label: &str,
    edit: &TextEdit,
    key: &str,
    hint: &str,
    lines: usize,
    names: Option<El>,
    complete: Option<Complete>,
    k: &Kit,
    f: impl for<'a> Fn(&'a mut App) -> &'a mut TextEdit + 'static,
) -> El {
    let c = &k.c;
    let style = crate::theme::ty::field(k);
    let mut head = row()
        .w_full()
        .center_y()
        .gap(8.0)
        .child(text(label.to_owned()).font(k.ty.small).color(c.text_muted))
        .child(el().grow(1.0));
    if let Some(names) = names {
        head = head.child(names);
    }
    col().w_full().gap(4.0).child(head).child(
        col()
            .w_full()
            .px(12.0)
            .py(8.0)
            .min_h(lines as f32 * 22.0 + 16.0)
            .radius(k.radius.md)
            .border(
                1.0,
                if edit.focused {
                    c.border_strong
                } else {
                    c.border_base
                },
            )
            .child({
                let input = text_input(edit, style)
                    .key(key.to_owned())
                    .w_full()
                    .color(c.text_base)
                    .placeholder(hint.to_owned(), c.text_faint)
                    .caret(c.text_base, k.selection())
                    .label(label.to_owned())
                    .on_edit(f);
                match complete {
                    Some(c) => {
                        input.on_input_key(move |s: &mut App, key, _| s.wf_complete_key(&c, key))
                    }
                    None => input,
                }
            }),
    )
}

/// A list that opens from a small trigger and gives back the entry chosen.
#[allow(clippy::too_many_arguments)]
pub(super) fn pick<F>(
    key: String,
    menu: Menu,
    options: &[&str],
    current: Option<&str>,
    placeholder: &str,
    field_look: bool,
    s: &App,
    k: &Kit,
    layer: &mut Vec<El>,
    choose: F,
) -> El
where
    F: Fn(&mut App, &str) + Clone + 'static,
{
    let open = s.wf_menu.as_ref() == Some(&menu);
    let toggle = menu.clone();
    Select::new(key, options)
        .current(current)
        .placeholder(placeholder.to_owned())
        .label(placeholder.to_owned())
        .appearance(if field_look {
            SelectAppearance::Field
        } else {
            SelectAppearance::Inline
        })
        .open(open)
        .on_toggle(move |s: &mut App| s.toggle_menu(&toggle))
        .on_close(|s: &mut App| s.wf_menu = None)
        .on_choose(move |s: &mut App, name: &str, _| choose(s, name))
        .build(k, layer)
}

/// The list of kinds of step to add, in two sections: what most workflows are made of, and the rest.
pub(super) fn add_menu<F>(
    key: String,
    menu: Menu,
    placeholder: &str,
    s: &App,
    k: &Kit,
    layer: &mut Vec<El>,
    choose: F,
) -> El
where
    F: Fn(&mut App, &str) + Clone + 'static,
{
    let options: Vec<(&str, &str)> = KINDS.iter().map(|x| (x.group, x.label)).collect();
    let hints: Vec<Option<&str>> = KINDS.iter().map(|x| Some(x.hint)).collect();
    let toggle = menu.clone();
    Select::grouped(key, &options)
        .details(&hints)
        .placeholder(placeholder.to_owned())
        .label(placeholder.to_owned())
        .appearance(SelectAppearance::Inline)
        .open(s.wf_menu.as_ref() == Some(&menu))
        .on_toggle(move |s: &mut App| s.toggle_menu(&toggle))
        .on_close(|s: &mut App| s.wf_menu = None)
        .on_choose(move |s: &mut App, name: &str, _| choose(s, name))
        .build(k, layer)
}

fn agent_row(s: &App, ed: &Editor, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let people = s.agent_choices();
    let name = people
        .iter()
        .find(|(_, id)| *id == ed.agent_id)
        .map(|(n, _)| n.clone());
    let control = if ed.workflow.is_some() {
        // A workflow stays with the agent that has it.
        text(name.unwrap_or_default())
            .font(k.ty.body)
            .color(c.text_base)
    } else {
        let names: Vec<&str> = people.iter().map(|(n, _)| n.as_str()).collect();
        return col()
            .w_full()
            .gap(4.0)
            .child(text("Agent").font(k.ty.small).color(c.text_muted))
            .child(row().child(pick(
                "wfe-agent".to_owned(),
                Menu::Agent,
                &names,
                name.as_deref(),
                "Choose an agent",
                true,
                s,
                k,
                layer,
                |s, n| s.choose_agent(n),
            )));
    };
    col()
        .w_full()
        .gap(4.0)
        .child(text("Agent").font(k.ty.small).color(c.text_muted))
        .child(control)
}

fn inputs(s: &App, ed: &Editor, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let mut out = col().key("wfe-inputs").w_full().gap(8.0).child(
        row()
            .w_full()
            .center_y()
            .gap(8.0)
            .child(
                text("Inputs: what a run is given")
                    .font(k.ty.ui_medium)
                    .color(c.text_base),
            )
            .child(el().grow(1.0))
            .child(
                small("Add input", ButtonVariant::Outline)
                    .on_press(|s: &mut App| s.editor_add_input())
                    .build(k)
                    .key("wfe-add-input"),
            ),
    );
    for (i, row_) in ed.inputs.iter().enumerate() {
        let kind = row_.kind;
        let picker = pick(
            format!("wfe-input-kind-{i}"),
            Menu::InputKind(i),
            &INPUT_KINDS,
            Some(kind),
            "Type",
            false,
            s,
            k,
            layer,
            move |s, name| s.editor_set_input_kind(i, name),
        );
        out = out.child(
            col()
                .key(format!("wfe-input-{i}"))
                .w_full()
                .gap(6.0)
                .padding(12.0)
                .radius(k.radius.md)
                .fill(c.layer_01)
                .child(
                    row()
                        .w_full()
                        .center_y()
                        .gap(8.0)
                        .child(col().grow(1.0).child(field(
                            "Name",
                            &row_.name,
                            &format!("wfe-input-name-{i}"),
                            "folder",
                            1,
                            None,
                            k,
                            move |s: &mut App| s.editor_text(&Target::InputName(i)),
                        )))
                        .child(picker)
                        .child(
                            small("Remove", ButtonVariant::Ghost)
                                .on_press(move |s: &mut App| s.editor_remove_input(i))
                                .build(k)
                                .key(format!("wfe-input-remove-{i}")),
                        ),
                )
                .child(field(
                    "Value when none is given (empty: it is needed)",
                    &row_.default,
                    &format!("wfe-input-default-{i}"),
                    "",
                    1,
                    None,
                    k,
                    move |s: &mut App| s.editor_text(&Target::InputDefault(i)),
                ))
                .child(field(
                    "What it is",
                    &row_.description,
                    &format!("wfe-input-description-{i}"),
                    "Shown beside the box when running",
                    1,
                    None,
                    k,
                    move |s: &mut App| s.editor_text(&Target::InputDescription(i)),
                )),
        );
    }
    out
}

/// A key for the parts of the step at `at`, the same wherever it is drawn.
fn loc_key(at: &Loc) -> String {
    let mut key = String::new();
    for (a, b) in &at.place {
        key.push_str(&format!("{a}-{b}-"));
    }
    key.push_str(&at.index.to_string());
    key
}

/// The steps of the list at `place`, and a menu to add one more.
fn steps(s: &App, ed: &Editor, place: &Place, k: &Kit, layer: &mut Vec<El>) -> El {
    let list = list_at(ed, place);
    let mut out = col().w_full().gap(8.0);
    for (index, node) in list.iter().enumerate() {
        let at = Loc {
            place: place.clone(),
            index,
        };
        out = out.child(card(s, ed, &at, node, list.len(), true, k, layer));
    }
    // After an If or Switch there is nothing to add: each output goes on in its own list.
    if ed.ends_in_branch(place) {
        return out;
    }
    let target = place.clone();
    let key = format!(
        "wfe-add-{}",
        place
            .iter()
            .map(|(a, b)| format!("{a}-{b}"))
            .collect::<Vec<_>>()
            .join("-")
    );
    out.child(row().child(add_menu(
        key,
        Menu::Add(place.clone()),
        "+ Add a step",
        s,
        k,
        layer,
        move |s, label| s.editor_add(&target, label),
    )))
}

pub(super) fn list_at<'a>(ed: &'a Editor, place: &Place) -> &'a [Node] {
    let mut list: &[Node] = &ed.steps;
    for &(step, block) in place {
        match list.get(step).and_then(|n| n.blocks.get(block)) {
            Some(b) => list = &b.steps,
            None => return &[],
        }
    }
    list
}

#[allow(clippy::too_many_arguments)]
pub(super) fn card(
    s: &App,
    ed: &Editor,
    at: &Loc,
    n: &Node,
    len: usize,
    with_steps: bool,
    k: &Kit,
    layer: &mut Vec<El>,
) -> El {
    let c = &k.c;
    let key = loc_key(at);
    let up = at.clone();
    let down = at.clone();
    let gone = at.clone();
    let advanced = s.wf_advanced;
    let controls = row()
        .gap(2.0)
        .center_y()
        .child(
            small("Advanced", ButtonVariant::Ghost)
                .on_press(|s: &mut App| s.wf_advanced = !s.wf_advanced)
                .build(k)
                .key(format!("wfe-advanced-{key}")),
        )
        .child(
            small("↑", ButtonVariant::Ghost)
                .disabled(at.index == 0)
                .on_press(move |s: &mut App| s.editor_shift(&up, -1))
                .build(k)
                .key(format!("wfe-up-{key}")),
        )
        .child(
            small("↓", ButtonVariant::Ghost)
                .disabled(at.index + 1 >= len)
                .on_press(move |s: &mut App| s.editor_shift(&down, 1))
                .build(k)
                .key(format!("wfe-down-{key}")),
        )
        .child(
            small("Remove", ButtonVariant::Ghost)
                .on_press(move |s: &mut App| s.editor_remove(&gone))
                .build(k)
                .key(format!("wfe-remove-{key}")),
        );
    let label = text(n.label()).font(k.ty.ui_medium).color(c.text_base);
    let head = row()
        .w_full()
        .center_y()
        .gap(8.0)
        .child(label)
        .child(el().grow(1.0))
        .child(controls);
    let mut out = col()
        .key(format!("wfe-card-{key}"))
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(k.radius.md)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .child(head);
    // How the steps are named inside is for those who use the results by name.
    if advanced {
        let id_at = at.clone();
        let bind_at = at.clone();
        out = out.child(
            row()
                .w_full()
                .gap(8.0)
                .child(col().grow(1.0).min_w(0.0).child(field(
                    "Id",
                    &n.id,
                    &format!("wfe-id-{key}"),
                    "step-1",
                    1,
                    None,
                    k,
                    move |s: &mut App| s.editor_text(&Target::Id(id_at.clone())),
                )))
                .child(col().grow(1.0).min_w(0.0).child(field(
                    "Keep the result as",
                    &n.bind,
                    &format!("wfe-bind-{key}"),
                    "name",
                    1,
                    None,
                    k,
                    move |s: &mut App| s.editor_text(&Target::Bind(bind_at.clone())),
                ))),
        );
    }

    if n.kind == TOOL {
        // Handing a task to an agent is its own kind of step in the list, so it has no tool to
        // choose.
        if n.tool != crate::tools::DELEGATE {
            out = out.child(tool_field(s, at, n, &key, k, layer));
            if !n.tool.is_empty() {
                let about = crate::tools::shown(&n.tool, &s.mcp.servers).about;
                out = out.children(help_line(&about, k));
            }
        }
        for p in tool_params(s, at, n, &key, k, layer) {
            out = out.child(p);
        }
    }
    for (i, spec) in specs(&n.kind).iter().enumerate() {
        let Some(edit) = n.fields.get(i) else {
            continue;
        };
        if spec.key == "cond" && n.when.is_some() {
            out = out.child(comparison(s, ed, at, n, k, layer));
            continue;
        }
        let target = Target::Field(at.clone(), i);
        let f_at = at.clone();
        let box_id = format!("wfe-field-{key}-{i}");
        let complete = (spec.refs != Refs::None).then(|| Complete {
            at: at.clone(),
            key: spec.key.to_owned(),
            refs: spec.refs,
            target: target.clone(),
            anchor: box_id.clone(),
        });
        if let Some(c) = &complete {
            finishing(s, c, edit, k, layer);
        }
        out = out.child(field_with(
            spec.label,
            edit,
            &box_id,
            spec.hint,
            spec.lines,
            None,
            complete,
            k,
            move |s: &mut App| s.editor_text(&Target::Field(f_at.clone(), i)),
        ));
        if spec.key == "cond" {
            out = out.child(expression_toggle(at, n, k));
        }
    }
    if n.kind == GENERATE {
        out = out.children(reply_fields(at, n, &key, s, k, layer));
    }
    if n.kind == FOR_EACH || n.kind == PARALLEL {
        let on = n.go_on;
        let go = at.clone();
        let label = if n.kind == FOR_EACH {
            "Carry on when a round fails"
        } else {
            "Carry on when a branch fails"
        };
        out = out.child(
            row()
                .center_y()
                .gap(8.0)
                .child(
                    Switch::new(on)
                        .label(label)
                        .hide_label()
                        .on_toggle(move |s: &mut App| s.editor_toggle_go_on(&go))
                        .build(k),
                )
                .child(text(label).font(k.ty.small).color(c.text_muted)),
        );
    }
    if n.kind == SWITCH {
        let on = n.all;
        let flip = at.clone();
        let label = "Run every case that matches, not only the first";
        out = out.child(
            col()
                .w_full()
                .gap(4.0)
                .child(
                    row()
                        .center_y()
                        .gap(8.0)
                        .child(
                            Switch::new(on)
                                .label(label)
                                .hide_label()
                                .on_toggle(move |s: &mut App| s.editor_toggle_all(&flip))
                                .build(k),
                        )
                        .child(text(label).font(k.ty.small).color(c.text_muted)),
                )
                .children(help_line(
                    "Each case is compared with the value above. To test different conditions, switch on true and write a condition in each case, such as score > 50.",
                    k,
                )),
        );
    }
    for (b, block) in n.blocks.iter().enumerate() {
        out = out.child(inside(s, ed, at, b, block, with_steps, k, layer));
    }
    if n.kind == PARALLEL {
        let branch_at = at.clone();
        out = out.child(
            row().child(
                small("Add a branch", ButtonVariant::Outline)
                    .on_press(move |s: &mut App| s.editor_add_branch(&branch_at))
                    .build(k)
                    .key(format!("wfe-add-branch-{key}")),
            ),
        );
    }
    if n.kind == SWITCH {
        let case_at = at.clone();
        out = out.child(
            row().child(
                small("Add a case", ButtonVariant::Outline)
                    .on_press(move |s: &mut App| s.editor_add_case(&case_at))
                    .build(k)
                    .key(format!("wfe-add-case-{key}")),
            ),
        );
    }
    out
}

/// The tool a step uses, chosen from the same searchable list an agent's tools are.
fn tool_field(s: &App, at: &Loc, n: &Node, key: &str, k: &Kit, layer: &mut Vec<El>) -> El {
    use super::multi_picker::{Spec, build};
    use crate::picker::Item;
    let mine = s.wf_tool_step.as_ref() == Some(at);
    let items: Vec<Item> = s
        .step_tools(&n.tool)
        .into_iter()
        .map(|name| {
            let shown = crate::tools::shown(&name, &s.mcp.servers);
            Item {
                name,
                label: shown.label,
                description: shown.about,
                group: shown.group,
            }
        })
        .collect();
    let opened = at.clone();
    build(
        s,
        k,
        layer,
        Spec {
            key: crate::workflow::pause::intern(&format!("wfe-tool-{key}")),
            title: "Tool",
            placeholder: "Choose a tool",
            noun: ("tool", "tools"),
            none: "No tools to use.",
            items,
            chosen: Some(n.tool.clone())
                .filter(|t| !t.is_empty())
                .into_iter()
                .collect(),
            get: if mine {
                |s| &s.wf_tool_picker
            } else {
                |s| &s.wf_tool_closed
            },
            edit: |s| &mut s.wf_tool_picker,
            toggle: |s, name| s.editor_choose_tool(name),
            single: true,
            on_open: Some(std::rc::Rc::new(move |s: &mut App| {
                s.wf_tool_step = Some(opened.clone());
            })),
        },
    )
}

/// The arguments of a tool step as a form: a box for each, named in words, those the tool needs
/// first and the rest under "Optional".
#[allow(clippy::too_many_arguments)]
fn tool_params(s: &App, at: &Loc, n: &Node, key: &str, k: &Kit, layer: &mut Vec<El>) -> Vec<El> {
    let c = &k.c;
    let mut specs = s
        .step_tool_schema(&n.tool)
        .map(|schema| params_of(&schema))
        .unwrap_or_default();
    // What the step has that its tool does not list is not lost: it shows as it is.
    for (name, p) in &n.params {
        if !specs.iter().any(|x| &x.key == name) {
            specs.push(ParamSpec {
                key: name.clone(),
                label: name.clone(),
                about: String::new(),
                help: String::new(),
                kind: p.kind,
                required: false,
                options: Vec::new(),
                lines: if p.kind == ArgKind::Json { 3 } else { 1 },
            });
        }
    }
    let blank = TextEdit::new("");
    let mut out = Vec::new();
    let mut optional_shown = false;
    for p in specs {
        let given = n.params.get(&p.key);
        let typed = given.map_or("", |g| g.edit.text.as_str());
        // What the tool does not need follows under its own heading, so none is out of sight.
        if !p.required && !optional_shown {
            optional_shown = true;
            out.push(text("Optional").font(k.ty.small_medium).color(c.text_faint));
        }
        let label = p.label.clone();
        let (at_for, name, kind) = (at.clone(), p.key.clone(), p.kind);
        let box_key = format!("wfe-param-{key}-{}", p.key);
        let item = if !p.options.is_empty() {
            let options: Vec<&str> = p.options.iter().map(String::as_str).collect();
            let choose = (at.clone(), p.key.clone());
            col()
                .w_full()
                .gap(4.0)
                .child(text(label).font(k.ty.small).color(c.text_muted))
                .children(help_line(&p.help, k))
                .child(row().child(pick(
                    box_key,
                    Menu::Param(at.clone(), p.key.clone()),
                    &options,
                    Some(typed).filter(|t| !t.is_empty()),
                    "Choose",
                    true,
                    s,
                    k,
                    layer,
                    move |s, label| s.editor_set_param(&choose.0, &choose.1, ArgKind::Text, label),
                )))
        } else if p.kind == ArgKind::Flag {
            let on = typed.trim() == "true";
            let switch = row()
                .center_y()
                .gap(8.0)
                .child(
                    Switch::new(on)
                        .label(label.clone())
                        .hide_label()
                        .on_toggle(move |s: &mut App| {
                            s.editor_set_param(
                                &at_for,
                                &name,
                                ArgKind::Flag,
                                if on { "false" } else { "true" },
                            );
                        })
                        .build(k),
                )
                .child(text(label).font(k.ty.small).color(c.text_muted));
            col()
                .w_full()
                .gap(4.0)
                .child(switch)
                .children(help_line(&p.help, k))
        } else {
            let target = Target::Param(at.clone(), p.key.clone(), kind);
            let complete = (kind == ArgKind::Text).then(|| Complete {
                at: at.clone(),
                key: p.key.clone(),
                refs: Refs::Braces,
                target: target.clone(),
                anchor: box_key.clone(),
            });
            if let Some(c) = &complete {
                finishing(s, c, given.map_or(&blank, |g| &g.edit), k, layer);
            }
            col()
                .w_full()
                .gap(4.0)
                .child(field_with(
                    &label,
                    given.map_or(&blank, |g| &g.edit),
                    &box_key,
                    &p.about,
                    p.lines,
                    None,
                    complete,
                    k,
                    move |s: &mut App| {
                        s.editor_text(&Target::Param(at_for.clone(), name.clone(), kind))
                    },
                ))
                .children(help_line(&p.help, k))
        };
        out.push(item);
    }
    out
}

/// The fields an AI step answers with: none is plain text, some is an object of those fields that
/// later steps can read as `name.field`, in a condition, a switch or a loop.
fn reply_fields(at: &Loc, n: &Node, key: &str, s: &App, k: &Kit, layer: &mut Vec<El>) -> Vec<El> {
    let c = &k.c;
    let mut out = vec![
        text("Answer as fields")
            .font(k.ty.small_medium)
            .color(c.text_muted),
    ];
    out.extend(help_line(
        "Leave empty for plain text. With fields the AI must answer in this shape, and is asked again if it does not, so later steps can use result.field.",
        k,
    ));
    let labels: Vec<&str> = FieldKind::ALL.iter().map(|f| f.label()).collect();
    for (i, row_) in n.reply.iter().enumerate() {
        let (pick_at, rm_at) = (at.clone(), at.clone());
        let picker = pick(
            format!("wfe-reply-kind-{key}-{i}"),
            Menu::ReplyKind(at.clone(), i),
            &labels,
            Some(row_.kind.label()),
            "Type",
            false,
            s,
            k,
            layer,
            move |s, label| s.editor_set_reply_kind(&pick_at, i, label),
        );
        let name_at = at.clone();
        out.push(
            row()
                .w_full()
                .center_y()
                .gap(8.0)
                .child(col().grow(1.0).child(field(
                    "Field name",
                    &row_.name,
                    &format!("wfe-reply-{key}-{i}"),
                    "score",
                    1,
                    None,
                    k,
                    move |s: &mut App| s.editor_text(&Target::Reply(name_at.clone(), i)),
                )))
                .child(row().pt(18.0).child(picker))
                .child(
                    row().pt(18.0).child(
                        small("Remove", ButtonVariant::Ghost)
                            .on_press(move |s: &mut App| s.editor_remove_reply(&rm_at, i))
                            .build(k)
                            .key(format!("wfe-reply-remove-{key}-{i}")),
                    ),
                ),
        );
    }
    let add_at = at.clone();
    out.push(
        row().child(
            small("Add a field", ButtonVariant::Outline)
                .on_press(move |s: &mut App| s.editor_add_reply(&add_at))
                .build(k)
                .key(format!("wfe-add-reply-{key}")),
        ),
    );
    out
}

/// What a tool's argument is for, in small type under its name; nothing when it says nothing.
fn help_line(help: &str, k: &Kit) -> Option<El> {
    (!help.is_empty()).then(|| {
        text(help.to_owned())
            .font(k.ty.caption())
            .color(k.c.text_faint)
            .wrap()
    })
}

/// An `if` or `while` condition as "[result] [is] [value]".
fn comparison(s: &App, ed: &Editor, at: &Loc, n: &Node, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let Some(when) = &n.when else {
        return el();
    };
    let key = loc_key(at);
    let found = ed.results(at, Some("cond"));
    let labels: Vec<&str> = found.iter().map(|f| f.label.as_str()).collect();
    // What is chosen shows in words when it is one of the results, else as it was written.
    let current = found
        .iter()
        .find(|f| ed.named(&f.source).as_deref() == Some(when.left.as_str()))
        .map(|f| f.label.as_str())
        .or(Some(when.left.as_str()).filter(|l| !l.is_empty()));
    let (left_at, op_at) = (at.clone(), at.clone());
    let ops: Vec<&str> = cond::OPS.iter().map(|o| o.label()).collect();
    let mut line = col()
        .w_full()
        .gap(8.0)
        .child(
            text(if n.kind == WHILE {
                "Repeat while"
            } else {
                "Check whether"
            })
            .font(k.ty.small)
            .color(c.text_muted),
        )
        .child(row().w_full().child(pick(
            format!("wfe-left-{key}"),
            Menu::Left(at.clone()),
            &labels,
            current,
            "Choose a result",
            true,
            s,
            k,
            layer,
            move |s, label| s.editor_set_left(&left_at, label),
        )))
        .child(row().w_full().child(pick(
            format!("wfe-compare-{key}"),
            Menu::Compare(at.clone()),
            &ops,
            Some(when.op.label()),
            "How",
            true,
            s,
            k,
            layer,
            move |s, label| s.editor_set_compare(&op_at, label),
        )));
    if when.op.takes_value() {
        let right_at = at.clone();
        line = line.child(field(
            "Value",
            &when.right,
            &format!("wfe-right-{key}"),
            "pdf",
            1,
            None,
            k,
            move |s: &mut App| s.editor_text(&Target::Right(right_at.clone())),
        ));
    }
    line.child(expression_toggle(at, n, k))
}

/// The button that changes a condition between its parts and the expression it stands for. An
/// expression that is not of the parts' shape has none to go back to.
fn expression_toggle(at: &Loc, n: &Node, k: &Kit) -> El {
    let parts = n.when.is_some();
    let src = n.condition_text();
    if !parts && cond::parse(&src).is_none() && !src.trim().is_empty() {
        return el();
    }
    let at = at.clone();
    let key = loc_key(&at);
    row().child(
        small(
            if parts {
                "Write it as an expression"
            } else {
                "Use the simple form"
            },
            ButtonVariant::Ghost,
        )
        .on_press(move |s: &mut App| s.editor_toggle_expression(&at))
        .build(k)
        .key(format!("wfe-expression-{key}")),
    )
}

/// One list of steps inside a step, under its heading.
#[allow(clippy::too_many_arguments)]
fn inside(
    s: &App,
    ed: &Editor,
    at: &Loc,
    b: usize,
    block: &Block,
    with_steps: bool,
    k: &Kit,
    layer: &mut Vec<El>,
) -> El {
    let c = &k.c;
    let key = loc_key(at);
    let mut place = at.place.clone();
    place.push((at.index, b));
    let heading = match block.part {
        Part::Then => text("Then").font(k.ty.small).color(c.text_muted),
        Part::Else => text("Otherwise").font(k.ty.small).color(c.text_muted),
        Part::Default => text("Otherwise").font(k.ty.small).color(c.text_muted),
        Part::Body => text("Each round").font(k.ty.small).color(c.text_muted),
        Part::Try => text("Try").font(k.ty.small).color(c.text_muted),
        Part::Catch => text("If it still fails")
            .font(k.ty.small)
            .color(c.text_muted),
        Part::Case => text("When").font(k.ty.small).color(c.text_muted),
        Part::Branch => text("Branch").font(k.ty.small).color(c.text_muted),
    };
    let mut head = row().w_full().center_y().gap(8.0).child(heading);
    if block.part == Part::Case {
        let when_at = at.clone();
        let rm = at.clone();
        head = head
            .child(col().grow(1.0).child(field(
                "Equals",
                &block.when,
                &format!("wfe-when-{key}-{b}"),
                "\"notes\"",
                1,
                None,
                k,
                move |s: &mut App| s.editor_text(&Target::When(when_at.clone(), b)),
            )))
            .child(
                small("Remove case", ButtonVariant::Ghost)
                    .on_press(move |s: &mut App| s.editor_remove_case(&rm, b))
                    .build(k)
                    .key(format!("wfe-remove-case-{key}-{b}")),
            );
    }
    if block.part == Part::Branch {
        let name_at = at.clone();
        let rm = at.clone();
        head = head
            .child(col().grow(1.0).child(field(
                "Name",
                &block.when,
                &format!("wfe-branch-{key}-{b}"),
                "fetch",
                1,
                None,
                k,
                move |s: &mut App| s.editor_text(&Target::When(name_at.clone(), b)),
            )))
            .child(
                small("Remove branch", ButtonVariant::Ghost)
                    .on_press(move |s: &mut App| s.editor_remove_branch(&rm, b))
                    .build(k)
                    .key(format!("wfe-remove-branch-{key}-{b}")),
            );
    }
    let block_el = col()
        .key(format!("wfe-block-{key}-{b}"))
        .w_full()
        .gap(6.0)
        .pl(12.0)
        .child(head);
    // Beside the graph the steps inside are boxes there, so only the heading of the list is here.
    if with_steps {
        block_el.child(steps(s, ed, &place, k, layer))
    } else {
        block_el
    }
}
