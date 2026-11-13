//! The workflow editor: the steps as a stack of cards, each with the boxes of its kind, and a
//! list of steps inside a card for each branch or loop it holds. What is wrong with what is
//! typed is listed above the buttons.

use rune::{El, ScrollSpec, TextEdit, col, el, row, text, text_input};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Select, SelectAppearance, Switch};

use crate::app::App;
use crate::workflow::definition::{FOR_EACH, STEP_TOOLS, SWITCH};
use crate::workflow::editor::{
    Block, Editor, INPUT_KINDS, KINDS, Loc, Menu, Node, Part, Place, Refs, Target, specs,
};

/// The width the editor's text stops at.
const WIDTH: f32 = 760.0;

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let Some(ed) = &s.wf_editor else {
        return el();
    };
    let c = &k.c;
    let title = if ed.workflow.is_some() {
        "Change workflow"
    } else {
        "New workflow"
    };
    let mut out = col()
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
        .child(text(title).font(k.ty.body_medium).color(c.text_base))
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
        ));
    out = out.child(agent_row(s, ed, k, layer));
    out = out.child(inputs(s, ed, k, layer));
    out = out
        .child(text("Steps").font(k.ty.ui_medium).color(c.text_base))
        .child(steps(s, ed, &Vec::new(), k, layer))
        .child(field(
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
    out.child(
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
            ),
    )
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
fn pick<F>(
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
        out = out.child(card(s, ed, &at, node, list.len(), k, layer));
    }
    let labels: Vec<&str> = KINDS.iter().map(|(_, l)| *l).collect();
    let target = place.clone();
    let key = format!(
        "wfe-add-{}",
        place
            .iter()
            .map(|(a, b)| format!("{a}-{b}"))
            .collect::<Vec<_>>()
            .join("-")
    );
    out.child(row().child(pick(
        key,
        Menu::Add(place.clone()),
        &labels,
        None,
        "+ Add a step",
        false,
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
        .find(|(k, _)| *k == kind)
        .map_or("Step", |(_, l)| *l)
}

fn card(s: &App, ed: &Editor, at: &Loc, n: &Node, len: usize, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let key = loc_key(at);
    let up = at.clone();
    let down = at.clone();
    let gone = at.clone();
    let id_at = at.clone();
    let bind_at = at.clone();
    let head = row()
        .w_full()
        .center_y()
        .gap(8.0)
        .child(
            text(kind_label(&n.kind))
                .font(k.ty.ui_medium)
                .color(c.text_base),
        )
        .child(col().w(150.0).child(field(
            "Id",
            &n.id,
            &format!("wfe-id-{key}"),
            "step-1",
            1,
            None,
            k,
            move |s: &mut App| s.editor_text(&Target::Id(id_at.clone())),
        )))
        .child(col().w(150.0).child(field(
            "Keep the result as",
            &n.bind,
            &format!("wfe-bind-{key}"),
            "name",
            1,
            None,
            k,
            move |s: &mut App| s.editor_text(&Target::Bind(bind_at.clone())),
        )))
        .child(el().grow(1.0))
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
    let mut out = col()
        .key(format!("wfe-card-{key}"))
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(k.radius.md)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .child(head);

    if n.kind == crate::workflow::definition::TOOL {
        let tool_at = at.clone();
        out = out.child(row().child(pick(
            format!("wfe-tool-{key}"),
            Menu::Tool(at.clone()),
            &STEP_TOOLS,
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
    }
    if n.kind == FOR_EACH {
        let on = n.go_on;
        let go = at.clone();
        out = out.child(
            row()
                .center_y()
                .gap(8.0)
                .child(
                    Switch::new(on)
                        .label("Carry on when a round fails")
                        .hide_label()
                        .on_toggle(move |s: &mut App| s.editor_toggle_go_on(&go))
                        .build(k),
                )
                .child(
                    text("Carry on when a round fails")
                        .font(k.ty.small)
                        .color(c.text_muted),
                ),
        );
    }
    for (b, block) in n.blocks.iter().enumerate() {
        out = out.child(inside(s, ed, at, b, block, k, layer));
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

/// The picker of names for a box, when the box reads names.
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
    let names = ed.names(at, Some(box_key));
    if names.is_empty() {
        return None;
    }
    let list: Vec<&str> = names.iter().map(String::as_str).collect();
    let target_for = target.clone();
    Some(pick(
        format!("wfe-names-{key}-{box_key}"),
        Menu::Names(target.clone()),
        &list,
        None,
        "Insert a name",
        false,
        s,
        k,
        layer,
        move |s, name| s.editor_insert(&target_for, refs == Refs::Braces, name),
    ))
}

/// One list of steps inside a step, under its heading.
#[allow(clippy::too_many_arguments)]
fn inside(
    s: &App,
    ed: &Editor,
    at: &Loc,
    b: usize,
    block: &Block,
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
        Part::Case => text("When").font(k.ty.small).color(c.text_muted),
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
    col()
        .key(format!("wfe-block-{key}-{b}"))
        .w_full()
        .gap(6.0)
        .pl(12.0)
        .child(head)
        .child(steps(s, ed, &place, k, layer))
}
