//! Making and changing agents: one form for a new agent, an edit, and a new team member, and the
//! delete confirmation. The name is the page; everything else is optional and can change later.

use rune::{Color, El, ScrollSpec, TextStyle, col, icon, row, text};
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
                    .on_edit(|s: &mut App| &mut s.roster.form.description)
                    .build(k)
                    .key("description"),
                k,
            )
        }))
        .child(card(
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
    let heading = |words: &str| {
        text(words.to_owned())
            .font(k.ty.small_medium)
            .color(c.text_muted)
    };
    let mut col_ = col()
        .w_full()
        .gap(10.0)
        .child(heading("Colour"))
        .child(swatches(s, k))
        .child(heading("Picture"))
        .child(pictures(s, k))
        .child(hint(
            "Or drop your own picture anywhere on this page. PNG or JPEG: the middle square of it is used.",
            k,
        ));
    if let Some(why) = &f.avatar_error {
        col_ = col_.child(
            text(why.clone())
                .font(k.ty.small)
                .color(c.warning.icon)
                .wrap(),
        );
    }
    col_
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
        .child(row().w_full().gap(8.0).child(one(0)).child(one(1)))
        .child(row().w_full().gap(8.0).child(one(2)).child(one(3)))
}

/// The muted line under a section's title. It wraps: a long description must stay inside its card.
fn hint(words: impl Into<String>, k: &Kit) -> El {
    text(words).font(k.ty.small).color(k.c.text_muted).wrap()
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
                .child(hint(description, k)),
        )
        .child(body)
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
                .child(skills_field(s, k, layer))
                .child(tools_field(s, k, layer))
                .child(workdir_field(s, k, layer))
                .child(topics_field(s, k)),
        );
    }
    out
}

/// The skills block: a title, what it is for, and the multi-select.
fn skills_field(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let about = if s.skills.is_empty() {
        "No skills found. Put them in ~/.agents/skills, one folder each with a SKILL.md."
    } else {
        "Ready-made know-how you can hand it. Its text goes to the model with the brief."
    };
    col()
        .w_full()
        .gap(8.0)
        .child(text("Skills").font(k.ty.body_medium).color(c.text_base))
        .child(hint(about, k))
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
            },
        ))
        .children(cost(s, k))
}

/// What the chosen skills add to every message, in words, once there are any. Amber when large.
fn cost(s: &App, k: &Kit) -> Option<El> {
    let chosen = &s.roster.form.skills;
    let tokens = crate::skills::estimate_tokens(&s.skills, chosen);
    if chosen.is_empty() || tokens == 0 {
        return None;
    }
    let big = tokens >= BIG_SKILLS;
    let mut line = format!(
        "These add {} to every message.",
        crate::skills::tokens_label(tokens)
    );
    if big {
        line.push_str(" That is a lot: fewer skills will be cheaper and faster.");
    }
    Some(
        text(line)
            .font(k.ty.small)
            .color(if big {
                k.c.warning.icon
            } else {
                k.c.text_muted
            })
            .wrap(),
    )
}

/// From this many tokens the form warns.
const BIG_SKILLS: usize = 8_000;

/// The working folder block: where this agent's commands run.
fn workdir_field(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    col()
        .w_full()
        .gap(8.0)
        .child(
            text("Working folder")
                .font(k.ty.body_medium)
                .color(c.text_base),
        )
        .child(hint(
            "Where its commands run. If you choose none, that is your home folder.",
            k,
        ))
        .child(super::folder_picker::build(s, k, layer))
        .child(trust_field(s, k))
}

/// What the agent answers about on channels (see `guard::intake`).
fn topics_field(s: &App, k: &Kit) -> El {
    let c = &k.c;
    col()
        .w_full()
        .gap(8.0)
        .child(
            text("On channels, only answer about")
                .font(k.ty.body_medium)
                .color(c.text_base),
        )
        .child(hint(
            "Words or topics, split by commas. A message from Telegram, Slack or WhatsApp that names none of them is turned away before it reaches the agent. Empty, it answers anything.",
            k,
        ))
        .child(
            TextInput::new(&s.roster.form.topics)
                .placeholder("e.g. billing, refunds, orders")
                .label("Channel topics")
                .on_edit(|s: &mut App| &mut s.roster.form.topics)
                .build(k)
                .key("channel-topics"),
        )
}

/// Whether what the agent reads and runs in its folder counts as yours (see `guard::taint`).
fn trust_field(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let on = s.roster.form.trust_workdir;
    row()
        .key("trust-workdir")
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

/// The tools block: the TypeSafe and MCP tools the agent may use. The app's own tools are every
/// agent's, so they are neither listed nor chosen here.
fn tools_field(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    col()
        .w_full()
        .gap(8.0)
        .child(
            text("What it can use (Tools)")
                .font(k.ty.body_medium)
                .color(c.text_base),
        )
        .child(hint(
            "Every agent can ask you, run commands, work with files in its folder, fetch web pages, and manage workflows, schedules, agents and MCP servers. Add tools from MCP servers or TypeSafe here.",
            k,
        ))
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
