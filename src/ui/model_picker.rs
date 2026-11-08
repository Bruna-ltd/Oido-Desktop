//! The model picker: a searchable list of the models of every connected provider, under a heading
//! per provider. Rune-kit's combobox is flat, so this is its own: a popover with a search field and
//! a virtualized list, where a heading is a row like any other (same height) so only the rows on
//! screen are built, however many models a provider such as OpenRouter has.

use std::rc::Rc;

use rune::{Color, El, GrowthSpec, Key, KeyEvent, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Placement, Popover,
    Select, SelectAppearance, TextInput, glyphs, scroll_bar,
};

use crate::models::{Caps, ModelRef};
use crate::picker::{self, Group, PickerState, Row};

const ROW: f32 = 28.0;
const MAX_HEIGHT: f32 = 320.0;
/// The most width Rune's popover gives its content; anything wider spills past the frame.
const WIDTH: f32 = 320.0;

pub struct ModelPicker<S> {
    key: String,
    groups: Vec<Group>,
    current: Option<ModelRef>,
    current_name: Option<String>,
    placeholder: String,
    label: String,
    width: f32,
    clearable: bool,
    trigger: Option<El>,
    on_open: Rc<dyn Fn(&mut S)>,
    on_connect: Rc<dyn Fn(&mut S)>,
}

impl<S: 'static> ModelPicker<S> {
    pub fn new(key: impl Into<String>, groups: Vec<Group>) -> ModelPicker<S> {
        ModelPicker {
            key: key.into(),
            groups,
            current: None,
            current_name: None,
            placeholder: "Choose a model".into(),
            label: "Model".into(),
            width: WIDTH,
            clearable: false,
            trigger: None,
            on_open: Rc::new(|_| {}),
            on_connect: Rc::new(|_| {}),
        }
    }

    /// The chosen model and the name to show for it.
    pub fn current(mut self, model: Option<ModelRef>, name: Option<String>) -> Self {
        self.current = model;
        self.current_name = name;
        self
    }

    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }

    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = text.into();
        self
    }

    /// A clear button once something is chosen, and choosing the current model again clears it.
    pub fn clearable(mut self, on: bool) -> Self {
        self.clearable = on;
        self
    }

    /// Replaces the field with any element, such as a quiet pill. Pressing it opens the list.
    pub fn trigger(mut self, trigger: El) -> Self {
        self.trigger = Some(trigger);
        self
    }

    /// Runs as the list opens: where stale model lists are refreshed.
    pub fn on_open(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_open = Rc::new(f);
        self
    }

    /// Runs from the empty state's button, when no provider has models yet.
    pub fn on_connect(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_connect = Rc::new(f);
        self
    }

    /// `state` is the picker's search, highlight and open flag, in app state. `on_change` gets the
    /// choice (or `None` when cleared).
    pub fn build(
        self,
        k: &Kit,
        layer: &mut Vec<El>,
        state: &PickerState,
        access: impl for<'a> Fn(&'a mut S) -> &'a mut PickerState + Clone + 'static,
        on_change: impl Fn(&mut S, Option<ModelRef>) + Clone + 'static,
    ) -> El {
        let (on_open, on_connect) = (self.on_open.clone(), self.on_connect.clone());
        let key = self.key.clone();
        let current = self.current.clone();
        let mut trigger = self.trigger.unwrap_or_else(|| {
            Select::new::<&str>(format!("{key}-value"), &[])
                .current(self.current_name.as_deref())
                .placeholder(self.placeholder.clone())
                .label(self.label.clone())
                .appearance(SelectAppearance::Field)
                .width(self.width)
                .trigger(k)
        });
        trigger = trigger.on_press_tx({
            let (access, current, key) = (access.clone(), current.clone(), key.clone());
            move |s: &mut S, tx: &mut Tx| {
                let st = access(s);
                if st.open {
                    st.close();
                    return;
                }
                st.show(current.as_ref());
                if let Some(c) = &current {
                    tx.scroll_into_view(row_key(&key, c));
                }
                on_open(s);
            }
        });
        let mut popover = Popover::new(&key)
            .trigger(trigger)
            .label(self.label.clone())
            .placement(Placement::BottomStart)
            .padding(0.0)
            .open(state.open);
        popover = popover.on_close({
            let access = access.clone();
            move |s: &mut S| access(s).close()
        });
        let field = if state.open {
            let groups = Rc::new(self.groups);
            let body = list_body(
                k,
                &key,
                &groups,
                state,
                current.as_ref(),
                self.width,
                &access,
                &on_change,
                &on_connect,
            );
            popover.child(body).build(k, layer)
        } else {
            popover.build(k, layer)
        };
        if !(self.clearable && current.is_some()) {
            return field;
        }
        let clear = IconButton::new(&glyphs::CLOSE)
            .variant(IconButtonVariant::GhostMuted)
            .label("Clear selection")
            .on_press(move |s: &mut S| on_change(s, None))
            .build(k)
            .key(format!("{key}-clear"));
        row().gap(4.0).center_y().child(field).child(clear)
    }
}

fn row_key(key: &str, m: &ModelRef) -> String {
    format!("{key}-option-{}-{}", m.provider, m.id)
}

#[allow(clippy::too_many_arguments)]
fn list_body<S: 'static>(
    k: &Kit,
    key: &str,
    groups: &Rc<Vec<Group>>,
    state: &PickerState,
    current: Option<&ModelRef>,
    width: f32,
    access: &(impl for<'a> Fn(&'a mut S) -> &'a mut PickerState + Clone + 'static),
    on_change: &(impl Fn(&mut S, Option<ModelRef>) + Clone + 'static),
    on_connect: &Rc<dyn Fn(&mut S)>,
) -> El {
    let c = &k.c;
    let rows = picker::rows(groups, &state.search.text);
    let active = picker::active_index(&rows, state.active.as_ref());
    let active_model = active.and_then(|i| match &rows[i] {
        Row::Model { model, .. } => Some(model.clone()),
        Row::Heading { .. } => None,
    });

    let search = TextInput::new(&state.search)
        .plain()
        .leading_icon(k.glyph.search)
        .height(36.0)
        .width(None)
        .placeholder("Search models")
        .label("Search models")
        .active_descendant(active_model.as_ref().map(|m| row_key(key, m)))
        .on_edit({
            let access = access.clone();
            move |s: &mut S| &mut access(s).search
        })
        // Typing starts over from the best match.
        .on_change({
            let access = access.clone();
            move |s: &mut S, _: &mut Tx| access(s).active = None
        })
        .on_key(keys(
            key.to_owned(),
            groups.clone(),
            access.clone(),
            on_change.clone(),
            (MAX_HEIGHT / ROW) as isize - 1,
        ))
        .build(k)
        .key("search");

    // Pinned to the popup's width: left to size itself it grows to its widest row and spills out.
    let inner = width - 8.0;
    let mut list = col()
        .key("rows")
        .w(inner)
        .growth(GrowthSpec::default())
        .virtualize(ROW)
        .reveal_selected(true);
    for row_ in &rows {
        list = list.child(match row_ {
            Row::Heading { provider, label } => heading(k, key, provider, label),
            Row::Model { model, name, caps } => {
                let on = Some(model) == active_model.as_ref();
                let chosen = Some(model) == current;
                model_row(k, key, model, name, *caps, on, chosen, access, on_change)
            }
        });
    }
    let viewport = col()
        .key("viewport")
        .role("listbox")
        .w(inner)
        .h((rows.len() as f32 * ROW).min(MAX_HEIGHT))
        .shrink()
        .min_h(if rows.is_empty() { 0.0 } else { ROW })
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .label("Models")
        .clip_soft(0.0)
        .child(list);
    let mut results = col()
        .key("results")
        .w_full()
        .padding(4.0)
        .shrink()
        .min_h(0.0);
    results = if rows.is_empty() {
        results.child(empty(k, groups.is_empty(), {
            let on_connect = on_connect.clone();
            move |s: &mut S| on_connect(s)
        }))
    } else {
        results.child(
            scroll_bar(k, &format!("{key}-scroll-area"), "viewport", viewport)
                .w_full()
                .shrink()
                .min_h(0.0),
        )
    };
    col()
        .w(width)
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

fn heading(k: &Kit, key: &str, provider: &str, label: &str) -> El {
    row()
        .key(format!("{key}-group-{provider}"))
        .w_full()
        .h(ROW)
        .px(8.0)
        .center_y()
        .child(
            text(label.to_owned())
                .font(k.ty.small_medium)
                .color(k.c.text_muted),
        )
}

#[allow(clippy::too_many_arguments)]
fn model_row<S: 'static>(
    k: &Kit,
    key: &str,
    model: &ModelRef,
    name: &str,
    caps: Caps,
    active: bool,
    chosen: bool,
    access: &(impl for<'a> Fn(&'a mut S) -> &'a mut PickerState + Clone + 'static),
    on_change: &(impl Fn(&mut S, Option<ModelRef>) + Clone + 'static),
) -> El {
    let c = &k.c;
    let mut r = row()
        .key(row_key(key, model))
        .role("option")
        .label(name.to_owned())
        .focus_selected(active)
        .w_full()
        .h(ROW)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(6.0)
        .fill(if active { c.hover } else { Color::TRANSPARENT })
        .child(
            text(name.to_owned())
                .font(k.ty.ui_compact)
                .color(c.text_base)
                .grow(1.0)
                .shrink()
                .truncate()
                .a11y_hidden(),
        );
    for (on, tag) in [(caps.vision, "vision"), (caps.reasoning, "reasoning")] {
        if on == Some(true) {
            r = r.child(tag_pill(k, tag));
        }
    }
    r = r.child(
        icon(k.glyph.check_small, 14.0)
            .color(c.icon_base)
            .opacity(if chosen { 1.0 } else { 0.0 }),
    );
    let (hover_access, hover_model) = (access.clone(), model.clone());
    let (pick_access, pick_model, on_change) = (access.clone(), model.clone(), on_change.clone());
    r.on_hover(move |s: &mut S| hover_access(s).active = Some(hover_model.clone()))
        .on_press_tx(move |s: &mut S, _: &mut Tx| {
            pick_access(s).close();
            on_change(s, Some(pick_model.clone()));
        })
}

fn tag_pill(k: &Kit, label: &str) -> El {
    row()
        .px(5.0)
        .py(1.0)
        .radius(4.0)
        .border(1.0, k.c.border_muted)
        .a11y_hidden()
        .child(
            text(label.to_owned())
                .font(k.ty.text_xsmall)
                .color(k.c.text_faint),
        )
}

fn empty<S: 'static>(k: &Kit, no_models: bool, on_connect: impl Fn(&mut S) + 'static) -> El {
    if !no_models {
        return row().w_full().h(ROW).px(8.0).center_y().child(
            text("Nothing matches that.")
                .font(k.ty.ui_compact)
                .color(k.c.text_weak),
        );
    }
    col()
        .w_full()
        .padding(8.0)
        .gap(8.0)
        .child(
            text("No models yet. A provider's models appear here once you connect it.")
                .font(k.ty.ui_compact)
                .color(k.c.text_weak)
                .wrap(),
        )
        .child(
            Button::new("Connect a provider")
                .variant(ButtonVariant::Neutral)
                .control_size(ControlSize::Small)
                .on_press(on_connect)
                .build(k)
                .key("connect-provider"),
        )
}

/// The search field's keys: Escape closes, Enter chooses, arrows and Ctrl-N/P step (wrapping),
/// Page keys jump a page, and Home and End jump to the ends while the search is empty.
fn keys<S: 'static>(
    key: String,
    groups: Rc<Vec<Group>>,
    access: impl for<'a> Fn(&'a mut S) -> &'a mut PickerState + 'static,
    on_change: impl Fn(&mut S, Option<ModelRef>) + 'static,
    page: isize,
) -> impl Fn(&mut S, &KeyEvent, &mut Tx) -> bool + 'static {
    move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
        let st = access(s);
        if st.search.preedit.is_some() {
            return false;
        }
        if ev.key == Key::Escape {
            if !tx.key_repeat() {
                st.close();
            }
            return true;
        }
        if ev.mods.alt || ev.mods.logo {
            return false;
        }
        let rows = picker::rows(&groups, &st.search.text);
        if ev.key == Key::Enter {
            if tx.key_repeat() {
                return true;
            }
            let chosen =
                picker::active_index(&rows, st.active.as_ref()).and_then(|i| match &rows[i] {
                    Row::Model { model, .. } => Some(model.clone()),
                    Row::Heading { .. } => None,
                });
            st.close();
            if let Some(m) = chosen {
                on_change(s, Some(m));
            }
            return true;
        }
        let control = ev.mods.ctrl && !ev.mods.shift;
        let (by, wrap) = match &ev.key {
            Key::PageDown | Key::PageUp | Key::Home | Key::End
                if ev.mods.cmd || ev.mods.ctrl || ev.mods.shift =>
            {
                return false;
            }
            Key::PageDown => (page, false),
            Key::PageUp => (-page, false),
            Key::End if st.search.text.is_empty() => (isize::MAX, false),
            Key::Home if st.search.text.is_empty() => (isize::MIN, false),
            Key::Down if !ev.mods.cmd && !ev.mods.ctrl => (1, true),
            Key::Up if !ev.mods.cmd && !ev.mods.ctrl => (-1, true),
            Key::Text(c) if control && c == "n" => (1, true),
            Key::Text(c) if control && c == "p" => (-1, true),
            Key::Command('n') if control => (1, true),
            Key::Command('p') if control => (-1, true),
            _ => return false,
        };
        if let Some(next) = picker::shifted(&rows, st.active.as_ref(), by, wrap) {
            tx.scroll_into_view(row_key(&key, &next));
            st.active = Some(next);
        }
        true
    }
}
