//! The working-folder browser: a small list of folders to pick from, for the agent form's field
//! and for the message box's folder pill. Rune has no system folder dialog, so it is drawn here.

use std::borrow::Cow;
use std::path::Path;
use std::rc::Rc;

use rune::{Color, El, GrowthSpec, ScrollSpec, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Placement, Popover,
    Select, SelectAppearance, glyphs, scroll_bar,
};

use crate::app::App;
use crate::folders::Browser;

const ROW: f32 = 30.0;
const MAX_HEIGHT: f32 = 240.0;
/// The most width Rune's popover gives its content.
const WIDTH: f32 = 320.0;

/// Whose folder the browser picks: the agent form's draft, or the open agent's own (saved at once).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Form,
    Agent,
}

impl Target {
    fn key(self) -> &'static str {
        match self {
            Target::Form => "workdir",
            Target::Agent => "composer-workdir",
        }
    }

    fn browser(self, s: &App) -> &Browser {
        match self {
            Target::Form => &s.roster.form.folders,
            Target::Agent => &s.workdir_browser,
        }
    }

    fn browser_mut(self, s: &mut App) -> &mut Browser {
        match self {
            Target::Form => &mut s.roster.form.folders,
            Target::Agent => &mut s.workdir_browser,
        }
    }

    pub fn chosen(self, s: &App) -> Option<&str> {
        match self {
            Target::Form => s.roster.form.workdir.as_deref(),
            Target::Agent => s.current().and_then(|a| a.workdir.as_deref()),
        }
    }

    fn choose(self, s: &mut App, folder: Option<String>) {
        match self {
            Target::Form => s.roster.form.workdir = folder,
            Target::Agent => s.set_workdir(folder),
        }
    }

    /// Opens the browser at the chosen folder, or closes it.
    pub fn toggle(self, s: &mut App) {
        if self.browser(s).open {
            self.browser_mut(s).close();
        } else {
            let start = self.chosen(s).map(str::to_owned);
            self.browser_mut(s).show(start.as_deref().map(Path::new));
        }
    }
}

/// The agent form's field: the chosen folder (or the home folder when none), and a way to clear it.
pub fn build(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let target = Target::Form;
    let chosen = target.chosen(s);
    let shown = chosen.unwrap_or("Home folder").to_owned();
    let trigger = Select::new::<&str>(format!("{}-value", target.key()), &[])
        .current(Some(&shown))
        .label("Working folder")
        .appearance(SelectAppearance::Field)
        .width(WIDTH)
        .trigger(k)
        .on_press(move |s: &mut App| target.toggle(s));
    let field = popover(s, target, trigger, Placement::BottomStart, k, layer);
    // Always wrapped, so a caller's key never lands on the popover's own anchor.
    let mut out = row().gap(4.0).center_y().child(field);
    if chosen.is_some() {
        out = out.child(
            IconButton::new(&glyphs::CLOSE)
                .variant(IconButtonVariant::GhostMuted)
                .label("Use the home folder")
                .on_press(move |s: &mut App| target.choose(s, None))
                .build(k)
                .key(format!("{}-clear", target.key())),
        );
    }
    out
}

/// `trigger` with the browser for `target` opening from it.
pub fn popover(
    s: &App,
    target: Target,
    trigger: El,
    placement: Placement,
    k: &Kit,
    layer: &mut Vec<El>,
) -> El {
    let open = target.browser(s).open;
    let popover = Popover::new(target.key())
        .trigger(trigger)
        .label("Choose a folder")
        .placement(placement)
        .padding(0.0)
        .open(open)
        .on_close(move |s: &mut App| target.browser_mut(s).close());
    if open {
        popover.child(browser(s, target, k)).build(k, layer)
    } else {
        popover.build(k, layer)
    }
}

fn browser(s: &App, target: Target, k: &Kit) -> El {
    let c = &k.c;
    let b = target.browser(s);
    let key = target.key();
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
                .on_press(move |s: &mut App| target.browser_mut(s).up())
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
                move |_: &App, i| Cow::Owned(format!("{key}-folder-{}", names[i]))
            },
            move |_: &App, i| folder_row(&kit, target, &names[i]),
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
            scroll_bar(k, &format!("{key}-scroll-area"), "viewport", viewport)
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
                .on_press(move |s: &mut App| {
                    let b = target.browser_mut(s);
                    let folder = b.at.to_string_lossy().into_owned();
                    b.close();
                    target.choose(s, Some(folder));
                })
                .build(k)
                .key("use"),
        )
        .children(
            (target == Target::Agent && target.chosen(s).is_some()).then(|| {
                Button::new("No folder")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .help("Commands run in the home folder, and @ lists no files")
                    .on_press(move |s: &mut App| {
                        target.browser_mut(s).close();
                        target.choose(s, None);
                    })
                    .build(k)
                    .key("none")
            }),
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
            .on_press(move |s: &mut App| target.browser_mut(s).toggle_hidden())
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

fn folder_row(k: &Kit, target: Target, name: &str) -> El {
    let c = &k.c;
    let name = name.to_owned();
    let go = name.clone();
    row()
        .key(format!("{}-folder-{name}", target.key()))
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
        .on_press(move |s: &mut App| target.browser_mut(s).enter(&go))
}
