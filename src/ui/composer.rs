//! Studio's message box: a bare auto-growing field in a rounded container, with think effort, the
//! model and the agent's working folder on the left, the cost estimate, voice and send (or stop,
//! while the agent works) on the right. Above the field: what is attached, the message waiting to go, and why
//! something cannot be sent. Typing `/` lists commands and `@` the working folder's files; files
//! dropped on the chat or pasted are attached. `crate::composer` holds what it does.

use std::path::Path;

use rune::{
    Align, El, Side, SurfaceFit, TextStyle, auto_grow, col, el, icon, row, text, text_input,
};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Placement, PopupItem,
    PopupList, glyphs,
};

use super::folder_picker::{self, Target};
use super::icons;
use super::model_picker::ModelPicker;
use crate::agents::Agent;
use crate::app::{App, Effort};
use crate::composer::{Listing, Suggestion, Suggestions, Trigger};
use crate::models::can_think;
use crate::prompt::Attachment;
use crate::providers::ModelStatus;

/// The side of a picture's thumbnail in the box.
const THUMB: f32 = 64.0;

const IDLE: &str = "Write a message. / for commands, @ for files.";
const BUSY: &str = "Write your next message; it goes when the reply ends.";
const ASKED: &str = "Answer or decide on the request above first.";

pub fn build(s: &App, agent: &Agent, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let running = s.busy();
    let asked = s.awaiting_answer();
    let status = s.providers.status(agent.model.as_ref());
    let ready = matches!(status, ModelStatus::Ready(_));
    let has_content = !s.composer.is_blank();
    // Unknown abilities stay enabled: only a model known not to reason greys the button out.
    let thinks = match &status {
        ModelStatus::Ready(m) => s.providers.model_info(m).is_none_or(|i| can_think(&i.caps)),
        _ => true,
    };
    let blind = s.composer.attachments.iter().any(Attachment::is_image) && !s.sees_images();

    let style = TextStyle {
        line_height: 22.0,
        ..k.ty.body
    };
    let field = text_input(&s.composer.edit, style)
        .key("composer-input")
        .w_full()
        .color(c.text_base)
        .placeholder(
            if asked {
                ASKED
            } else if running {
                BUSY
            } else {
                IDLE
            },
            c.text_faint,
        )
        .caret(c.text_base, k.selection())
        .label("Message")
        .on_edit(|s: &mut App| &mut s.composer.edit)
        .on_input_key(|s: &mut App, key, _| s.composer_key(key))
        .on_input_paste(|s: &mut App, content, _| {
            if let Some(image) = content.image() {
                s.attach_png(image.png().to_vec());
                true
            } else if !content.files().is_empty() {
                s.attach_paths(content.files().to_vec());
                true
            } else {
                false
            }
        })
        .on_submit(|s: &mut App, _| s.send());
    let input = col()
        .w_full()
        .px(16.0)
        .pt(12.0)
        .pb(6.0)
        .child(auto_grow(field, 1, 8));

    let think = Button::new(s.effort.label())
        .variant(if s.effort == Effort::Off {
            ButtonVariant::Ghost
        } else {
            ButtonVariant::Contrast
        })
        .control_size(ControlSize::Small)
        .help(if thinks {
            "How hard the agent thinks before answering. Click to cycle, or type /think."
        } else {
            "This model has no thinking setting."
        })
        .disabled(running || !thinks)
        .on_press(|s: &mut App| s.effort = s.effort.next())
        .build(k)
        .key("think");
    let voice = IconButton::new(&glyphs::MIC)
        .variant(IconButtonVariant::GhostMuted)
        .help("Hold to talk")
        .label("Hold to talk")
        .build(k)
        .key("voice");
    let can_send = has_content && ready && !asked && !blind;
    let mut send = IconButton::new(&glyphs::ARROW_RIGHT)
        .variant(if can_send {
            IconButtonVariant::Submit
        } else {
            IconButtonVariant::Ghost
        })
        .label(if running {
            "Send when the reply ends"
        } else {
            "Send"
        })
        .disabled(!can_send)
        .on_press(|s: &mut App| s.send());
    if let Some(why) = reason(&status) {
        send = send.help(why);
    } else if running {
        send = send.help("Send when the reply ends");
    }
    let stop = IconButton::new(&icons::STOP)
        .variant(IconButtonVariant::Contrast)
        .label("Stop")
        .help("Stop the reply (Esc)")
        .on_press(|s: &mut App| s.stop())
        .build(k)
        .key("stop");

    let mut card = col()
        .key("composer")
        .w_full()
        .radius(24.0)
        .fill(c.layer_01)
        .border(
            1.0,
            if s.composer.edit.focused {
                c.border_strong
            } else {
                c.border_base
            },
        );
    let notices = [
        reason(&status),
        blind.then(|| {
            "This model cannot look at pictures. Remove them, or choose a model that can."
                .to_owned()
        }),
        s.composer.notice.clone(),
    ];
    for (i, why) in notices.into_iter().flatten().enumerate() {
        card = card.child(
            row()
                .key(format!("composer-notice-{i}"))
                .w_full()
                .px(16.0)
                .pt(10.0)
                .child(text(why).font(k.ty.small).color(k.c.warning.icon).wrap()),
        );
    }
    if let Some(waiting) = s.queued_here() {
        card = card.child(queued(waiting, k));
    }
    if !s.composer.attachments.is_empty() {
        card = card.child(
            row()
                .key("attachments")
                .w_full()
                .px(16.0)
                .pt(10.0)
                .gap(6.0)
                .center_y()
                .wrap()
                .children(s.composer.attachments.iter().map(|a| chip(a, k))),
        );
    }
    let mut actions = row()
        .w_full()
        .px(10.0)
        .pb(8.0)
        .gap(4.0)
        .center_y()
        .child(think)
        .child(model_pill(s, agent, &status, k, layer))
        .child(folder_pill(s, agent, k, layer))
        .child(el().grow(1.0));
    if has_content {
        let tokens = estimate(s);
        actions = actions.child(
            text(tokens)
                .key("composer-tokens")
                .font(k.ty.small)
                .color(c.text_faint)
                .help("A rough count of what this message costs, files and pictures included"),
        );
    }
    actions = actions.child(voice);
    if running {
        if has_content {
            actions = actions.child(send.build(k).key("send"));
        }
        actions = actions.child(stop);
    } else {
        actions = actions.child(send.build(k).key("send"));
    }

    if let Some(list) = s.suggestions() {
        popup(s, list, k, layer);
    }
    col()
        .w_full()
        .gap(6.0)
        .child(card.child(input).child(actions))
        .child(hints(running, k))
}

/// What the keys do, under the box.
fn hints(running: bool, k: &Kit) -> El {
    let line = if running {
        "Esc stop  ·  ↵ queue  ·  ⇧↵ new line  ·  / commands  ·  @ files"
    } else {
        "↵ send  ·  ⇧↵ new line  ·  ↑ last message  ·  / commands  ·  @ files"
    };
    row()
        .key("composer-hints")
        .w_full()
        .px(16.0)
        .child(text(line).font(k.ty.small).color(k.c.text_faint).truncate())
}

/// About how many tokens the draft costs, as a short label.
fn estimate(s: &App) -> String {
    let words = crate::prompt::Prompt {
        text: s.composer.edit.text.clone(),
        attachments: s.composer.attachments.clone(),
    };
    match words.estimated_tokens() {
        1 => "≈1 token".to_owned(),
        n if n < 1000 => format!("≈{n} tokens"),
        n => format!("≈{:.1}k tokens", n as f64 / 1000.0),
    }
}

/// The message waiting for the reply to end, with a way to take it back.
fn queued(waiting: &crate::prompt::Prompt, k: &Kit) -> El {
    let first = waiting.text.lines().next().unwrap_or_default();
    let summary = match waiting.attachments.len() {
        0 => first.to_owned(),
        1 => format!("{first} (+1 file)"),
        n => format!("{first} (+{n} files)"),
    };
    row()
        .key("composer-queued")
        .w_full()
        .px(16.0)
        .pt(10.0)
        .gap(8.0)
        .center_y()
        .child(
            text("Waiting to send:")
                .font(k.ty.small)
                .color(k.c.text_muted),
        )
        .child(
            text(summary)
                .font(k.ty.small)
                .color(k.c.text_base)
                .grow(1.0)
                .shrink()
                .truncate(),
        )
        .child(
            Button::new("Edit")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .help("Take it back into the box, to change it or not send it")
                .on_press(|s: &mut App| s.unqueue())
                .build(k)
                .key("unqueue"),
        )
}

/// One attachment, with a button that removes it: a picture as a thumbnail, a text file as its
/// name and length.
fn chip(a: &Attachment, k: &Kit) -> El {
    let name = a.name().to_owned();
    let remove = IconButton::new(&glyphs::CLOSE)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label(format!("Remove {name}"))
        .on_press(move |s: &mut App| s.detach(&name))
        .build(k);
    if let Attachment::Image(img) = a {
        let mut tile = el()
            .key(format!("attachment-{}", img.name))
            .size(THUMB, THUMB)
            .radius(12.0)
            .fill(k.c.layer_02)
            .border(1.0, k.c.border_base)
            .help(format!("{} · {}", img.name, a.detail()));
        if let Some(px) = super::picture::pixels(img) {
            tile = tile.pixels_fit(&px, SurfaceFit::Fill);
        }
        // The remove button sits on the picture's corner, on a disc so it shows on any picture.
        return tile.child(
            el().pin(None, Some(2.0), Some(2.0), None)
                .radius(12.0)
                .fill(k.c.layer_01)
                .child(remove),
        );
    }
    row()
        .key(format!("attachment-{}", a.name()))
        .h(28.0)
        .pl(10.0)
        .pr(2.0)
        .gap(6.0)
        .center_y()
        .radius(14.0)
        .fill(k.c.layer_02)
        .child(icon(&glyphs::FILE_TEXT, 14.0).color(k.c.text_muted))
        .child(
            text(a.name())
                .font(k.ty.small)
                .color(k.c.text_base)
                .max_w(220.0)
                .truncate(),
        )
        .child(text(a.detail()).font(k.ty.small).color(k.c.text_faint))
        .child(remove)
}

/// The `/` or `@` list, above the box.
fn popup(s: &App, list: Suggestions, k: &Kit, layer: &mut Vec<El>) {
    let active = s.composer.active(list.rows.len());
    let empty = match (&list.trigger, list.files) {
        (Trigger::File { .. }, Listing::NoFolder) => {
            "Choose a working folder for this agent (the folder button below) to mention its files"
        }
        (Trigger::File { .. }, Listing::Looking) => "Looking through the working folder…",
        (Trigger::File { .. }, Listing::Ready) => "No files match",
        (Trigger::Command(_), _) => "No commands match",
    };
    let items = list.rows.iter().map(|r| match r {
        Suggestion::Command(c) => PopupItem::new(r.id(), c.label())
            .description(c.description())
            .leading(icon(&glyphs::SPARKLE, 12.0).color(k.c.text_faint)),
        Suggestion::File(path) => {
            let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
            PopupItem::new(r.id(), name)
                .description(folder)
                .leading(icon(&glyphs::FILE, 12.0).color(k.c.text_faint))
        }
    });
    PopupList::new("composer-suggest", "composer")
        .anchored("composer", Side::Top, Align::Start, 6.0)
        .width(420.0)
        .max_height(280.0)
        .items(items)
        .active(Some(active))
        .empty_message(empty)
        .label(match list.trigger {
            Trigger::Command(_) => "Commands",
            Trigger::File { .. } => "Files",
        })
        .on_choose(|s: &mut App, id, _| s.choose_suggestion(id))
        .on_close(|s: &mut App| s.composer.dismiss())
        .build(k, layer);
}

/// Why this agent cannot answer, in words, when it cannot.
fn reason(status: &ModelStatus) -> Option<String> {
    match status {
        ModelStatus::Ready(_) => None,
        ModelStatus::Choose => Some("Choose a model to start chatting.".to_owned()),
        ModelStatus::Unavailable { why, .. } => {
            Some(format!("{why} Choose another model, or connect it again."))
        }
    }
}

/// The agent's model: a quiet pill that opens the grouped list. A model that cannot answer is kept
/// and shown in amber, not swapped for another.
fn model_pill(s: &App, agent: &Agent, status: &ModelStatus, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let (name, ink) = match status {
        ModelStatus::Ready(m) => (s.providers.model_name(m), c.text_muted),
        ModelStatus::Choose => ("Choose a model".to_owned(), k.c.warning.icon),
        ModelStatus::Unavailable { model, .. } => (s.providers.model_name(model), k.c.warning.icon),
    };
    let pill = k.interactive(
        row()
            .key("model-pill")
            .h(28.0)
            .px(10.0)
            .gap(4.0)
            .center_y()
            .radius(14.0)
            .role("button")
            .label(format!("Model: {name}"))
            .child(text(name).font(k.ty.small).color(ink))
            .child(icon(&glyphs::CHEVRON_DOWN, 10.0).color(c.text_faint)),
    );
    let current = agent.model.clone();
    let current_name = current.as_ref().map(|m| s.providers.model_name(m));
    ModelPicker::new("composer-model", s.providers.groups())
        .current(current, current_name)
        .label("Model")
        .trigger(pill)
        .on_open(|s: &mut App| s.models_opened())
        .on_connect(|s: &mut App| {
            s.model_picker.close();
            s.route = crate::app::Route::Providers;
        })
        .build(
            k,
            layer,
            &s.model_picker,
            |s: &mut App| &mut s.model_picker,
            |s: &mut App, v| {
                // The picker is not clearable, so a choice always carries a model.
                if v.is_some() {
                    s.set_model(v);
                }
            },
        )
}

/// The agent's working folder: a quiet pill showing the folder's name that opens a folder browser.
/// The choice is saved to the agent at once, and `@` lists that folder's files.
fn folder_pill(s: &App, agent: &Agent, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let (name, ink, help) = match agent.workdir.as_deref() {
        Some(dir) => (
            Path::new(dir)
                .file_name()
                .map_or_else(|| dir.to_owned(), |n| n.to_string_lossy().into_owned()),
            c.text_muted,
            format!("Working folder: {dir}"),
        ),
        None => (
            "No folder".to_owned(),
            c.text_faint,
            "No working folder: commands run in the home folder, and @ lists no files. Click to \
             choose one."
                .to_owned(),
        ),
    };
    let pill = k
        .interactive(
            row()
                .key("folder-pill")
                .h(28.0)
                .px(10.0)
                .gap(6.0)
                .center_y()
                .radius(14.0)
                .role("button")
                .label(format!("Working folder: {name}"))
                .help(help)
                .child(icon(&glyphs::FOLDER, 12.0).color(ink))
                .child(
                    text(name)
                        .font(k.ty.small)
                        .color(ink)
                        .max_w(180.0)
                        .truncate(),
                )
                .child(icon(&glyphs::CHEVRON_DOWN, 10.0).color(c.text_faint)),
        )
        .on_press(|s: &mut App| Target::Agent.toggle(s));
    folder_picker::popover(s, Target::Agent, pill, Placement::TopStart, k, layer)
}
