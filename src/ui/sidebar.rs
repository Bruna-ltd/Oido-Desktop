//! Left sidebar: the agents, pinned first, each with its team of members beneath it. Two levels
//! at most. A row's actions live in a menu you reach three ways: its ⋯ button, a right-click, or
//! the same commands on the edit screen, so none is hidden behind a gesture.

use rune::{El, PointerPress, el, row, text};
use rune_kit::shell::{RowStatus, Sidebar, SidebarRow, SidebarSection};
use rune_kit::{IconButton, IconButtonVariant, Kit, Menu, MenuItem, context_menu, glyphs};

use super::icons;
use crate::agents::{Agent, Target};
use crate::app::{App, Route};
use crate::model::AgentStatus;
use crate::theme;

pub fn build(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let r = &s.roster;
    let mut sb = Sidebar::new()
        .label("Agents")
        .inset(0.0)
        .header_gap(4.0)
        .header_padding(0.0, 16.0)
        .row_gap(2.0)
        .header(
            SidebarRow::new("Search")
                .icon(&glyphs::SEARCH)
                .detail("⌘K")
                .height(32.0)
                .build(k)
                .key("search"),
        );
    if !r.pinned().is_empty() {
        let rows = group(s, r.pinned(), k, layer);
        sb = sb.section(
            SidebarSection::new("Pinned")
                .key("pinned")
                .collapsed(s.pinned_closed)
                .on_toggle(|s: &mut App, _| s.pinned_closed ^= true)
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
            .on_toggle(|s: &mut App, _| s.others_closed ^= true)
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
    sb.build(k)
}

/// Each agent's row, then its team (when unfolded).
fn group(s: &App, agents: Vec<&Agent>, k: &Kit, layer: &mut Vec<El>) -> Vec<El> {
    let mut rows = Vec::new();
    for a in agents {
        rows.push(agent_row(s, a, None, k, layer));
        if s.roster.team_open(&a.id) {
            for m in &a.subagents {
                rows.push(agent_row(s, m, Some(&a.id), k, layer));
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
    let mut r = SidebarRow::new(a.name.clone())
        .height(if member { 30.0 } else { 36.0 })
        .avatar(avatar(a, k, if member { 14.0 } else { 18.0 }))
        .depth(u8::from(member))
        .selected(selected)
        .trailing(controls(s, a, &target, member, k, layer))
        .on_select({
            let t = target.clone();
            move |s: &mut App, _| s.select_agent(t.clone())
        });
    match a.family_status() {
        AgentStatus::Running => r = r.status(RowStatus::Busy),
        AgentStatus::Waiting => r = r.status(RowStatus::Attention),
        AgentStatus::Idle => {}
    }
    let t = target.clone();
    r.build(k)
        .key(format!("agent-{}", a.id))
        .on_secondary_press(move |s: &mut App, p, _| {
            s.roster.open_menu(t.clone(), Some((p.x, p.y)))
        })
}

/// Team count and fold button (agents with a team), and the ⋯ menu.
fn controls(s: &App, a: &Agent, target: &Target, member: bool, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let mut r = row().gap(2.0).center_y();
    if !member && !a.subagents.is_empty() {
        let open = s.roster.team_open(&a.id);
        let id = a.id.clone();
        r = r
            .child(
                text(a.subagents.len().to_string())
                    .font(k.ty.small)
                    .color(c.text_faint),
            )
            .child(
                IconButton::new(if open {
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
                .on_press(move |s: &mut App| s.roster.toggle_team(&id))
                .build(k)
                .key(format!("team-{}", a.id)),
            );
    }
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
    r.child(menu.build(k, layer))
}

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
    row()
        .size(size, size)
        .radius(size / 4.0)
        .center()
        .fill(theme::avatar_color(a.hue()))
        .child(text(a.initial()).font(k.ty.small_medium).color(k.c.white))
}
