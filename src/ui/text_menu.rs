//! Right-click on text: Copy over a chat message, Cut, Copy, Paste and Select all over the
//! composer. rune opens native menus only on macOS and Windows, so on Linux these are drawn here,
//! in the same kit menu the sidebar rows use.

use rune::{El, PointerPress, el};
use rune_kit::{Kit, Menu, MenuItem, context_menu};

use crate::app::App;

/// The menu open at a right-click, and what it acts on.
pub struct TextMenu {
    at: (f32, f32),
    on: On,
}

enum On {
    /// A message: what is selected in it, else the whole of message `i`.
    Message { selected: Option<String>, i: usize },
    /// The composer's text box.
    Composer,
}

/// Makes a right-click on `e` open the Copy menu for message `i`.
pub fn message(e: El, i: usize) -> El {
    e.on_secondary_press(move |s: &mut App, p: PointerPress, tx| {
        let selected = tx
            .text_selection()
            .map(|t| t.text().to_owned())
            .filter(|t| !t.is_empty());
        s.text_menu = Some(TextMenu {
            at: (p.x, p.y),
            on: On::Message { selected, i },
        });
    })
}

/// Makes a right-click on the composer's box open its edit menu.
pub fn composer(e: El) -> El {
    e.on_secondary_press(|s: &mut App, p: PointerPress, _| {
        s.text_menu = Some(TextMenu {
            at: (p.x, p.y),
            on: On::Composer,
        });
    })
}

pub fn build(s: &App, k: &Kit, layer: &mut Vec<El>) {
    let Some(open) = &s.text_menu else { return };
    let at = PointerPress {
        x: open.at.0,
        y: open.at.1,
    };
    let menu = context_menu("text-menu", el().size(0.0, 0.0), Some(at))
        .on_close(|s: &mut App| s.text_menu = None);
    match &open.on {
        On::Message { selected, i } => message_items(menu, s, selected.clone(), *i),
        On::Composer => composer_items(menu, s),
    }
    .build(k, layer);
}

fn message_items(menu: Menu, s: &App, selected: Option<String>, i: usize) -> Menu {
    let (label, text) = match selected {
        Some(t) => ("Copy", Some(t)),
        None => ("Copy message", s.message_text(i)),
    };
    menu.item(
        MenuItem::new(label)
            .disabled(text.is_none())
            .on_select_tx(move |s: &mut App, tx| {
                s.text_menu = None;
                if let Some(t) = &text {
                    tx.copy(t.clone());
                }
            }),
    )
}

fn composer_items(menu: Menu, s: &App) -> Menu {
    let edit = &s.composer.edit;
    let (has_selection, has_text) = (!edit.selected().is_empty(), !edit.text.is_empty());
    menu.item(
        MenuItem::new("Cut")
            .disabled(!has_selection)
            .on_select_tx(|s: &mut App, tx| {
                s.text_menu = None;
                tx.copy(s.composer.edit.selected().to_owned());
                s.composer.edit.insert("");
            }),
    )
    .item(
        MenuItem::new("Copy")
            .disabled(!has_selection)
            .on_select_tx(|s: &mut App, tx| {
                s.text_menu = None;
                tx.copy(s.composer.edit.selected().to_owned());
            }),
    )
    .item(MenuItem::new("Paste").on_select_tx(|s: &mut App, tx| {
        s.text_menu = None;
        tx.request_paste();
    }))
    .separator()
    .item(
        MenuItem::new("Select all")
            .disabled(!has_text)
            .on_select(|s: &mut App| {
                s.text_menu = None;
                let end = s.composer.edit.text.len();
                s.composer.edit.set_selection(0, end);
            }),
    )
}
