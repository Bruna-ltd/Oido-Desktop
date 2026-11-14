//! OidoDesktop: the native Oido Studio. `cargo run` opens the window;
//! `cargo run -- --snap out.png` renders it headless.

mod agent_tool;
mod agents;
mod app;
mod avatars;
mod background;
mod browser;
mod channels;
mod chatgpt;
mod command;
mod completion;
mod composer;
mod connect;
mod context;
mod credentials;
mod delegate;
#[cfg(any(test, debug_assertions))]
mod demo;
mod files;
mod flow_editor;
mod flow_graph;
mod flows;
mod folders;
mod guard;
mod instance;
mod launcher;
mod listing;
mod loopback;
mod mcp;
mod model;
mod models;
mod notify;
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
mod tray;
mod typesafe;
mod ui;
mod update;
mod usage;
mod vault;
mod web;
mod window;
mod workflow;

use app::App;
use rune::{AppSpec, Chrome, KeyEvent, Tx};
use rune_kit::shell;

fn on_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if shell::is_sidebar_toggle(ev) {
        s.sidebar.toggle();
        return true;
    }
    if ev.is_command('i') && ev.mods.alt {
        s.inspector.toggle();
        return true;
    }
    // Closing the window leaves the app in the tray; this quits it.
    if ev.is_command('q') {
        tx.quit();
        return true;
    }
    false
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("oido-desktop {}", update::VERSION);
        return;
    }
    // Adds Oido to the app launcher and stops, so an install need not open the window to do it.
    if args.iter().any(|a| a == "--register") {
        if let Err(e) = launcher::register() {
            eprintln!("oido: could not add Oido to the app launcher: {e}");
            std::process::exit(1);
        }
        return;
    }
    let headless = rune::is_headless();
    // One app per database: launched again, Oido shows the window it already has.
    let claim = if headless {
        None
    } else {
        match instance::claim() {
            instance::Launch::Run(claim) => claim,
            instance::Launch::Exit => return,
        }
    };
    if cfg!(not(debug_assertions)) && !headless {
        launcher::install();
    }
    rune::set_app_id(launcher::APP_ID);
    let chrome = Chrome {
        full_size_content: true,
        traffic_lights: Some((16.0, 25.0)),
        drag_regions: true,
    };
    let mut app = rune::App::new(AppSpec {
        on_key: Some(on_key),
        // Named screens for headless renders; a release build has none.
        #[cfg(debug_assertions)]
        scenes: demo::SCENES,
        ..AppSpec::new("Oido", (1280.0, 800.0), app::init, ui::view)
    })
    .chrome(chrome)
    // Notifications about finished replies only come while the window is out of sight.
    .on_focus(|s: &mut App, focused, _| s.focused = focused)
    // Nothing follows the commands, background processes and local MCP servers once the app is
    // gone, so they end with it.
    .on_quit(|s: &mut App, _| {
        s.processes.stop_all();
        command::end_all();
        mcp::client::stop_all();
    });
    // Worker threads (checking a key, refreshing models) hand their results back through this.
    app::set_handle(app.handle());
    if let Some(claim) = claim {
        let handle = app.handle();
        instance::listen(claim, move |token| handle.activate(token));
    }
    // With an icon in the tray, closing the window keeps the app running; without one it quits.
    if !headless && tray::start(app.handle()) {
        app = app.keep_running();
    }
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
        handle.update(|s: &mut app::App, _| {
            s.fire_due(app::now());
            // Workflows waiting for a long delay go on once it is over.
            s.resume_due(app::now());
            // Calls held for a go-ahead nobody gave in time do not run.
            s.expire_holds(app::now());
            // Subscription tokens near expiry are refreshed before a call needs them.
            s.refresh_subscription_tokens();
        });
    });
    app.run();
}
