//! Providers: the services that run the agents. Connected first, your own servers, then the rest.
//! One connect form is open at a time, so closed rows stay a single quiet line.

use rune::color::hex;
use rune::{Color, El, ScrollSpec, TextStyle, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Divider, Kit, TextInput};

use super::model_picker::ModelPicker;

use crate::app::{App, now};
use crate::providers::{NEW_SERVER, Provider, Providers};

const PAGE: f32 = 768.0;

pub fn screen(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let p = &s.providers;
    let c = &k.c;
    let title = TextStyle {
        size: 28.0,
        line_height: 34.0,
        ..k.ty.body_medium
    };
    let status = match p.list.iter().filter(|x| x.connected).count() {
        0 => "None connected yet".to_owned(),
        1 => "1 connected".to_owned(),
        n => format!("{n} connected"),
    };
    let mut page = col()
        .w_full()
        .max_w(PAGE)
        .px(24.0)
        .py(32.0)
        .gap(28.0)
        .child(
            col()
                .gap(10.0)
                .child(text("AI providers").font(title).color(c.text_base))
                .child(
                    text("The services that actually run your agents. Connect at least one with a key from your account there, and your agents can start working.")
                        .font(k.ty.body)
                        .color(c.text_muted)
                        .wrap()
                        .max_w(520.0),
                )
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(
                            el().size(8.0, 8.0)
                                .radius(4.0)
                                .border(1.0, c.text_base)
                                .fill(if p.list.iter().any(|x| x.connected) { c.text_base } else { Color::TRANSPARENT }),
                        )
                        .child(text(status).font(k.ty.small).color(c.text_muted)),
                )
                .child(default_model(s, k, layer)),
        );
    let connected = p.connected();
    if !connected.is_empty() {
        page = page.child(section(
            "Connected",
            connected.len(),
            connected.iter().map(|x| connected_row(s, x, k)).collect(),
            k,
        ));
    }
    let servers = p.servers();
    let mut server_rows: Vec<El> = servers.iter().map(|x| server_row(s, x, k)).collect();
    server_rows.push(add_server_row(s, k));
    page = page.child(section("Custom servers", servers.len(), server_rows, k));
    let available = p.available();
    if !available.is_empty() {
        page = page.child(section(
            "Not connected",
            available.len(),
            available.iter().map(|x| available_row(s, x, k)).collect(),
            k,
        ));
    }
    col()
        .key("providers")
        .w_full()
        .h_full()
        .min_h(0.0)
        .items_center()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

fn section(title: &str, count: usize, rows: Vec<El>, k: &Kit) -> El {
    let c = &k.c;
    let mut group = col()
        .w_full()
        .radius(k.radius.lg)
        .border(1.0, c.border_muted)
        .fill(c.layer_01);
    let last = rows.len().saturating_sub(1);
    for (i, r) in rows.into_iter().enumerate() {
        group = group.child(r);
        if i != last {
            group = group.child(Divider::horizontal().build(k));
        }
    }
    col()
        .w_full()
        .gap(10.0)
        .child(
            row()
                .w_full()
                .center_y()
                .child(
                    text(title.to_owned())
                        .font(k.ty.small_medium)
                        .color(c.text_muted),
                )
                .child(el().grow(1.0))
                .child(text(count.to_string()).font(k.ty.small).color(c.text_faint)),
        )
        .child(group)
}

/// The brand tile: the provider's own colour with its initial, like an app icon.
fn tile(label: &str, color: &str, k: &Kit) -> El {
    let initial = label.chars().next().unwrap_or('?').to_string();
    row()
        .size(36.0, 36.0)
        .radius(10.0)
        .center()
        .fill(hex(color).unwrap_or(k.c.layer_03))
        .border(1.0, k.c.border_muted)
        .child(text(initial).font(k.ty.ui_medium).color(k.c.white))
}

fn ghost(label: &str, key: String, k: &Kit, on: impl Fn(&mut App) + 'static) -> El {
    Button::new(label)
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .on_press(on)
        .build(k)
        .key(key)
}

/// Asks the provider for its models again. While it does, it says so and cannot be pressed twice.
fn refresh_button(s: &App, id: &str, k: &Kit) -> El {
    let busy = s.providers.busy.contains(id);
    let on = id.to_owned();
    Button::new(if busy {
        "Refreshing…"
    } else {
        "Refresh models"
    })
    .variant(ButtonVariant::Ghost)
    .control_size(ControlSize::Small)
    .help("Ask this provider for its current models")
    .disabled(busy)
    .on_press(move |s: &mut App| s.refresh_models(&on))
    .build(k)
    .key(format!("refresh-{id}"))
}

/// "42 models" with how long ago they were fetched, or that there are none.
fn models_line(p: &Providers, id: &str) -> Option<String> {
    let saved = p.models.get(id)?;
    let n = saved.models.len();
    let count = match n {
        0 => "no models".to_owned(),
        1 => "1 model".to_owned(),
        n => format!("{n} models"),
    };
    Some(match saved.fetched_at {
        Some(at) => format!("{count} · updated {}", ago(now() - at)),
        None => count,
    })
}

fn ago(secs: i64) -> String {
    match secs {
        ..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

/// What agents with no model of their own use: set here, shown by name.
fn default_model(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let p = &s.providers;
    let current = p.default_model.clone();
    let name = current.as_ref().map(|m| p.model_name(m));
    col()
        .gap(6.0)
        .child(
            text("Default model")
                .font(k.ty.small_medium)
                .color(k.c.text_muted),
        )
        .child(
            ModelPicker::new("default-model", p.groups())
                .current(current, name)
                .placeholder("Choose a default model")
                .label("Default model")
                .clearable(true)
                .on_open(|s: &mut App| s.models_opened())
                .on_connect(|s: &mut App| s.providers.default_picker.close())
                .build(
                    k,
                    layer,
                    &p.default_picker,
                    |s: &mut App| &mut s.providers.default_picker,
                    |s: &mut App, v| s.set_default_model(v),
                )
                .key("default-model"),
        )
        .child(
            text("Used by agents that have not chosen a model of their own.")
                .font(k.ty.small)
                .color(k.c.text_faint),
        )
}

fn names(label: &str, detail: String, error: Option<&String>, k: &Kit) -> El {
    let c = &k.c;
    let mut col_ = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(4.0)
        .child(
            text(label.to_owned())
                .font(k.ty.ui)
                .color(c.text_base)
                .truncate(),
        )
        .child(text(detail).font(k.ty.small).color(c.text_muted).truncate());
    if let Some(why) = error {
        col_ = col_.child(
            text(why.clone())
                .font(k.ty.small)
                .color(c.danger.fg)
                .wrap()
                .key("row-error"),
        );
    }
    col_
}

fn connected_row(s: &App, p: &Provider, k: &Kit) -> El {
    let id = p.id.clone();
    let detail = [
        p.key_masked.clone(),
        p.base_url.clone(),
        models_line(&s.providers, &p.id),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let off = ghost(
        "Disconnect",
        format!("disconnect-{id}"),
        k,
        move |s: &mut App| s.disconnect_provider(&id),
    );
    k.interactive(
        row()
            .key(format!("prov-{}", p.id))
            .w_full()
            .padding(14.0)
            .gap(12.0)
            .center_y()
            .child(tile(&p.label, p.color, k))
            .child(names(
                &p.label,
                if detail.is_empty() {
                    "Signed in".into()
                } else {
                    detail
                },
                s.providers.row_errors.get(&p.id),
                k,
            ))
            .child(refresh_button(s, &p.id, k))
            .child(off),
    )
}

fn pill(label: &str, k: &Kit) -> El {
    row()
        .px(12.0)
        .py(6.0)
        .radius(16.0)
        .fill(k.c.layer_02)
        .border(1.0, k.c.border_muted)
        .child(text(label.to_owned()).font(k.ty.small).color(k.c.text_base))
}

/// A row that opens its form when pressed, then the form below it.
fn expandable(
    key: String,
    tile: El,
    label: &str,
    action: &str,
    toggle: impl Fn(&mut App) + 'static,
    form: Option<El>,
    k: &Kit,
) -> El {
    let open = form.is_some();
    let head = k.interactive(
        row()
            .key(key.clone())
            .w_full()
            .min_h(64.0)
            .padding(14.0)
            .gap(12.0)
            .center_y()
            .expanded(open)
            .on_press(toggle)
            .child(tile)
            .child(
                text(label.to_owned())
                    .font(k.ty.ui)
                    .color(k.c.text_base)
                    .truncate()
                    .grow(1.0),
            )
            .child(pill(if open { "Close" } else { action }, k)),
    );
    let mut r = col()
        .w_full()
        .fill(if open {
            k.c.layer_02.alpha(0.4)
        } else {
            Color::TRANSPARENT
        })
        .child(head);
    if let Some(form) = form {
        r = r.child(form);
    }
    r
}

fn available_row(s: &App, p: &Provider, k: &Kit) -> El {
    let open = s.providers.open.as_deref() == Some(p.id.as_str());
    let id = p.id.clone();
    let form = open.then(|| {
        let connect = {
            let id = id.clone();
            let busy = s.providers.busy.contains(&id);
            Button::new(if busy { "Connecting…" } else { "Connect" })
                .variant(ButtonVariant::Contrast)
                .disabled(busy || !s.providers.can_connect())
                .on_press(move |s: &mut App| s.connect_provider(&id))
                .build(k)
                .key("connect")
        };
        let field = {
            let id = id.clone();
            TextInput::new(&s.providers.key)
                .placeholder("Paste the key from your account")
                .label("API key")
                .secure()
                .width(None)
                .on_edit(|s: &mut App| &mut s.providers.key)
                .on_submit(move |s: &mut App, _| s.connect_provider(&id))
                .build(k)
                .key("key")
                .grow(1.0)
        };
        form_box(
            s,
            k,
            col().gap(8.0).child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(field)
                    .child(connect),
            ),
        )
    });
    let toggle = {
        let id = id.clone();
        move |s: &mut App| s.providers.toggle(&id)
    };
    expandable(
        format!("prov-{id}"),
        tile(&p.label, p.color, k),
        &p.label,
        "Connect",
        toggle,
        form,
        k,
    )
}

/// A form's frame: the fields, why it failed if it did, and where the key goes.
fn form_box(s: &App, k: &Kit, content: El) -> El {
    let mut f = col()
        .w_full()
        .pl(62.0)
        .pr(14.0)
        .pb(14.0)
        .gap(8.0)
        .child(content);
    if let Some(why) = &s.providers.error {
        f = f.child(
            text(why.clone())
                .font(k.ty.small)
                .color(k.c.danger.fg)
                .wrap()
                .key("form-error"),
        );
    }
    f.child(
        text("Saved encrypted on this computer. The key that unlocks it stays in your system keyring.")
            .font(k.ty.small)
            .color(k.c.text_faint)
            .wrap(),
    )
}

/// One form for adding a server (`NEW_SERVER`) or editing one.
fn server_form(s: &App, id: &str, k: &Kit) -> El {
    let is_new = id == NEW_SERVER;
    let submit = {
        let id = id.to_owned();
        move |s: &mut App, _: &mut rune::Tx| s.save_server(&id)
    };
    let name = TextInput::new(&s.providers.name)
        .placeholder("Name, e.g. RunPod Llama 70B")
        .label("Server name")
        .width(None)
        .on_edit(|s: &mut App| &mut s.providers.name)
        .on_submit(submit.clone())
        .build(k)
        .key("name");
    let url = TextInput::new(&s.providers.url)
        .placeholder("Server URL, e.g. https://xxxx.proxy.runpod.net/v1")
        .label("Server URL")
        .width(None)
        .on_edit(|s: &mut App| &mut s.providers.url)
        .on_submit(submit.clone())
        .build(k)
        .key("url");
    let key = TextInput::new(&s.providers.key)
        .placeholder(if is_new {
            "Key, if your server needs one"
        } else {
            "Leave blank to keep the current key"
        })
        .label("API key")
        .secure()
        .width(None)
        .on_edit(|s: &mut App| &mut s.providers.key)
        .on_submit(submit.clone())
        .build(k)
        .key("key")
        .grow(1.0);
    let save = {
        let id = id.to_owned();
        // The server is asked for its models first; for a new one its id is not known until named.
        let busy = if is_new {
            s.providers
                .busy
                .contains(&s.providers.new_server_id(&s.providers.name.text))
        } else {
            s.providers.busy.contains(id.as_str())
        };
        Button::new(match (busy, is_new) {
            (true, _) => "Checking…",
            (false, true) => "Add",
            (false, false) => "Save",
        })
        .variant(ButtonVariant::Contrast)
        .disabled(busy || !s.providers.can_save(id.as_str()))
        .on_press(move |s: &mut App| s.save_server(&id))
        .build(k)
        .key("save")
    };
    form_box(
        s,
        k,
        col()
            .gap(8.0)
            .child(name)
            .child(url)
            .child(row().w_full().gap(8.0).center_y().child(key).child(save)),
    )
}

fn server_row(s: &App, p: &Provider, k: &Kit) -> El {
    let open = s.providers.open.as_deref() == Some(p.id.as_str());
    let detail = [
        p.base_url.clone(),
        p.key_masked.clone(),
        models_line(&s.providers, &p.id),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let edit = {
        let id = p.id.clone();
        ghost(
            if open { "Close" } else { "Edit" },
            format!("edit-{}", p.id),
            k,
            move |s: &mut App| s.providers.toggle(&id),
        )
    };
    let off = {
        let id = p.id.clone();
        ghost(
            "Disconnect",
            format!("disconnect-{}", p.id),
            k,
            move |s: &mut App| s.disconnect_provider(&id),
        )
    };
    let head = k.interactive(
        row()
            .key(format!("prov-{}", p.id))
            .w_full()
            .padding(14.0)
            .gap(12.0)
            .center_y()
            .child(tile(&p.label, p.color, k))
            .child(names(
                &p.label,
                if detail.is_empty() {
                    "Connected".into()
                } else {
                    detail
                },
                s.providers.row_errors.get(&p.id),
                k,
            ))
            .child(edit)
            .child(refresh_button(s, &p.id, k))
            .child(off),
    );
    let mut r = col().w_full().child(head);
    if open {
        r = r.child(server_form(s, &p.id, k));
    }
    r
}

fn add_server_row(s: &App, k: &Kit) -> El {
    let open = s.providers.open.as_deref() == Some(NEW_SERVER);
    let form = open.then(|| server_form(s, NEW_SERVER, k));
    expandable(
        "prov-add".into(),
        tile("Server", crate::providers::SERVER_COLOR, k),
        "Add a server you run yourself",
        "Add",
        |s: &mut App| s.providers.toggle(NEW_SERVER),
        form,
        k,
    )
}
