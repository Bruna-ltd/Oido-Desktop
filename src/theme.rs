//! Oido's look: Studio's monochrome dark palette (`DESIGN.md`) on rune-kit's semantic tokens.

use rune::{Color, oklch};
use rune_kit::{Kit, PaletteSeed};

/// Studio is dark-first and monochrome; the one chroma is the approval-gate green.
pub fn kit() -> Kit {
    let mut k = Kit::from_palette(PaletteSeed {
        bg: oklch(0.08, 0.0, 0.0),
        fg: oklch(0.94, 0.0, 0.0),
        accent: green(),
    });
    k.radius.sm = 6.0;
    k.radius.md = 10.0;
    k.radius.lg = 14.0;
    k
}

/// Studio's `landing-green`: running, approved, "asks before it acts".
pub fn green() -> Color {
    oklch(0.80, 0.17, 150.0)
}
/// Waiting on a human.
pub fn amber() -> Color {
    oklch(0.80, 0.14, 80.0)
}

pub fn violet() -> Color {
    oklch(0.74, 0.12, 295.0)
}

pub fn avatar_color(hue: f32) -> Color {
    oklch(0.62, 0.12, hue)
}
