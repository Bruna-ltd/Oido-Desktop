//! The panel that lists the kinds of step to add, as n8n's "What happens next?" does: a search,
//! then each kind with its picture and what it is for. It opens in the place of the form beside
//! the graph, from any button that adds a step there.

use rune::{El, ScrollSpec, col, el, icon, row, text, text_input};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit, glyphs};

use super::workflow_graph::picture;
use crate::app::App;
use crate::workflow::editor::{KINDS, Kind};

/// The kinds that match what was typed, in the order they are listed: the words of the search
/// are each looked for in a kind's name, its group and what it is for.
pub fn matching(search: &str) -> Vec<&'static Kind> {
    let words: Vec<String> = search.split_whitespace().map(str::to_lowercase).collect();
    KINDS
        .iter()
        .filter(|kind| {
            let all = format!("{} {} {}", kind.label, kind.group, kind.hint).to_lowercase();
            words.iter().all(|w| all.contains(w))
        })
        .collect()
}

pub fn panel(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let style = rune::TextStyle {
        line_height: 20.0,
        ..k.ty.body
    };
    let found = matching(&s.wf_pick.text);
    let mut list = col().w_full().gap(2.0);
    let mut group = "";
    for kind in &found {
        if kind.group != group {
            group = kind.group;
            list = list.child(
                text(group.to_owned())
                    .font(k.ty.small)
                    .color(c.text_muted)
                    .pl(4.0)
                    .pt(10.0)
                    .pb(4.0),
            );
        }
        list = list.child(item(kind, k));
    }
    if found.is_empty() {
        list = list.child(
            text("No step matches. Try fewer words.")
                .font(k.ty.small)
                .color(c.text_muted)
                .pt(10.0),
        );
    }
    col()
        .key("wfg-picker")
        .w(super::workflow_graph::INSPECTOR_W)
        .h_full()
        .min_h(0.0)
        .gap(12.0)
        .child(
            row()
                .w_full()
                .center_y()
                .child(
                    text("What happens next?")
                        .font(k.ty.body_medium)
                        .color(c.text_base),
                )
                .child(el().grow(1.0))
                .child(
                    IconButton::new(&glyphs::CLOSE)
                        .variant(IconButtonVariant::GhostMuted)
                        .control_size(ControlSize::Small)
                        .label("Close")
                        .on_press(|s: &mut App| s.wf_menu = None)
                        .build(k)
                        .key("wfg-picker-close"),
                ),
        )
        .child(
            row()
                .w_full()
                .center_y()
                .gap(8.0)
                .px(12.0)
                .py(8.0)
                .radius(k.radius.md)
                .border(
                    1.0,
                    if s.wf_pick.focused {
                        c.border_strong
                    } else {
                        c.border_base
                    },
                )
                .child(icon(&glyphs::SEARCH, 14.0).color(c.text_muted))
                .child(
                    text_input(&s.wf_pick, style)
                        .key("wfg-picker-search")
                        .grow(1.0)
                        .color(c.text_base)
                        .placeholder("Search steps".to_owned(), c.text_faint)
                        .caret(c.text_base, k.selection())
                        .label("Search steps".to_owned())
                        .on_edit(|s: &mut App| &mut s.wf_pick),
                ),
        )
        .child(
            col()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
}

/// One kind of step: its picture, its name and what it is for. Pressing it adds it.
fn item(kind: &'static Kind, k: &Kit) -> El {
    let c = &k.c;
    let label = kind.label;
    let e = row()
        .key(format!("wfg-pick-{}", kind.kind))
        .w_full()
        .gap(12.0)
        .px(8.0)
        .py(8.0)
        .center_y()
        .radius(k.radius.md)
        .role("button")
        .label(format!("{}. {}", kind.label, kind.hint))
        .on_press(move |s: &mut App| s.pick_kind(label))
        .child(
            row()
                .size(40.0, 40.0)
                .radius(10.0)
                .center()
                .fill(c.layer_02)
                .border(1.0, c.border_muted)
                .child(icon(picture(kind.kind), 22.0).color(c.text_base)),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(kind.label.to_owned())
                        .font(k.ty.ui_medium)
                        .color(c.text_base),
                )
                .child(
                    text(kind.hint.to_owned())
                        .font(k.ty.small)
                        .color(c.text_muted)
                        .wrap(),
                ),
        );
    k.interactive(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_keeps_the_kinds_that_have_every_word_in_name_group_or_hint() {
        assert_eq!(matching("").len(), KINDS.len());
        assert_eq!(matching("   ").len(), KINDS.len());
        let found: Vec<_> = matching("each FILE").iter().map(|k| k.kind).collect();
        assert_eq!(found, ["for_each"]);
        assert!(matching("zzz").is_empty());
        assert!(matching("more").iter().all(|k| k.group == "More"));
    }
}
