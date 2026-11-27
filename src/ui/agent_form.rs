//! Making and changing agents: one form for a new agent, an edit, and a new team member, and the
//! delete confirmation. The name is the page; everything else is optional and can change later.

use rune::{Color, El, ScrollSpec, col, el, icon, row, text};
use rune_kit::shell::Toolbar;
use rune_kit::{Button, ButtonVariant, ControlSize, Dialog, Kit, TextArea, TextInput};

use super::icons;
use super::model_picker::ModelPicker;
use crate::agents::{FormKind, HUES, TEMPLATES, hue_of, initial_of};
use crate::app::App;
use crate::avatars::{Avatar, PRESETS};
use crate::picker::Item;
use crate::theme;

const PAGE: f32 = 720.0;

/// A plain text field's text sits this far in from its box; the name pulls back by it so its first
/// letter lines up with the caption above.
const NAME_INSET: f32 = 8.0;

/// The instructions field stops growing here and scrolls: a long brief must not fill the screen.
const PROMPT_MAX: f32 = 320.0;

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
        .gap(16.0)
        .child(hero(s, &kind, member, k).pb(12.0));
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
                .max_height(PROMPT_MAX)
                .width(None)
                .on_edit(|s: &mut App| &mut s.roster.form.prompt)
                .build(k)
                .key("prompt"),
            k,
        ))
        .children(member.then(|| {
            card(
                "When should it be used?",
                &format!(
                    "One line {} reads to decide whether to hand it a task.",
                    parent_name.as_deref().unwrap_or("its lead")
                ),
                TextInput::new(&r.form.description)
                    .placeholder("e.g. Checks a draft's numbers against their sources.")
                    .label("When to use")
                    .width(None)
                    .on_edit(|s: &mut App| &mut s.roster.form.description)
                    .build(k)
                    .key("description"),
                k,
            )
        }))
        .child(inline_card(
            if member { "Which AI should it use?" } else { "Which AI should answer?" },
            &match &parent_name {
                Some(p) if member => format!("Leave it on the default and it uses the same one as {p}."),
                _ => "The model behind this agent. If you are not sure, leave it on the default.".to_owned(),
            },
            model_select(s, member, k, layer),
            k,
        ))
        .child(more_options(s, k, layer));
    if let FormKind::Edit(t) = &kind {
        page = page.child(spend(s, &t.id, k));
    }
    col()
        .key("agent-form")
        .w_full()
        .h_full()
        .min_h(0.0)
        .items_center()
        // A picture dropped anywhere on the form becomes the agent's face.
        .on_drop_files(|s: &mut App, files, _| s.avatar_dropped(files.to_vec()))
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
    let tile = match &f.avatar {
        Some(a) => super::avatar::picture(a, 56.0, 16.0),
        None => row()
            .size(56.0, 56.0)
            .radius(16.0)
            .center()
            .fill(theme::avatar_color(hue))
            .child(
                text(initial_of(&f.name.text))
                    .font(k.ty.body_medium)
                    .color(c.white),
            ),
    };
    // A pencil on the corner says the picture can be changed. It sits inside the box that takes the
    // press, so all of it can be pressed.
    let pencil = row()
        .size(22.0, 22.0)
        .radius(11.0)
        .center()
        .fill(c.bg_inverse)
        .border(2.0, c.page)
        .pin(None, None, Some(0.0), Some(0.0))
        .child(icon(&icons::EDIT, 12.0).color(c.icon_inverse));
    let face = row()
        .key("avatar")
        .size(62.0, 62.0)
        .expanded(f.picker)
        .role("button")
        .label("Change picture")
        .on_press(|s: &mut App| s.roster.form.picker ^= true)
        .child(tile)
        .child(pencil);
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
        .text_style(crate::theme::ty::title(k))
        .width(None)
        .on_edit(|s: &mut App| &mut s.roster.form.name)
        .on_submit(|s: &mut App, _| s.submit_form())
        .build(k)
        .key("name")
        .ml(-NAME_INSET);
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
            face_picker(s, k),
            k,
        ));
    }
    out
}

/// The ways to give an agent a face: a colour behind its initial, one of the pictures that come
/// with the app, or a picture of your own, dropped on the page.
fn face_picker(s: &App, k: &Kit) -> El {
    let f = &s.roster.form;
    let c = &k.c;
    // A heading sits close to its tiles; the two groups stand apart.
    let group = |words: &str| {
        col().w_full().gap(8.0).child(
            text(words.to_owned())
                .font(k.ty.small_medium)
                .color(c.text_muted),
        )
    };
    let mut picture = group("Picture").child(pictures(s, k)).child(hint(
        "Or drop your own picture anywhere on this page. PNG or JPEG: the middle square of it is used.",
        k,
    ));
    if let Some(why) = &f.avatar_error {
        picture = picture.child(
            text(why.clone())
                .font(k.ty.caption())
                .color(c.warning.icon)
                .wrap(),
        );
    }
    col()
        .w_full()
        .gap(20.0)
        .child(group("Colour").child(swatches(s, k)))
        .child(picture)
}

/// The auto colour (from the name) and a few to choose from.
fn swatches(s: &App, k: &Kit) -> El {
    let f = &s.roster.form;
    let tile = |key: String, hue: Option<f32>| {
        // A face is a colour or a picture, so a picture's colour is not shown as chosen.
        let on = f.avatar.is_none() && f.hue == hue;
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
                .on_press(move |s: &mut App| s.roster.pick_colour(hue))
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

/// Tiles in a row this many across; the picture the person gave leads, when there is one.
const PER_ROW: usize = 7;

/// The pictures that come with the app, and the one the person gave, to choose from.
fn pictures(s: &App, k: &Kit) -> El {
    let f = &s.roster.form;
    let tile = |key: String, face: Avatar, label: String| {
        let on = f.avatar.as_ref() == Some(&face);
        let pressed = face.clone();
        // The ring stands off from the picture: they are both light, so touching they would read
        // as one bigger tile.
        k.interactive(
            row()
                .key(key)
                .size(48.0, 48.0)
                .radius(14.0)
                .center()
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
                .label(label)
                .on_press(move |s: &mut App| s.roster.pick_avatar(pressed.clone()))
                .child(super::avatar::picture(&face, 40.0, 10.0)),
        )
    };
    let mut tiles = Vec::new();
    if let Some(own @ Avatar::Upload(_)) = &f.avatar {
        tiles.push(tile(
            "face-upload".into(),
            own.clone(),
            "Your picture".into(),
        ));
    }
    for p in &PRESETS {
        tiles.push(tile(
            format!("face-{}", p.id),
            Avatar::Preset(p.id),
            format!("Use this picture: {}", p.label),
        ));
    }
    let mut grid = col().gap(10.0);
    let mut rest = tiles.into_iter();
    loop {
        let line: Vec<El> = rest.by_ref().take(PER_ROW).collect();
        if line.is_empty() {
            return grid;
        }
        grid = grid.child(row().gap(10.0).children(line));
    }
}

/// Below this width a pair of example tiles moves to its own line: their titles would clip.
const TEMPLATE_PAIR: f32 = 300.0;

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
                .px(12.0)
                .py(10.0)
                .gap(2.0)
                .radius(k.radius.md)
                .fill(c.layer_01)
                .border(1.0, if on { c.border_strong } else { c.border_muted })
                .role("radio")
                .checked(on)
                .on_press(move |s: &mut App| s.roster.pick_template(i))
                .child(text(t.label).font(k.ty.ui_medium).color(c.text_base))
                .child(hint(t.hint, k)),
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
        // Two pairs that wrap: four across when there is room, two by two when there is not, never
        // three and one.
        .child(
            row()
                .w_full()
                .flex_wrap()
                .gap(8.0)
                .children((0..TEMPLATES.len()).step_by(2).map(|i| {
                    row()
                        .grow(1.0)
                        .min_w(TEMPLATE_PAIR)
                        .gap(8.0)
                        .child(one(i))
                        .child(one(i + 1))
                })),
        )
}

/// The muted line under a section's title. It wraps: a long description must stay inside its card.
fn hint(words: impl Into<String>, k: &Kit) -> El {
    text(words)
        .font(k.ty.caption())
        .color(k.c.text_muted)
        .wrap()
}

/// A title and the muted line under it. `field` titles sit a step below a card's.
fn head(title: &str, description: &str, style: rune::TextStyle, k: &Kit) -> El {
    col()
        .min_w(0.0)
        .gap(2.0)
        .child(text(title.to_owned()).font(style).color(k.c.text_base))
        .child(hint(description, k))
}

/// The box every section of the form sits in.
fn panel(k: &Kit) -> El {
    col()
        .w_full()
        .radius(k.radius.lg)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
}

/// A section whose control needs the width: the title and line above it.
fn card(title: &str, description: &str, body: El, k: &Kit) -> El {
    panel(k)
        .padding(20.0)
        .gap(14.0)
        .child(head(title, description, k.ty.body_medium, k))
        .child(body)
}

/// A section that is one choice: the title and line on the left, the control on the right. When
/// the two do not fit side by side the control wraps under the words, as in `card`.
fn inline_card(title: &str, description: &str, control: El, k: &Kit) -> El {
    panel(k).padding(20.0).child(
        row()
            .w_full()
            .flex_wrap()
            .gaps(14.0, 24.0)
            .center_y()
            .child(
                head(title, description, k.ty.body_medium, k)
                    .grow(1.0)
                    .min_w(INLINE_WORDS),
            )
            .child(control),
    )
}

/// The narrowest the words of an `inline_card` get before its control wraps under them.
const INLINE_WORDS: f32 = 260.0;

/// One setting inside More options: its title and what it does, then the control.
fn field(title: &str, about: &str, k: &Kit) -> El {
    col()
        .w_full()
        .gap(8.0)
        .child(head(title, about, k.ty.ui_medium, k))
}

/// The line between two settings in More options.
fn rule(k: &Kit) -> El {
    el().w_full().h(1.0).fill(k.c.border_muted)
}

/// What the agent used over the last 30 days, and a way to see it in full on Usage.
fn spend(s: &App, agent_id: &str, k: &Kit) -> El {
    use crate::usage::{Filters, Range, report, today};
    let filters = Filters {
        agent: Some(agent_id.to_owned()),
        ..Filters::default()
    };
    let r = report(&s.agent_spend, &s.prices, &filters, Range::Month, today());
    let said = if r.totals.calls == 0 {
        "Nothing in the last 30 days.".to_owned()
    } else {
        format!(
            "{} over {} model calls, {} tokens.",
            if r.cost.unknown() {
                "No price known".to_owned()
            } else {
                format!("About {}", super::usage::money(&r.cost))
            },
            r.totals.calls,
            super::usage::tokens(r.totals.tokens())
        )
    };
    let id = agent_id.to_owned();
    card(
        "What has it cost?",
        "Its model calls in the last 30 days, at current prices.",
        row()
            .key("agent-spend")
            .w_full()
            .gap(12.0)
            .center_y()
            .child(
                text(said)
                    .font(k.ty.body)
                    .color(k.c.text_base)
                    .grow(1.0)
                    .wrap(),
            )
            .child(
                Button::new("See usage")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut App| s.usage_of_agent(&id))
                    .build(k)
                    .key("agent-spend-usage"),
            ),
        k,
    )
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
fn more_options(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
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
    let mut out = panel(k).child(head);
    if f.advanced {
        let fields = [
            skills_field(s, k, layer),
            learning_field(s, k),
            tools_field(s, k, layer),
            workflows_field(s, k, layer),
            workdir_field(s, k, layer),
            topics_field(s, k),
            blocked_field(s, k, layer),
            approvals_field(s, k, layer),
        ];
        let mut body = col().w_full().px(20.0).pb(20.0).gap(20.0);
        for (i, f) in fields.into_iter().enumerate() {
            if i > 0 {
                body = body.child(rule(k));
            }
            body = body.child(f);
        }
        out = out.child(rule(k)).child(body.pt(20.0));
    }
    out
}

/// The skills block: a title, what it is for, and the multi-select.
fn skills_field(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let about = if s.skills.is_empty() {
        "No skills found. Put them in ~/.agents/skills, one folder each with a SKILL.md."
    } else {
        "Ready-made know-how you can hand it. Its text goes to the model with the brief."
    };
    field("Skills", about, k)
        .child(super::multi_picker::build(
            s,
            k,
            layer,
            super::multi_picker::Spec {
                key: "skills",
                title: "Skills",
                placeholder: "Add skills",
                noun: ("skill", "skills"),
                none: "No skills found in ~/.agents/skills.",
                items: s
                    .skills
                    .iter()
                    .map(|k| Item::plain(k.name.clone(), k.description.clone()))
                    .collect(),
                chosen: s.roster.form.skills.clone(),
                get: |s| &s.roster.form.skill_picker,
                edit: |s| &mut s.roster.form.skill_picker,
                toggle: |s, name| s.roster.toggle_skill(name),
                single: false,
                on_open: Some(std::rc::Rc::new(App::reload_skills)),
            },
        ))
        .children(cost(s, k))
}

/// Whether TypeSafe judges this agent's runs for a skill: as Settings says, or its own choice.
fn learning_field(s: &App, k: &Kit) -> El {
    let global = if s.score_runs { "on" } else { "off" };
    field(
        "Spot runs worth a skill",
        &format!(
            "When one of its runs ends well, TypeSafe checks whether it taught something no skill has yet, and the chat offers to make one. Settings has this {global} for every agent; choose here to differ."
        ),
        k,
    )
    .child(super::guard::choices(
        "agent-score",
        [
            (None, format!("As Settings ({global})")),
            (Some(true), "On".to_owned()),
            (Some(false), "Off".to_owned()),
        ],
        s.roster.form.score,
        |s, v| s.roster.form.score = v,
        k,
    ))
}

/// What the chosen skills add to every message, in words, once there are any: their descriptions,
/// since their instructions are read only when a task needs them. Amber when large.
fn cost(s: &App, k: &Kit) -> Option<El> {
    let chosen = &s.roster.form.skills;
    let tokens = crate::skills::estimate_tokens(&s.skills, chosen);
    if chosen.is_empty() || tokens == 0 {
        return None;
    }
    let big = tokens >= BIG_SKILLS;
    let mut line = format!(
        "Their descriptions add {} to every message; a skill's instructions are read only when a task needs them.",
        crate::skills::tokens_label(tokens)
    );
    if big {
        line.push_str(" That is a lot: fewer skills will be cheaper and faster.");
    }
    Some(
        text(line)
            .font(k.ty.caption())
            .color(if big {
                k.c.warning.icon
            } else {
                k.c.text_muted
            })
            .wrap(),
    )
}

/// From this many tokens the form warns: dozens of skills, or very long descriptions.
const BIG_SKILLS: usize = 2_000;

/// The working folder block: where this agent's commands run.
fn workdir_field(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    field(
        "Working folder",
        "Where its commands run. If you choose none, that is your home folder.",
        k,
    )
    .child(super::folder_picker::build(s, k, layer))
    .child(trust_field(s, k))
}

/// What the agent answers about on channels (see `guard::intake`).
fn topics_field(s: &App, k: &Kit) -> El {
    field(
        "On channels, only answer about",
        "Words or topics, split by commas. A message from Telegram, Slack or WhatsApp that names none of them is turned away before it reaches the agent. Empty, it answers anything.",
        k,
    )
    .child(
            TextInput::new(&s.roster.form.topics)
                .placeholder("e.g. billing, refunds, orders")
                .label("Channel topics")
                .on_edit(|s: &mut App| &mut s.roster.form.topics)
                .build(k)
                .key("channel-topics"),
        )
}

/// Places this agent may not reach, beside the ones blocked for every agent.
fn blocked_field(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    field(
        "Never touch",
        "Folders and files this agent's tools may not reach, split by commas, on top of the places blocked for every agent on the Guardrails screen. Add (reading) to stop only reading, (changing) to stop only changing.",
        k,
    )
    .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    col().grow(1.0).min_w(0.0).child(
                        TextInput::new(&s.roster.form.blocked)
                            .placeholder("e.g. ~/Documents/taxes, ../other-project")
                            .label("Never touch")
                            .on_edit(|s: &mut App| &mut s.roster.form.blocked)
                            .build(k)
                            .w_full()
                            .key("blocked-places"),
                    ),
                )
                .child(super::folder_picker::add_place(
                    s,
                    super::folder_picker::Target::FormPlaces,
                    k,
                    layer,
                )),
        )
}

/// Whether what the agent reads and runs in its folder counts as yours (see `guard::taint`).
fn trust_field(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let on = s.roster.form.trust_workdir;
    row()
        .key("trust-workdir")
        .pt(8.0)
        .w_full()
        .gap(16.0)
        .center_y()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text("Trust what it reads and runs there")
                        .font(k.ty.ui_medium)
                        .color(c.text_base),
                )
                .child(hint(
                    "Off, a file it read or a command's output counts as outside text, like a web page: after reading one, it asks before it acts. Turn off for a folder of things you did not write, such as downloads or a cloned project.",
                    k,
                )),
        )
        .child(
            rune_kit::Switch::new(on)
                .label("Trust what it reads and runs there")
                .hide_label()
                .on_toggle(move |s: &mut App| s.roster.form.trust_workdir = !on)
                .build(k),
        )
}

/// The approvals block: tools each use of which waits for your go-ahead, and where that is asked.
fn approvals_field(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    use crate::guard::SendTo;
    let a = &s.roster.form.approvals;
    let on = a.on;
    let switch = row()
        .key("approvals-on")
        .w_full()
        .gap(16.0)
        .center_y()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text("Ask before it uses chosen tools")
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base),
                )
                .child(hint(
                    "Each use of a tool chosen below waits for your go-ahead, on a card here and where approvals go. The guard's own rules still apply.",
                    k,
                )),
        )
        .child(
            rune_kit::Switch::new(on)
                .label("Ask before it uses chosen tools")
                .hide_label()
                .on_toggle(move |s: &mut App| s.roster.form.approvals.on = !on)
                .build(k),
        );
    let mut out = col().w_full().gap(12.0).child(switch);
    if !on {
        return out;
    }
    let where_ = match (a.to, a.to.kind()) {
        (_, Some(kind)) => s.approver_on(kind).map(|(kind, _)| kind),
        (SendTo::Desktop, None) => Err(Some("Only the card here.".to_owned())),
        _ => s.approver().map(|(kind, _)| kind).map_err(|why| {
            why.or_else(|| Some("Channels sends approvals nowhere: only the card here.".into()))
        }),
    };
    let said = match where_ {
        Ok(kind) => format!("Cards also go to you on {}.", kind.title()),
        Err(why) => why.unwrap_or_default(),
    };
    let labels: Vec<&str> = SendTo::ALL.iter().map(|t| t.label()).collect();
    let selected = SendTo::ALL.iter().position(|t| *t == a.to);
    out = out.child(
        col()
            .gap(6.0)
            .child(
                rune_kit::SegmentedControl::new(&labels, selected)
                    .label("Ask where")
                    .width(None)
                    .on_pick(|s: &mut App, i, _| {
                        s.roster.form.approvals.to = SendTo::ALL[i.min(SendTo::ALL.len() - 1)];
                    })
                    .build(k)
                    .key("approvals-to"),
            )
            .child(hint(&said, k)),
    );
    let items = s
        .all_tools()
        .iter()
        .filter(|t| t.name != crate::tools::ASK)
        .filter(|t| crate::tools::is_builtin(&t.name) || s.roster.form.tools.contains(&t.name))
        .map(|t| {
            let shown = crate::tools::shown(&t.name, &s.mcp.servers);
            Item {
                name: t.name.clone(),
                label: shown.label,
                description: shown.about,
                group: shown.group,
            }
        })
        .collect();
    out.child(super::multi_picker::build(
        s,
        k,
        layer,
        super::multi_picker::Spec {
            key: "approval-tools",
            title: "Tools that ask",
            placeholder: "Choose tools",
            noun: ("tool", "tools"),
            none: "No tools.",
            items,
            chosen: a.tools.clone(),
            get: |s| &s.roster.form.approval_picker,
            edit: |s| &mut s.roster.form.approval_picker,
            toggle: |s, name| s.roster.toggle_approval_tool(name),
            single: false,
            on_open: None,
        },
    ))
}

/// The tools block: the TypeSafe and MCP tools the agent may use. The app's own tools are every
/// agent's, so they are neither listed nor chosen here.
fn tools_field(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    field(
        "What it can use (Tools)",
        "Every agent can ask you, run commands, work with files in its folder, fetch web pages, and save and run workflows. Add tools from MCP servers, TypeSafe or your browser here, and the ones that manage schedules, agents and MCP servers.",
        k,
    )
    .child(super::multi_picker::build(
            s,
            k,
            layer,
            super::multi_picker::Spec {
                key: "tools",
                title: "Tools",
                placeholder: "Add tools",
                noun: ("tool", "tools"),
                none: "No other tools yet: add an MCP server or connect TypeSafe.",
                items: s
                    .all_tools()
                    .iter()
                    .filter(|t| !crate::tools::is_builtin(&t.name))
                    .filter(|t| s.typesafe_connected() || !crate::tools::needs_typesafe(&t.name))
                    .map(|t| {
                        let shown = crate::tools::shown(&t.name, &s.mcp.servers);
                        Item {
                            name: t.name.clone(),
                            label: shown.label,
                            description: shown.about,
                            group: shown.group,
                        }
                    })
                    .collect(),
                chosen: s.roster.form.tools.clone(),
                get: |s| &s.roster.form.tool_picker,
                edit: |s| &mut s.roster.form.tool_picker,
                toggle: |s, name| s.roster.toggle_tool(name),
                single: false,
                on_open: None,
            },
        ))
}

/// The workflows block: other agents' workflows this one may run too. Its own are always its,
/// so they are not listed.
fn workflows_field(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let own = match &s.roster.editing {
        Some(crate::agents::FormKind::Edit(t)) => Some(t.id.as_str()),
        _ => None,
    };
    let items: Vec<Item> = s
        .workflows
        .iter()
        .filter(|w| Some(w.agent_id.as_str()) != own)
        .map(|w| Item {
            name: w.id.clone(),
            label: w.name.clone(),
            description: w.goal.clone(),
            group: s
                .roster
                .agent_ref(&w.agent_id)
                .map_or_else(String::new, |a| a.name.clone()),
        })
        .collect();
    // A workflow deleted since it was added is no longer offered, nor shown as chosen.
    let chosen: Vec<String> = s
        .roster
        .form
        .workflows
        .iter()
        .filter(|id| items.iter().any(|i| &i.name == *id))
        .cloned()
        .collect();
    field(
        "Workflows",
        "Its own workflows are always its. Add other agents' workflows here, so it knows them and can run them, with its own model, tools and folder.",
        k,
    )
    .child(super::multi_picker::build(
        s,
        k,
        layer,
        super::multi_picker::Spec {
            key: "workflows",
            title: "Workflows",
            placeholder: "Add workflows",
            noun: ("workflow", "workflows"),
            none: "No other agent has a workflow yet.",
            items,
            chosen,
            get: |s| &s.roster.form.workflow_picker,
            edit: |s| &mut s.roster.form.workflow_picker,
            toggle: |s, id| s.roster.toggle_workflow(id),
            single: false,
            on_open: None,
        },
    ))
}

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
