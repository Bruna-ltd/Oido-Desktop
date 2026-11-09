//! Oido's look: Studio's monochrome dark palette (`DESIGN.md`), or an Omarchy theme's colours, on
//! rune-kit's semantic tokens.
//!
//! The role colours (running, waiting, the model's steps) ride on the kit as `success`, `warning`
//! and `info` icon colours, so a screen reads them from the kit it is drawn with.

use std::time::Duration;

use rune::{Color, oklch};
use rune_kit::{Kit, PaletteSeed};
use serde::{Deserialize, Serialize};

use crate::omarchy;

/// The colours a look is made of.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Colors {
    pub bg: Color,
    pub fg: Color,
    pub accent: Color,
    /// Running, approved, done: `k.c.success.icon`.
    pub green: Color,
    /// Waiting on a human, failed: `k.c.warning.icon`.
    pub amber: Color,
    /// The model's own steps: `k.c.info.icon`.
    pub violet: Color,
}

impl Colors {
    /// Studio is dark-first and monochrome; the one chroma is the approval-gate green.
    pub fn oido() -> Colors {
        // Studio's `landing-green`: running, approved, "asks before it acts".
        let green = oklch(0.80, 0.17, 150.0);
        Colors {
            bg: oklch(0.08, 0.0, 0.0),
            fg: oklch(0.94, 0.0, 0.0),
            accent: green,
            green,
            amber: oklch(0.80, 0.14, 80.0),
            violet: oklch(0.74, 0.12, 295.0),
        }
    }

    pub fn kit(&self) -> Kit {
        let mut k = Kit::from_palette(PaletteSeed {
            bg: self.bg,
            fg: self.fg,
            accent: self.accent,
        });
        k.radius.sm = 6.0;
        k.radius.md = 10.0;
        k.radius.lg = 14.0;
        k.c.success.icon = self.green;
        k.c.warning.icon = self.amber;
        k.c.info.icon = self.violet;
        k
    }
}

/// Which look the app wears, as the person chose it on the Settings page.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "theme", rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Oido,
    /// Whatever Omarchy theme the desktop wears, following it when it changes.
    System,
    /// One Omarchy theme, by its folder name.
    Omarchy(String),
}

/// The chosen appearance and the Omarchy themes it can pick from.
#[derive(Clone, Debug, Default)]
pub struct Look {
    pub appearance: Appearance,
    /// Installed Omarchy themes, by name. Empty without Omarchy.
    pub themes: Vec<omarchy::Theme>,
    /// The theme the desktop wears now.
    pub system: Option<omarchy::Theme>,
}

impl Look {
    /// Reads the installed themes and the desktop's current one.
    pub fn load(appearance: Appearance) -> Look {
        let mut look = Look {
            appearance,
            ..Look::default()
        };
        look.reload();
        look
    }

    /// Reads Omarchy's themes again, after it switched or installed one.
    pub fn reload(&mut self) {
        let dirs = omarchy::Dirs::locate();
        self.themes = dirs.as_ref().map(omarchy::Dirs::themes).unwrap_or_default();
        self.system = dirs.as_ref().and_then(omarchy::Dirs::current);
    }

    /// The colours to draw with. A chosen theme that is gone (uninstalled, or Omarchy not here)
    /// falls back to Oido's own.
    pub fn colors(&self) -> Colors {
        let theme = match &self.appearance {
            Appearance::Oido => None,
            Appearance::System => self.system.as_ref(),
            Appearance::Omarchy(slug) => self.themes.iter().find(|t| &t.slug == slug),
        };
        theme.map_or_else(Colors::oido, |t| t.colors)
    }

    pub fn kit(&self) -> Kit {
        self.colors().kit()
    }
}

/// How often the desktop is checked for a theme switch.
const FOLLOW_EVERY: Duration = Duration::from_secs(2);

/// Calls `on_switch` whenever Omarchy switches theme, from a thread of its own that lives as long
/// as the app. Does nothing without a home to find Omarchy in.
pub fn follow_omarchy(on_switch: impl Fn() + Send + 'static) {
    let Some(dirs) = omarchy::Dirs::locate() else {
        return;
    };
    std::thread::spawn(move || {
        let mut seen = dirs.generation();
        loop {
            std::thread::sleep(FOLLOW_EVERY);
            let now = dirs.generation();
            if now != seen {
                seen = now;
                on_switch();
            }
        }
    });
}

pub fn avatar_color(hue: f32) -> Color {
    oklch(0.62, 0.12, hue)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(slug: &str, bg: Color) -> omarchy::Theme {
        omarchy::Theme {
            slug: slug.to_owned(),
            colors: Colors {
                bg,
                ..Colors::oido()
            },
        }
    }

    #[test]
    fn the_chosen_theme_is_worn_and_a_missing_one_falls_back_to_oidos() {
        let mut look = Look {
            appearance: Appearance::Omarchy("nord".to_owned()),
            themes: vec![theme("nord", Color::WHITE)],
            system: Some(theme("kanagawa", Color::BLACK)),
        };
        assert_eq!(look.colors().bg, Color::WHITE);
        look.appearance = Appearance::System;
        assert_eq!(look.colors().bg, Color::BLACK);
        look.appearance = Appearance::Omarchy("gone".to_owned());
        assert_eq!(look.colors(), Colors::oido());
        look.system = None;
        look.appearance = Appearance::System;
        assert_eq!(look.colors(), Colors::oido());
    }

    #[test]
    fn the_kit_carries_the_role_colours() {
        let c = Colors::oido();
        let k = c.kit();
        assert_eq!(k.c.success.icon, c.green);
        assert_eq!(k.c.warning.icon, c.amber);
        assert_eq!(k.c.info.icon, c.violet);
    }

    #[test]
    fn an_appearance_saves_and_reads_back() {
        for a in [
            Appearance::Oido,
            Appearance::System,
            Appearance::Omarchy("tokyo-night".to_owned()),
        ] {
            let json = serde_json::to_string(&a).unwrap_or_default();
            assert_eq!(serde_json::from_str::<Appearance>(&json).ok(), Some(a));
        }
    }
}
