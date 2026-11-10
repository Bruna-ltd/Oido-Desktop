//! OidoDesktop: the native Oido Studio. `cargo run` opens the window;
//! `cargo run -- --snap out.png` renders it headless.

mod agents;
mod app;
mod command;
mod completion;
mod composer;
mod connect;
mod context;
mod delegate;
#[cfg(any(test, debug_assertions))]
mod demo;
mod files;
mod flows;
mod folders;
mod listing;
mod mcp;
mod model;
mod models;
mod omarchy;
mod persist;
mod picker;
mod pricing;
mod prompt;
mod providers;
mod run;
mod runs;
mod schedule;
mod schedules;
mod skills;
mod store;
#[cfg(test)]
mod testing;
mod theme;
mod tools;
mod typesafe;
mod ui;
mod update;
mod usage;
mod vault;
mod window;
mod workflow;

use app::App;
use rune::{AppSpec, Chrome, KeyEvent, Tx};
use rune_kit::shell;

fn on_key(s: &mut App, ev: &KeyEvent, _: &mut Tx) -> bool {
    if shell::is_sidebar_toggle(ev) {
        s.sidebar.toggle();
        return true;
    }
    if ev.is_command('i') && ev.mods.alt {
        s.inspector.toggle();
        return true;
    }
    false
}

fn main() {
    if std::env::args().skip(1).any(|a| a == "--version") {
        println!("oido-desktop {}", update::VERSION);
        return;
    }
    let chrome = Chrome {
        full_size_content: true,
        traffic_lights: Some((16.0, 25.0)),
        drag_regions: true,
    };
    let app = rune::App::new(AppSpec {
        on_key: Some(on_key),
        // Named screens for headless renders; a release build has none.
        #[cfg(debug_assertions)]
        scenes: demo::SCENES,
        ..AppSpec::new("Oido", (1280.0, 800.0), app::init, ui::view)
    })
    .chrome(chrome);
    // Worker threads (checking a key, refreshing models) hand their results back through this.
    app::set_handle(app.handle());
    // The Settings page can follow Omarchy's theme: a switch there reads the themes again.
    let handle = app.handle();
    theme::follow_omarchy(move || handle.update(|s: &mut app::App, _| s.look.reload()));
    // Automatic updates: a newer release is downloaded and installed for the next launch.
    let handle = app.handle();
    update::every_so_often(move || {
        handle.update(|s: &mut app::App, _| {
            if s.updates.auto {
                s.check_for_updates();
            }
            // Model prices, for the cost shown on Usage and Runs.
            s.refresh_prices();
        });
    });
    // Schedules: what is due fires now (catching up on what was missed) and every half minute.
    let handle = app.handle();
    schedules::every_tick(move || {
        handle.update(|s: &mut app::App, _| s.fire_due(app::now()));
    });
    app.run();
}
