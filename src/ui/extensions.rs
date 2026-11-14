//! Extensions: tool sets built into the app, for services without an MCP server worth using.
//! The Google account they share sits on top; each extension below is a switch, and opening it
//! lists its tools, each with a switch of its own.

use rune::{El, ScrollSpec, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Switch, TextInput};

use super::providers::{ghost, section_header};
use crate::app::App;
use crate::extensions::{ALL, Extension, id_of};

const PAGE: f32 = 960.0;
const SIDE: f32 = 24.0;
const FIELD: f32 = 380.0;

pub fn screen(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let mut page = col()
        .w_full()
        .max_w(PAGE)
        .px(SIDE)
        .py(32.0)
        .gap(28.0)
        .child(
            col()
                .gap(10.0)
                .child(
                    text("Extensions")
                        .font(rune::TextStyle {
                            size: 28.0,
                            line_height: 34.0,
                            ..k.ty.body_medium
                        })
                        .color(c.text_base),
                )
                .child(
                    text("Tools built into Oido for services that have no MCP server: turn one on, sign in, and give an agent the tools it needs in its form. Everything runs on this computer, as you. An agent uses a tool the moment it calls it, with no question asked, so switch off the ones you do not want it to have.")
                        .font(k.ty.body)
                        .color(c.text_muted)
                        .wrap()
                        .max_w(560.0),
                ),
        );
    if let Some(note) = &s.ext.notice {
        page = page.child(
            text(note.clone())
                .key("ext-notice")
                .font(k.ty.small)
                .color(c.text_base)
                .wrap(),
        );
    }
    page = page.child(account_card(s, k)).child(
        col()
            .w_full()
            .gap(10.0)
            .child(section_header("Extensions", ALL.len(), k))
            .child(
                col()
                    .w_full()
                    .gap(12.0)
                    .children(ALL.iter().map(|e| extension_card(s, *e, k))),
            ),
    );
    col()
        .key("extensions")
        .w_full()
        .h_full()
        .min_h(0.0)
        .items_center()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

fn card(k: &Kit) -> El {
    col()
        .min_w(0.0)
        .padding(16.0)
        .gap(12.0)
        .radius(k.radius.lg)
        .border(1.0, k.c.border_muted)
        .fill(k.c.layer_01)
}

fn button(
    label: &str,
    key: &str,
    primary: bool,
    disabled: bool,
    k: &Kit,
    on: impl Fn(&mut App) + 'static,
) -> El {
    Button::new(label)
        .variant(if primary {
            ButtonVariant::Submit
        } else {
            ButtonVariant::Outline
        })
        .control_size(ControlSize::Small)
        .disabled(disabled)
        .on_press(on)
        .build(k)
        .key(key.to_owned())
}

/// The Google client and sign-in every Google extension shares.
fn account_card(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let a = &s.ext.account;
    let waiting = s.ext.signing_in.is_some();
    let state = match (&a.email, a.signed_in, a.has_client) {
        (Some(email), true, _) => format!("Signed in as {email}"),
        (None, true, _) => "Signed in".to_owned(),
        (_, false, true) => "Client saved, not signed in".to_owned(),
        _ => "Not set up".to_owned(),
    };
    let mut actions = row().w_full().gap(8.0).center_y().flex_wrap().child(button(
        "Save client",
        "ext-save-client",
        !a.has_client,
        !s.ext_can_save_client(),
        k,
        |s: &mut App| s.ext_save_client(),
    ));
    if waiting {
        actions = actions.child(button(
            "Cancel",
            "ext-cancel",
            false,
            false,
            k,
            |s: &mut App| s.ext_cancel_sign_in(),
        ));
    } else {
        actions = actions.child(button(
            if a.signed_in {
                "Sign in again"
            } else {
                "Sign in"
            },
            "ext-signin",
            a.has_client,
            !a.has_client,
            k,
            |s: &mut App| s.ext_sign_in(),
        ));
    }
    if a.signed_in {
        actions = actions.child(button(
            "Sign out",
            "ext-signout",
            false,
            waiting,
            k,
            |s: &mut App| s.ext_sign_out(),
        ));
    }
    card(k)
        .key("ext-account")
        .child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(super::logo::of("Google", crate::extensions::GOOGLE_LOGO, k))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(text("Google account").font(k.ty.ui).color(c.text_base))
                        .child(
                            text(state)
                                .key("ext-account-state")
                                .font(k.ty.small)
                                .color(c.text_muted)
                                .truncate(),
                        ),
                ),
        )
        .child(
            text("Google's services sign in through a client of your own, so your data never passes through anyone else. In Google Cloud Console make a project, turn on the APIs you want, make an OAuth client of type Desktop app, and paste its id and secret here. Set the app's publishing status to In production, or Google ends the sign-in every 7 days.")
                .font(k.ty.small)
                .color(c.text_faint)
                .wrap(),
        )
        .child(
            TextInput::new(&s.ext.client_id)
                .placeholder("Client ID")
                .label("Client ID")
                .width(Some(FIELD))
                .on_edit(|s: &mut App| &mut s.ext.client_id)
                .build(k)
                .key("ext-client-id"),
        )
        .child(
            TextInput::new(&s.ext.client_secret)
                .placeholder(if a.has_client {
                    "Saved. Paste a new secret to replace it"
                } else {
                    "Client secret"
                })
                .label("Client secret")
                .secure()
                .width(Some(FIELD))
                .on_edit(|s: &mut App| &mut s.ext.client_secret)
                .build(k)
                .key("ext-client-secret"),
        )
        .child(actions)
}

fn extension_card(s: &App, ext: &'static dyn Extension, k: &Kit) -> El {
    let c = &k.c;
    let id = id_of(ext);
    let on = s.ext.enabled(ext);
    let open = s.ext.open.as_deref() == Some(id.as_str());
    let blocked = on.then(|| s.ext.blocked(ext)).flatten();
    let state = if !on {
        "Off".to_owned()
    } else if blocked.is_some() {
        "On, waiting for sign-in".to_owned()
    } else {
        let tools = ext.tools();
        let live = tools.iter().filter(|t| s.ext.tool_on(ext, t.name)).count();
        format!("On · {live} of {} tools", tools.len())
    };
    let head = row()
        .w_full()
        .gap(12.0)
        .center_y()
        .child(super::logo::of(ext.title(), ext.logo(), k))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(ext.title())
                        .font(k.ty.ui)
                        .color(c.text_base)
                        .truncate(),
                )
                .child(text(state).font(k.ty.small).color(c.text_muted).truncate()),
        )
        .child(
            Switch::new(on)
                .label(format!("Use {}", ext.title()))
                .hide_label()
                .on_toggle(move |s: &mut App| s.ext_toggle(ext))
                .build(k)
                .key(format!("ext-toggle-{id}")),
        );
    let mut r = card(k).key(format!("ext-{id}")).child(head).child(
        text(ext.blurb())
            .font(k.ty.small)
            .color(c.text_faint)
            .wrap(),
    );
    if let Some(why) = blocked {
        r = r.child(
            text(why)
                .key(format!("ext-blocked-{id}"))
                .font(k.ty.small)
                .color(c.danger.fg)
                .wrap(),
        );
    }
    r = r.child(row().w_full().child(ghost(
        if open { "Hide tools" } else { "Tools" },
        format!("ext-tools-{id}"),
        k,
        move |s: &mut App| s.ext_toggle_open(ext),
    )));
    if open {
        r = r.child(tools_list(s, ext, k));
    }
    r
}

fn tools_list(s: &App, ext: &'static dyn Extension, k: &Kit) -> El {
    let c = &k.c;
    let id = id_of(ext);
    col()
        .w_full()
        .gap(10.0)
        .children(ext.tools().into_iter().map(|t| {
            let name = t.name;
            let first = t.description.lines().next().unwrap_or_default().to_owned();
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            text(format!("{name} · {}", t.access.label()))
                                .font(k.ty.mono)
                                .color(c.text_base)
                                .truncate(),
                        )
                        .child(
                            text(first)
                                .font(k.ty.small)
                                .color(c.text_muted)
                                .max_lines(2),
                        ),
                )
                .child(
                    Switch::new(s.ext.tool_on(ext, name))
                        .label(format!("Allow {name}"))
                        .hide_label()
                        .on_toggle(move |s: &mut App| s.ext_toggle_tool(ext, name))
                        .build(k)
                        .key(format!("ext-tool-{id}-{name}")),
                )
        }))
}
