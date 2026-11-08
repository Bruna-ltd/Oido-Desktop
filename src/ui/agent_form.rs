//! Making and changing agents: one form for a new agent, an edit, and a new team member, and the
//! delete confirmation. The name is the page; everything else is optional and can change later.

use rune::{Color, El, ScrollSpec, TextStyle, col, row, text};
use rune_kit::shell::Toolbar;
use rune_kit::{Button, ButtonVariant, Dialog, Kit, TextArea, TextInput};

use super::model_picker::ModelPicker;
use crate::agents::{FormKind, HUES, SKILLS, TEMPLATES, TOOLS, hue_of, initial_of};
use crate::app::App;
use crate::theme;

const PAGE: f32 = 720.0;

/// Cancel on the left, the one primary action on the right, disabled until the name is in.
pub fn toolbar(s: &App, k: &Kit, tb: Toolbar) -> Toolbar {
    let (_, submit) = s.form_labels();
    tb.leading(
        Button::new("Cancel")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut App| s.cancel_form())
            .build(k)
            .key("cancel"),
    )
    .trailing(
        Button::new(submit)
            .variant(ButtonVariant::Contrast)
            .disabled(!s.roster.can_submit())
            .on_press(|s: &mut App| s.submit_form())
            .build(k)
            .key("submit"),
    )
}

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let r = &s.roster;
    let Some(kind) = r.editing.clone() else {
        return col().key("agent-form");
    };
    let member = match &kind {
        FormKind::NewMember(_) => true,
        FormKind::Edit(t) => t.is_member(),
        FormKind::NewAgent => false,
    };
    let parent_name = match &kind {
        FormKind::NewMember(p) => r.list.iter().find(|a| &a.id == p).map(|a| a.name.clone()),
        FormKind::Edit(t) => t
            .parent
            .as_ref()
            .and_then(|p| r.list.iter().find(|a| &a.id == p))
            .map(|a| a.name.clone()),
        FormKind::NewAgent => None,
    };
    let mut page = col()
        .w_full()
        .max_w(PAGE)
        .px(24.0)
        .py(32.0)
        .gap(24.0)
        .child(hero(s, &kind, member, k));
    if matches!(kind, FormKind::NewAgent) {
        page = page.child(templates(s, k));
    }
    page = page
        .child(card(
            if member { "What is its job?" } else { "What should it do?" },
            "Its job, its tone, and anything it should never do. Write it the way you would brief a new colleague.",
            TextArea::new(&r.form.prompt)
                .placeholder(if member {
                    "e.g. You check every answer for spelling and tone before it goes out."
                } else {
                    "e.g. You answer billing questions for our customers. Be brief and polite."
                })
                .label("Instructions")
                .rows(6)
                .width(None)
                .on_edit(|s: &mut App| &mut s.roster.form.prompt)
                .build(k)
                .key("prompt"),
            k,
        ))
        .child(card(
            if member { "Which AI should it use?" } else { "Which AI should answer?" },
            &match &parent_name {
                Some(p) if member => format!("Leave it on the default and it uses the same one as {p}."),
                _ => "The model behind this agent. If you are not sure, leave it on the default.".to_owned(),
            },
            model_select(s, member, k, layer),
            k,
        ))
        .child(more_options(s, k));
    col()
        .key("agent-form")
        .w_full()
        .h_full()
        .min_h(0.0)
        .items_center()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

fn hero(s: &App, kind: &FormKind, member: bool, k: &Kit) -> El {
    let c = &k.c;
    let f = &s.roster.form;
    let hue = f.hue.unwrap_or_else(|| hue_of(&f.name.text));
    let caption = match kind {
        FormKind::NewAgent => "New agent",
        FormKind::NewMember(_) => "New team member",
        FormKind::Edit(t) if t.is_member() => "Edit team member",
        FormKind::Edit(_) => "Edit agent",
    };
    let face = row()
        .key("avatar")
        .size(56.0, 56.0)
        .radius(16.0)
        .center()
        .fill(theme::avatar_color(hue))
        .expanded(f.picker)
        .role("button")
        .label("Change picture")
        .on_press(|s: &mut App| s.roster.form.picker ^= true)
        .child(
            text(initial_of(&f.name.text))
                .font(k.ty.body_medium)
                .color(c.white),
        );
    let name = TextInput::new(&f.name)
        .placeholder(if member {
            "Name this team member"
        } else {
            "Name your agent"
        })
        .label(if member {
            "Team member name"
        } else {
            "Agent name"
        })
        .plain()
        .text_style(TextStyle {
            size: 28.0,
            line_height: 34.0,
            ..k.ty.body_medium
        })
        .width(None)
        .on_edit(|s: &mut App| &mut s.roster.form.name)
        .on_submit(|s: &mut App, _| s.submit_form())
        .build(k)
        .key("name");
    let mut out = col().w_full().gap(16.0).child(
        row()
            .w_full()
            .gap(16.0)
            .center_y()
            .child(k.interactive(face))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(text(caption).font(k.ty.small).color(c.text_muted))
                    .child(name),
            ),
    );
    if matches!(kind, FormKind::NewAgent | FormKind::NewMember(_)) {
        out = out.child(
            text("Give it a name and tell it what to do. That is all it needs, and you can change anything later.")
                .font(k.ty.body)
                .color(c.text_muted)
                .wrap()
                .max_w(520.0),
        );
    }
    if f.picker {
        out = out.child(card(
            "Pick a face",
            "Shown next to it in your list.",
            swatches(s, k),
            k,
        ));
    }
    out
}

/// The auto colour (from the name) and a few to choose from.
fn swatches(s: &App, k: &Kit) -> El {
    let f = &s.roster.form;
    let tile = |key: String, hue: Option<f32>| {
        let on = f.hue == hue;
        let shown = hue.unwrap_or_else(|| hue_of(&f.name.text));
        k.interactive(
            row()
                .key(key)
                .size(44.0, 44.0)
                .radius(12.0)
                .center()
                .fill(theme::avatar_color(shown))
                .border(
                    2.0,
                    if on {
                        k.c.text_base
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .role("radio")
                .checked(on)
                .label(if hue.is_none() {
                    "Use the colour from its name".to_owned()
                } else {
                    "Use this colour".to_owned()
                })
                .on_press(move |s: &mut App| s.roster.form.hue = hue)
                .child(
                    text(initial_of(&f.name.text))
                        .font(k.ty.ui_medium)
                        .color(k.c.white),
                ),
        )
    };
    let mut r = row().gap(10.0).child(tile("hue-auto".into(), None));
    for (i, h) in HUES.iter().enumerate() {
        r = r.child(tile(format!("hue-{i}"), Some(*h)));
    }
    r
}

/// Four starting points. They fill the name and instructions and nothing else.
fn templates(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let one = |i: usize| {
        let t = &TEMPLATES[i];
        let on = s.roster.form.template == Some(i);
        k.interactive(
            col()
                .key(format!("template-{i}"))
                .grow(1.0)
                .min_w(0.0)
                .px(14.0)
                .py(12.0)
                .gap(2.0)
                .radius(k.radius.md)
                .fill(c.layer_01)
                .border(1.0, if on { c.border_strong } else { c.border_muted })
                .role("radio")
                .checked(on)
                .on_press(move |s: &mut App| s.roster.pick_template(i))
                .child(text(t.label).font(k.ty.ui_medium).color(c.text_base))
                .child(text(t.hint).font(k.ty.small).color(c.text_muted)),
        )
    };
    col()
        .w_full()
        .gap(8.0)
        .child(
            text("Or start from an example")
                .font(k.ty.small_medium)
                .color(c.text_muted),
        )
        .child(row().w_full().gap(8.0).child(one(0)).child(one(1)))
        .child(row().w_full().gap(8.0).child(one(2)).child(one(3)))
}

fn card(title: &str, description: &str, body: El, k: &Kit) -> El {
    let c = &k.c;
    col()
        .w_full()
        .padding(20.0)
        .gap(14.0)
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .child(
            col()
                .gap(4.0)
                .child(
                    text(title.to_owned())
                        .font(k.ty.body_medium)
                        .color(c.text_base),
                )
                .child(
                    text(description.to_owned())
                        .font(k.ty.small)
                        .color(c.text_muted)
                        .wrap(),
                ),
        )
        .child(body)
}

fn model_select(s: &App, member: bool, k: &Kit, layer: &mut Vec<El>) -> El {
    let f = &s.roster.form;
    // "Use the default" says which model that is, when there is one to say.
    let default = match &s.providers.default_model {
        Some(m) => format!("Use the default ({})", s.providers.model_name(m)),
        None => "Use the default".to_owned(),
    };
    ModelPicker::new("model", s.providers.groups())
        .current(
            f.model.clone(),
            f.model.as_ref().map(|m| s.providers.model_name(m)),
        )
        .placeholder(if member {
            "Use the same as the main agent".to_owned()
        } else {
            default
        })
        .label("Model")
        .clearable(true)
        .on_open(|s: &mut App| s.models_opened())
        .on_connect(|s: &mut App| {
            // Off to the Providers page; what was typed here is not kept.
            s.cancel_form();
            s.route = crate::app::Route::Providers;
        })
        .build(
            k,
            layer,
            &f.model_picker,
            |s: &mut App| &mut s.roster.form.model_picker,
            |s: &mut App, v| s.roster.form.model = v,
        )
        .key("model")
}

/// Optional and technical: closed until asked for, with a one-line summary of what is set.
fn more_options(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let f = &s.roster.form;
    let head = k.interactive(
        row()
            .key("more-options")
            .w_full()
            .padding(20.0)
            .gap(12.0)
            .center_y()
            .expanded(f.advanced)
            .on_press(|s: &mut App| s.roster.form.advanced ^= true)
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(
                        text("More options")
                            .font(k.ty.body_medium)
                            .color(c.text_base),
                    )
                    .child(
                        text(f.summary())
                            .font(k.ty.small)
                            .color(c.text_muted)
                            .truncate(),
                    ),
            )
            .child(
                text(if f.advanced { "Hide" } else { "Show" })
                    .font(k.ty.small)
                    .color(c.text_muted),
            ),
    );
    let mut out = col()
        .w_full()
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .child(head);
    if f.advanced {
        out = out.child(
            col()
                .w_full()
                .px(20.0)
                .pb(20.0)
                .gap(20.0)
                .child(chips(
                    "Skills",
                    "Ready-made know-how you can hand it.",
                    &SKILLS,
                    &f.skills,
                    true,
                    k,
                ))
                .child(chips(
                    "What it can use (Tools)",
                    "Only what you pick here. Leave it empty and it can talk, but not act.",
                    &TOOLS,
                    &f.tools,
                    false,
                    k,
                )),
        );
    }
    out
}

fn chips(
    title: &str,
    hint: &str,
    all: &[&'static str],
    on: &[String],
    skills: bool,
    k: &Kit,
) -> El {
    let c = &k.c;
    let mut r = row().w_full().gap(6.0);
    let mut rows = col().w_full().gap(6.0);
    for (i, name) in all.iter().copied().enumerate() {
        let selected = on.iter().any(|x| x == name);
        let chip = k.interactive(
            row()
                .key(format!("{}-{name}", if skills { "skill" } else { "tool" }))
                .px(12.0)
                .py(6.0)
                .radius(16.0)
                .fill(if selected {
                    c.text_base
                } else {
                    Color::TRANSPARENT
                })
                .border(1.0, if selected { c.text_base } else { c.border_base })
                .role("checkbox")
                .checked(selected)
                .on_press(move |s: &mut App| {
                    if skills {
                        s.roster.toggle_skill(name)
                    } else {
                        s.roster.toggle_tool(name)
                    }
                })
                .child(text(name).font(k.ty.small).color(if selected {
                    c.text_inverse
                } else {
                    c.text_base
                })),
        );
        r = r.child(chip);
        if (i + 1) % 4 == 0 || i + 1 == all.len() {
            rows = rows.child(std::mem::replace(&mut r, row().w_full().gap(6.0)));
        }
    }
    col()
        .w_full()
        .gap(8.0)
        .child(
            col()
                .gap(2.0)
                .child(
                    text(title.to_owned())
                        .font(k.ty.ui_medium)
                        .color(c.text_base),
                )
                .child(text(hint.to_owned()).font(k.ty.small).color(c.text_muted)),
        )
        .child(rows)
}

/// Deleting is irreversible, so it asks: a plain statement, Cancel, and the destructive button
/// named for what it does.
pub fn confirm_delete(s: &App, k: &Kit, layer: &mut Vec<El>) {
    let Some(t) = &s.roster.confirm else { return };
    let Some(a) = s.roster.find(t) else { return };
    let member = t.is_member();
    let team = match a.subagents.len() {
        0 => String::new(),
        1 => ", its team member".to_owned(),
        n => format!(", its {n} team members"),
    };
    let label = if member {
        "Remove team member"
    } else {
        "Delete agent"
    };
    let buttons = row()
        .w_full()
        .gap(8.0)
        .justify_end()
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Neutral)
                .on_press(|s: &mut App| s.roster.confirm = None)
                .build(k)
                .key("cancel-delete"),
        )
        .child(
            Button::new(label)
                .variant(ButtonVariant::Danger)
                .on_press(|s: &mut App| s.confirm_delete())
                .build(k)
                .key("confirm-delete"),
        );
    // The sentence lives in the body: a dialog's own description line does not wrap.
    let body = col()
        .w_full()
        .px(16.0)
        .pb(16.0)
        .gap(16.0)
        .child(
            text(format!(
                "This permanently removes {}{team} and all its chats. It cannot be undone.",
                a.name
            ))
            .font(k.ty.ui)
            .color(k.c.text_muted)
            .wrap(),
        )
        .child(buttons);
    Dialog::new("delete-agent", label)
        .open(true)
        .width(440.0)
        .fit()
        .child(body)
        .on_close(|s: &mut App| s.roster.confirm = None)
        .build(k, layer);
}
