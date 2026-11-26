//! Left sidebar: the agents, pinned first, each with its team of members beneath it. Two levels
//! at most. A row's actions live in a menu you reach three ways: its ⋯ button, a right-click, or
//! the same commands on the edit screen, so none is hidden behind a gesture.

use std::rc::Rc;

use rune::{Anim, Drag, El, PointerPress, Tx, el, row, text};
use rune_kit::shell::{Sidebar, SidebarRow, SidebarSection};
use rune_kit::{IconButton, IconButtonVariant, Kit, Menu, MenuItem, Spinner, context_menu, glyphs};

use super::icons;
use crate::agents::{Agent, Roster, Target};
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
        entries(menu, &m.target, agent, r).build(k, layer);
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
/// An agent drags to a new place in its section, its team with it; a member, within its team.
fn group(s: &App, agents: Vec<&Agent>, k: &Kit, layer: &mut Vec<El>) -> Vec<El> {
    let r = &s.roster;
    let count = agents.len();
    let open = |a: &Agent| r.team_open(&a.id) && !a.subagents.is_empty();
    // How far each family reaches down the list, to the next one's top: what a dragged agent
    // moves past.
    let extents: Rc<[f32]> = agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let mut h = AGENT_H + ROW_GAP;
            if open(a) {
                h += a.subagents.len() as f32 * (MEMBER_H + ROW_GAP);
                if i + 1 < count {
                    h += TEAM_END + ROW_GAP;
                }
            }
            h
        })
        .collect();
    let drag = r
        .drag
        .as_ref()
        .filter(|d| d.target.parent.is_none() && agents.iter().any(|a| a.id == d.target.id));
    let mut rows = Vec::new();
    for (i, a) in agents.iter().enumerate() {
        let (shift, lifted) = drag.map_or((0.0, false), |d| (d.shift(i), d.from == i));
        let place = Place {
            index: i,
            count,
            extents: extents.clone(),
            shift,
            lifted,
        };
        rows.push(agent_row(s, a, None, place, k, layer));
        if !open(a) {
            continue;
        }
        let n = a.subagents.len();
        let team: Rc<[f32]> = vec![MEMBER_H + ROW_GAP; n].into();
        let in_team = r
            .drag
            .as_ref()
            .filter(|d| d.target.parent.as_deref() == Some(a.id.as_str()));
        for (j, m) in a.subagents.iter().enumerate() {
            let place = Place {
                index: j,
                count: n,
                extents: team.clone(),
                shift: shift + in_team.map_or(0.0, |d| d.shift(j)),
                lifted: lifted || in_team.is_some_and(|d| d.from == j),
            };
            rows.push(agent_row(s, m, Some(&a.id), place, k, layer));
        }
        if i + 1 < count {
            let mut gap = el()
                .key(format!("after-team-{}", a.id))
                .h(TEAM_END)
                .a11y_hidden()
                .offset(0.0, shift);
            if lifted {
                gap = gap.raise().offset_motion(Anim::INSTANT);
            }
            rows.push(gap);
        }
    }
    rows
}

/// A row's agent row height, a team member's, and the air after an unfolded team.
const AGENT_H: f32 = 36.0;
const MEMBER_H: f32 = 28.0;
const TEAM_END: f32 = 4.0;
/// Points between a section's rows (the kit's).
const ROW_GAP: f32 = 2.0;

/// Where a row sits among its siblings, and where a drag has it now.
struct Place {
    index: usize,
    count: usize,
    /// Each sibling's reach down the list (see [`crate::agents::RowDrag`]).
    extents: Rc<[f32]>,
    shift: f32,
    /// It is the dragged row, or of the dragged agent's team.
    lifted: bool,
}

fn agent_row(
    s: &App,
    a: &Agent,
    parent: Option<&str>,
    place: Place,
    k: &Kit,
    layer: &mut Vec<El>,
) -> El {
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
    let dragged = target.clone();
    let row = SidebarRow::new(a.name.clone())
        .height(if member { MEMBER_H } else { AGENT_H })
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
        .position_in_set(place.index + 1, place.count)
        .offset(0.0, place.shift);
    let row = if place.lifted {
        // Follows the pointer exactly while the rest springs around it, opaque so the rows it
        // passes don't show through.
        k.elevate(
            row.raise()
                .offset_motion(Anim::INSTANT)
                .fill(k.c.surface_raised),
            &k.elevation.raised,
        )
    } else {
        row
    };
    let Place { index, extents, .. } = place;
    // A press that doesn't move is still a select.
    row.on_drag(move |s: &mut App, d: &mut Drag, tx: &mut Tx| {
        s.drag_row(&dragged, index, &extents, d, tx)
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
        &s.roster,
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
fn entries(menu: Menu, target: &Target, a: &Agent, r: &Roster) -> Menu {
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
    // The drag's moves, for the keyboard and anyone who'd rather not drag.
    if let Some((i, count)) = r.place_among(target) {
        for (label, to) in [("Move up", i.checked_sub(1)), ("Move down", Some(i + 1))] {
            let Some(to) = to.filter(|&to| to < count) else {
                continue;
            };
            let t = target.clone();
            menu = menu.item(MenuItem::new(label).on_select(move |s: &mut App| {
                s.roster.menu = None;
                s.move_agent(&t, to);
            }));
        }
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
