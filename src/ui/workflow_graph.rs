//! The workflow editor's graph view: the steps as boxes and wires, frames around the steps that
//! hold other steps, and the form of the selected one beside them. Where things go is
//! `workflow/layout.rs`; what a key or the camera does is `flow_graph.rs`.

use rune::{
    Drag, El, Key, KeyEvent, LineCap, MagnifyEvent, Path, PointerEvent, PointerPress, Shape,
    WheelEvent, canvas, col, el, icon, row, text,
};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, MenuItem, context_menu, glyphs};

use super::workflow_editor::pick;
use crate::app::App;
use crate::workflow::editor::{Editor, KINDS, Loc, Menu};
use crate::workflow::engine::StepLog;
use crate::workflow::layout::{
    Dir, Form, Gap, Item, Label, Layout, NODE_H, NODE_W, Rect, Wire, layout,
};
use crate::workflow::pause::Decision;
use crate::workflow::trace::{NodeRun, State, loc_of, problems_by_id, runs_by_step};

/// The width of the form beside the drawing.
pub const INSPECTOR_W: f32 = 380.0;

/// The drawing, for a viewport that grows to fill what is left.
pub fn view(s: &App, ed: &Editor, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let g = &s.wf_graph;
    let l = layout(ed);
    let selected = s.selected_step();
    let compact = g.compact();
    // What a run did at each step, and what is wrong with each.
    let shown = s.shown_run();
    let runs = shown.as_ref().map_or_else(Vec::new, |r| {
        runs_by_step(ed, &r.lines, r.waiting.as_deref())
    });
    let problems = problems_by_id(&ed.problems);
    let look = |item: &Item| Look {
        run: runs.iter().find(|(at, _)| *at == item.loc).map(|(_, r)| r),
        problem: problems.get(&item.id).map(String::as_str),
    };
    // The step being carried is dimmed where it was.
    let carried = g.drag.as_ref().filter(|d| d.moving).map(|d| d.from.clone());

    let mut world = el()
        .key("wfg-world")
        .pin(Some(s.world_origin()), Some(0.0), None, None)
        .size(l.width, l.height)
        .live_offset(&g.live_pan)
        .live_scale(&g.live_zoom);
    // Frames at the back, then the wires over them, then the steps, so a wire shows inside a
    // frame and a step is never covered.
    for item in l.items.iter().filter(|i| i.form == Form::Frame) {
        let faded = carried.as_ref() == Some(&item.loc);
        world = world.child(frame(
            item,
            selected.as_ref() == Some(&item.loc),
            faded,
            &look(item),
            k,
        ));
    }
    world = world.child(wires(&l, selected.as_ref(), k));
    for item in &l.items {
        world = match item.form {
            Form::Frame => world,
            Form::Step => {
                let faded = carried.as_ref() == Some(&item.loc);
                world.child(step(
                    item,
                    selected.as_ref() == Some(&item.loc),
                    faded,
                    compact,
                    &look(item),
                    k,
                ))
            }
            Form::Empty => world.child(slot(item, s, k, layer)),
        };
    }
    for label in &l.labels {
        world = world.child(chip(label, k));
    }
    world = world.child(start(&l.start, k));
    for gap in l.gaps.iter().filter(|g| shows(s, g, selected.as_ref())) {
        world = world.child(add_button(s, gap, k, layer));
    }
    world = world.children(carrying(s, &l, k));

    col()
        .key("wfg-view")
        .grow(1.0)
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .clip_soft(0.0)
        .focusable(true)
        .focus_ring(2.0, -2.0, c.bg_accent)
        .label("Workflow steps. Arrow keys move between steps.")
        .on_drawn_layout(|s: &mut App, r: [f32; 4]| {
            s.wf_graph.viewport = r;
            s.fit_graph_if_needed();
        })
        .on_wheel(|s: &mut App, e: WheelEvent, tx| {
            s.pan_graph(e.dx, e.dy);
            tx.live_only();
            true
        })
        .on_magnify(|s: &mut App, e: MagnifyEvent, tx| {
            // Only a zoom that crosses the size where nodes drop a line needs the view again.
            if !s.zoom_graph(e.x, e.y, e.delta) {
                tx.live_only();
            }
            true
        })
        .on_key(|s: &mut App, e: &KeyEvent, _| graph_key(s, e))
        .on_drag(|s: &mut App, d: &mut Drag, tx| s.drag_graph(d, tx))
        .on_secondary_press_local(|s: &mut App, p: PointerEvent, _| {
            let found = s.wf_editor.as_ref().and_then(|ed| {
                layout(ed)
                    .hit(s.world_point([p.x, p.y]))
                    .map(|i| i.loc.clone())
            });
            if let Some(at) = found {
                s.select_step(Some(at.clone()));
                s.wf_graph.menu = Some((at, [p.window_x, p.window_y]));
            }
        })
        .on_press(|s: &mut App| s.select_step(None))
        .child(world)
        .child(menu(s, k, layer))
}

/// A key in the drawing: arrows move the selection; Option and an arrow moves the step; Delete
/// removes it; Ctrl-D copies it; Ctrl-Z and Shift-Ctrl-Z undo and redo; `/` opens the list to add one.
fn graph_key(s: &mut App, e: &KeyEvent) -> bool {
    let m = e.mods;
    match (&e.key, m.cmd, m.shift, m.alt) {
        (Key::Command('z'), true, false, false) => s.undo_editor(),
        (Key::Command('z'), true, true, false) | (Key::Command('y'), true, false, false) => {
            s.redo_editor();
        }
        (Key::Command('d'), true, false, false) => s.duplicate_selected(),
        (Key::Up, false, false, true) => s.shift_selected(-1),
        (Key::Down, false, false, true) => s.shift_selected(1),
        (Key::Delete | Key::Backspace, false, false, false) => s.delete_selected(),
        (Key::Text(t), false, false, false) if t == "/" => s.open_add_menu(),
        (Key::Up, false, false, false) => s.move_selection(Dir::Up),
        (Key::Down, false, false, false) => s.move_selection(Dir::Down),
        (Key::Left, false, false, false) => s.move_selection(Dir::Left),
        (Key::Right, false, false, false) => s.move_selection(Dir::Right),
        (Key::Escape, false, false, false) => s.select_step(None),
        _ => return false,
    }
    true
}

/// What a menu item does to the step it was opened on: closes the menu, selects the step, acts.
fn on(at: &Loc, act: impl Fn(&mut App) + 'static) -> impl Fn(&mut App) + 'static {
    let at = at.clone();
    move |s: &mut App| {
        s.wf_graph.menu = None;
        s.select_step(Some(at.clone()));
        act(s);
    }
}

/// The menu a right-click opens on a step.
fn menu(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let Some((at, [x, y])) = s.wf_graph.menu.clone() else {
        return el();
    };
    context_menu(
        "wfg-context",
        el().size(0.0, 0.0),
        Some(PointerPress { x, y }),
    )
    .on_close(|s: &mut App| s.wf_graph.menu = None)
    .item(
        MenuItem::new("Duplicate")
            .shortcut("Ctrl+D")
            .on_select(on(&at, App::duplicate_selected)),
    )
    .item(
        MenuItem::new("Move up")
            .shortcut("Alt+Up")
            .on_select(on(&at, |s| s.shift_selected(-1))),
    )
    .item(
        MenuItem::new("Move down")
            .shortcut("Alt+Down")
            .on_select(on(&at, |s| s.shift_selected(1))),
    )
    .separator()
    .item(
        MenuItem::new("Delete")
            .shortcut("Del")
            .danger(true)
            .on_select(on(&at, App::delete_selected)),
    )
    .build(k, layer)
}

/// Whether the button to add a step at a gap shows: next to the selected step, on the gap whose
/// list is open, and at the end of the top list when nothing is selected, so the wires are not all
/// covered in buttons.
fn shows(s: &App, g: &Gap, selected: Option<&Loc>) -> bool {
    let open = s.wf_menu == Some(Menu::AddAt(g.place.clone(), g.index));
    let next_to = selected
        .is_some_and(|at| g.place == at.place && (g.index == at.index || g.index == at.index + 1));
    let bare_end = selected.is_none() && g.place.is_empty() && g.index == end_of_top(s);
    open || next_to || bare_end
}

fn end_of_top(s: &App) -> usize {
    s.wf_editor.as_ref().map_or(0, |ed| ed.steps.len())
}

/// The button on a wire that opens the list of steps to add there.
fn add_button(s: &App, g: &Gap, k: &Kit, layer: &mut Vec<El>) -> El {
    let labels: Vec<&str> = KINDS.iter().map(|(_, l)| *l).collect();
    let (place, index) = (g.place.clone(), g.index);
    let key = format!(
        "wfg-add-{}-{index}",
        loc_key(&Loc {
            place: g.place.clone(),
            index: 0
        })
    );
    el().pin(Some(g.at[0] - 14.0), Some(g.at[1] - 14.0), None, None)
        .size(28.0, 28.0)
        .center()
        .child(pick(
            key,
            Menu::AddAt(g.place.clone(), g.index),
            &labels,
            None,
            "+",
            false,
            s,
            k,
            layer,
            move |s, label| s.editor_add_at(&place, index, label),
        ))
}

/// What is drawn over the drawing while a step is carried: where it would land, the frame it
/// would be in, and the step itself under the pointer.
fn carrying(s: &App, l: &Layout, k: &Kit) -> Vec<El> {
    let c = &k.c;
    let Some(drag) = s.wf_graph.drag.as_ref().filter(|d| d.moving) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(drop) = drag.drop.as_ref().filter(|d| !d.unchanged || drag.copy) {
        if let Some(frame) = drop.frame.as_ref().and_then(|f| l.find(f)) {
            out.push(
                pinned(el().key("wfg-lit"), &frame.rect)
                    .radius(k.radius.lg)
                    .border(2.0, c.bg_accent),
            );
        }
        let m = &drop.marker;
        let line = if m.h > 4.0 {
            // An empty list's slot is outlined, not drawn through.
            pinned(el().key("wfg-marker"), m)
                .radius(k.radius.md)
                .border(2.0, c.bg_accent)
        } else {
            pinned(el().key("wfg-marker"), m)
                .radius(1.5)
                .fill(c.bg_accent)
        };
        out.push(line);
    }
    let title = l
        .find(&drag.from)
        .map(|i| i.title.clone())
        .unwrap_or_default();
    let ghost = Rect {
        x: drag.point[0] - drag.grab[0].min(NODE_W - 8.0),
        y: drag.point[1] - drag.grab[1].min(NODE_H - 8.0),
        w: NODE_W,
        h: NODE_H,
    };
    out.push(
        pinned(row().key("wfg-ghost"), &ghost)
            .px(12.0)
            .center_y()
            .radius(k.radius.md)
            .fill(c.layer_03)
            .border(2.0, c.bg_accent)
            .opacity(0.85)
            .child(
                text(if drag.copy {
                    format!("Copy of {title}")
                } else {
                    title
                })
                .font(k.ty.ui_medium)
                .color(c.text_base)
                .truncate(),
            ),
    );
    out
}

/// Every wire as one curve-drawing path; those that touch the selected step are drawn again in
/// the accent colour.
fn wires(l: &Layout, selected: Option<&Loc>, k: &Kit) -> El {
    let c = &k.c;
    let touching: Option<Rect> = selected.and_then(|at| l.find(at)).map(|i| i.rect);
    let mut plain = Path::new();
    let mut lit = Path::new();
    for w in &l.wires {
        let on = touching.is_some_and(|r| {
            let (top, bottom) = ([r.centre_x(), r.y], [r.centre_x(), r.bottom()]);
            w.to == top || w.from == bottom
        });
        let into = if on { &mut lit } else { &mut plain };
        *into = curve(std::mem::take(into), w);
    }
    let shape = Shape::new()
        .stroke_path(&plain, 1.5, LineCap::Round, c.border_strong)
        .stroke_path(&lit, 2.0, LineCap::Round, c.bg_accent);
    canvas(shape)
        .key("wfg-wires")
        .pin(Some(0.0), Some(0.0), None, None)
        .size(l.width, l.height)
}

/// One wire added to `path`: straight down when it is in line, else an S through the middle.
fn curve(path: Path, w: &Wire) -> Path {
    let (a, b) = (w.from, w.to);
    let path = path.move_to(a);
    if (a[0] - b[0]).abs() < 0.5 {
        return path.line_to(b);
    }
    let mid = (a[1] + b[1]) / 2.0;
    path.cubic_to([a[0], mid], [b[0], mid], b)
}

fn pinned(e: El, r: &Rect) -> El {
    e.pin(Some(r.x), Some(r.y), None, None).size(r.w, r.h)
}

fn loc_key(at: &Loc) -> String {
    let place: Vec<String> = at.place.iter().map(|(a, b)| format!("{a}-{b}")).collect();
    format!("{}-{}", place.join("-"), at.index)
}

/// What a run did at a step and what is wrong with it, to draw on its box.
struct Look<'a> {
    run: Option<&'a NodeRun>,
    problem: Option<&'a str>,
}

impl Look<'_> {
    fn state(&self) -> Option<State> {
        self.run.map(|r| r.state)
    }

    /// The colour of the box's edge when it has something to say, else none.
    fn edge(&self, k: &Kit) -> Option<rune::Color> {
        if self.problem.is_some() || self.state() == Some(State::Failed) {
            Some(k.c.warning.icon)
        } else if matches!(self.state(), Some(State::Running | State::Waiting)) {
            Some(k.c.bg_accent)
        } else {
            None
        }
    }

    /// The small mark at the right of the box: a glyph and a word, never colour alone.
    fn mark(&self, k: &Kit) -> Option<El> {
        let c = &k.c;
        if self.problem.is_some() {
            return Some(
                text("!")
                    .font(k.ty.ui_medium)
                    .color(c.warning.icon)
                    .help("This step has a problem"),
            );
        }
        let run = self.run?;
        Some(match run.state {
            State::Done => {
                let mut r = row()
                    .gap(3.0)
                    .center_y()
                    .child(icon(&glyphs::CHECK_SMALL, 14.0).color(c.text_muted));
                if run.rounds > 1 {
                    r = r.child(
                        text(format!("×{}", run.rounds))
                            .font(k.ty.small)
                            .color(c.text_muted),
                    );
                }
                r
            }
            State::Failed => row()
                .gap(3.0)
                .center_y()
                .child(icon(&glyphs::CIRCLE_X, 14.0).color(c.warning.icon))
                .child(text("failed").font(k.ty.small).color(c.warning.icon)),
            State::Running => row().child(text("running").font(k.ty.small).color(c.bg_accent)),
            State::Waiting => row().child(text("waiting").font(k.ty.small).color(c.bg_accent)),
        })
    }

    /// The second line of the box: the problem, or why it failed, or its id.
    fn second<'b>(&'b self, item: &'b Item) -> (&'b str, bool) {
        match (self.problem, self.run) {
            (Some(p), _) => (p, true),
            (None, Some(r)) if r.state == State::Failed => {
                (r.error.as_deref().unwrap_or("failed"), true)
            }
            _ => (item.detail.as_str(), false),
        }
    }
}

/// One step's box: what it does, and its id with what it keeps the result as.
fn step(item: &Item, selected: bool, faded: bool, compact: bool, look: &Look, k: &Kit) -> El {
    let c = &k.c;
    let at = item.loc.clone();
    let mut words = col().grow(1.0).min_w(0.0).gap(1.0).child(
        text(item.title.clone())
            .font(k.ty.ui_medium)
            .color(c.text_base)
            .truncate(),
    );
    if !compact {
        let (line, bad) = look.second(item);
        words = words.child(
            text(line.to_owned())
                .font(k.ty.small)
                .color(if bad { c.warning.icon } else { c.text_muted })
                .truncate(),
        );
    }
    let edge = look.edge(k);
    let mut e = pinned(row().key(format!("wfg-{}", loc_key(&item.loc))), &item.rect)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.md)
        .fill(if selected { c.layer_03 } else { c.layer_01 })
        .border(
            if selected || edge.is_some() { 2.0 } else { 1.0 },
            if selected {
                c.bg_accent
            } else {
                edge.unwrap_or(c.border_base)
            },
        )
        .role("button")
        .label(accessible(item, look))
        .help(tip(item, look))
        .opacity(if faded { 0.4 } else { 1.0 })
        .on_press(move |s: &mut App| s.select_step(Some(at.clone())));
    if item.model {
        // A model works here: dashed, and a spark, so it is not mistaken for a fixed step.
        e = e
            .border_dash(5.0, 3.0)
            .child(icon(&glyphs::SPARKLE, 14.0).color(c.text_muted));
    }
    e = e.child(words);
    if let Some(mark) = look.mark(k) {
        e = e.child(mark);
    }
    k.interactive(e)
}

/// The frame around a step that holds other steps, with its title bar.
fn frame(item: &Item, selected: bool, faded: bool, look: &Look, k: &Kit) -> El {
    let c = &k.c;
    let at = item.loc.clone();
    let mut bar = row()
        .w_full()
        .h(crate::workflow::layout::HEAD)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .child(
            text(item.title.clone())
                .font(k.ty.ui_medium)
                .color(c.text_base)
                .truncate(),
        )
        .child(el().grow(1.0));
    if let Some(mark) = look.mark(k) {
        bar = bar.child(mark);
    }
    let bar = bar.child(
        text(item.detail.clone())
            .font(k.ty.small)
            .color(c.text_muted)
            .truncate(),
    );
    let edge = look.edge(k);
    let e = pinned(col().key(format!("wfg-{}", loc_key(&item.loc))), &item.rect)
        .radius(k.radius.lg)
        .fill(c.layer_02)
        .border(
            if selected || edge.is_some() { 2.0 } else { 1.0 },
            if selected {
                c.bg_accent
            } else {
                edge.unwrap_or(c.border_muted)
            },
        )
        .role("button")
        .label(accessible(item, look))
        .help(tip(item, look))
        .opacity(if faded { 0.4 } else { 1.0 })
        .on_press(move |s: &mut App| s.select_step(Some(at.clone())))
        .child(bar);
    k.interactive(e)
}

/// Where a list with nothing in it will take its first step: press it to choose one.
fn slot(item: &Item, s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let labels: Vec<&str> = KINDS.iter().map(|(_, l)| *l).collect();
    let place = item.loc.place.clone();
    let target = place.clone();
    pinned(
        row().key(format!("wfg-slot-{}", loc_key(&item.loc))),
        &item.rect,
    )
    .radius(k.radius.md)
    .border(1.0, c.border_base)
    .border_dash(4.0, 3.0)
    .center()
    .child(pick(
        format!("wfg-slot-add-{}", loc_key(&item.loc)),
        Menu::AddAt(place, 0),
        &labels,
        None,
        "Nothing here yet: add a step",
        false,
        s,
        k,
        layer,
        move |s, label| s.editor_add_at(&target, 0, label),
    ))
}

/// The name of a column.
fn chip(label: &Label, k: &Kit) -> El {
    let c = &k.c;
    pinned(row(), &label.rect)
        .radius(k.radius.md)
        .fill(c.layer_03)
        .center()
        .child(
            text(label.text.clone())
                .font(k.ty.small)
                .color(c.text_muted)
                .truncate(),
        )
}

fn start(r: &Rect, k: &Kit) -> El {
    let c = &k.c;
    pinned(row().key("wfg-start"), r)
        .radius(k.radius.md)
        .fill(c.layer_03)
        .center()
        .child(text("Start").font(k.ty.small).color(c.text_base))
}

/// A line above the drawing while a run is on it: which run, how far it got, and, if it waits for
/// you, what it asks, with the buttons that answer.
pub fn banner(s: &App, ed: &Editor, k: &Kit) -> Option<El> {
    let c = &k.c;
    let shown = s.shown_run()?;
    let runs = runs_by_step(ed, &shown.lines, shown.waiting.as_deref());
    let total = layout(ed)
        .items
        .iter()
        .filter(|i| i.form != Form::Empty)
        .count();
    let failed = runs
        .iter()
        .filter(|(_, r)| r.state == State::Failed)
        .count();
    let mut said = format!("{} · reached {} of {total} steps", shown.label, runs.len());
    if failed > 0 {
        said.push_str(&format!(", {failed} failed"));
    }
    let mut out = row()
        .key("wfg-banner")
        .w_full()
        .center_y()
        .gap(10.0)
        .px(12.0)
        .py(8.0)
        .radius(k.radius.md)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .child(text(said).font(k.ty.small).color(c.text_base));
    if let Some((wf, run, pause)) = shown.answer {
        let button = |label: &str, key: &str, variant: ButtonVariant| {
            Button::new(label.to_owned())
                .variant(variant)
                .control_size(ControlSize::Small)
                .build(k)
                .key(key.to_owned())
        };
        let ask = if pause.until.is_none() {
            pause.prompt.clone()
        } else {
            format!(
                "{}. It goes on by itself when the wait is over.",
                pause.prompt
            )
        };
        let (w1, r1, w2, r2, w3, r3) = (wf.clone(), run.clone(), wf.clone(), run.clone(), wf, run);
        out = out
            .child(
                text(format!("Waiting: {ask}"))
                    .font(k.ty.small)
                    .color(c.text_base)
                    .truncate(),
            )
            .child(
                button(
                    if pause.until.is_none() {
                        "Approve"
                    } else {
                        "Go on now"
                    },
                    "wfg-approve",
                    ButtonVariant::Submit,
                )
                .on_press(move |s: &mut App| s.answer_wait(&w1, &r1, Decision::Go, true)),
            );
        if pause.until.is_none() {
            out = out.child(
                button("Decline", "wfg-decline", ButtonVariant::Outline)
                    .on_press(move |s: &mut App| s.answer_wait(&w2, &r2, Decision::Declined, true)),
            );
        }
        out = out.child(
            button("Stop it", "wfg-stop-wait", ButtonVariant::Ghost)
                .on_press(move |s: &mut App| s.stop_wait(&w3, &r3)),
        );
    }
    Some(
        out.child(el().grow(1.0)).child(
            Button::new("Hide run")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.hide_run())
                .build(k)
                .key("wfg-hide-run"),
        ),
    )
}

/// What the shown run did at the selected step, round by round, under its form.
pub fn run_panel(s: &App, ed: &Editor, at: &Loc, k: &Kit) -> Option<El> {
    let c = &k.c;
    let shown = s.shown_run()?;
    let mine: Vec<&StepLog> = shown
        .lines
        .iter()
        .filter(|l| loc_of(ed, &l.path).as_ref() == Some(at))
        .collect();
    if mine.is_empty() {
        return None;
    }
    let mut out = col()
        .key("wfg-run-panel")
        .w_full()
        .gap(6.0)
        .padding(12.0)
        .radius(k.radius.md)
        .border(1.0, c.border_muted)
        .child(
            text(format!("{}: this step", shown.label))
                .font(k.ty.ui_medium)
                .color(c.text_base),
        );
    for line in mine {
        let head = [line.round_note(), Some(format!("{} ms", line.ms))]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        let (body, bad) = match (&line.error, line.running) {
            (Some(e), _) => (e.clone(), true),
            (None, true) => ("Still running.".to_owned(), false),
            (None, false) if line.output.is_empty() => ("Nothing came back.".to_owned(), false),
            (None, false) => (line.output.clone(), false),
        };
        let (cut, more) = crate::runs::clip(&body, 800);
        out = out
            .child(text(head).font(k.ty.small).color(c.text_muted))
            .child(
                text(if more { format!("{cut}…") } else { cut })
                    .font(k.ty.mono)
                    .color(if bad { c.warning.icon } else { c.text_base })
                    .wrap()
                    .selectable(true),
            );
    }
    Some(out)
}

/// The hint on a step: its line, and under it what is wrong or why it failed, which a small
/// drawing leaves off the box.
fn tip(item: &Item, look: &Look) -> String {
    match look.second(item) {
        (said, true) => format!("{}\n{said}", item.title),
        _ => item.title.clone(),
    }
}

/// What a screen reader says for a step, with what a run did there and what is wrong with it.
fn accessible(item: &Item, look: &Look) -> String {
    let what = match item.form {
        Form::Frame => "holds other steps",
        _ if item.model => "a model writes or chooses here",
        _ => "runs the same way every time",
    };
    let mut said = format!("{}, {what}. {}", item.title, item.detail);
    if let Some(p) = look.problem {
        said.push_str(&format!(" Problem: {p}."));
    }
    if let Some(r) = look.run {
        let state = match r.state {
            State::Done => "done",
            State::Failed => "failed",
            State::Running => "running",
            State::Waiting => "waiting",
        };
        said.push_str(&format!(" In the run: {state}"));
        if r.rounds > 1 {
            said.push_str(&format!(", {} times", r.rounds));
        }
        said.push('.');
    }
    said
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use rune::Mods;
    use serde_json::json;

    use super::*;
    use crate::workflow::Saved;

    fn app() -> App {
        let mut app = crate::testing::samples();
        let w = Saved {
            id: "w1".into(),
            agent_id: "a1".into(),
            name: "Flow".into(),
            goal: String::new(),
            definition: serde_json::from_value(json!({"steps": [
                {"id": "a", "kind": "tool", "tool": "shell", "args": {"command": "echo a"}},
                {"id": "b", "kind": "tool", "tool": "shell", "args": {"command": "echo b"}},
                {"id": "c", "kind": "tool", "tool": "shell", "args": {"command": "echo c"}}
            ]}))
            .unwrap(),
            created_at: 0,
        };
        app.wf_editor = Some(Editor::from_saved(&w));
        app
    }

    fn key(k: Key, cmd: bool, shift: bool, alt: bool) -> KeyEvent {
        KeyEvent {
            key: k,
            mods: Mods {
                cmd,
                shift,
                alt,
                ..Mods::default()
            },
        }
    }

    fn order(app: &App) -> String {
        app.wf_editor
            .as_ref()
            .unwrap()
            .steps
            .iter()
            .map(|n| n.id.text.clone())
            .collect()
    }

    #[test]
    fn the_keys_move_edit_and_undo_in_the_drawing_and_leave_other_keys_alone() {
        let mut app = app();
        let press = |app: &mut App, k, cmd, shift, alt| graph_key(app, &key(k, cmd, shift, alt));
        assert!(
            press(&mut app, Key::Down, false, false, false),
            "starts at the first step"
        );
        assert!(press(&mut app, Key::Down, false, false, false));
        assert_eq!(app.selected_step().unwrap().index, 1);
        // Option-down moves the step, the selection with it.
        assert!(press(&mut app, Key::Down, false, false, true));
        assert_eq!(
            (order(&app).as_str(), app.selected_step().unwrap().index),
            ("acb", 2)
        );
        // Ctrl-D copies it, Delete removes the copy, Ctrl-Z brings the copy back, Shift-Ctrl-Z again.
        assert!(press(&mut app, Key::Command('d'), true, false, false));
        assert_eq!(app.wf_editor.as_ref().unwrap().steps.len(), 4);
        assert!(press(&mut app, Key::Delete, false, false, false));
        assert_eq!(app.wf_editor.as_ref().unwrap().steps.len(), 3);
        assert!(press(&mut app, Key::Command('z'), true, false, false));
        assert_eq!(app.wf_editor.as_ref().unwrap().steps.len(), 4);
        assert!(press(&mut app, Key::Command('z'), true, true, false));
        assert_eq!(app.wf_editor.as_ref().unwrap().steps.len(), 3);
        // `/` opens the list to add a step after the selected one.
        assert!(press(&mut app, Key::Text("/".into()), false, false, false));
        assert_eq!(app.wf_menu, Some(Menu::AddStep));
        // Escape clears the selection; a key it has no use for is not taken.
        assert!(press(&mut app, Key::Escape, false, false, false));
        assert_eq!(app.selected_step(), None);
        assert!(!press(&mut app, Key::Text("x".into()), false, false, false));
        assert!(
            !press(&mut app, Key::Up, false, true, false),
            "Shift-Up is not ours"
        );
    }
}
