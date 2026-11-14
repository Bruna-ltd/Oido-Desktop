//! The workflow editor's graph view: which view is showing, what is selected, and the camera. The
//! drawing is `ui/workflow_graph.rs` and where things go is `workflow/layout.rs`.

use rune::Live;

use crate::app::App;
use crate::workflow::editor::Loc;
use crate::workflow::layout::{self, Dir};

/// Zoom stays where text can still be read: the smallest type on a node is 11 pt.
pub const ZOOM_MIN: f32 = 0.7;
pub const ZOOM_MAX: f32 = 1.4;
/// Below this a node drops its second line instead of shrinking it further.
pub const ZOOM_COMPACT: f32 = 0.85;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum View {
    /// Every step as a form, one under the other.
    #[default]
    Cards,
    /// The steps as boxes and wires, with the selected one's form beside them.
    Graph,
}

/// The graph view's state. The camera is two [`Live`] values, so a pan or a pinch moves the
/// drawing without running the view again.
pub struct GraphView {
    pub shown: View,
    pub selected: Option<Loc>,
    pub pan: [f32; 2],
    pub zoom: f32,
    pub live_pan: Live<[f32; 2]>,
    pub live_zoom: Live<f32>,
    /// Where the view is drawn, as the last frame reported it.
    pub viewport: [f32; 4],
    /// The drawing is to be fitted to the view as soon as the view's size is known.
    pub needs_fit: bool,
}

impl Default for GraphView {
    fn default() -> Self {
        GraphView {
            shown: View::default(),
            selected: None,
            pan: [0.0, 0.0],
            zoom: 1.0,
            live_pan: Live::new([0.0, 0.0]),
            live_zoom: Live::new(1.0),
            viewport: [0.0; 4],
            needs_fit: true,
        }
    }
}

impl GraphView {
    fn set_camera(&mut self, pan: [f32; 2], zoom: f32) {
        (self.pan, self.zoom) = (pan, zoom.clamp(ZOOM_MIN, ZOOM_MAX));
        self.live_pan.set(self.pan);
        self.live_zoom.set(self.zoom);
    }

    /// Whether nodes show only their first line at this zoom.
    pub fn compact(&self) -> bool {
        self.zoom < ZOOM_COMPACT
    }
}

impl App {
    pub fn set_view(&mut self, view: View) {
        if view == View::Graph && self.wf_graph.shown != View::Graph {
            self.wf_graph.needs_fit = true;
        }
        self.wf_graph.shown = view;
        self.wf_menu = None;
    }

    /// Selects a step, or nothing.
    pub fn select_step(&mut self, at: Option<Loc>) {
        self.wf_graph.selected = at;
        self.wf_menu = None;
    }

    /// The selection, if it is still a step the editor has.
    pub fn selected_step(&self) -> Option<Loc> {
        let ed = self.wf_editor.as_ref()?;
        let at = self.wf_graph.selected.clone()?;
        layout::layout(ed).find(&at).map(|_| at)
    }

    /// An arrow key in the graph: moves the selection along the tree, starting at the first step
    /// when nothing is selected.
    pub fn move_selection(&mut self, dir: Dir) {
        let Some(ed) = self.wf_editor.as_ref() else {
            return;
        };
        let next = match self.selected_step() {
            Some(from) => layout::step_from(ed, &from, dir),
            None => ed.steps.first().map(|_| Loc {
                place: Vec::new(),
                index: 0,
            }),
        };
        if let Some(at) = next {
            self.wf_graph.selected = Some(at);
        }
    }

    /// The drawing's size, for working out where the camera is.
    fn world_size(&self) -> [f32; 2] {
        self.wf_editor.as_ref().map_or([0.0; 2], |ed| {
            let l = layout::layout(ed);
            [l.width, l.height]
        })
    }

    /// Where the drawing's top-left is in the view, before pan and zoom: it is centred across.
    pub(crate) fn world_origin(&self) -> f32 {
        ((self.wf_graph.viewport[2] - self.world_size()[0]) / 2.0).max(0.0)
    }

    /// The wheel or a two-finger scroll moves the drawing.
    pub fn pan_graph(&mut self, dx: f32, dy: f32) {
        let g = &mut self.wf_graph;
        g.set_camera([g.pan[0] + dx, g.pan[1] + dy], g.zoom);
    }

    /// A pinch, or Ctrl and the wheel, zooms about the pointer at `(x, y)` in the view, so the
    /// point under it stays there. True when that crossed the size at which nodes drop their
    /// second line, which the drawing has to be made again for.
    pub fn zoom_graph(&mut self, x: f32, y: f32, delta: f32) -> bool {
        let [w, h] = self.world_size();
        let (cx, cy, x0) = (w / 2.0, h / 2.0, self.world_origin());
        let g = &mut self.wf_graph;
        let (px, py) = (
            cx + (x - x0 - cx - g.pan[0]) / g.zoom,
            cy + (y - cy - g.pan[1]) / g.zoom,
        );
        let before = g.zoom;
        let zoom = (g.zoom * (1.0 + delta)).clamp(ZOOM_MIN, ZOOM_MAX);
        g.set_camera(
            [x - x0 - cx - (px - cx) * zoom, y - cy - (py - cy) * zoom],
            zoom,
        );
        (before < ZOOM_COMPACT) != (zoom < ZOOM_COMPACT)
    }

    /// Makes the drawing as big as fits the view, up to full size and down to where text can
    /// still be read, and shows its top; centred across if it fits, else from its left edge.
    pub fn fit_graph(&mut self) {
        let [w, h] = self.world_size();
        let v = self.wf_graph.viewport;
        if v[2] <= 0.0 || v[3] <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let zoom = (v[2] / w).min(v[3] / h).clamp(ZOOM_MIN, 1.0);
        let left = ((v[2] - w * zoom) / 2.0).max(0.0);
        let pan = [
            left - self.world_origin() - (w / 2.0) * (1.0 - zoom),
            -(h / 2.0) * (1.0 - zoom),
        ];
        self.wf_graph.set_camera(pan, zoom);
        self.wf_graph.needs_fit = false;
    }

    /// Fits the drawing once, when the view has said how big it is.
    pub fn fit_graph_if_needed(&mut self) {
        if self.wf_graph.needs_fit {
            self.fit_graph();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::workflow::Saved;
    use crate::workflow::editor::Editor;

    fn app_with_editor() -> App {
        let mut app = crate::testing::samples();
        let w = Saved {
            id: "w1".into(),
            agent_id: "a1".into(),
            name: "Flow".into(),
            goal: String::new(),
            definition: serde_json::from_value(json!({"steps": [
                {"id": "a", "kind": "tool", "tool": "shell", "args": {"command": "echo a"}},
                {"id": "i", "kind": "if", "cond": "true",
                 "then": [{"id": "t", "kind": "tool", "tool": "shell", "args": {"command": "echo t"}}],
                 "else": []}
            ]}))
            .unwrap(),
            created_at: 0,
        };
        app.wf_editor = Some(Editor::from_saved(&w));
        app
    }

    fn at(place: &[(usize, usize)], index: usize) -> Loc {
        Loc {
            place: place.to_vec(),
            index,
        }
    }

    #[test]
    fn the_arrow_keys_start_at_the_first_step_and_move_along_the_tree() {
        let mut app = app_with_editor();
        assert_eq!(app.selected_step(), None);
        app.move_selection(Dir::Down);
        assert_eq!(
            app.selected_step(),
            Some(at(&[], 0)),
            "nothing selected: the first step"
        );
        app.move_selection(Dir::Down);
        assert_eq!(app.selected_step(), Some(at(&[], 1)));
        app.move_selection(Dir::Right);
        assert_eq!(app.selected_step(), Some(at(&[(1, 0)], 0)));
        app.move_selection(Dir::Left);
        assert_eq!(app.selected_step(), Some(at(&[], 1)));
        app.move_selection(Dir::Down);
        assert_eq!(app.selected_step(), Some(at(&[], 1)), "at the end it stays");
        app.select_step(None);
        assert_eq!(app.selected_step(), None);
    }

    #[test]
    fn a_selection_the_editor_no_longer_has_is_no_selection() {
        let mut app = app_with_editor();
        app.select_step(Some(at(&[(1, 0)], 0)));
        assert!(app.selected_step().is_some());
        app.wf_editor.as_mut().unwrap().steps[1].blocks[0]
            .steps
            .clear();
        assert_eq!(app.selected_step(), None);
        app.select_step(Some(at(&[], 9)));
        assert_eq!(app.selected_step(), None);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer_and_stays_where_text_is_readable() {
        let mut app = app_with_editor();
        let size = app.world_size();
        app.wf_graph.viewport = [0.0, 0.0, 1000.0, 800.0];
        let (x, y) = (480.0, 300.0);
        // The drawing's point under (x, y), by the camera as it is, and where it ends up.
        let under = |app: &App| {
            let g = &app.wf_graph;
            let (cx, cy, x0) = (size[0] / 2.0, size[1] / 2.0, app.world_origin());
            [
                cx + (x - x0 - cx - g.pan[0]) / g.zoom,
                cy + (y - cy - g.pan[1]) / g.zoom,
            ]
        };
        let before = under(&app);
        assert!(!app.zoom_graph(x, y, 0.2), "bigger, still full lines");
        assert!((app.wf_graph.zoom - 1.2).abs() < 1e-4);
        let after = under(&app);
        assert!((before[0] - after[0]).abs() < 0.01 && (before[1] - after[1]).abs() < 0.01);
        for _ in 0..20 {
            app.zoom_graph(x, y, 0.5);
        }
        assert_eq!(app.wf_graph.zoom, ZOOM_MAX);
        let mut crossed = false;
        for _ in 0..30 {
            crossed |= app.zoom_graph(x, y, -0.1);
        }
        assert_eq!(app.wf_graph.zoom, ZOOM_MIN);
        assert!(
            crossed,
            "going below the size where lines drop is reported once"
        );
        assert!(app.wf_graph.compact());
        app.fit_graph();
        assert_eq!(
            (app.wf_graph.zoom, app.wf_graph.pan),
            (1.0, [0.0, 0.0]),
            "a small drawing is centred at full size"
        );
        // One wider than the view is shrunk to fit, but not past where text can be read.
        app.wf_graph.viewport = [0.0, 0.0, 200.0, 800.0];
        app.fit_graph();
        assert_eq!(app.wf_graph.zoom, ZOOM_MIN);
        assert!(!app.wf_graph.needs_fit);
        let was = app.wf_graph.pan;
        app.pan_graph(-30.0, 12.0);
        assert_eq!(app.wf_graph.pan, [was[0] - 30.0, was[1] + 12.0]);
    }
}
