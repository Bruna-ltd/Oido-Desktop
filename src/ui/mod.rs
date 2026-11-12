//! The window: Studio's shell (agents and sections on the left, the screen in the middle, details
//! on the right) built from rune-kit's `SplitView`.

mod agent_form;
mod avatar;
mod channels;
mod chat;
mod chat_header;
mod composer;
mod flow;
mod folder_picker;
mod guard;
mod icons;
mod inspector;
mod logo;
mod math;
mod mcp;
mod model_picker;
mod multi_picker;
mod picture;
mod providers;
mod qr;
mod rail;
mod runs;
pub(crate) mod schedules;
mod settings;
mod sidebar;
pub(crate) mod usage;
mod workflows;

use std::time::Duration;

use rune::{Cx, El, Spring, col, row, text};
use rune_kit::shell::{self, PaneMotion, SidebarPeek, SidebarToggle, SplitView, Toolbar};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit};

use crate::app::{App, Route};

/// Waits for the logos of the servers at `urls` to decode, for a headless picture of the page.
#[cfg(debug_assertions)]
pub fn wait_for_logos(urls: &[&str]) {
    logo::wait_all(urls);
}

/// Waits for these agent pictures to decode, for a headless picture of the screen.
#[cfg(debug_assertions)]
pub fn wait_for_avatars<'a>(faces: impl IntoIterator<Item = &'a crate::avatars::Avatar>) {
    avatar::wait_all(faces);
}

/// Waits for attached pictures to decode, for a headless picture of the screen.
#[cfg(debug_assertions)]
pub fn wait_for_pictures(images: &[&crate::prompt::Image]) {
    picture::wait(images);
}

/// Waits for a picture a reply shows to decode, for a headless picture of the screen.
#[cfg(debug_assertions)]
pub fn wait_for_reply_picture(src: &str, folder: &std::path::Path) {
    if let Some(image) = picture::markdown(src, folder) {
        let _ = image.wait();
    }
}

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let k = s.look.kit();
    // Menus, selects and dialogs mount above everything, so each screen adds to this.
    let mut layer = Vec::new();
    let (title, main): (String, El) = match s.route {
        Route::Chat => (String::new(), chat::screen(s, &k, &mut layer)),
        Route::AgentForm => {
            let (title, _) = s.form_labels();
            (title, agent_form::screen(s, &k, &mut layer))
        }
        Route::Runs => ("Runs".to_owned(), runs::screen(s, &k)),
        Route::Providers => ("Providers".to_owned(), providers::screen(s, &k, &mut layer)),
        Route::Workflows => ("Workflows".to_owned(), workflows::screen(s, &k)),
        Route::Schedules => ("Schedules".to_owned(), schedules::screen(s, &k, &mut layer)),
        Route::Usage => ("Usage".to_owned(), usage::screen(s, &k)),
        Route::Mcp => ("MCP".to_owned(), mcp::screen(s, cx, &k)),
        Route::Channels => ("Channels".to_owned(), channels::screen(s, &k, &mut layer)),
        Route::Guardrails => ("Guardrails".to_owned(), guard::screen(s, &k)),
        Route::Settings => ("Settings".to_owned(), settings::screen(s, &k, &mut layer)),
        Route::Soon(name) => (name.to_owned(), soon(name, &k)),
    };
    // Only Runs has a details pane; Chat is the conversation alone, as in Studio.
    let runs = s.route == Route::Runs;
    let mut toolbar = Toolbar::new();
    if s.route != Route::Chat {
        toolbar = toolbar.title(title);
    }
    if s.route == Route::AgentForm {
        toolbar = agent_form::toolbar(s, &k, toolbar);
    }
    if runs {
        toolbar = toolbar.trailing(
            IconButton::new(&shell::INSPECTOR)
                .variant(IconButtonVariant::GhostMuted)
                .expanded(s.inspector.is_open())
                .label(if s.inspector.is_open() {
                    "Hide details"
                } else {
                    "Show details"
                })
                .on_press(|s: &mut App| s.inspector.toggle())
                .build(&k)
                .key("inspector-toggle"),
        );
    }
    let needs_you = s
        .roster
        .list
        .iter()
        .any(|a| a.family_status() == crate::model::AgentStatus::Waiting);
    agent_form::confirm_delete(s, &k, &mut layer);
    let sidebar = sidebar::build(s, &k, &mut layer);
    let mut split = SplitView::new(main)
        .sidebar(&s.sidebar, |s: &mut App| &mut s.sidebar, sidebar)
        .toolbar(toolbar)
        .titlebar_inset(cx.titlebar_inset())
        .frame_inset(8.0)
        .toolbar_in_panel()
        .sidebar_padding(18.0, 10.0)
        .sidebar_toggle(
            SidebarToggle::new()
                .size(ControlSize::Large)
                .past_lights(16.0)
                .edge(18.0)
                .gap(4.0)
                .in_header()
                .unread(needs_you),
        )
        .pane_motion(
            PaneMotion::new()
                .spring(Spring::new(0.3, 0.0))
                .scale_with_width(480.0, 0.15, 0.3)
                .fade_tail(0.15),
        )
        .sidebar_peek(
            SidebarPeek::new()
                .edge(16.0, ms(40))
                .toggle_delay(ms(80))
                .below(56.0)
                .hold(15.0, ms(100))
                .spring(Spring::new(0.18, 0.0)),
        )
        .reduced_motion(cx.is_reduced_motion());
    if runs {
        split = split.inspector(
            &s.inspector,
            |s: &mut App| &mut s.inspector,
            inspector::build(s, &k),
        );
    }
    let split = split.build(&k);
    let window = row()
        .w_full()
        .h_full()
        .fill(k.c.page)
        .child(split.grow(1.0).min_w(0.0))
        .child(rail::build(s, &k));
    rune_kit::root(window, layer)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A Studio section with no native screen yet.
fn soon(name: &str, k: &Kit) -> El {
    col()
        .key("soon")
        .w_full()
        .h_full()
        .center()
        .gap(6.0)
        .child(
            text(name.to_owned())
                .font(k.ty.body_medium)
                .color(k.c.text_base),
        )
        .child(
            text("Not built natively yet.")
                .font(k.ty.small)
                .color(k.c.text_muted),
        )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use rune::{Env, Runtime};

    use super::view;
    use crate::app::{App, Route};

    /// Presses `trigger` and returns where the list is drawn, as (x, y, w, h), with where the
    /// trigger is, as (x, y).
    fn press(app: App, trigger: &str) -> ([f32; 4], (f32, f32)) {
        let mut rt = Runtime::new(
            app,
            view,
            (1280.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        let at = rt.find(trigger).unwrap();
        rt.pointer_down(at.0, at.1);
        rt.pointer_up(at.0, at.1);
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        let list = ["model-content", "default-model-content"]
            .iter()
            .find_map(|k| rt.presented(k))
            .unwrap();
        (list.0, at)
    }

    /// The list sits against its field: below it, or above it when there is no room below, and
    /// starts at the field's left edge. In the window's corner it is nowhere near.
    fn assert_beside(list: [f32; 4], field: (f32, f32)) {
        let [x, y, _, h] = list;
        assert!(
            x > 100.0 && y > 100.0,
            "the list is drawn at ({x}, {y}), the window's corner"
        );
        assert!(
            (x - field.0).abs() < 330.0,
            "it starts at x={x}, far from its field at x={}",
            field.0
        );
        let (below, above) = (y >= field.1, y + h <= field.1 + 30.0);
        assert!(
            below || above,
            "the list (y {y}..{}) overlaps its field at y={}",
            y + h,
            field.1
        );
    }

    #[test]
    fn the_new_agents_model_list_opens_beside_its_field_not_in_the_corner() {
        let mut app = App::new(None);
        crate::demo::demo_models(&mut app);
        app.new_agent();
        let (list, field) = press(app, "model-value-trigger");
        assert_beside(list, field);
    }

    #[test]
    fn the_default_models_list_opens_beside_its_field_when_none_is_set() {
        let mut app = App::new(None);
        crate::demo::demo_models(&mut app);
        app.providers.default_model = None;
        app.route = Route::Providers;
        let (list, field) = press(app, "default-model-value-trigger");
        assert_beside(list, field);
    }

    #[test]
    fn the_edit_forms_model_list_still_opens_beside_its_field() {
        let mut app = crate::testing::samples();
        crate::demo::demo_models(&mut app);
        app.edit_agent(&crate::agents::Target::agent("atlas"));
        app.roster.form.model = Some(crate::models::ModelRef::new(
            "anthropic",
            "claude-sonnet-4-6",
        ));
        let (list, field) = press(app, "model-value-trigger");
        assert_beside(list, field);
    }
}
