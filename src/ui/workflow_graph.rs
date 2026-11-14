//! The workflow editor's graph view: the steps as boxes and wires, frames around the steps that
//! hold other steps, and the form of the selected one beside them. Where things go is
//! `workflow/layout.rs`; what a key or the camera does is `flow_graph.rs`.

use rune::{
    El, Key, KeyEvent, LineCap, MagnifyEvent, Path, Shape, WheelEvent, canvas, col, el, icon, row,
    text,
};
use rune_kit::{Kit, glyphs};

use crate::app::App;
use crate::workflow::editor::{Editor, Loc};
use crate::workflow::layout::{Dir, Form, Item, Label, Layout, Rect, Wire, layout};

/// The width of the form beside the drawing.
pub const INSPECTOR_W: f32 = 380.0;

/// The drawing, for a viewport that grows to fill what is left.
pub fn view(s: &App, ed: &Editor, k: &Kit) -> El {
    let c = &k.c;
    let g = &s.wf_graph;
    let l = layout(ed);
    let selected = s.selected_step();
    let compact = g.compact();

    let mut world = el()
        .key("wfg-world")
        .pin(Some(s.world_origin()), Some(0.0), None, None)
        .size(l.width, l.height)
        .live_offset(&g.live_pan)
        .live_scale(&g.live_zoom);
    // Frames at the back, then the wires over them, then the steps, so a wire shows inside a
    // frame and a step is never covered.
    for item in l.items.iter().filter(|i| i.form == Form::Frame) {
        world = world.child(frame(item, selected.as_ref() == Some(&item.loc), k));
    }
    world = world.child(wires(&l, selected.as_ref(), k));
    for item in &l.items {
        world = match item.form {
            Form::Frame => world,
            Form::Step => world.child(step(item, selected.as_ref() == Some(&item.loc), compact, k)),
            Form::Empty => world.child(slot(item, k)),
        };
    }
    for label in &l.labels {
        world = world.child(chip(label, k));
    }
    world = world.child(start(&l.start, k));

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
        .on_key(|s: &mut App, e: &KeyEvent, _| {
            let m = e.mods;
            if m.shift || m.cmd || m.ctrl || m.alt {
                return false;
            }
            match e.key {
                Key::Up => s.move_selection(Dir::Up),
                Key::Down => s.move_selection(Dir::Down),
                Key::Left => s.move_selection(Dir::Left),
                Key::Right => s.move_selection(Dir::Right),
                Key::Escape => s.select_step(None),
                _ => return false,
            }
            true
        })
        .on_press(|s: &mut App| s.select_step(None))
        .child(world)
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

/// One step's box: what it does, and its id with what it keeps the result as.
fn step(item: &Item, selected: bool, compact: bool, k: &Kit) -> El {
    let c = &k.c;
    let at = item.loc.clone();
    let mut words = col().grow(1.0).min_w(0.0).gap(1.0).child(
        text(item.title.clone())
            .font(k.ty.ui_medium)
            .color(c.text_base)
            .truncate(),
    );
    if !compact {
        words = words.child(
            text(item.detail.clone())
                .font(k.ty.small)
                .color(c.text_muted)
                .truncate(),
        );
    }
    let mut e = pinned(row().key(format!("wfg-{}", loc_key(&item.loc))), &item.rect)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.md)
        .fill(if selected { c.layer_03 } else { c.layer_01 })
        .border(
            if selected { 2.0 } else { 1.0 },
            if selected { c.bg_accent } else { c.border_base },
        )
        .role("button")
        .label(accessible(item))
        .help(item.title.clone())
        .on_press(move |s: &mut App| s.select_step(Some(at.clone())));
    if item.model {
        // A model works here: dashed, and a spark, so it is not mistaken for a fixed step.
        e = e
            .border_dash(5.0, 3.0)
            .child(icon(&glyphs::SPARKLE, 14.0).color(c.text_muted));
    }
    k.interactive(e.child(words))
}

/// The frame around a step that holds other steps, with its title bar.
fn frame(item: &Item, selected: bool, k: &Kit) -> El {
    let c = &k.c;
    let at = item.loc.clone();
    let bar = row()
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
        .child(el().grow(1.0))
        .child(
            text(item.detail.clone())
                .font(k.ty.small)
                .color(c.text_muted)
                .truncate(),
        );
    let e = pinned(col().key(format!("wfg-{}", loc_key(&item.loc))), &item.rect)
        .radius(k.radius.lg)
        .fill(c.layer_02)
        .border(
            if selected { 2.0 } else { 1.0 },
            if selected {
                c.bg_accent
            } else {
                c.border_muted
            },
        )
        .role("button")
        .label(accessible(item))
        .help(item.title.clone())
        .on_press(move |s: &mut App| s.select_step(Some(at.clone())))
        .child(bar);
    k.interactive(e)
}

/// Where a list with nothing in it will take its first step.
fn slot(item: &Item, k: &Kit) -> El {
    let c = &k.c;
    pinned(
        row().key(format!("wfg-slot-{}", loc_key(&item.loc))),
        &item.rect,
    )
    .radius(k.radius.md)
    .border(1.0, c.border_base)
    .border_dash(4.0, 3.0)
    .center()
    .child(
        text(item.title.clone())
            .font(k.ty.small)
            .color(c.text_muted),
    )
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

/// What a screen reader says for a step.
fn accessible(item: &Item) -> String {
    let what = match item.form {
        Form::Frame => "holds other steps",
        _ if item.model => "a model writes or chooses here",
        _ => "runs the same way every time",
    };
    format!("{}, {what}. {}", item.title, item.detail)
}
