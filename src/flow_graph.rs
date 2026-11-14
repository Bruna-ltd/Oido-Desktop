//! The workflow editor's graph view: which view is showing, what is selected, and the camera. The
//! drawing is `ui/workflow_graph.rs` and where things go is `workflow/layout.rs`.

use rune::{Drag, Live, Phase, Tx};

use crate::app::App;
use crate::workflow::editor::{KINDS, Loc, Place};
use crate::workflow::engine::{Ending, StepLog};
use crate::workflow::layout::{self, Dir, Drop};
use crate::workflow::{Pause, RunStatus};

/// Zoom stays where text can still be read: the smallest type on a node is 11 pt.
pub const ZOOM_MIN: f32 = 0.7;
pub const ZOOM_MAX: f32 = 1.4;
/// Below this a node drops its second line instead of shrinking it further.
pub const ZOOM_COMPACT: f32 = 0.85;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum View {
    /// Every step as a form, one under the other.
    Cards,
    /// The steps as boxes and wires, with the selected one's form beside them.
    #[default]
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
    /// A step being dragged, or the background being dragged to pan.
    pub drag: Option<StepDrag>,
    panning: Option<[f32; 2]>,
    /// The right-click menu: the step it is for, and where in the window it opened.
    pub menu: Option<(Loc, [f32; 2])>,
    /// Show the workflow's last run on the steps, when no test run is.
    pub show_last: bool,
}

/// A run drawn on the steps: its log, where it waits, and if it can be answered, how.
pub struct Shown {
    pub label: &'static str,
    pub lines: Vec<StepLog>,
    /// The id of the `wait` step it is stopped at.
    pub waiting: Option<String>,
    /// A run that waits for you: its workflow and its own id, and what it waits for.
    pub answer: Option<(String, String, Pause)>,
}

/// A step being dragged: where it is from, where the pointer is in the drawing, and where it
/// would land.
#[derive(Clone, PartialEq, Debug)]
pub struct StepDrag {
    pub from: Loc,
    /// Where in the step the pointer took hold of it, so it is carried by the same spot.
    pub grab: [f32; 2],
    /// The pointer has gone far enough from where it was pressed for this to be a drag.
    pub moving: bool,
    pub point: [f32; 2],
    pub drop: Option<Drop>,
    /// Option is held: it would leave a copy.
    pub copy: bool,
}

/// How far the pointer goes, in points, before a press on a step is a drag.
const DRAG_FROM: f32 = 3.0;

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
            drag: None,
            panning: None,
            menu: None,
            show_last: false,
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

    /// The run drawn on the steps: the test run if there is one (as it goes, or as it ended), else
    /// the workflow's last run if that is asked for and this is its workflow.
    pub fn shown_run(&self) -> Option<Shown> {
        let t = &self.wf_test;
        if t.running.is_some() {
            return Some(Shown {
                label: "Test run, going",
                lines: t.live.clone(),
                waiting: None,
                answer: None,
            });
        }
        if let Some(r) = &t.result {
            let waiting = match &r.ending {
                Ending::Waiting { step, .. } => Some(step.clone()),
                _ => None,
            };
            return Some(Shown {
                label: "Test run",
                lines: r.steps.clone(),
                waiting,
                answer: None,
            });
        }
        let mine = self.wf_editor.as_ref()?.workflow.as_ref()?;
        let last = self
            .wf_runs
            .first()
            .filter(|_| self.wf_graph.show_last && self.wf_open.as_ref() == Some(mine))?;
        Some(Shown {
            label: if last.status == RunStatus::Running {
                "Last run, going"
            } else {
                "Last run"
            },
            lines: last.steps.clone(),
            waiting: last.pause.as_ref().map(|p| p.step.clone()),
            answer: last
                .pause
                .clone()
                .map(|p| (last.workflow_id.clone(), last.id.clone(), p)),
        })
    }

    /// Whether there is a last run to show on the steps.
    pub fn has_last_run(&self) -> bool {
        let mine = self.wf_editor.as_ref().and_then(|ed| ed.workflow.as_ref());
        mine.is_some() && self.wf_open.as_ref() == mine && !self.wf_runs.is_empty()
    }

    /// Takes the run off the steps.
    pub fn hide_run(&mut self) {
        self.wf_test.result = None;
        self.wf_test.live.clear();
        self.wf_graph.show_last = false;
    }

    /// The point in the drawing at the point `q` of the view, undoing the camera.
    pub fn world_point(&self, q: [f32; 2]) -> [f32; 2] {
        let [w, h] = self.world_size();
        let (cx, cy, x0) = (w / 2.0, h / 2.0, self.world_origin());
        let g = &self.wf_graph;
        [
            cx + (q[0] - x0 - cx - g.pan[0]) / g.zoom,
            cy + (q[1] - cy - g.pan[1]) / g.zoom,
        ]
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

    /// A drag in the drawing: on a step it moves the step (or copies it, with Option held); on
    /// the background it pans.
    pub fn drag_graph(&mut self, d: &Drag, tx: &mut Tx) {
        let point = self.world_point([d.x, d.y]);
        let Some(ed) = self.wf_editor.as_ref() else {
            return;
        };
        match d.phase {
            Phase::Down => {
                let lay = layout::layout(ed);
                match lay.hit(point).map(|i| (i.loc.clone(), i.rect)) {
                    Some((from, rect)) => {
                        self.wf_graph.selected = Some(from.clone());
                        self.wf_graph.drag = Some(StepDrag {
                            from,
                            grab: [point[0] - rect.x, point[1] - rect.y],
                            moving: false,
                            point,
                            drop: None,
                            copy: false,
                        });
                    }
                    None => self.wf_graph.panning = Some(self.wf_graph.pan),
                }
            }
            Phase::Move => {
                if let Some(start) = self.wf_graph.panning {
                    let g = &mut self.wf_graph;
                    g.set_camera([start[0] + d.dx, start[1] + d.dy], g.zoom);
                    tx.live_only();
                    return;
                }
                let lay = layout::layout(ed);
                let copy = tx.mods().alt;
                if let Some(drag) = &mut self.wf_graph.drag {
                    drag.moving |= d.dx.hypot(d.dy) > DRAG_FROM;
                    drag.point = point;
                    drag.copy = copy;
                    drag.drop = drag
                        .moving
                        .then(|| lay.drop_target(&drag.from, point))
                        .flatten();
                }
            }
            Phase::Up => {
                self.wf_graph.panning = None;
                let Some(drag) = self.wf_graph.drag.take() else {
                    return;
                };
                if d.cancelled() || !drag.moving {
                    return;
                }
                if let Some(drop) = drag.drop.filter(|x| !x.unchanged || drag.copy) {
                    self.drop_step(&drag.from, &drop, drag.copy);
                }
            }
        }
    }

    /// Puts the dragged step where it was dropped, or a copy of it, and selects it there.
    fn drop_step(&mut self, from: &Loc, drop: &Drop, copy: bool) {
        let Some(ed) = self.wf_editor.as_mut() else {
            return;
        };
        let to = Loc {
            place: drop.place.clone(),
            index: drop.index,
        };
        let done = if copy {
            ed.duplicate(from, &to)
        } else {
            ed.move_to(from, &to)
        };
        if let Ok(at) = done {
            ed.check();
            self.wf_graph.selected = Some(at);
        }
    }

    /// Where a step added from the Add step button goes: after the selected one, else at the end
    /// of the top list.
    pub fn add_position(&self) -> (Place, usize) {
        match self.selected_step() {
            Some(at) => (at.place, at.index + 1),
            None => (
                Vec::new(),
                self.wf_editor.as_ref().map_or(0, |ed| ed.steps.len()),
            ),
        }
    }

    /// Opens the list of steps to add at the position the Add step button would use.
    pub fn open_add_menu(&mut self) {
        let (place, index) = self.add_position();
        self.wf_menu = Some(crate::workflow::editor::Menu::AddAt(place, index));
    }

    /// Adds a step of the kind with this label at a position, selects it, and closes the menu.
    pub fn editor_add_at(&mut self, place: &Place, index: usize, label: &str) {
        let kind = KINDS.iter().find(|(_, l)| *l == label).map(|(k, _)| *k);
        if let (Some(ed), Some(kind)) = (&mut self.wf_editor, kind)
            && let Some(at) = ed.add_at(place, index, kind)
        {
            ed.check();
            self.wf_graph.selected = Some(at);
        }
        self.wf_menu = None;
    }

    /// Deletes the selected step.
    pub fn delete_selected(&mut self) {
        let Some(at) = self.selected_step() else {
            return;
        };
        if let Some(ed) = &mut self.wf_editor {
            ed.remove(&at);
            ed.check();
        }
        // What was after it, or before it, or nothing.
        self.wf_graph.selected = self.wf_editor.as_ref().and_then(|ed| {
            [
                at.clone(),
                Loc {
                    index: at.index.saturating_sub(1),
                    ..at.clone()
                },
            ]
            .into_iter()
            .find(|l| layout::layout(ed).find(l).is_some())
        });
    }

    /// Moves the selected step up or down among its list, and keeps it selected.
    pub fn shift_selected(&mut self, by: isize) {
        let Some(at) = self.selected_step() else {
            return;
        };
        let Some(ed) = &mut self.wf_editor else {
            return;
        };
        let len = ed.block_mut(&at.place).map_or(0, |list| list.len());
        let Some(to) = at.index.checked_add_signed(by).filter(|t| *t < len) else {
            return;
        };
        ed.shift(&at, by);
        ed.check();
        self.wf_graph.selected = Some(Loc { index: to, ..at });
    }

    /// Puts a copy of the selected step right after it, and selects the copy.
    pub fn duplicate_selected(&mut self) {
        let Some(at) = self.selected_step() else {
            return;
        };
        let to = Loc {
            place: at.place.clone(),
            index: at.index + 1,
        };
        if let Some(ed) = &mut self.wf_editor
            && let Ok(copy) = ed.duplicate(&at, &to)
        {
            ed.check();
            self.wf_graph.selected = Some(copy);
        }
    }

    /// Takes back the last change to the steps.
    pub fn undo_editor(&mut self) {
        if let Some(ed) = &mut self.wf_editor {
            ed.undo();
            ed.check();
        }
    }

    pub fn redo_editor(&mut self) {
        if let Some(ed) = &mut self.wf_editor {
            ed.redo();
            ed.check();
        }
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

    fn ids(app: &App, place: &[(usize, usize)]) -> Vec<String> {
        let mut list = &app.wf_editor.as_ref().unwrap().steps;
        for &(step, block) in place {
            list = &list[step].blocks[block].steps;
        }
        list.iter().map(|n| n.id.text.clone()).collect()
    }

    #[test]
    fn a_dropped_step_moves_or_is_copied_to_where_the_drop_says_and_is_selected_there() {
        let mut app = app_with_editor();
        let lay = layout::layout(app.wf_editor.as_ref().unwrap());
        let t = lay.find(&at(&[(1, 0)], 0)).unwrap().rect;
        // Drag `a` into the then list, after `t`.
        let drop = lay
            .drop_target(&at(&[], 0), [t.centre_x(), t.bottom() + 3.0])
            .unwrap();
        app.drop_step(&at(&[], 0), &drop, false);
        assert_eq!(
            (ids(&app, &[]), ids(&app, &[(0, 0)])),
            (vec!["i".to_owned()], vec!["t".to_owned(), "a".to_owned()])
        );
        assert_eq!(app.selected_step(), Some(at(&[(0, 0)], 1)));
        app.undo_editor();
        assert_eq!(ids(&app, &[]), ["a", "i"]);
        // With Option held it leaves a copy, which has an id of its own.
        app.drop_step(&at(&[], 0), &drop, true);
        assert_eq!(ids(&app, &[]), ["a", "i"]);
        let copy = ids(&app, &[(1, 0)]);
        assert_eq!(copy.len(), 2);
        assert_ne!(copy[1], "a");
        assert!(app.wf_editor.as_ref().unwrap().definition().is_ok());
    }

    #[test]
    fn delete_duplicate_and_shift_work_on_the_selected_step_and_keep_something_selected() {
        let mut app = app_with_editor();
        app.select_step(Some(at(&[], 0)));
        app.duplicate_selected();
        assert_eq!(
            app.selected_step(),
            Some(at(&[], 1)),
            "the copy is selected"
        );
        assert_eq!(ids(&app, &[]).len(), 3);
        app.shift_selected(1);
        assert_eq!(app.selected_step(), Some(at(&[], 2)));
        app.shift_selected(1);
        assert_eq!(app.selected_step(), Some(at(&[], 2)), "at the end it stays");
        app.shift_selected(-3);
        assert_eq!(
            app.selected_step(),
            Some(at(&[], 2)),
            "past the start it stays"
        );
        app.shift_selected(-1);
        assert_eq!(app.selected_step(), Some(at(&[], 1)));
        app.delete_selected();
        assert_eq!(ids(&app, &[]).len(), 2);
        assert!(
            app.selected_step().is_some(),
            "the step after it, now in its place"
        );
        app.delete_selected();
        app.delete_selected();
        assert_eq!(app.selected_step(), None, "nothing left to select");
        for _ in 0..3 {
            app.undo_editor();
        }
        assert_eq!(ids(&app, &[]).len(), 3, "each change took one undo");
        app.redo_editor();
        assert_eq!(ids(&app, &[]).len(), 2);
    }

    #[test]
    fn a_step_is_added_after_the_selected_one_or_at_the_end_and_selected() {
        let mut app = app_with_editor();
        assert_eq!(app.add_position(), (vec![], 2), "the end of the top list");
        app.editor_add_at(&vec![], usize::MAX, "Set values");
        assert_eq!(app.selected_step(), Some(at(&[], 2)));
        app.select_step(Some(at(&[], 0)));
        assert_eq!(app.add_position(), (vec![], 1));
        let (place, index) = app.add_position();
        app.editor_add_at(&place, index, "Run a tool");
        assert_eq!(app.selected_step(), Some(at(&[], 1)));
        assert_eq!(ids(&app, &[]).len(), 4);
        app.select_step(Some(at(&[(2, 0)], 0)));
        assert_eq!(
            app.add_position(),
            (vec![(2, 0)], 1),
            "inside a frame, after the step there"
        );
    }

    fn log_line(path: &str, id: &str) -> StepLog {
        StepLog {
            id: id.into(),
            kind: "tool".into(),
            what: String::new(),
            output: String::new(),
            error: None,
            ms: 1,
            path: path.into(),
            rounds: Vec::new(),
            running: false,
        }
    }

    #[test]
    fn the_graph_shows_the_test_run_as_it_goes_then_as_it_ended_and_a_run_that_is_hidden_is_gone() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let mut app = app_with_editor();
        assert!(app.shown_run().is_none());
        // Going: what the worker last said, and only if it is the test the editor still shows.
        let flag = Arc::new(AtomicBool::new(false));
        app.wf_test.running = Some(Arc::clone(&flag));
        app.test_progress(&flag, vec![log_line("0", "a")]);
        let shown = app.shown_run().unwrap();
        assert!(shown.lines.len() == 1 && shown.label == "Test run, going");
        app.test_progress(&Arc::new(AtomicBool::new(false)), vec![]);
        assert_eq!(
            app.shown_run().unwrap().lines.len(),
            1,
            "another test's news is ignored"
        );
        // Ended: its own log, and nothing is waiting unless it ended at a wait.
        app.wf_test.running = None;
        app.wf_test.live.clear();
        app.wf_test.result = Some(crate::workflow::engine::Outcome {
            ending: Ending::Waiting {
                step: "a".into(),
                prompt: "ok?".into(),
                until: None,
            },
            output: String::new(),
            steps: vec![log_line("0", "a")],
            usage: Default::default(),
            snapshot: None,
        });
        let shown = app.shown_run().unwrap();
        assert!(
            shown.label == "Test run"
                && shown.waiting.as_deref() == Some("a")
                && shown.answer.is_none()
        );
        app.hide_run();
        assert!(app.shown_run().is_none());
    }

    #[test]
    fn the_last_run_shows_only_when_asked_for_and_only_for_the_workflow_in_the_editor() {
        use crate::workflow::{Pause, RunRecord};
        let mut app = app_with_editor();
        let ed = app.wf_editor.as_mut().unwrap();
        ed.workflow = Some("w1".into());
        app.wf_runs = vec![RunRecord {
            id: "r1".into(),
            workflow_id: "w1".into(),
            started_by: "you".into(),
            status: RunStatus::Waiting,
            inputs: "{}".into(),
            output: String::new(),
            failed_step: String::new(),
            error: String::new(),
            steps: vec![log_line("0", "a")],
            input_tokens: 0,
            output_tokens: 0,
            started_at: 0,
            finished_at: None,
            pause: Some(Pause {
                step: "a".into(),
                prompt: "Go on?".into(),
                until: None,
            }),
        }];
        assert!(!app.has_last_run(), "the page is not on this workflow");
        app.wf_open = Some("w1".into());
        assert!(app.has_last_run());
        assert!(app.shown_run().is_none(), "not until it is asked for");
        app.wf_graph.show_last = true;
        let shown = app.shown_run().unwrap();
        let (wf, run, pause) = shown.answer.unwrap();
        assert_eq!(
            (wf.as_str(), run.as_str(), pause.prompt.as_str()),
            ("w1", "r1", "Go on?")
        );
        assert_eq!(shown.waiting.as_deref(), Some("a"));
        // Another workflow's runs are not drawn on this one.
        app.wf_open = Some("other".into());
        assert!(app.shown_run().is_none());
    }
}
