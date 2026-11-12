//! MCP servers: other programs that give agents tools. Any number can be registered; each one's
//! tools then show up in an agent's form to be given to it.

use rune::{Cx, El, ScrollSpec, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Divider, Kit, SegmentedControl, Switch, TextInput,
};

use super::logo;
use super::providers::{ghost, pill, section_header, tile};
use crate::app::App;
use crate::mcp::{Auth, Kind, Preset, Server, Status, Transport};

const PAGE: f32 = 960.0;
const SIDE: f32 = 24.0;
/// Servers and presets sit in as many columns as fit, up to this many.
const COLUMNS: usize = 2;
/// The narrowest a card is drawn before the grid gives up a column, so a name and a button fit.
const CARD_MIN: f32 = 240.0;
const GAP: f32 = 12.0;
/// The widest the form's choices (remote or local, how it signs in) are drawn; narrower pages
/// give them what they have.
const CHOICE: f32 = 380.0;

/// How many columns of cards the page has room for. The window less its sidebars is the page's
/// width, and a narrow one gets a single column instead of cards too thin to read.
fn columns(cx: &Cx<App>) -> usize {
    cx.layout_with("mcp", |[_, _, width, _]| {
        let room = width.min(PAGE) - 2.0 * SIDE;
        ((room + GAP) / (CARD_MIN + GAP))
            .floor()
            .clamp(1.0, COLUMNS as f32) as usize
    })
    .unwrap_or(COLUMNS)
}

pub fn screen(s: &App, cx: &Cx<App>, k: &Kit) -> El {
    let c = &k.c;
    let columns = columns(cx);
    let m = &s.mcp;
    // A server answers to its name, its address and the names of its tools.
    let shown: Vec<&Server> = m
        .servers
        .iter()
        .filter(|x| {
            let target = x.target();
            let mut fields = vec![x.name.as_str(), target.as_str()];
            fields.extend(x.tools.iter().map(|t| t.name.as_str()));
            m.matches(&fields)
        })
        .collect();
    let presets: Vec<&Preset> = crate::mcp::PRESETS
        .iter()
        .filter(|p| !m.has_preset(p) && m.matches(&[p.name, p.description, p.url]))
        .collect();
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
                    text("MCP servers")
                        .font(rune::TextStyle {
                            size: 28.0,
                            line_height: 34.0,
                            ..k.ty.body_medium
                        })
                        .color(c.text_base),
                )
                .child(
                    text("Programs that give agents more to do: search docs, read issues, query a database. Register as many as you like, then give an agent the tools it needs in its form. An agent runs them as soon as it calls them, with no question asked.")
                        .font(k.ty.body)
                        .color(c.text_muted)
                        .wrap()
                        .max_w(560.0),
                ),
        );
    if let Some(note) = &m.notice {
        page = page.child(
            text(note.clone())
                .key("mcp-notice")
                .font(k.ty.small)
                .color(c.text_base)
                .wrap(),
        );
    }
    page = page.child(
        TextInput::new(&m.search)
            .placeholder("Search servers and presets")
            .label("Search")
            .width(None)
            .on_edit(|s: &mut App| &mut s.mcp.search)
            .build(k)
            .key("mcp-search"),
    );
    page = page.child(add_panel(s, k));
    if !shown.is_empty() {
        page = page.child(cards(
            "Servers",
            shown.iter().map(|x| server_card(s, x, k)).collect(),
            columns,
            k,
        ));
    } else if m.servers.is_empty() {
        page = page.child(
            text("No servers yet. Add one above, or start from a preset below.")
                .key("mcp-empty")
                .font(k.ty.small)
                .color(c.text_faint)
                .wrap(),
        );
    } else if presets.is_empty() {
        page = page.child(
            text("Nothing matches. Clear the search, or add a server by its address above.")
                .key("mcp-none")
                .font(k.ty.small)
                .color(c.text_faint)
                .wrap(),
        );
    }
    if !presets.is_empty() {
        page = page.child(cards(
            "Quick add",
            presets.iter().map(|p| preset_card(p, k)).collect(),
            columns,
            k,
        ));
    }
    col()
        .key("mcp")
        .w_full()
        .h_full()
        .min_h(0.0)
        .items_center()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

/// What the server is doing: working, off, or how many tools it gave.
fn state(s: &App, server: &Server) -> String {
    if s.mcp.busy.contains(&server.id) {
        "Connecting…".to_owned()
    } else if !server.enabled {
        "Off".to_owned()
    } else {
        match server.status {
            Status::Pending => "Not reached yet".to_owned(),
            Status::Reachable => match server.tools.len() {
                1 => "1 tool".to_owned(),
                n => format!("{n} tools"),
            },
            Status::Unreachable => "Not reachable".to_owned(),
        }
    }
}

/// A titled group of cards in columns.
fn cards(title: &str, cards: Vec<El>, columns: usize, k: &Kit) -> El {
    col()
        .w_full()
        .gap(10.0)
        .child(section_header(title, cards.len(), k))
        .child(
            col()
                .w_full()
                .gap(GAP)
                .equal_columns(columns)
                .children(cards),
        )
}

/// A card's frame: the layer a server or preset sits on.
fn card(k: &Kit) -> El {
    col()
        .min_w(0.0)
        .padding(16.0)
        .gap(12.0)
        .radius(k.radius.lg)
        .border(1.0, k.c.border_muted)
        .fill(k.c.layer_01)
}

fn server_card(s: &App, server: &Server, k: &Kit) -> El {
    let c = &k.c;
    let id = server.id.clone();
    let busy = s.mcp.busy.contains(&id);
    let open = s.mcp.open.as_deref() == Some(id.as_str());
    let head = row()
        .w_full()
        .gap(12.0)
        .center_y()
        .child(server_tile(server, k))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(server.name.clone())
                        .font(k.ty.ui)
                        .color(c.text_base)
                        .truncate(),
                )
                .child(
                    text(state(s, server))
                        .font(k.ty.small)
                        .color(c.text_muted)
                        .truncate(),
                ),
        )
        .child({
            let toggled = id.clone();
            Switch::new(server.enabled)
                .label(format!("Use {}", server.name))
                .hide_label()
                .on_toggle(move |s: &mut App| s.mcp_toggle(&toggled))
                .build(k)
                .key(format!("mcp-toggle-{id}"))
        });
    let mut actions = row().w_full().gap(4.0).center_y().flex_wrap().child({
        let id = id.clone();
        ghost(
            if open { "Hide tools" } else { "Tools" },
            format!("mcp-tools-{id}"),
            k,
            move |s: &mut App| s.mcp_toggle_tools(&id),
        )
    });
    if server.auth == Auth::OAuth {
        let id = id.clone();
        actions = actions.child(
            Button::new(if busy { "Signing in…" } else { "Sign in" })
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .disabled(busy)
                .on_press(move |s: &mut App| s.mcp_sign_in(&id))
                .build(k)
                .key(format!("mcp-signin-{}", server.id)),
        );
    }
    actions = actions
        .child({
            let id = id.clone();
            Button::new(if busy { "Refreshing…" } else { "Refresh" })
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .help("Ask the server for its tools again")
                .disabled(busy || !server.enabled)
                .on_press(move |s: &mut App| s.mcp_resync(&id))
                .build(k)
                .key(format!("mcp-refresh-{}", server.id))
        })
        .child(el().grow(1.0))
        .child({
            let id = id.clone();
            ghost(
                "Remove",
                format!("mcp-remove-{id}"),
                k,
                move |s: &mut App| s.mcp_remove(&id),
            )
        });
    let mut r = card(k).key(format!("mcp-{id}")).child(head).child(
        text(server.target())
            .font(k.ty.small)
            .color(c.text_faint)
            .truncate(),
    );
    if server.status == Status::Unreachable && !busy {
        r = r.child(
            text(server.detail.clone())
                .key("row-error")
                .font(k.ty.small)
                .color(c.danger.fg)
                .max_lines(3),
        );
    }
    r = r.child(actions);
    if open {
        r = r.child(tools_list(server, k));
    }
    r
}

/// The server's logo when it is a preset's, else a tile with its initial.
fn server_tile(server: &Server, k: &Kit) -> El {
    match &server.transport {
        Transport::Http { url } => logo::tile(url, k),
        Transport::Stdio { .. } => None,
    }
    .unwrap_or_else(|| tile(&server.name, crate::providers::SERVER_COLOR, k))
}

fn tools_list(server: &Server, k: &Kit) -> El {
    let c = &k.c;
    let mut list = col().w_full().gap(8.0);
    if server.tools.is_empty() {
        list = list.child(
            text("No tools yet. They appear once the server has been reached.")
                .font(k.ty.small)
                .color(c.text_faint)
                .wrap(),
        );
    }
    for t in &server.tools {
        let first = t.description.lines().next().unwrap_or_default().to_owned();
        list = list.child(
            col()
                .gap(2.0)
                .min_w(0.0)
                .child(
                    text(server.qualified(&t.name))
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
        );
    }
    col()
        .w_full()
        .gap(12.0)
        .child(Divider::horizontal().build(k))
        .child(list)
}

/// A preset to start from: its name and what it does, and the press that fills the form with it.
fn preset_card(p: &Preset, k: &Kit) -> El {
    let preset = *p;
    let c = &k.c;
    let description = format!(
        "{}{}",
        p.description,
        if p.auth == Auth::OAuth {
            " Signs in with your browser."
        } else {
            ""
        }
    );
    k.interactive(
        card(k)
            .key(format!("mcp-preset-{}", p.name))
            // The form it fills is at the top of the page; the person is far down it.
            .on_press_tx(move |s: &mut App, tx: &mut rune::Tx| {
                s.mcp_use_preset(&preset);
                tx.scroll_to("mcp", 0.0, 0.0);
            })
            .child(
                row()
                    .w_full()
                    .gap(12.0)
                    .center_y()
                    .child(
                        logo::tile(p.url, k)
                            .unwrap_or_else(|| tile(p.name, crate::providers::SERVER_COLOR, k)),
                    )
                    .child(
                        text(p.name)
                            .font(k.ty.ui)
                            .color(c.text_base)
                            .truncate()
                            .grow(1.0),
                    )
                    .child(pill("Add", k)),
            )
            .child(
                text(description)
                    .font(k.ty.small)
                    .color(c.text_muted)
                    .max_lines(3),
            ),
    )
}

/// The way in for a server the presets do not have: a row that opens its form below it.
fn add_panel(s: &App, k: &Kit) -> El {
    col()
        .w_full()
        .radius(k.radius.lg)
        .border(1.0, k.c.border_muted)
        .fill(k.c.layer_01)
        .child(add_row(s, k))
}

fn add_row(s: &App, k: &Kit) -> El {
    let open = s.mcp.adding;
    let head = k.interactive(
        row()
            .key("mcp-add")
            .w_full()
            .min_h(64.0)
            .padding(14.0)
            .gap(12.0)
            .center_y()
            .expanded(open)
            .on_press(|s: &mut App| s.mcp_toggle_form())
            .child(tile("Server", crate::providers::SERVER_COLOR, k))
            .child(
                text("Add a remote server or a program on this computer")
                    .font(k.ty.ui)
                    .color(k.c.text_base)
                    .wrap()
                    .min_w(0.0)
                    .grow(1.0),
            )
            .child(pill(if open { "Close" } else { "Add" }, k)),
    );
    let mut r = col().w_full().child(head);
    if open {
        r = r.child(form(s, k));
    }
    r
}

fn form(s: &App, k: &Kit) -> El {
    let m = &s.mcp;
    let f = &m.form;
    let c = &k.c;
    let kind = SegmentedControl::new(
        &["Remote server", "Program on this computer"],
        Some(usize::from(f.kind == Kind::Local)),
    )
    .label("What to add")
    .width(None)
    .on_pick(|s: &mut App, i, _| {
        s.mcp_set_kind(if i == 0 { Kind::Remote } else { Kind::Local });
    })
    .build(k)
    .max_w(CHOICE)
    .key("mcp-kind");
    let submit = |s: &mut App, _: &mut rune::Tx| s.mcp_add();
    let name = TextInput::new(&f.name)
        .placeholder("Name, e.g. Docs")
        .label("Name")
        .width(None)
        .on_edit(|s: &mut App| &mut s.mcp.form.name)
        .on_submit(submit)
        .build(k)
        .key("mcp-name");
    let mut fields = col().gap(8.0).child(kind).child(name);
    match f.kind {
        Kind::Remote => {
            fields = fields.child(
                TextInput::new(&f.url)
                    .placeholder("Address, e.g. https://mcp.example.com/mcp")
                    .label("Address")
                    .width(None)
                    .on_edit(|s: &mut App| &mut s.mcp.form.url)
                    .on_submit(submit)
                    .build(k)
                    .key("mcp-url"),
            );
            let selected = match f.auth {
                Auth::None => 0,
                Auth::Key => 1,
                Auth::OAuth => 2,
            };
            fields = fields.child(
                SegmentedControl::new(&["No sign-in", "Key", "Browser sign-in"], Some(selected))
                    .label("How it knows you")
                    .width(None)
                    .on_pick(|s: &mut App, i, _| {
                        s.mcp_set_auth([Auth::None, Auth::Key, Auth::OAuth][i.min(2)]);
                    })
                    .build(k)
                    .max_w(CHOICE)
                    .key("mcp-auth"),
            );
            if f.auth == Auth::Key {
                fields = fields.child(
                    TextInput::new(&f.key)
                        .placeholder("The key the server gave you")
                        .label("Key")
                        .secure()
                        .width(None)
                        .on_edit(|s: &mut App| &mut s.mcp.form.key)
                        .on_submit(submit)
                        .build(k)
                        .key("mcp-key"),
                );
            }
        }
        Kind::Local => {
            fields = fields
                .child(
                    TextInput::new(&f.command)
                        .placeholder("Command, e.g. npx")
                        .label("Command")
                        .width(None)
                        .on_edit(|s: &mut App| &mut s.mcp.form.command)
                        .on_submit(submit)
                        .build(k)
                        .key("mcp-command"),
                )
                .child(
                    TextInput::new(&f.args)
                        .placeholder("Arguments, e.g. -y @modelcontextprotocol/server-everything")
                        .label("Arguments")
                        .width(None)
                        .on_edit(|s: &mut App| &mut s.mcp.form.args)
                        .on_submit(submit)
                        .build(k)
                        .key("mcp-args"),
                )
                .child(
                    TextInput::new(&f.env)
                        .placeholder("Variables, if it needs any: API_KEY=abc OTHER=xyz")
                        .label("Environment variables")
                        .secure()
                        .width(None)
                        .on_edit(|s: &mut App| &mut s.mcp.form.env)
                        .on_submit(submit)
                        .build(k)
                        .key("mcp-env"),
                );
        }
    }
    let signing = m.signing_in.is_some();
    let add = Button::new(match (f.kind, m.confirming) {
        (Kind::Local, true) => "Run it and add",
        _ => "Add",
    })
    .variant(ButtonVariant::Contrast)
    .disabled(!m.can_add() || signing)
    .on_press(|s: &mut App| s.mcp_add())
    .build(k)
    .key("mcp-submit");
    let mut buttons = row().gap(8.0).center_y().child(add);
    if signing {
        buttons = buttons.child(ghost(
            "Cancel sign-in",
            "mcp-cancel".into(),
            k,
            |s: &mut App| {
                s.mcp_cancel_sign_in();
            },
        ));
    } else if m.confirming {
        buttons = buttons.child(ghost("Cancel", "mcp-unconfirm".into(), k, |s: &mut App| {
            s.mcp.confirming = false;
        }));
    }
    let mut box_ = col()
        .w_full()
        .max_w(640.0)
        .pl(62.0)
        .pr(14.0)
        .pb(14.0)
        .gap(10.0)
        .child(fields);
    if f.kind == Kind::Local && m.confirming {
        box_ = box_.child(
            col()
                .key("mcp-confirm")
                .w_full()
                .padding(12.0)
                .gap(6.0)
                .radius(k.radius.lg)
                .fill(c.layer_02)
                .border(1.0, c.border_muted)
                .child(
                    text("Adding this runs a program on your computer, with your permissions, now and whenever an agent uses it:")
                        .font(k.ty.small)
                        .color(c.text_muted)
                        .wrap(),
                )
                .child(text(m.command_line()).font(k.ty.mono).color(c.text_base).wrap()),
        );
    }
    box_.child(buttons).child(
        text("Keys, variables and sign-ins are saved encrypted on this computer. The key that unlocks them stays in your system keyring.")
            .font(k.ty.small)
            .color(c.text_faint)
            .wrap(),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use rune::{Env, Runtime};

    use crate::app::{App, Route};

    #[test]
    fn adding_a_preset_from_far_down_the_page_brings_its_form_into_view() {
        let mut app = App::new(None);
        app.route = Route::Mcp;
        let mut rt = Runtime::new(
            app,
            super::super::view,
            (1280.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        // Down the page until a preset near its end is on screen, clear of the title bar.
        for _ in 0..200 {
            if rt
                .find("mcp-preset-Stripe")
                .is_some_and(|(_, y)| (150.0..700.0).contains(&y))
            {
                break;
            }
            rt.wheel(640.0, 450.0, -60.0);
            rt.frame(1.0 / 60.0);
        }
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt
            .find("mcp-preset-Stripe")
            .expect("the preset is on the page");
        assert!(
            (150.0..700.0).contains(&y),
            "the preset is on screen at y={y}"
        );
        // A click as a hand makes it: arrive, press, let go, with frames between.
        rt.pointer_move(x, y);
        rt.frame(1.0 / 60.0);
        rt.pointer_down(x, y);
        rt.frame(1.0 / 60.0);
        rt.pointer_up(x, y);
        for _ in 0..90 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.state.mcp.adding, "the press opened the form");
        let (_, name_y) = rt.find("mcp-name").expect("the form is on the page");
        assert!(
            (0.0..900.0).contains(&name_y),
            "the form's name field is at y={name_y}, off the screen"
        );
    }
}
