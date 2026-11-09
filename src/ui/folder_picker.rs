//! The working-folder field: shows the chosen folder (or the home folder when none), and opens a
//! small browser to pick another. Rune has no system folder dialog, so the browser is drawn here.

use std::borrow::Cow;
use std::path::Path;
use std::rc::Rc;

use rune::{Color, El, GrowthSpec, ScrollSpec, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Placement, Popover,
    Select, SelectAppearance, glyphs, scroll_bar,
};

use crate::app::App;

const ROW: f32 = 30.0;
const MAX_HEIGHT: f32 = 240.0;
/// The most width Rune's popover gives its content.
const WIDTH: f32 = 320.0;
const KEY: &str = "workdir";

pub fn build(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let form = &s.roster.form;
    let chosen = form.workdir.clone();
    let shown = chosen.clone().unwrap_or_else(|| "Home folder".to_owned());
    let trigger = Select::new::<&str>(format!("{KEY}-value"), &[])
        .current(Some(&shown))
        .label("Working folder")
        .appearance(SelectAppearance::Field)
        .width(WIDTH)
        .trigger(k)
        .on_press(|s: &mut App| {
            let form = &mut s.roster.form;
            if form.folders.open {
                form.folders.close();
            } else {
                let start = form.workdir.clone();
                form.folders.show(start.as_deref().map(Path::new));
            }
        });
    let popover = Popover::new(KEY)
        .trigger(trigger)
        .label("Choose a folder")
        .placement(Placement::BottomStart)
        .padding(0.0)
        .open(form.folders.open)
        .on_close(|s: &mut App| s.roster.form.folders.close());
    let field = if form.folders.open {
        popover.child(browser(s, k)).build(k, layer)
    } else {
        popover.build(k, layer)
    };
    // Always wrapped, so a caller's key never lands on the popover's own anchor.
    let mut out = row().gap(4.0).center_y().child(field);
    if chosen.is_some() {
        out = out.child(
            IconButton::new(&glyphs::CLOSE)
                .variant(IconButtonVariant::GhostMuted)
                .label("Use the home folder")
                .on_press(|s: &mut App| s.roster.form.workdir = None)
                .build(k)
                .key(format!("{KEY}-clear")),
        );
    }
    out
}

fn browser(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let b = &s.roster.form.folders;
    let header = row()
        .key("header")
        .w_full()
        .px(8.0)
        .py(6.0)
        .gap(6.0)
        .center_y()
        .child(
            IconButton::new(&glyphs::CHEVRON_UP)
                .variant(IconButtonVariant::GhostMuted)
                .label("Up one folder")
                .disabled(b.at.parent().is_none())
                .on_press(|s: &mut App| s.roster.form.folders.up())
                .build(k)
                .key("up"),
        )
        .child(
            text(b.at.display().to_string())
                .font(k.ty.mono)
                .color(c.text_base)
                .grow(1.0)
                .shrink()
                .truncate(),
        );

    let inner = WIDTH - 8.0;
    let names = Rc::new(b.entries.clone());
    let count = names.len();
    let kit = k.clone();
    let list = col()
        .key("rows")
        .w(inner)
        .growth(GrowthSpec::default())
        .virtual_rows(
            count,
            ROW,
            {
                let names = names.clone();
                move |_: &App, i| Cow::Owned(format!("{KEY}-folder-{}", names[i]))
            },
            move |_: &App, i| folder_row(&kit, &names[i]),
        );
    let viewport = col()
        .key("viewport")
        .w(inner)
        .h((count as f32 * ROW).min(MAX_HEIGHT))
        .shrink()
        .min_h(if count == 0 { 0.0 } else { ROW })
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .label("Folders")
        .clip_soft(0.0)
        .child(list);
    let mut results = col()
        .key("results")
        .w_full()
        .padding(4.0)
        .shrink()
        .min_h(0.0);
    results = if let Some(why) = &b.error {
        results.child(
            text(why.clone())
                .font(k.ty.small)
                .color(k.c.warning.icon)
                .wrap()
                .px(8.0),
        )
    } else if count == 0 {
        results.child(
            row().w_full().h(ROW).px(8.0).center_y().child(
                text("No folders in here.")
                    .font(k.ty.ui_compact)
                    .color(c.text_weak),
            ),
        )
    } else {
        results.child(
            scroll_bar(k, &format!("{KEY}-scroll-area"), "viewport", viewport)
                .w_full()
                .shrink()
                .min_h(0.0),
        )
    };
    let footer = row()
        .key("footer")
        .w_full()
        .px(8.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .child(
            Button::new("Use this folder")
                .variant(ButtonVariant::Submit)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| {
                    let form = &mut s.roster.form;
                    form.workdir = Some(form.folders.at.to_string_lossy().into_owned());
                    form.folders.close();
                })
                .build(k)
                .key("use"),
        )
        .child(el().grow(1.0))
        .child(
            Button::new(if b.hidden {
                "Hide hidden"
            } else {
                "Show hidden"
            })
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.roster.form.folders.toggle_hidden())
            .build(k)
            .key("hidden"),
        );
    let rule = || el().w_full().h(1.0).fill(c.border_weak).a11y_hidden();
    col()
        .w(WIDTH)
        .shrink()
        .min_h(0.0)
        .child(header)
        .child(rule())
        .child(results)
        .child(rule())
        .child(footer)
}

fn folder_row(k: &Kit, name: &str) -> El {
    let c = &k.c;
    let name = name.to_owned();
    let go = name.clone();
    row()
        .key(format!("{KEY}-folder-{name}"))
        .role("button")
        .label(format!("Open {name}"))
        .w_full()
        .h(ROW)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .fill(Color::TRANSPARENT)
        .child(icon(k.glyph.caret_right, 12.0).color(c.text_faint))
        .child(
            text(name)
                .font(k.ty.ui_compact)
                .color(c.text_base)
                .grow(1.0)
                .shrink()
                .truncate()
                .a11y_hidden(),
        )
        .on_press(move |s: &mut App| s.roster.form.folders.enter(&go))
}
