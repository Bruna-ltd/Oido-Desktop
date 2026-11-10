//! A searchable multi-select: a field that opens a list with a check on each chosen item, and the
//! chosen ones as removable chips beneath it. It is the model picker's sibling, but you can choose
//! several, so choosing leaves it open. Skills and tools both use it.

use std::borrow::Cow;
use std::rc::Rc;

use rune::{Color, El, GrowthSpec, Key, KeyEvent, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Placement, Popover,
    Select, SelectAppearance, TextInput, glyphs, scroll_bar,
};

use crate::app::App;
use crate::picker::{self, Item, MultiPicker};

const ROW: f32 = 32.0;
const MAX_HEIGHT: f32 = 320.0;
/// The most width Rune's popover gives its content.
const WIDTH: f32 = 320.0;

/// What one multi-select is for and where its state lives.
pub struct Spec {
    /// Prefix of every element key: "skills", "tools".
    pub key: &'static str,
    /// "Skills".
    pub title: &'static str,
    /// The empty field: "Add skills".
    pub placeholder: &'static str,
    /// The summary once something is chosen: ("skill", "skills").
    pub noun: (&'static str, &'static str),
    /// The list's text when there is nothing to choose from.
    pub none: &'static str,
    pub items: Vec<Item>,
    pub chosen: Vec<String>,
    pub get: fn(&App) -> &MultiPicker,
    pub edit: fn(&mut App) -> &mut MultiPicker,
    pub toggle: fn(&mut App, &str),
}

fn row_key(key: &str, name: &str) -> String {
    format!("{key}-option-{name}")
}

/// One row of the open list: a group's heading, or the item at this place among those found.
enum Line {
    /// The group's name and the names of its items that are shown.
    Heading(String, Vec<String>),
    Item(usize),
}

/// The rows for `found`: a heading over each group's items, when the items have groups.
fn lines(found: &[&Item]) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::with_capacity(found.len());
    let mut group: Option<&str> = None;
    let mut heading: Option<usize> = None;
    for (i, item) in found.iter().enumerate() {
        if group != Some(item.group.as_str()) {
            group = Some(item.group.as_str());
            heading = (!item.group.is_empty()).then(|| {
                out.push(Line::Heading(item.group.clone(), Vec::new()));
                out.len() - 1
            });
        }
        if let Some(Line::Heading(_, members)) = heading.and_then(|h| out.get_mut(h)) {
            members.push(item.name.clone());
        }
        out.push(Line::Item(i));
    }
    out
}

pub fn build(s: &App, k: &Kit, layer: &mut Vec<El>, spec: Spec) -> El {
    let (key, edit, toggle) = (spec.key, spec.edit, spec.toggle);
    let summary = match spec.chosen.len() {
        0 => None,
        1 => Some(format!("1 {}", spec.noun.0)),
        n => Some(format!("{n} {}", spec.noun.1)),
    };
    let trigger = Select::new::<&str>(format!("{key}-value"), &[])
        .current(summary.as_deref())
        .placeholder(spec.placeholder)
        .label(spec.title)
        .appearance(SelectAppearance::Field)
        .width(WIDTH)
        .trigger(k)
        .on_press(move |s: &mut App| {
            let picker = edit(s);
            if picker.open {
                picker.close();
            } else {
                picker.show();
            }
        });
    let state = (spec.get)(s);
    let popover = Popover::new(key)
        .trigger(trigger)
        .label(spec.title)
        .placement(Placement::BottomStart)
        .padding(0.0)
        .open(state.open)
        .on_close(move |s: &mut App| edit(s).close());
    let field = if state.open {
        popover.child(list_body(s, k, &spec)).build(k, layer)
    } else {
        popover.build(k, layer)
    };
    // Always wrapped, so a caller's key never lands on the popover's own anchor.
    let mut out = col().w_full().gap(10.0).child(row().child(field));
    if !spec.chosen.is_empty() {
        out = out.child(chips(key, &spec.chosen, &spec.items, toggle, k));
    }
    out
}

fn list_body(s: &App, k: &Kit, spec: &Spec) -> El {
    let c = &k.c;
    let state = (spec.get)(s);
    let key = spec.key;
    let found = picker::matching(&spec.items, &state.search.text);
    let active = state.active.min(found.len().saturating_sub(1));
    let active_name = found.get(active).map(|f| f.name.clone());
    let names: Vec<String> = found.iter().map(|f| f.name.clone()).collect();

    let (edit, toggle, get) = (spec.edit, spec.toggle, spec.get);
    let search = TextInput::new(&state.search)
        .plain()
        .leading_icon(k.glyph.search)
        .height(36.0)
        .width(None)
        .placeholder(format!("Search {}", spec.noun.1))
        .label(format!("Search {}", spec.noun.1))
        .active_descendant(active_name.as_deref().map(|n| row_key(key, n)))
        .on_edit(move |s: &mut App| &mut edit(s).search)
        .on_change(move |s: &mut App, _: &mut Tx| edit(s).active = 0)
        .on_key(keys(
            key,
            names,
            (MAX_HEIGHT / ROW) as usize - 1,
            edit,
            toggle,
        ))
        .build(k)
        .key("search");

    let inner = WIDTH - 8.0;
    // Only the rows on screen are built.
    let shown: Rc<Vec<Item>> = Rc::new(found.iter().map(|f| (*f).clone()).collect());
    let rows: Rc<Vec<Line>> = Rc::new(lines(&found));
    let count = rows.len();
    let kit = k.clone();
    let chosen = spec.chosen.clone();
    let list = col()
        .key("rows")
        .w(inner)
        .growth(GrowthSpec::default())
        .virtual_rows(
            count,
            ROW,
            {
                let (shown, rows) = (shown.clone(), rows.clone());
                move |_: &App, i| {
                    Cow::Owned(match &rows[i] {
                        Line::Heading(g, _) => format!("{key}-group-{g}"),
                        Line::Item(at) => row_key(key, &shown[*at].name),
                    })
                }
            },
            move |_: &App, i| match &rows[i] {
                Line::Heading(g, members) => heading(&kit, key, g, members, &chosen, toggle),
                Line::Item(at) => {
                    let item = &shown[*at];
                    let on = chosen.contains(&item.name);
                    option(&kit, key, item, *at, *at == active, on, edit, toggle)
                }
            },
        )
        .reveal_selected(true);
    let _ = get;
    let viewport = col()
        .key("viewport")
        .role("listbox")
        .w(inner)
        .h((count as f32 * ROW).min(MAX_HEIGHT))
        .shrink()
        .min_h(if count == 0 { 0.0 } else { ROW })
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .label(spec.title)
        .clip_soft(0.0)
        .child(list);
    let mut results = col()
        .key("results")
        .w_full()
        .padding(4.0)
        .shrink()
        .min_h(0.0);
    results = if count == 0 {
        results.child(
            row().w_full().h(ROW).px(8.0).center_y().child(
                text(if spec.items.is_empty() {
                    spec.none
                } else {
                    "Nothing matches that."
                })
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
    col()
        .w(WIDTH)
        .shrink()
        .min_h(0.0)
        .child(row().key("header").w_full().px(4.0).child(search))
        .child(
            el().key("divider")
                .w_full()
                .h(1.0)
                .fill(c.border_weak)
                .a11y_hidden(),
        )
        .child(results)
}

/// A group's name over its items, with a button that chooses every item shown under it, or
/// removes them all once they are all chosen. The heading itself cannot be chosen or reached by
/// the arrow keys.
fn heading(
    k: &Kit,
    key: &str,
    group: &str,
    members: &[String],
    chosen: &[String],
    toggle: fn(&mut App, &str),
) -> El {
    let all_on = members.iter().all(|m| chosen.contains(m));
    // Toggling exactly these chooses the rest, or removes every one.
    let flip: Vec<String> = members
        .iter()
        .filter(|m| chosen.contains(m) == all_on)
        .cloned()
        .collect();
    row()
        .w_full()
        .h(ROW)
        .pl(8.0)
        .gap(8.0)
        .center_y()
        .child(
            text(group.to_owned())
                .font(k.ty.small)
                .color(k.c.text_faint)
                .grow(1.0)
                .shrink()
                .truncate(),
        )
        .child(
            Button::new(if all_on { "Remove all" } else { "Select all" })
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .help(if all_on {
                    format!("Remove every {group} item shown")
                } else {
                    format!("Choose every {group} item shown")
                })
                .on_press(move |s: &mut App| {
                    for name in &flip {
                        toggle(s, name);
                    }
                })
                .build(k)
                .key(format!("{key}-all-{group}")),
        )
}

#[allow(clippy::too_many_arguments)]
fn option(
    k: &Kit,
    key: &'static str,
    item: &Item,
    at: usize,
    active: bool,
    on: bool,
    edit: fn(&mut App) -> &mut MultiPicker,
    toggle: fn(&mut App, &str),
) -> El {
    let c = &k.c;
    let name = item.name.clone();
    row()
        .key(row_key(key, &item.name))
        .role("option")
        .label(item.label.clone())
        .checked(on)
        .focus_selected(active)
        .w_full()
        .h(ROW)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .fill(if active { c.hover } else { Color::TRANSPARENT })
        .child(
            icon(k.glyph.check_small, 14.0)
                .color(c.icon_base)
                .opacity(if on { 1.0 } else { 0.0 }),
        )
        .child(
            text(item.label.clone())
                .font(k.ty.ui_compact)
                .color(c.text_base)
                .shrink()
                .truncate()
                .a11y_hidden(),
        )
        .child(
            // The text engine has no word-break data for Chinese or Japanese and logs an error
            // each time it lays them out, so such a description is left out of the row.
            text(if crate::skills::needs_complex_text(&item.description) {
                String::new()
            } else {
                item.description.clone()
            })
            .font(k.ty.small)
            .color(c.text_faint)
            .grow(1.0)
            .shrink()
            .truncate()
            .a11y_hidden(),
        )
        .on_hover(move |s: &mut App| edit(s).active = at)
        .on_press(move |s: &mut App| toggle(s, &name))
}

/// The chosen items, each removable. One that is gone from the list still shows, so it can be
/// removed.
fn chips(key: &str, chosen: &[String], items: &[Item], toggle: fn(&mut App, &str), k: &Kit) -> El {
    let c = &k.c;
    let mut rows = col().w_full().gap(6.0);
    let mut line = row().w_full().gap(6.0);
    for (i, name) in chosen.iter().enumerate() {
        let remove = name.clone();
        let label = items
            .iter()
            .find(|it| it.name == *name)
            .map_or(name.as_str(), |it| it.label.as_str());
        line = line.child(
            row()
                .key(format!("{key}-chip-{name}"))
                .h(28.0)
                .pl(12.0)
                .pr(4.0)
                .gap(2.0)
                .center_y()
                .radius(14.0)
                .border(1.0, c.border_base)
                .child(text(label.to_owned()).font(k.ty.small).color(c.text_base))
                .child(
                    IconButton::new(&glyphs::CLOSE)
                        .variant(IconButtonVariant::GhostMuted)
                        .label(format!("Remove {label}"))
                        .on_press(move |s: &mut App| toggle(s, &remove))
                        .build(k)
                        .key(format!("{key}-remove-{name}")),
                ),
        );
        if (i + 1) % 3 == 0 || i + 1 == chosen.len() {
            rows = rows.child(std::mem::replace(&mut line, row().w_full().gap(6.0)));
        }
    }
    rows
}

/// The search field's keys: Escape closes, Enter toggles the highlighted item, arrows and
/// Ctrl-N/P step (wrapping), Page keys jump a page.
fn keys(
    key: &'static str,
    names: Vec<String>,
    page: usize,
    edit: fn(&mut App) -> &mut MultiPicker,
    toggle: fn(&mut App, &str),
) -> impl Fn(&mut App, &KeyEvent, &mut Tx) -> bool + 'static {
    move |s: &mut App, ev: &KeyEvent, tx: &mut Tx| {
        let picker = edit(s);
        if picker.search.preedit.is_some() {
            return false;
        }
        if ev.key == Key::Escape {
            if !tx.key_repeat() {
                picker.close();
            }
            return true;
        }
        if ev.mods.alt || ev.mods.logo {
            return false;
        }
        let last = names.len().saturating_sub(1);
        let at = picker.active.min(last);
        if ev.key == Key::Enter {
            if !tx.key_repeat()
                && let Some(name) = names.get(at).cloned()
            {
                toggle(s, &name);
            }
            return true;
        }
        let control = ev.mods.ctrl && !ev.mods.shift;
        let next = match &ev.key {
            Key::PageDown if !ev.mods.cmd && !ev.mods.ctrl => (at + page).min(last),
            Key::PageUp if !ev.mods.cmd && !ev.mods.ctrl => at.saturating_sub(page),
            Key::Down if !ev.mods.cmd && !ev.mods.ctrl => wrap(at, 1, names.len()),
            Key::Up if !ev.mods.cmd && !ev.mods.ctrl => wrap(at, -1, names.len()),
            Key::Text(c) if control && c == "n" => wrap(at, 1, names.len()),
            Key::Text(c) if control && c == "p" => wrap(at, -1, names.len()),
            Key::Command('n') if control => wrap(at, 1, names.len()),
            Key::Command('p') if control => wrap(at, -1, names.len()),
            _ => return false,
        };
        picker.active = next;
        if let Some(name) = names.get(next) {
            tx.scroll_into_view(row_key(key, name));
        }
        true
    }
}

fn wrap(at: usize, by: isize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        (at as isize + by).rem_euclid(len as isize) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_heading_lists_the_items_shown_under_it() {
        let item = |n: &str, g: &str| Item {
            name: n.into(),
            label: n.into(),
            description: String::new(),
            group: g.into(),
        };
        let all = [
            item("ask", "Defaults"),
            item("shell", "Defaults"),
            item("send", "Resend"),
        ];
        let found: Vec<&Item> = all.iter().collect();
        let got: Vec<String> = lines(&found)
            .iter()
            .map(|l| match l {
                Line::Heading(g, m) => format!("{g}: {}", m.join(",")),
                Line::Item(i) => found[*i].name.clone(),
            })
            .collect();
        assert_eq!(
            got,
            [
                "Defaults: ask,shell",
                "ask",
                "shell",
                "Resend: send",
                "send"
            ]
        );
        let plain = [Item::plain("a".into(), String::new())];
        let found: Vec<&Item> = plain.iter().collect();
        assert!(
            matches!(lines(&found)[..], [Line::Item(0)]),
            "no headings without groups"
        );
    }
}
