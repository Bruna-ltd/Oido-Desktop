//! Studio's icons. The rail's come from Lucide (lucide.dev, ISC; see LICENSE-lucide), drawn on
//! its 24 point grid; the few others are 16 point strokes in `currentColor`; and Oido's logo.

use rune::Icon;

/// A Lucide icon: its elements as they come, stroked a touch lighter than Lucide's 2 so they
/// match the 1.2 point strokes of the 16 point icons.
macro_rules! lucide {
    ($name:ident, $id:literal, $body:literal) => {
        pub static $name: Icon = Icon {
            name: $id,
            view: 24.0,
            body: concat!(r#"<g stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" fill="none">"#, $body, "</g>"),
        };
    };
}

macro_rules! icon {
    ($name:ident, $id:literal, $body:literal) => {
        pub static $name: Icon = Icon {
            name: $id,
            view: 16.0,
            body: concat!(r#"<g stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none">"#, $body, "</g>"),
        };
    };
}

// Lucide `pencil`.
lucide!(
    PENCIL,
    "oido-pencil",
    r#"<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>"#
);
// Lucide `calendar-clock`.
lucide!(
    SCHEDULED,
    "oido-scheduled",
    r#"<path d="M16 14v2.2l1.6 1"/><path d="M16 2v3"/><path d="M21 7.338V5a2 2 0 00-2-2H5a2 2 0 00-2 2v14a2 2 0 002 2h2.338"/><path d="M3 9h5.859"/><path d="M8 2v3"/><circle cx="16" cy="16" r="6"/>"#
);
// Lucide `chart-column`.
lucide!(
    USAGE,
    "oido-usage",
    r#"<path d="M3 3v16a2 2 0 0 0 2 2h16"/><path d="M18 17V9"/><path d="M13 17V5"/><path d="M8 17v-3"/>"#
);
// Lucide `activity`.
lucide!(
    RUNS,
    "oido-runs",
    r#"<path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2"/>"#
);
// Lucide `key-round`.
lucide!(
    PROVIDERS,
    "oido-providers",
    r#"<path d="M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z"/><circle cx="16.5" cy="7.5" r=".5" fill="currentColor"/>"#
);
// Lucide `messages-square`.
lucide!(
    CHANNELS,
    "oido-channels",
    r#"<path d="M16 10a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 14.286V4a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"/><path d="M20 9a2 2 0 0 1 2 2v10.286a.71.71 0 0 1-1.212.502l-2.202-2.202A2 2 0 0 0 17.172 19H10a2 2 0 0 1-2-2v-1"/>"#
);
// Lucide `blocks`.
lucide!(
    EXTENSIONS,
    "oido-extensions",
    r#"<path d="M10 22V7a1 1 0 0 0-1-1H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-5a1 1 0 0 0-1-1H2"/><rect x="14" y="2" width="8" height="8" rx="1"/>"#
);
// Lucide `plug`.
lucide!(
    MCP,
    "oido-mcp",
    r#"<path d="M12 22v-5"/><path d="M15 8V2"/><path d="M17 8a1 1 0 0 1 1 1v4a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V9a1 1 0 0 1 1-1z"/><path d="M9 8V2"/>"#
);
// Lucide `workflow`.
lucide!(
    WORKFLOWS,
    "oido-workflows",
    r#"<rect width="8" height="8" x="3" y="3" rx="2"/><path d="M7 11v4a2 2 0 0 0 2 2h4"/><rect width="8" height="8" x="13" y="13" rx="2"/>"#
);
// Lucide `shield-alert`.
lucide!(
    GUARDRAILS,
    "oido-guardrails",
    r#"<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="M12 8v4"/><path d="M12 16h.01"/>"#
);
// Lucide `settings-2`.
lucide!(
    SETTINGS,
    "oido-settings",
    r#"<path d="M14 17H5"/><path d="M19 7h-9"/><circle cx="17" cy="17" r="3"/><circle cx="7" cy="7" r="3"/>"#
);
// Lucide `pencil`.
lucide!(
    EDIT,
    "oido-edit",
    r#"<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>"#
);
icon!(
    CHEVRON_LEFT,
    "oido-chevron-left",
    r#"<path d="M10 3.5L5.5 8L10 12.5"/>"#
);
icon!(
    CHEVRON_RIGHT,
    "oido-chevron-right",
    r#"<path d="M6 3.5L10.5 8L6 12.5"/>"#
);
icon!(PLUS, "oido-plus", r#"<path d="M8 3.5V12.5M3.5 8H12.5"/>"#);
icon!(
    STOP,
    "oido-stop",
    r#"<rect x="4.5" y="4.5" width="7" height="7" rx="1.2" fill="currentColor"/>"#
);
icon!(
    MORE,
    "oido-more",
    r#"<circle cx="3.5" cy="8" r="1" fill="currentColor"/><circle cx="8" cy="8" r="1" fill="currentColor"/><circle cx="12.5" cy="8" r="1" fill="currentColor"/>"#
);

// Icons of the workflow canvas's nodes and controls.
// Lucide `terminal`.
lucide!(
    NODE_TOOL,
    "oido-terminal",
    r#"<path d="M12 19h8"/><path d="m4 17 6-6-6-6"/>"#
);

// Lucide `sparkles`.
lucide!(
    NODE_AI,
    "oido-sparkles",
    r#"<path d="M11.017 2.814a1 1 0 0 1 1.966 0l1.051 5.558a2 2 0 0 0 1.594 1.594l5.558 1.051a1 1 0 0 1 0 1.966l-5.558 1.051a2 2 0 0 0-1.594 1.594l-1.051 5.558a1 1 0 0 1-1.966 0l-1.051-5.558a2 2 0 0 0-1.594-1.594l-5.558-1.051a1 1 0 0 1 0-1.966l5.558-1.051a2 2 0 0 0 1.594-1.594z"/><path d="M20 2v4"/><path d="M22 4h-4"/><circle cx="4" cy="20" r="2"/>"#
);

// Lucide `bot`.
lucide!(
    NODE_AGENT,
    "oido-bot",
    r#"<path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M15 13v2"/><path d="M9 13v2"/>"#
);

// Lucide `git-branch`.
lucide!(
    NODE_IF,
    "oido-git-branch",
    r#"<path d="M15 6a9 9 0 0 0-9 9V3"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/>"#
);

// Lucide `repeat`.
lucide!(
    NODE_LOOP,
    "oido-repeat",
    r#"<path d="m17 2 4 4-4 4"/><path d="M3 11v-1a4 4 0 0 1 4-4h14"/><path d="m7 22-4-4 4-4"/><path d="M21 13v1a4 4 0 0 1-4 4H3"/>"#
);

// Lucide `hourglass`.
lucide!(
    NODE_WAIT,
    "oido-hourglass",
    r#"<path d="M5 22h14"/><path d="M5 2h14"/><path d="M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22"/><path d="M7 2v4.172a2 2 0 0 0 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2"/>"#
);

// Lucide `git-fork`.
lucide!(
    NODE_PARALLEL,
    "oido-git-fork",
    r#"<circle cx="12" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><circle cx="18" cy="6" r="3"/><path d="M18 9v2c0 .6-.4 1-1 1H7c-.6 0-1-.4-1-1V9"/><path d="M12 12v3"/>"#
);

// Lucide `list-checks`.
lucide!(
    NODE_DECIDE,
    "oido-list-checks",
    r#"<path d="M13 5h8"/><path d="M13 12h8"/><path d="M13 19h8"/><path d="m3 17 2 2 4-4"/><path d="m3 7 2 2 4-4"/>"#
);

// Lucide `split`.
lucide!(
    NODE_SWITCH,
    "oido-split",
    r#"<path d="M16 3h5v5"/><path d="M8 3H3v5"/><path d="M12 22v-8.3a4 4 0 0 0-1.172-2.872L3 3"/><path d="m15 9 6-6"/>"#
);

// Lucide `refresh-cw`.
lucide!(
    NODE_WHILE,
    "oido-refresh-cw",
    r#"<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>"#
);

// Lucide `undo-2`.
lucide!(
    NODE_TRY,
    "oido-undo-2",
    r#"<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"/>"#
);

// Lucide `variable`.
lucide!(
    NODE_SET,
    "oido-variable",
    r#"<path d="M8 21s-4-3-4-9 4-9 4-9"/><path d="M16 3s4 3 4 9-4 9-4 9"/><line x1="15" x2="9" y1="9" y2="15"/><line x1="9" x2="15" y1="9" y2="15"/>"#
);

// Lucide `code-xml`.
lucide!(
    NODE_CODE,
    "oido-code-xml",
    r#"<path d="m18 16 4-4-4-4"/><path d="m6 8-4 4 4 4"/><path d="m14.5 4-5 16"/>"#
);

// Lucide `play`.
lucide!(
    NODE_MANUAL,
    "oido-play",
    r#"<path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"/>"#
);

// Lucide `clock`.
lucide!(
    NODE_SCHEDULE,
    "oido-clock",
    r#"<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>"#
);

// Lucide `plus`.
lucide!(
    NODE_PLUS,
    "oido-plus",
    r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#
);

// Lucide `zoom-in`.
lucide!(
    NODE_ZOOM_IN,
    "oido-zoom-in",
    r#"<circle cx="11" cy="11" r="8"/><line x1="21" x2="16.65" y1="21" y2="16.65"/><line x1="11" x2="11" y1="8" y2="14"/><line x1="8" x2="14" y1="11" y2="11"/>"#
);

// Lucide `zoom-out`.
lucide!(
    NODE_ZOOM_OUT,
    "oido-zoom-out",
    r#"<circle cx="11" cy="11" r="8"/><line x1="21" x2="16.65" y1="21" y2="16.65"/><line x1="8" x2="14" y1="11" y2="11"/>"#
);

// Lucide `maximize`.
lucide!(
    NODE_FIT,
    "oido-maximize",
    r#"<path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>"#
);

// Lucide `flask-conical`.
lucide!(
    NODE_FLASK,
    "oido-flask-conical",
    r#"<path d="M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2"/><path d="M6.453 15h11.094"/><path d="M8.5 2h7"/>"#
);
// Lucide `undo-2`.
lucide!(
    NODE_UNDO,
    "oido-undo-2",
    r#"<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"/>"#
);
// Lucide `redo-2`.
lucide!(
    NODE_REDO,
    "oido-redo-2",
    r#"<path d="m15 14 5-5-5-5"/><path d="M20 9H9.5A5.5 5.5 0 0 0 4 14.5A5.5 5.5 0 0 0 9.5 20H13"/>"#
);

/// Oido's mark, at the top of the rail: 2 point strokes on a 32 point grid.
pub static LOGO: Icon = Icon {
    name: "oido-logo",
    view: 32.0,
    body: r#"<g stroke="currentColor" stroke-width="2" fill="none"><path d="M1 1L16 1A15 15 0 0 1 31 16A15 15 0 0 1 16 31A15 15 0 0 1 1 16Z"/><circle cx="16" cy="16" r="6.5"/></g>"#,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rail_icon_draws_at_rail_size() {
        let rail = [
            &SCHEDULED,
            &USAGE,
            &RUNS,
            &PROVIDERS,
            &CHANNELS,
            &EXTENSIONS,
            &MCP,
            &WORKFLOWS,
            &GUARDRAILS,
            &SETTINGS,
        ];
        for icon in rail {
            let cov = rune::text::icon_coverage(icon, 16);
            let inked = cov.iter().filter(|&&c| c > 64).count();
            assert!(inked > 12, "{} draws {inked} pixels", icon.name);
        }
    }
}
