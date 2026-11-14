//! Studio's right rail: icon-only by default, labelled when expanded. Operate sections on top,
//! Configure sections below, Settings at the foot.

use rune::{Color, El, Icon, col, el, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, Kit, glyphs};

use super::icons;
use crate::app::{App, Route};

struct Item {
    label: &'static str,
    icon: &'static Icon,
    route: Route,
}

static OPERATE: [Item; 3] = [
    Item {
        label: "Schedules",
        icon: &icons::SCHEDULED,
        route: Route::Schedules,
    },
    Item {
        label: "Usage",
        icon: &icons::USAGE,
        route: Route::Usage,
    },
    Item {
        label: "Runs",
        icon: &icons::RUNS,
        route: Route::Runs,
    },
];

static CONFIGURE: [Item; 6] = [
    Item {
        label: "Providers",
        icon: &icons::PROVIDERS,
        route: Route::Providers,
    },
    Item {
        label: "Workflows",
        icon: &icons::WORKFLOWS,
        route: Route::Workflows,
    },
    Item {
        label: "Channels",
        icon: &icons::CHANNELS,
        route: Route::Channels,
    },
    Item {
        label: "Extensions",
        icon: &icons::EXTENSIONS,
        route: Route::Extensions,
    },
    Item {
        label: "MCP",
        icon: &icons::MCP,
        route: Route::Mcp,
    },
    Item {
        label: "Guardrails",
        icon: &icons::GUARDRAILS,
        route: Route::Guardrails,
    },
];

static SETTINGS: Item = Item {
    label: "Settings",
    icon: &icons::SETTINGS,
    route: Route::Settings,
};

pub const COLLAPSED: f32 = 56.0;
pub const EXPANDED: f32 = 184.0;

pub fn build(s: &App, k: &Kit) -> El {
    let open = s.rail_open;
    let group = |items: &'static [Item]| {
        col()
            .w_full()
            .gap(2.0)
            .children(items.iter().map(|i| item(s, i, k)))
    };
    let rule = || {
        el().w(24.0)
            .h(1.0)
            .my(6.0)
            .fill(k.c.border_muted)
            .self_center()
    };
    let toggle = IconButton::new(if open {
        &glyphs::TRIANGLE_RIGHT
    } else {
        &glyphs::CHEVRON_UP
    })
    .variant(IconButtonVariant::GhostMuted)
    .label(if open {
        "Collapse sections"
    } else {
        "Expand sections"
    })
    .on_press(|s: &mut App| s.rail_open ^= true)
    .build(k)
    .key("rail-toggle");
    col()
        .key("rail")
        .w(if open { EXPANDED } else { COLLAPSED })
        .h_full()
        .min_h(0.0)
        .px(8.0)
        .py(12.0)
        .gap(2.0)
        .child(
            // Centred on the items' icons, collapsed or expanded.
            row()
                .key("rail-logo")
                .h(40.0)
                .px(8.0)
                .center_y()
                .a11y_hidden()
                .child(icon(&icons::LOGO, 24.0).color(k.c.text_strong)),
        )
        .child(rule())
        .child(group(&OPERATE))
        .child(rule())
        .child(group(&CONFIGURE))
        .child(el().grow(1.0))
        .child(item(s, &SETTINGS, k))
        .child(el().h(4.0))
        .child(row().w_full().justify_center().child(toggle))
}

fn item(s: &App, it: &'static Item, k: &Kit) -> El {
    let c = &k.c;
    let active = s.route == it.route;
    let ink = if active { c.text_base } else { c.text_muted };
    let mut r = row()
        .key(format!("rail-{}", it.label))
        .h(40.0)
        .w_full()
        .px(11.0)
        .gap(12.0)
        .center_y()
        .radius(10.0)
        .fill(if active {
            c.layer_02
        } else {
            Color::TRANSPARENT
        })
        .on_press(move |s: &mut App| s.go(it.route))
        .role("link")
        .label(it.label)
        .child(icon(it.icon, 18.0).color(ink));
    if s.rail_open {
        r = r.child(
            text(it.label)
                .font(if active { k.ty.ui_medium } else { k.ty.ui })
                .color(ink)
                .truncate(),
        );
    } else {
        r = r.help(it.label);
    }
    k.interactive(r)
}
