//! The workflow editor: the steps as a stack of cards, each with the boxes of its kind, and a
//! list of steps inside a card for each branch or loop it holds. What is wrong with what is
//! typed is listed above the buttons.

use rune::{El, ScrollSpec, TextEdit, col, el, row, text, text_input};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Kit, SegmentedControl, Select, SelectAppearance, Switch,
};

use super::workflow_graph;
use crate::app::App;
use crate::flow_graph::View;
use crate::workflow::cond;
use crate::workflow::definition::{FOR_EACH, PARALLEL, STEP_TOOLS, SWITCH, TOOL, WHILE};
use crate::workflow::editor::{
    Block, Editor, INPUT_KINDS, KINDS, Loc, Menu, Node, Part, Place, Refs, Target, specs,
};

/// The width the editor's text stops at.
const WIDTH: f32 = 760.0;

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let Some(ed) = &s.wf_editor else {
        return el();
    };
    match s.wf_graph.shown {
        View::Cards => cards(s, ed, k, layer),
        View::Graph => graph(s, ed, k, layer),
    }
}

/// The title and the switch between the two views.
fn heading(s: &App, ed: &Editor, k: &Kit) -> El {
    let title = if ed.workflow.is_some() {
        "Change workflow"
    } else {
        "New workflow"
    };
    let shown = match s.wf_graph.shown {
        View::Cards => 0,
        View::Graph => 1,
    };
    row()
        .w_full()
        .center_y()
        .gap(12.0)
        .child(text(title).font(k.ty.body_medium).color(k.c.text_base))
        .child(
            SegmentedControl::new(&["Cards", "Graph"], Some(shown))
                .label("View")
                .fit()
                .on_pick(|s: &mut App, i, _| {
                    s.set_view(if i == 0 { View::Cards } else { View::Graph });
                })
                .build(k),
        )
        .child(history(ed, k))
}

/// Undo and redo, each naming what it would do.
fn history(ed: &Editor, k: &Kit) -> El {
    let button = |label: &str, what: Option<&str>, key: &str| {
        Button::new(label.to_owned())
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .disabled(what.is_none())
            .build(k)
            .key(key.to_owned())
            .help(match what {
                Some(w) => format!("{label} {}", w.to_lowercase()),
                None => format!("Nothing to {}", label.to_lowercase()),
            })
    };
    row()
        .gap(2.0)
        .child(button("Undo", ed.undoes(), "wfe-undo").on_press(|s: &mut App| s.undo_editor()))
        .child(button("Redo", ed.redoes(), "wfe-redo").on_press(|s: &mut App| s.redo_editor()))
}

/// Save, Check and Cancel.
fn buttons(k: &Kit) -> El {
    row()
        .gap(8.0)
        .child(
            Button::new("Save")
                .variant(ButtonVariant::Submit)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.save_editor())
                .build(k)
                .key("wfe-save"),
        )
        .child(
            Button::new("Check")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.check_editor())
                .build(k)
                .key("wfe-check"),
        )
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.cancel_editor())
                .build(k)
                .key("wfe-cancel"),
        )
}

/// Name, what it is for, who owns it, and what a run is given.
fn basics(s: &App, ed: &Editor, k: &Kit, layer: &mut Vec<El>) -> El {
    col()
        .w_full()
        .gap(14.0)
        .child(field(
            "Name",
            &ed.name,
            "wfe-name",
            "Weekly report",
            1,
            None,
            k,
            |s: &mut App| s.editor_text(&Target::Name),
        ))
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
                    .font(k.ty.small)
                    .color(k.c.warning.icon)
                    .wrap()
                    .selectable(true)
            }),
        ));
    }
    out.child(test_run(s, ed, k))
}

/// Every step as a form, one under the other.
fn cards(s: &App, ed: &Editor, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    col()
        .key("workflow-editor")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .max_w(WIDTH)
        .padding(20.0)
        .gap(14.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(heading(s, ed, k))
        .child(basics(s, ed, k, layer))
        .child(text("Steps").font(k.ty.ui_medium).color(c.text_base))
        .child(steps(s, ed, &Vec::new(), k, layer))
        .child(tail(s, ed, k))
        .child(buttons(k))
}

/// The steps as boxes and wires, with the form of the selected one beside them.
fn graph(s: &App, ed: &Editor, k: &Kit, layer: &mut Vec<El>) -> El {
    let (place, index) = s.add_position();
    let target = place.clone();
    let header = heading(s, ed, k)
        .child(add_menu(
            "wfg-add-step".to_owned(),
            Menu::AddStep,
            "Add step",
            s,
            k,
            layer,
            move |s, label| s.editor_add_at(&target, index, label),
        ))
        .child(el().grow(1.0))
        .children(s.has_last_run().then(|| {
            let on = s.wf_graph.show_last;
            small(
                if on { "Hide last run" } else { "Show last run" },
                ButtonVariant::Ghost,
            )
            .on_press(move |s: &mut App| {
                s.hide_run();
                s.wf_graph.show_last = !on;
            })
            .build(k)
            .key("wfg-last-run")
        }))
        .child(
            small("Fit", ButtonVariant::Ghost)
                .on_press(|s: &mut App| s.fit_graph())
                .build(k)
                .key("wfg-fit"),
        )
        .child(buttons(k));
    col()
        .key("workflow-editor")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(20.0)
        .gap(12.0)
        .child(header)
        .children(workflow_graph::banner(s, ed, k))
        .child(
            row()
                .grow(1.0)
                .min_h(0.0)
                .gap(16.0)
                .child(workflow_graph::view(s, ed, k, layer))
                .child(inspector(s, ed, k, layer)),
        )
}

/// The form of the selected step, or the workflow's own when nothing is selected.
fn inspector(s: &App, ed: &Editor, k: &Kit, layer: &mut Vec<El>) -> El {
    let mut out = col()
        .key("wfg-inspector")
        .w(workflow_graph::INSPECTOR_W)
        .h_full()
        .min_h(0.0)
        .gap(14.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let chosen = s.selected_step().and_then(|at| {
        let list = list_at(ed, &at.place);
        list.get(at.index).map(|n| (at, n, list.len()))
    });
    match chosen {
        Some((at, node, len)) => {
            out = out
                .child(
                    row().child(
                        small("Workflow settings", ButtonVariant::Ghost)
                            .on_press(|s: &mut App| s.select_step(None))
                            .build(k)
                            .key("wfg-settings"),
                    ),
                )
                .child(card(s, ed, &at, node, len, false, k, layer))
                .children(workflow_graph::run_panel(s, ed, &at, k));
        }
        None => {
            out = out.child(basics(s, ed, k, layer)).child(tail(s, ed, k));
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
                .font(k.ty.small)
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
                .font(k.ty.small)
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
    let c = &k.c;
    let style = rune::TextStyle {
        line_height: 22.0,
        ..k.ty.body
    };
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
            .child(
                text_input(edit, style)
                    .key(key.to_owned())
                    .w_full()
                    .color(c.text_base)
                    .placeholder(hint.to_owned(), c.text_faint)
                    .caret(c.text_base, k.selection())
                    .label(label.to_owned())
                    .on_edit(f),
            ),
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

fn list_at<'a>(ed: &'a Editor, place: &Place) -> &'a [Node] {
    let mut list: &[Node] = &ed.steps;
    for &(step, block) in place {
        match list.get(step).and_then(|n| n.blocks.get(block)) {
            Some(b) => list = &b.steps,
            None => return &[],
        }
    }
    list
}

fn kind_label(kind: &str) -> &'static str {
    KINDS
        .iter()
        .find(|x| x.kind == kind)
        .map_or("Step", |x| x.label)
}

#[allow(clippy::too_many_arguments)]
fn card(
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
    let id_at = at.clone();
    let bind_at = at.clone();
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
    let label = text(kind_label(&n.kind))
        .font(k.ty.ui_medium)
        .color(c.text_base);
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

    if n.kind == crate::workflow::definition::TOOL {
        let tool_at = at.clone();
        // The built-in tools, then those of the MCP servers that are on, and the one this step
        // already names if its server is not.
        let mut tools: Vec<String> = STEP_TOOLS.iter().map(|t| (*t).to_owned()).collect();
        for server in s.mcp.servers.iter().filter(|m| m.offers()) {
            tools.extend(server.tools.iter().map(|t| server.qualified(&t.name)));
        }
        // Handing a task over needs a team to hand it to.
        if s.roster
            .agent_ref(&ed.agent_id)
            .is_some_and(|a| !a.subagents.is_empty())
        {
            tools.push(crate::tools::DELEGATE.to_owned());
        }
        if !n.tool.is_empty() && !tools.contains(&n.tool) {
            tools.push(n.tool.clone());
        }
        let tools: Vec<&str> = tools.iter().map(String::as_str).collect();
        out = out.child(row().child(pick(
            format!("wfe-tool-{key}"),
            Menu::Tool(at.clone()),
            &tools,
            Some(&n.tool),
            "Tool",
            true,
            s,
            k,
            layer,
            move |s, name| s.editor_set_tool(&tool_at, name),
        )));
    }
    for (i, spec) in specs(&n.kind).iter().enumerate() {
        let Some(edit) = n.fields.get(i) else {
            continue;
        };
        if spec.key == "cond" && n.when.is_some() {
            out = out.child(comparison(s, ed, at, n, k, layer));
            continue;
        }
        // A shell command is typed as it is; the JSON of the other tools' arguments, and of a
        // shell command with more than a command, is there for those who ask or whose step has it.
        let shell = n.tool == crate::tools::SHELL;
        if n.kind == TOOL
            && ((spec.key == "command" && !shell)
                || (spec.key == "args" && shell && !advanced && edit.text.trim().is_empty()))
        {
            continue;
        }
        let target = Target::Field(at.clone(), i);
        let names = names_for(s, ed, at, spec.key, spec.refs, &target, &key, k, layer);
        let f_at = at.clone();
        out = out.child(field(
            spec.label,
            edit,
            &format!("wfe-field-{key}-{i}"),
            spec.hint,
            spec.lines,
            names,
            k,
            move |s: &mut App| s.editor_text(&Target::Field(f_at.clone(), i)),
        ));
        if spec.key == "cond" {
            out = out.child(expression_toggle(at, n, k));
        }
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

/// The picker of what a box can use, when the box reads results: "Output of: `ls`", not the name
/// the step keeps it under.
#[allow(clippy::too_many_arguments)]
fn names_for(
    s: &App,
    ed: &Editor,
    at: &Loc,
    box_key: &str,
    refs: Refs,
    target: &Target,
    key: &str,
    k: &Kit,
    layer: &mut Vec<El>,
) -> Option<El> {
    if refs == Refs::None {
        return None;
    }
    let found = ed.results(at, Some(box_key));
    if found.is_empty() {
        return None;
    }
    let list: Vec<&str> = found.iter().map(|f| f.label.as_str()).collect();
    let (target_for, at_for, box_for) = (target.clone(), at.clone(), box_key.to_owned());
    Some(pick(
        format!("wfe-names-{key}-{box_key}"),
        Menu::Names(target.clone()),
        &list,
        None,
        "Use a result",
        false,
        s,
        k,
        layer,
        move |s, label| {
            s.editor_use(&target_for, &at_for, &box_for, refs == Refs::Braces, label);
        },
    ))
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
