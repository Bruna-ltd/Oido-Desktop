//! The app's settings, in sections. Appearance: Oido's own look, the desktop's Omarchy theme, or
//! any one Omarchy theme.

use rune::{Color, El, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{Kit, Placement, Popover, Select, SelectAppearance, scroll_bar};

use crate::app::App;
use crate::theme::{Appearance, Colors, Look};

const COLUMN: f32 = 760.0;

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    col()
        .key("settings")
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
                .gap(24.0)
                .child(section("Appearance", vec![theme_row(s, k, layer)], k)),
        )
}

/// A titled group of setting rows on one card, rules between them.
fn section(title: &str, rows: Vec<El>, k: &Kit) -> El {
    let c = &k.c;
    let mut card = col()
        .w_full()
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_muted);
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            card = card.child(el().w_full().h(1.0).fill(c.border_muted));
        }
        card = card.child(r);
    }
    col()
        .key(format!("section-{}", title.to_lowercase()))
        .w_full()
        .gap(8.0)
        .role("group")
        .label(title.to_owned())
        .child(
            text(title.to_owned())
                .font(k.ty.body_medium)
                .color(c.text_base)
                .px(4.0),
        )
        .child(card)
}

/// One setting: its name and what it does on the left, its control on the right.
fn setting(key: &str, name: &str, help: &str, control: El, k: &Kit) -> El {
    row()
        .key(format!("setting-{key}"))
        .w_full()
        .padding(16.0)
        .gap(16.0)
        .center_y()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(name.to_owned())
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base),
                )
                .child(
                    text(help.to_owned())
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .wrap(),
                ),
        )
        .child(control)
}

/// The theme list's rows are this tall, and it grows to this many before it scrolls.
const ROW: f32 = 32.0;
const MAX_ROWS: f32 = 9.5;
const LIST_WIDTH: f32 = 280.0;

fn theme_row(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let offered = choices(&s.look);
    let current = offered.iter().find(|ch| ch.appearance == s.look.appearance);
    let trigger = Select::new::<&str>("theme-value", &[])
        .current(current.map(|ch| ch.label.as_str()))
        // A chosen theme that is gone shows what is worn instead: Oido's own.
        .placeholder("Oido")
        .label("Theme")
        .appearance(SelectAppearance::Field)
        .width(LIST_WIDTH)
        .open(s.theme_open)
        .trigger(k)
        .on_press_tx(|s: &mut App, tx: &mut Tx| {
            s.theme_open ^= true;
            if s.theme_open {
                tx.scroll_into_view(row_key(&s.look.appearance));
            }
        });
    let popover = Popover::new("theme")
        .trigger(trigger)
        .label("Theme")
        .placement(Placement::BottomStart)
        .padding(0.0)
        .open(s.theme_open)
        .on_close(|s: &mut App| s.theme_open = false);
    let field = if s.theme_open {
        popover
            .child(theme_list(&offered, &s.look.appearance, k))
            .build(k, layer)
    } else {
        popover.build(k, layer)
    };
    let help = if s.look.themes.is_empty() {
        "Oido's own dark look. Omarchy's themes are offered here when Omarchy is installed."
    } else {
        "Oido's own look, the desktop's Omarchy theme as it changes, or any Omarchy theme."
    };
    setting(
        "theme",
        "Theme",
        help,
        row()
            .gap(10.0)
            .center_y()
            .child(swatch(s.look.colors(), 28.0, k).key("theme-swatch"))
            // Wrapped: the popover places its list against the field's own key.
            .child(row().child(field)),
        k,
    )
}

/// Every theme on offer, each with its colours, scrolling past [`MAX_ROWS`].
fn theme_list(offered: &[Choice], worn: &Appearance, k: &Kit) -> El {
    let rows = col().key("rows").w_full().gap(0.0).children(
        offered
            .iter()
            .map(|ch| theme_option(ch, *worn == ch.appearance, k)),
    );
    let viewport = col()
        .key("viewport")
        .role("listbox")
        .label("Themes")
        .w_full()
        .h((offered.len() as f32).min(MAX_ROWS) * ROW)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .clip_soft(0.0)
        .child(rows);
    col()
        .w(LIST_WIDTH)
        .padding(4.0)
        .child(scroll_bar(k, "theme-scroll-area", "viewport", viewport).w_full())
}

fn theme_option(ch: &Choice, chosen: bool, k: &Kit) -> El {
    let c = &k.c;
    let appearance = ch.appearance.clone();
    let mut words = row()
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .gap(6.0)
        .center_y()
        .child(
            text(ch.label.clone())
                .font(k.ty.ui_compact)
                .color(c.text_base)
                .truncate(),
        );
    if let Some(detail) = &ch.detail {
        words = words.child(
            text(detail.clone())
                .font(k.ty.small)
                .color(c.text_faint)
                .truncate(),
        );
    }
    let r = row()
        .key(row_key(&ch.appearance))
        .role("option")
        .label(ch.label.clone())
        .focusable(true)
        .focus_selected(chosen)
        .w_full()
        .h(ROW)
        .px(6.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .child(swatch(ch.colors, 20.0, k))
        .child(words)
        .child(
            icon(k.glyph.check_small, 14.0)
                .color(c.icon_base)
                .opacity(if chosen { 1.0 } else { 0.0 }),
        )
        .on_press(move |s: &mut App| {
            s.theme_open = false;
            s.set_appearance(appearance.clone());
        });
    k.interactive(r)
}

fn row_key(appearance: &Appearance) -> String {
    match appearance {
        Appearance::Oido => "theme-oido".to_owned(),
        Appearance::System => "theme-system".to_owned(),
        Appearance::Omarchy(slug) => format!("theme-omarchy-{slug}"),
    }
}

/// One theme the list offers: the name it shows, a note after it, and the colours it wears.
struct Choice {
    label: String,
    detail: Option<String>,
    colors: Colors,
    appearance: Appearance,
}

/// What the theme list offers: Oido, the desktop's theme when Omarchy has one, then every
/// installed Omarchy theme.
fn choices(look: &Look) -> Vec<Choice> {
    let mut out = vec![Choice {
        label: "Oido".to_owned(),
        detail: None,
        colors: Colors::oido(),
        appearance: Appearance::Oido,
    }];
    if let Some(system) = &look.system {
        out.push(Choice {
            label: "Match Omarchy".to_owned(),
            detail: Some(system.title()),
            colors: system.colors,
            appearance: Appearance::System,
        });
    }
    out.extend(look.themes.iter().map(|t| Choice {
        label: t.title(),
        detail: None,
        colors: t.colors,
        appearance: Appearance::Omarchy(t.slug.clone()),
    }));
    out
}

/// A look in small: its accent and role colours as dots on its background, `height` tall.
fn swatch(colors: Colors, height: f32, k: &Kit) -> El {
    let d = (height * 0.3).round();
    let dot = |fill: Color| el().size(d, d).radius(d / 2.0).fill(fill);
    row()
        .h(height)
        .px(height * 0.28)
        .gap(height * 0.14)
        .center_y()
        .radius(k.radius.sm)
        .fill(colors.bg)
        .border(1.0, k.c.border_muted)
        .a11y_hidden()
        .children([colors.accent, colors.green, colors.amber, colors.violet].map(dot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::omarchy;

    #[test]
    fn the_list_offers_oido_the_desktops_theme_and_each_installed_one() {
        let theme = |slug: &str| omarchy::Theme {
            slug: slug.to_owned(),
            colors: Colors::oido(),
        };
        let mut look = Look {
            appearance: Appearance::Oido,
            themes: vec![theme("tokyo-night")],
            system: Some(theme("kanagawa")),
        };
        let names: Vec<String> = choices(&look).into_iter().map(|ch| ch.label).collect();
        assert_eq!(names, ["Oido", "Match Omarchy", "Tokyo Night"]);
        look.system = None;
        assert_eq!(
            choices(&look).len(),
            2,
            "no desktop theme, nothing to match"
        );
    }
}
