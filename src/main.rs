//! OidoDesktop: the native Oido Studio. `cargo run` opens the window;
//! `cargo run -- --snap out.png` renders it headless.

mod agents;
mod app;
mod connect;
mod listing;
mod model;
mod models;
mod persist;
mod picker;
mod providers;
mod store;
#[cfg(test)]
mod testing;
mod theme;
mod ui;
mod vault;

use agents::Target;
use app::{App, Route};
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
        scenes: &[
            ("runs", |s: &mut App| s.route = Route::Runs),
            ("forge", |s: &mut App| {
                s.roster.select(Target::agent("forge"))
            }),
            ("empty", |s: &mut App| {
                s.roster.select(Target::agent("scout"))
            }),
            ("agent-new", |s: &mut App| s.new_agent()),
            ("agent-new-template", |s: &mut App| {
                s.new_agent();
                s.roster.pick_template(0);
            }),
            ("agent-edit", |s: &mut App| {
                s.edit_agent(&Target::agent("atlas"));
                s.roster.form.advanced = true;
            }),
            ("member-new", |s: &mut App| s.new_member("forge")),
            ("menu", |s: &mut App| {
                s.roster.open_menu(Target::agent("forge"), None)
            }),
            ("menu-member", |s: &mut App| {
                s.roster
                    .open_menu(Target::member("forge", "reviewer"), None)
            }),
            // With providers connected and their models listed (made up, for looking at the screens).
            ("models", |s: &mut App| {
                demo_models(s);
                s.route = Route::Providers;
            }),
            ("picker", |s: &mut App| {
                demo_models(s);
                s.model_picker.show(None);
            }),
            ("picker-search", |s: &mut App| {
                demo_models(s);
                s.model_picker.show(None);
                s.model_picker.search = rune::TextEdit::new("son");
            }),
            ("picker-empty", |s: &mut App| {
                s.model_picker.show(None);
            }),
            ("form-model", |s: &mut App| {
                demo_models(s);
                s.edit_agent(&Target::agent("atlas"));
                s.roster.form.model = Some(models::ModelRef::new("anthropic", "claude-sonnet-4-6"));
                s.roster.form.model_picker.show(s.roster.form.model.clone().as_ref());
            }),
            ("model-ready", |s: &mut App| {
                demo_models(s);
                s.set_model(Some(models::ModelRef::new("anthropic", "claude-opus-4-7")));
            }),
            ("model-gone", |s: &mut App| {
                demo_models(s);
                s.set_model(Some(models::ModelRef::new("gemini", "gemini-2.5-pro")));
            }),
            ("confirm-delete", |s: &mut App| {
                s.roster.confirm = Some(Target::agent("forge"))
            }),
            ("providers", |s: &mut App| s.route = Route::Providers),
            ("provider-form", |s: &mut App| {
                s.route = Route::Providers;
                s.providers.toggle("openai");
            }),
            ("provider-error", |s: &mut App| {
                s.route = Route::Providers;
                s.providers.toggle("openai");
                s.providers.fail(
                    "openai",
                    "the system keyring is not available (no secret service), so the key was not saved".into(),
                );
            }),
            ("server-form", |s: &mut App| {
                s.route = Route::Providers;
                s.providers.toggle(providers::NEW_SERVER);
            }),
        ],
        ..AppSpec::new("Oido", (1280.0, 800.0), app::init, ui::view)
    })
    .chrome(chrome);
    // Worker threads (checking a key, refreshing models) hand their results back through this.
    app::set_handle(app.handle());
    app.run();
}

/// Connects a few providers with made-up model lists, as if they had been checked.
fn demo_models(s: &mut App) {
    use models::{ModelRef, RawModel};
    let list = |ids: &[&str]| {
        listing::finish_all(
            ids.iter()
                .map(|id| RawModel {
                    id: (*id).into(),
                    ..RawModel::default()
                })
                .collect(),
        )
    };
    for (id, ids) in [
        (
            "anthropic",
            vec![
                "claude-opus-4-7",
                "claude-sonnet-4-6",
                "claude-haiku-4-5-20251001",
                "claude-3-5-sonnet-20241022",
            ],
        ),
        (
            "openai",
            vec![
                "gpt-4o",
                "gpt-4o-mini",
                "text-embedding-3-large",
                "whisper-1",
            ],
        ),
        (
            "deepseek",
            vec!["deepseek-v4-pro", "deepseek-reasoner", "deepseek-v3.2"],
        ),
    ] {
        s.providers.connect(id);
        s.providers.set_models(id, list(&ids), app::now());
        s.providers.set_hint(id, "sk-demo-1234567890");
    }
    s.providers.default_model = Some(ModelRef::new("anthropic", "claude-sonnet-4-6"));
}
