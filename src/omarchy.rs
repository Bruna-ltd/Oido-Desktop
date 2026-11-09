//! Reading Omarchy's themes, so the app can wear the desktop's colours.
//!
//! A theme is a folder holding a `colors.toml`. Stock themes are under `$OMARCHY_PATH/themes`
//! (`/usr/share/omarchy` when unset), the person's own under `~/.config/omarchy/themes`, where a
//! folder with a stock theme's name overlays it, file by file. The theme the desktop wears now is
//! copied to `~/.local/state/omarchy/current/theme`, its name is in `theme.name` beside it, and
//! `theme.generation` changes with every switch.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use rune::color::hex;
use serde::Deserialize;

use crate::theme::Colors;

/// An installed Omarchy theme.
#[derive(Clone, PartialEq, Debug)]
pub struct Theme {
    /// Its folder's name, which is what `omarchy theme set` takes.
    pub slug: String,
    pub colors: Colors,
}

impl Theme {
    /// "tokyo-night" as "Tokyo Night".
    pub fn title(&self) -> String {
        title(&self.slug)
    }
}

/// Where Omarchy keeps its themes on this computer.
#[derive(Clone, Debug)]
pub struct Dirs {
    stock: PathBuf,
    user: PathBuf,
    current: PathBuf,
}

impl Dirs {
    /// Omarchy's folders under the person's home. `None` when there is no home to look in.
    pub fn locate() -> Option<Dirs> {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        let root = std::env::var_os("OMARCHY_PATH")
            .map_or_else(|| PathBuf::from("/usr/share/omarchy"), PathBuf::from);
        Some(Dirs {
            stock: root.join("themes"),
            user: home.join(".config/omarchy/themes"),
            current: home.join(".local/state/omarchy/current"),
        })
    }

    /// Every installed theme with readable colours, by name. The person's own colours win over a
    /// stock theme's of the same name.
    pub fn themes(&self) -> Vec<Theme> {
        let slugs: BTreeSet<String> = [&self.stock, &self.user]
            .into_iter()
            .filter_map(|dir| fs::read_dir(dir).ok())
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        slugs
            .into_iter()
            .filter_map(|slug| {
                let colors = [&self.user, &self.stock]
                    .into_iter()
                    .find_map(|dir| read_colors(&dir.join(&slug)))?;
                Some(Theme { slug, colors })
            })
            .collect()
    }

    /// The theme the desktop wears now. `None` when Omarchy has not set one.
    pub fn current(&self) -> Option<Theme> {
        let slug = fs::read_to_string(self.current.join("theme.name")).ok()?;
        Some(Theme {
            slug: slug.trim().to_owned(),
            colors: read_colors(&self.current.join("theme"))?,
        })
    }

    /// Changes whenever Omarchy switches theme: cheap to read, for noticing a switch.
    pub fn generation(&self) -> Option<String> {
        fs::read_to_string(self.current.join("theme.generation")).ok()
    }
}

/// The colours in a theme folder's `colors.toml`.
fn read_colors(theme_dir: &Path) -> Option<Colors> {
    parse_colors(&fs::read_to_string(theme_dir.join("colors.toml")).ok()?)
}

/// The keys of `colors.toml` this app uses; the rest are for terminals and the like.
#[derive(Deserialize)]
struct ColorsFile {
    background: String,
    foreground: String,
    accent: Option<String>,
    green: Option<String>,
    yellow: Option<String>,
    magenta: Option<String>,
}

/// A theme's colours. Background and foreground are required; a missing or unreadable role colour
/// falls back to Oido's own.
fn parse_colors(text: &str) -> Option<Colors> {
    let file: ColorsFile = toml::from_str(text).ok()?;
    let oido = Colors::oido();
    let or = |c: &Option<String>, fallback| c.as_deref().and_then(hex).unwrap_or(fallback);
    let fg = hex(&file.foreground)?;
    Some(Colors {
        bg: hex(&file.background)?,
        fg,
        accent: or(&file.accent, fg),
        green: or(&file.green, oido.green),
        amber: or(&file.yellow, oido.amber),
        violet: or(&file.magenta, oido.violet),
    })
}

fn title(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    const CATPPUCCIN: &str = r##"
mode = "dark"
accent = "#89b4fa"
background = "#1e1e2e"
foreground = "#cdd6f4"
green = "#a6e3a1"
yellow = "#f9e2af"
magenta = "#f5c2e7"
bright_red = "#f38ba8"
"##;

    #[test]
    fn a_themes_colours_are_read_and_its_other_keys_ignored() {
        let c = parse_colors(CATPPUCCIN).unwrap();
        assert_eq!(c.bg, hex("#1e1e2e").unwrap());
        assert_eq!(c.fg, hex("#cdd6f4").unwrap());
        assert_eq!(c.accent, hex("#89b4fa").unwrap());
        assert_eq!(c.amber, hex("#f9e2af").unwrap());
    }

    #[test]
    fn missing_role_colours_fall_back_and_missing_ground_colours_refuse() {
        let c = parse_colors("background = \"#ffffff\"\nforeground = \"#000000\"").unwrap();
        assert_eq!(c.accent, c.fg);
        assert_eq!(c.green, Colors::oido().green);
        assert!(parse_colors("background = \"#ffffff\"").is_none());
        assert!(parse_colors("background = \"white\"\nforeground = \"#000\"").is_none());
    }

    #[test]
    fn a_users_theme_overlays_the_stock_one_of_the_same_name() {
        let root = std::env::temp_dir().join(format!("oido-omarchy-{}", std::process::id()));
        let write = |dir: &Path, bg: &str| {
            fs::create_dir_all(dir).unwrap();
            fs::write(
                dir.join("colors.toml"),
                format!("background = \"{bg}\"\nforeground = \"#ffffff\""),
            )
            .unwrap();
        };
        let dirs = Dirs {
            stock: root.join("stock"),
            user: root.join("user"),
            current: root.join("current"),
        };
        write(&dirs.stock.join("nord"), "#2e3440");
        write(&dirs.stock.join("tokyo-night"), "#1a1b26");
        write(&dirs.user.join("nord"), "#000000");
        fs::create_dir_all(dirs.user.join("broken")).unwrap();
        write(&dirs.current.join("theme"), "#1a1b26");
        fs::write(dirs.current.join("theme.name"), "tokyo-night\n").unwrap();

        let themes = dirs.themes();
        fs::remove_dir_all(&root).unwrap();
        let slugs: Vec<&str> = themes.iter().map(|t| t.slug.as_str()).collect();
        assert_eq!(slugs, ["nord", "tokyo-night"]);
        assert_eq!(themes[0].colors.bg, hex("#000000").unwrap());
        assert_eq!(themes[1].title(), "Tokyo Night");
    }

    #[test]
    fn the_current_theme_is_named_from_its_state_file() {
        let root = std::env::temp_dir().join(format!("oido-omarchy-cur-{}", std::process::id()));
        let dirs = Dirs {
            stock: root.join("stock"),
            user: root.join("user"),
            current: root.join("current"),
        };
        assert!(dirs.current().is_none());
        fs::create_dir_all(dirs.current.join("theme")).unwrap();
        fs::write(
            dirs.current.join("theme/colors.toml"),
            "background = \"#1f1f28\"\nforeground = \"#dcd7ba\"",
        )
        .unwrap();
        fs::write(dirs.current.join("theme.name"), "kanagawa\n").unwrap();
        let current = dirs.current();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(current.unwrap().slug, "kanagawa");
    }
}
