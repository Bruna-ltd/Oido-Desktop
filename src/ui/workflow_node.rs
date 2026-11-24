//! A step in its own window, as n8n opens a node: what came before on the left, as a tree whose
//! names go into the step's boxes when pressed; the step's settings in the middle; what it made
//! when it ran on the right.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Dialog, Kit};
use serde_json::Value;

use super::workflow_editor::{card, list_at, step_actions};
use crate::app::App;
use crate::flow_node::{self, Row};
use crate::workflow::definition::plain;
use crate::workflow::editor::{Avail, Editor, Loc, Root};

const SIDE_W: f32 = 300.0;
const INDENT: f32 = 14.0;

/// The window of the selected step, when it is open.
pub fn window(s: &App, ed: &Editor, k: &Kit, layer: &mut Vec<El>) {
    let Some(at) = s.selected_step().filter(|_| s.node_open()) else {
        return;
    };
    let list = list_at(ed, &at.place);
    let Some(node) = list.get(at.index) else {
        return;
    };
    let [_, _, vw, vh] = s.wf_graph.viewport;
    let width = (vw + 360.0).clamp(820.0, 1280.0);
    let height = (vh + 120.0).clamp(460.0, 720.0);
    let title = plain(&node.to_step("", &mut Vec::new()));
    // Lists that open from the boxes inside are layers too, and go above the window's own.
    let mut inner: Vec<El> = Vec::new();
    let body = row()
        .key("wfn-body")
        .w_full()
        .h(height)
        .min_h(0.0)
        .child(input_pane(s, ed, &at, k))
        .child(divider(k))
        .child(settings_pane(s, ed, &at, node, list.len(), k, &mut inner))
        .child(divider(k))
        .child(output_pane(s, &at, k));
    Dialog::new("wf-node", title)
        .open(true)
        .width(width)
        .fit()
        .child(body)
        .on_close(|s: &mut App| s.close_node())
        .build(k, layer);
    layer.extend(inner);
}

fn divider(k: &Kit) -> El {
    el().w(1.0).h_full().fill(k.c.border_muted)
}

/// One side of the window: a heading, and under it a body that scrolls.
fn pane(title: &str, extra: Option<El>, body: El, k: &Kit) -> El {
    let c = &k.c;
    col()
        .w(SIDE_W)
        .h_full()
        .min_h(0.0)
        .child(
            row()
                .w_full()
                .px(14.0)
                .py(10.0)
                .gap(8.0)
                .center_y()
                .child(
                    text(title.to_owned())
                        .font(k.ty.ui_medium)
                        .color(c.text_base),
                )
                .child(el().grow(1.0))
                .children(extra),
        )
        .child(el().w_full().h(1.0).fill(c.border_muted))
        .child(body.w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec {
            stick_bottom: false,
        }))
}

fn faint(words: &str, k: &Kit) -> El {
    text(words.to_owned())
        .font(k.ty.caption())
        .color(k.c.text_faint)
        .wrap()
}

fn input_pane(s: &App, ed: &Editor, at: &Loc, k: &Kit) -> El {
    let c = &k.c;
    let before = at.clone();
    let run = Button::new("Run what comes before")
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .on_press(move |s: &mut App| s.run_before(&before))
        .build(k)
        .key("wfn-run-before");
    let mut body = col().padding(10.0).gap(12.0);
    let found = ed.available(at);
    if found.is_empty() {
        body = body.child(faint(
            "Nothing comes before this step, so there is nothing to use yet. Add an input to the workflow, or put a step before it.",
            k,
        ));
    }
    for (i, a) in found.iter().enumerate() {
        body = body.child(section(s, at, i, a, k));
    }
    body = body.child(
        text("Press a name to put it in the box you are typing in.")
            .font(k.ty.caption())
            .color(c.text_faint)
            .wrap(),
    );
    pane("Input", Some(run), body, k)
}

/// One thing the step can use: its title, which puts the whole of it in a box, and what it holds.
fn section(s: &App, at: &Loc, i: usize, a: &Avail, k: &Kit) -> El {
    let c = &k.c;
    let value = match &a.root {
        Root::Step(loc) => s.result_of(loc),
        Root::Name(_) => None,
    };
    let rows = flow_node::rows(
        &format!("in{i}"),
        value.as_ref(),
        &a.fields,
        &s.wf_graph.flipped,
    );
    let (to, root) = (at.clone(), a.root.clone());
    let head = k
        .interactive(
            row()
                .key(format!("wfn-in-{i}"))
                .w_full()
                .px(6.0)
                .py(4.0)
                .radius(k.radius.sm)
                .fill(c.layer_01)
                .preserve_focus_on_press(true)
                .help("Put all of it in the box")
                .on_press(move |s: &mut App| s.map_into(&to, &root, &[]))
                .child(
                    text(a.title.clone())
                        .font(k.ty.small_medium)
                        .color(c.text_base)
                        .truncate(),
                ),
        )
        .w_full();
    let mut out = col().w_full().gap(2.0).child(head);
    if rows.is_empty() {
        out = out.child(col().px(6.0).child(faint(
            match (&a.root, &value) {
                (Root::Step(_), Some(_)) => "It made a single value.",
                (Root::Step(_), None) => "Not run yet: no fields known. Run what comes before.",
                _ => "A single value.",
            },
            k,
        )));
    }
    for (j, r) in rows.iter().enumerate() {
        let (to, root, path) = (at.clone(), a.root.clone(), r.path.clone());
        let press = (!path.is_empty())
            .then(|| Box::new(move |s: &mut App| s.map_into(&to, &root, &path)) as Press);
        out = out.child(tree_row(r, format!("wfn-in-{i}-{j}"), press, k));
    }
    out
}

/// One line of a tree: its caret, name and a few words of what it holds. Pressing the name does
/// `press`; pressing the caret opens or shuts it.
type Press = Box<dyn Fn(&mut App)>;

fn tree_row(r: &Row, key: String, press: Option<Press>, k: &Kit) -> El {
    let c = &k.c;
    let flip = r.id.clone();
    let caret: El = if r.branch {
        k.interactive(
            el().key(format!("{key}-caret"))
                .size(16.0, 18.0)
                .preserve_focus_on_press(true)
                .on_press(move |s: &mut App| s.flip_branch(&flip))
                .child(
                    text(if r.open { "⌄" } else { "›" })
                        .font(k.ty.small)
                        .color(c.text_muted),
                ),
        )
    } else {
        el().w(16.0)
    };
    let mut line = row()
        .key(key)
        .w_full()
        .center_y()
        .gap(4.0)
        .pl(6.0 + r.depth as f32 * INDENT)
        .pr(6.0)
        .h(22.0)
        .radius(k.radius.sm)
        .child(caret)
        .child(
            text(r.label.clone())
                .font(k.ty.mono)
                .color(c.text_base)
                .truncate(),
        )
        .child(
            text(r.preview.clone())
                .font(k.ty.small)
                .color(c.text_faint)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .preserve_focus_on_press(true);
    if let Some(press) = press {
        line = k.interactive(line.on_press(move |s: &mut App| press(s)));
    }
    line
}

fn settings_pane(
    s: &App,
    ed: &Editor,
    at: &Loc,
    node: &crate::workflow::editor::Node,
    len: usize,
    k: &Kit,
    layer: &mut Vec<El>,
) -> El {
    col().grow(1.0).min_w(0.0).h_full().min_h(0.0).child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .padding(14.0)
            .gap(12.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(step_actions(s, ed, at, node, k, layer))
            .child(card(s, ed, at, node, len, false, k, layer)),
    )
}

fn output_pane(s: &App, at: &Loc, k: &Kit) -> El {
    let c = &k.c;
    let made = s.made_by(at);
    let note = made
        .as_ref()
        .map(|m| m.note.clone())
        .filter(|n| !n.is_empty());
    let mut body = col().padding(10.0).gap(8.0);
    match made {
        None => {
            let go = at.clone();
            body = body
                .child(faint(
                    "Not run yet. Execute this step to see what it makes.",
                    k,
                ))
                .child(
                    row().child(
                        Button::new("Execute step")
                            .variant(ButtonVariant::Outline)
                            .control_size(ControlSize::Small)
                            .on_press(move |s: &mut App| s.run_step(&go))
                            .build(k)
                            .key("wfn-run"),
                    ),
                );
        }
        Some(m) => {
            if m.running {
                body = body.child(faint("Still running.", k));
            }
            if let Some(why) = &m.error {
                body = body.child(
                    text(why.clone())
                        .font(k.ty.caption())
                        .color(c.warning.icon)
                        .wrap()
                        .selectable(true),
                );
            }
            match &m.value {
                Some(v @ (Value::Object(_) | Value::Array(_))) => {
                    let rows = flow_node::rows("out", Some(v), &[], &s.wf_graph.flipped);
                    if rows.is_empty() {
                        body = body.child(faint("It made an empty list.", k));
                    }
                    for (j, r) in rows.iter().enumerate() {
                        body = body.child(tree_row(r, format!("wfn-out-{j}"), None, k));
                    }
                }
                Some(v) => {
                    let said = match v {
                        Value::String(t) => t.clone(),
                        other => other.to_string(),
                    };
                    let (cut, more) = crate::runs::clip(&said, 4000);
                    body = body.child(
                        text(if more { format!("{cut}…") } else { cut })
                            .font(k.ty.mono)
                            .color(c.text_base)
                            .wrap()
                            .selectable(true),
                    );
                }
                None if m.error.is_none() && !m.running => {
                    body = body.child(faint("Nothing came back.", k));
                }
                None => {}
            }
        }
    }
    pane(
        "Output",
        note.map(|n| text(n).font(k.ty.small).color(c.text_faint)),
        body,
        k,
    )
}
