//! OidoDesktop: the native Oido Studio. `cargo run` opens the window;
//! `cargo run -- --snap out.png` renders it headless.

mod agents;
mod app;
mod command;
mod completion;
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
mod providers;
mod run;
mod runs;
mod skills;
mod store;
#[cfg(test)]
mod testing;
mod theme;
mod tools;
mod typesafe;
mod ui;
mod vault;
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
    app.run();
}
