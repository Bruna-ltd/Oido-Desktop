//! Left sidebar: the agents, pinned first, each with its team of members beneath it. Two levels
//! at most. A row's actions live in a menu you reach three ways: its ⋯ button, a right-click, or
//! the same commands on the edit screen, so none is hidden behind a gesture.

use rune::{El, PointerPress, el, row, text};
use rune_kit::shell::{Sidebar, SidebarRow, SidebarSection};
use rune_kit::{IconButton, IconButtonVariant, Kit, Menu, MenuItem, Spinner, context_menu, glyphs};

use super::icons;
use crate::agents::{Agent, Target};
use crate::app::{App, Route};
use crate::model::AgentStatus;
use crate::theme;
use crate::update::VERSION;

pub fn build(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let r = &s.roster;
    // The list's items are its sections: a clear break above each header, so Pinned and Agents
    // read as two groups, while rows inside one sit 2 points apart (the kit's).
    let mut sb = Sidebar::new()
        .label("Agents")
        .inset(0.0)
        .row_gap(SECTION_GAP);
    if !r.pinned().is_empty() {
        let rows = group(s, r.pinned(), k, layer);
        sb = sb.section(
            SidebarSection::new("Pinned")
                .key("pinned")
                .collapsed(s.pinned_closed)
                .on_toggle(|s: &mut App, _| s.toggle_pinned_section())
                .rows(rows),
        );
    }
    let rows = group(s, r.unpinned(), k, layer);
    let new_agent = IconButton::new(&icons::PLUS)
        .variant(IconButtonVariant::GhostMuted)
        .label("New agent")
        .help("New agent")
        .on_press(|s: &mut App| s.new_agent())
        .build(k)
        .key("new-agent");
    sb = sb.section(
        SidebarSection::new("Agents")
            .key("agents")
            .collapsed(s.others_closed)
            .on_toggle(|s: &mut App, _| s.toggle_others_section())
            .action(new_agent)
            .rows(rows),
    );
    // A right-click opens the same menu at the pointer.
    if let Some((m, (x, y), agent)) = r
        .menu
        .as_ref()
        .and_then(|m| Some((m, m.at?, r.find(&m.target)?)))
    {
        let menu = context_menu(
            "row-context",
            el().size(0.0, 0.0),
            Some(PointerPress { x, y }),
        );
        entries(menu, &m.target, agent).build(k, layer);
    }
    // Indented to the rows' own padding, so it lines up with their avatars.
    sb.footer(
        row().key("sidebar-version").pl(8.0).child(
            text(format!("Oido {VERSION}"))
                .font(k.ty.small)
                .color(k.c.text_faint),
        ),
    )
    .build(k)
}

/// Points between sections.
const SECTION_GAP: f32 = 14.0;

/// Each agent's row, then its team (when unfolded). Members hang close under their lead, and an
/// unfolded team ends with a little air, so a family reads as one unit before the next agent.
fn group(s: &App, agents: Vec<&Agent>, k: &Kit, layer: &mut Vec<El>) -> Vec<El> {
    let mut rows = Vec::new();
    let count = agents.len();
    for (i, a) in agents.into_iter().enumerate() {
        rows.push(agent_row(s, a, None, k, layer));
        if s.roster.team_open(&a.id) && !a.subagents.is_empty() {
            for m in &a.subagents {
                rows.push(agent_row(s, m, Some(&a.id), k, layer));
            }
            if i + 1 < count {
                rows.push(
                    el().key(format!("after-team-{}", a.id))
                        .h(4.0)
                        .a11y_hidden(),
                );
            }
        }
    }
    rows
}

fn agent_row(s: &App, a: &Agent, parent: Option<&str>, k: &Kit, layer: &mut Vec<El>) -> El {
    let target = match parent {
        Some(p) => Target::member(p, a.id.clone()),
        None => Target::agent(a.id.clone()),
    };
    let member = parent.is_some();
    let selected = s.route == Route::Chat && s.roster.selected.as_ref() == Some(&target);
    // The status is drawn in the trailing slot (see `controls`), so the ⋯ button can take its
    // place under the pointer; the row still says it in words.
    let status = row_status(s, a);
    let said = match status {
        Some(AgentStatus::Running) => "Working",
        Some(AgentStatus::Waiting) => "Waiting on you",
        _ => "",
    };
    let t = target.clone();
    SidebarRow::new(a.name.clone())
        .height(if member { 28.0 } else { 36.0 })
        .avatar(avatar(a, k, if member { 14.0 } else { 18.0 }))
        .depth(u8::from(member))
        .selected(selected)
        .trailing(controls(s, a, &target, member, status, k, layer))
        .on_select({
            let t = target.clone();
            move |s: &mut App, _| s.select_agent(t.clone())
        })
        .build(k)
        .description(said)
        .key(format!("agent-{}", a.id))
        .on_secondary_press(move |s: &mut App, p, _| {
            s.roster.open_menu(t.clone(), Some((p.x, p.y)))
        })
}

/// What the row's status slot shows: working (its team counts, and so does a process it started
/// that is still going between turns) or waiting on you. `None` when there is nothing to say.
fn row_status(s: &App, a: &Agent) -> Option<AgentStatus> {
    match a.family_status() {
        AgentStatus::Idle if s.background_running(a) > 0 => Some(AgentStatus::Running),
        AgentStatus::Idle => None,
        st => Some(st),
    }
}

/// The row's trailing column, one quiet mark per slot at rest and its control under the pointer:
/// the team's size turns into the fold button, and the status (spinner or dot) into the ⋯ menu.
/// Both slots have a fixed width, so marks line up down the list whatever each row carries.
fn controls(
    s: &App,
    a: &Agent,
    target: &Target,
    member: bool,
    status: Option<AgentStatus>,
    k: &Kit,
    layer: &mut Vec<El>,
) -> El {
    let c = &k.c;
    // A mark at rest that gives way to a control under the pointer, in one 24 point slot.
    let slot = |key: String, mark: Option<El>, control: El, pinned: bool| {
        let mut sl = row().key(key).size(SLOT, SLOT).center();
        if let Some(m) = mark {
            sl = sl.child(
                row()
                    .size(SLOT, SLOT)
                    .center()
                    .a11y_hidden()
                    .child(m)
                    .group_opacity(if pinned { 0.0 } else { 1.0 }, 0.0),
            );
        }
        sl.child(
            row()
                .absolute(0.0, 0.0)
                .child(control)
                .group_opacity(if pinned { 1.0 } else { 0.0 }, 1.0),
        )
    };
    let mut r = row().gap(2.0).center_y();
    if !member && !a.subagents.is_empty() {
        let open = s.roster.team_open(&a.id);
        let id = a.id.clone();
        let fold = IconButton::new(if open {
            &glyphs::CHEVRON_DOWN
        } else {
            &glyphs::CARET_RIGHT
        })
        .variant(IconButtonVariant::GhostMuted)
        .label(if open {
            "Hide team members"
        } else {
            "Show team members"
        })
        .on_press(move |s: &mut App| s.toggle_team(&id))
        .build(k);
        let count = text(a.subagents.len().to_string())
            .font(k.ty.small)
            .color(c.text_faint);
        r = r.child(slot(format!("team-{}", a.id), Some(count), fold, false));
    }
    let mark = status.map(|st| match st {
        AgentStatus::Running => Spinner::new().size(12.0).color(c.icon_muted).build(k),
        _ => el().size(7.0, 7.0).radius(3.5).fill(c.warning.icon),
    });
    let here = s
        .roster
        .menu_open_for(target)
        .is_some_and(|m| m.at.is_none());
    let t = target.clone();
    let more = IconButton::new(&icons::MORE)
        .variant(IconButtonVariant::GhostMuted)
        .label(if member {
            "Team member actions"
        } else {
            "Agent actions"
        })
        .on_press(move |s: &mut App| s.roster.open_menu(t.clone(), None))
        .build(k)
        .key(format!("more-{}", a.id));
    let menu = entries(
        Menu::new(format!("menu-{}", a.id)).trigger(more).open(here),
        target,
        a,
    );
    // While its menu is open the ⋯ stays put, so the menu hangs from something visible.
    r.child(slot(
        format!("actions-{}", a.id),
        mark,
        menu.build(k, layer),
        here,
    ))
}

/// One trailing slot: the size of a regular icon button.
const SLOT: f32 = 24.0;

/// The row's commands. Unavailable ones are left out, not dimmed; the destructive one is last.
fn entries(menu: Menu, target: &Target, a: &Agent) -> Menu {
    let member = target.is_member();
    let close = |s: &mut App| s.roster.menu = None;
    let mut menu = menu.on_close(close);
    if !member {
        let id = target.id.clone();
        menu = menu.item(
            MenuItem::new("Add team member").on_select(move |s: &mut App| {
                s.roster.menu = None;
                s.new_member(&id);
            }),
        );
    }
    let t = target.clone();
    menu = menu.item(
        MenuItem::new(if member {
            "Edit team member"
        } else {
            "Edit agent"
        })
        .on_select(move |s: &mut App| {
            s.roster.menu = None;
            s.edit_agent(&t);
        }),
    );
    if !member {
        let id = target.id.clone();
        menu = menu.item(
            MenuItem::new(if a.pinned {
                "Unpin from top"
            } else {
                "Pin to top"
            })
            .on_select(move |s: &mut App| {
                s.roster.menu = None;
                s.toggle_pin(&id);
            }),
        );
    }
    let t = target.clone();
    menu.separator().item(
        MenuItem::new(if member {
            "Remove team member"
        } else {
            "Delete agent"
        })
        .danger(true)
        .on_select(move |s: &mut App| {
            s.roster.menu = None;
            s.roster.confirm = Some(t.clone());
        }),
    )
}

pub(super) fn avatar(a: &Agent, k: &Kit, size: f32) -> El {
    if let Some(face) = &a.avatar {
        return super::avatar::picture(face, size, size / 4.0);
    }
    row()
        .size(size, size)
        .radius(size / 4.0)
        .center()
        .fill(theme::avatar_color(a.hue()))
        .child(text(a.initial()).font(k.ty.small_medium).color(k.c.white))
}
