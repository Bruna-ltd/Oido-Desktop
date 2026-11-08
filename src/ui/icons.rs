//! The rail's icons: 16 point strokes in `currentColor`, one per Studio section.

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
    PAPERCLIP,
    "oido-paperclip",
    r#"<path d="M12.6 7.4L8 12C6.6 13.4 4.4 13.4 3.1 12C1.8 10.7 1.8 8.6 3.1 7.2L8.2 2.1C9.1 1.2 10.5 1.2 11.4 2.1C12.3 3 12.3 4.4 11.4 5.3L6.4 10.3C5.9 10.8 5.1 10.8 4.7 10.3C4.2 9.8 4.2 9.1 4.7 8.6L9 4.3"/>"#
);
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
