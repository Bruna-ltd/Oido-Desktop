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
