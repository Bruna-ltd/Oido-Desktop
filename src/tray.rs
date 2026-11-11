//! Oido's icon in the desktop bar's tray, as Slack has, so the app keeps running with its window
//! closed: schedules fire and replies finish. A click opens the window; its menu opens it or quits.

use image::imageops::FilterType;
use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, StandardItem};
use rune::AppHandle;

use crate::app::App;
use crate::launcher;

/// The pixel sizes the icon is offered at; the bar picks the one that suits it.
const SIZES: [u32; 2] = [32, 64];

struct Tray {
    app: AppHandle<App>,
    icons: Vec<ksni::Icon>,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        launcher::APP_ID.into()
    }

    fn title(&self) -> String {
        "Oido".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icons.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Oido".into(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        show(&self.app);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open Oido".into(),
                activate: Box::new(|t: &mut Self| show(&t.app)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit Oido".into(),
                activate: Box::new(|t: &mut Self| t.app.quit()),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Brings the window back, with the bar's activation token when it sent one, so the compositor
/// lets it take focus.
fn show(app: &AppHandle<App>) {
    match ksni::current_xdg_activation_token() {
        Some(token) => app.activate(token),
        None => app.open(),
    }
}

/// Puts the icon in the tray. False when the desktop has no tray to show it in; closing the
/// window must then quit, or the app would run on with no way back to it.
pub fn start(app: AppHandle<App>) -> bool {
    match (Tray {
        app,
        icons: icons(),
    })
    .spawn()
    {
        Ok(_) => true,
        Err(e) => {
            eprintln!("oido: no tray icon, so closing the window quits: {e}");
            false
        }
    }
}

/// The app icon at each of [`SIZES`], as ARGB in network byte order.
fn icons() -> Vec<ksni::Icon> {
    let Ok(image) = image::load_from_memory(launcher::PNG) else {
        return Vec::new();
    };
    let image = image.to_rgba8();
    SIZES
        .iter()
        .map(|&px| {
            let scaled = image::imageops::resize(&image, px, px, FilterType::Lanczos3);
            let data = scaled
                .pixels()
                .flat_map(|p| {
                    let [r, g, b, a] = p.0;
                    [a, r, g, b]
                })
                .collect();
            ksni::Icon {
                width: px as i32,
                height: px as i32,
                data,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_comes_at_every_size() {
        let icons = icons();
        assert_eq!(icons.len(), SIZES.len(), "the bundled PNG decodes");
        for (icon, px) in icons.iter().zip(SIZES) {
            assert_eq!((icon.width, icon.height), (px as i32, px as i32));
            assert_eq!(icon.data.len(), (px * px * 4) as usize);
        }
        // The tile's corner is transparent and its middle opaque.
        let big = &icons[1];
        assert_eq!(big.data[0], 0, "corner alpha");
        let mid = ((32 * 64 + 32) * 4) as usize;
        assert_eq!(big.data[mid], 255, "middle alpha");
    }
}
