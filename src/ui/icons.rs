//! The rail's icons: 16 point strokes in `currentColor`, one per Studio section; and Oido's logo.

use rune::Icon;

macro_rules! icon {
    ($name:ident, $id:literal, $body:literal) => {
        pub static $name: Icon = Icon {
            name: $id,
            view: 16.0,
            body: concat!(r#"<g stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none">"#, $body, "</g>"),
        };
    };
}

icon!(
    SCHEDULED,
    "oido-scheduled",
    r#"<circle cx="8" cy="8" r="5.5"/><path d="M8 4.8V8L10.2 9.4"/>"#
);
icon!(
    USAGE,
    "oido-usage",
    r#"<path d="M3.5 13V8M8 13V3.5M12.5 13V6.5"/>"#
);
icon!(
    RUNS,
    "oido-runs",
    r#"<path d="M1.8 8H4.2L6 3.5L9.5 12.5L11.3 8H14.2"/>"#
);
icon!(
    PROVIDERS,
    "oido-providers",
    r#"<circle cx="5.5" cy="10.5" r="2.6"/><path d="M7.4 8.6L13 3M10.8 5.2L12.4 6.8"/>"#
);
icon!(
    CHANNELS,
    "oido-channels",
    r#"<path d="M3 6.5V9.5M13 6.5V9.5M6 4.5V11.5M10 4.5V11.5"/>"#
);
icon!(
    EXTENSIONS,
    "oido-extensions",
    r#"<rect x="2.6" y="2.6" width="4.8" height="4.8" rx="1"/><rect x="8.6" y="8.6" width="4.8" height="4.8" rx="1"/><rect x="2.6" y="8.6" width="4.8" height="4.8" rx="1"/><path d="M11 2.4V6.6M8.9 4.5H13.1"/>"#
);
icon!(
    MCP,
    "oido-mcp",
    r#"<path d="M6 3V6M10 3V6"/><path d="M4.5 6H11.5V8.5C11.5 10.4 9.9 12 8 12C6.1 12 4.5 10.4 4.5 8.5Z"/><path d="M8 12V14"/>"#
);
icon!(
    SKILLS,
    "oido-skills",
    r#"<path d="M5.5 3.5H4.2C3.3 3.5 2.6 4.2 2.6 5.1V6.5C2.6 7.4 2 8 2 8C2 8 2.6 8.6 2.6 9.5V10.9C2.6 11.8 3.3 12.5 4.2 12.5H5.5"/><path d="M10.5 3.5H11.8C12.7 3.5 13.4 4.2 13.4 5.1V6.5C13.4 7.4 14 8 14 8C14 8 13.4 8.6 13.4 9.5V10.9C13.4 11.8 12.7 12.5 11.8 12.5H10.5"/>"#
);
icon!(
    TOOLS,
    "oido-tools",
    r#"<path d="M9.8 3.2C8.2 2.7 6.4 3.5 5.7 5.1C5.3 6 5.4 7 5.8 7.8L2.8 10.8C2.3 11.3 2.3 12.1 2.8 12.6L3.4 13.2C3.9 13.7 4.7 13.7 5.2 13.2L8.2 10.2C9 10.6 10 10.7 10.9 10.3C12.5 9.6 13.3 7.8 12.8 6.2L11 8L9.4 7.4L8.8 5.8Z"/>"#
);
icon!(
    WORKFLOWS,
    "oido-workflows",
    r#"<circle cx="3.6" cy="4" r="1.6"/><circle cx="12.4" cy="8" r="1.6"/><circle cx="3.6" cy="12" r="1.6"/><path d="M5.2 4H8.4C10.4 4 10.8 6 10.8 8M5.2 12H8.4C10.4 12 10.8 10 10.8 8"/>"#
);
icon!(
    GUARDRAILS,
    "oido-guardrails",
    r#"<path d="M8 2L13 4V8C13 11 10.8 13 8 14C5.2 13 3 11 3 8V4Z"/><path d="M8 5.5V8.5M8 10.6H8.01"/>"#
);
icon!(
    SETTINGS,
    "oido-settings",
    r#"<path d="M3 5H13M3 11H13"/><circle cx="6" cy="5" r="1.6"/><circle cx="10.5" cy="11" r="1.6"/>"#
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

/// Oido's mark, at the top of the rail: 2 point strokes on a 32 point grid.
pub static LOGO: Icon = Icon {
    name: "oido-logo",
    view: 32.0,
    body: r#"<g stroke="currentColor" stroke-width="2" fill="none"><path d="M1 1L16 1A15 15 0 0 1 31 16A15 15 0 0 1 16 31A15 15 0 0 1 1 16Z"/><circle cx="16" cy="16" r="6.5"/></g>"#,
};
