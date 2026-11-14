//! Where each step goes when a workflow is drawn as a graph, and which step the arrow keys move to.
//! Pure: it reads the editor's tree and gives rectangles and wires, so it is tested without a
//! screen. The screen is `ui/workflow_graph.rs`.
//!
//! Steps flow down one spine. A step that holds lists of steps is a frame around them: the lists
//! sit side by side as columns, each a spine of its own, with a label above it (`Then`,
//! `Otherwise`, a branch's name). Wires join neighbours only, so nothing can loop back: a loop is a
//! frame, and a join under a frame is the frame's end.

use super::definition::{DECIDE, GENERATE, plain};
use super::editor::KINDS;
use super::editor::{Editor, Loc, Node, Part, Place};

pub const NODE_W: f32 = 240.0;
pub const NODE_H: f32 = 56.0;
/// Between a step and the next in a spine.
pub const GAP: f32 = 32.0;
/// Inside a frame, around its columns.
pub const PAD: f32 = 16.0;
/// A frame's title bar.
pub const HEAD: f32 = 28.0;
/// Between a frame's columns.
pub const COL_GAP: f32 = 24.0;
/// Above a column: its label and the space before its first step.
pub const LABEL_H: f32 = 24.0;
/// The chip that names a column.
pub const CHIP_H: f32 = 20.0;
/// Under a frame's columns: room for the join line and an add button on it.
pub const FOOT: f32 = 28.0;
/// How far under a list's last step the button that adds one after it sits.
const END_GAP: f32 = 14.0;
/// An empty list's slot.
pub const EMPTY_H: f32 = 36.0;
/// Around the whole drawing.
pub const MARGIN: f32 = 24.0;
pub const START_W: f32 = 88.0;
pub const START_H: f32 = 28.0;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn centre_x(&self) -> f32 {
        self.x + self.w / 2.0
    }

    /// Whether the point is inside.
    pub fn has(&self, p: [f32; 2]) -> bool {
        p[0] >= self.x && p[0] <= self.x + self.w && p[1] >= self.y && p[1] <= self.bottom()
    }
}

#[cfg(test)]
impl Rect {
    fn right(&self) -> f32 {
        self.x + self.w
    }

    fn contains(&self, other: &Rect) -> bool {
        other.x >= self.x - 0.01
            && other.y >= self.y - 0.01
            && other.right() <= self.right() + 0.01
            && other.bottom() <= self.bottom() + 0.01
    }

    fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.right() - 0.01
            && other.x < self.right() - 0.01
            && self.y < other.bottom() - 0.01
            && other.y < self.bottom() - 0.01
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    /// One step.
    Step,
    /// A step that holds lists of steps, drawn around them.
    Frame,
    /// The place a list with nothing in it will take its first step.
    Empty,
}

/// One box of the drawing.
#[derive(Clone, PartialEq, Debug)]
pub struct Item {
    /// The step; for an empty slot, the list it is in, at position 0.
    pub loc: Loc,
    pub form: Form,
    pub kind: String,
    /// The step's id, as typed.
    pub id: String,
    /// What the step does, in a line.
    pub title: String,
    /// Its id, and what it keeps its result as.
    pub detail: String,
    /// Whether a model works in it.
    pub model: bool,
    pub rect: Rect,
}

/// The name of a column, drawn as a chip at `rect`.
#[derive(Clone, PartialEq, Debug)]
pub struct Label {
    pub text: String,
    pub rect: Rect,
}

/// A wire from one point to another, drawn as a curve when they are not in line.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Wire {
    pub from: [f32; 2],
    pub to: [f32; 2],
}

/// A list of steps (the top one, or one inside a frame) as an area of the drawing: where a step
/// dropped there would go among the others.
#[derive(Clone, PartialEq, Debug)]
pub struct ListRegion {
    pub place: Place,
    pub rect: Rect,
    /// Each step's top and bottom, in order. Empty when the list is.
    pub spans: Vec<[f32; 2]>,
}

/// A place a step can be added, drawn as a button on the wire: the list, and the position in it.
#[derive(Clone, PartialEq, Debug)]
pub struct Gap {
    pub place: Place,
    pub index: usize,
    pub at: [f32; 2],
}

/// Where a step dragged to a point would land.
#[derive(Clone, PartialEq, Debug)]
pub struct Drop {
    /// The list and the position in it, counted as the lists are now.
    pub place: Place,
    pub index: usize,
    /// The line to draw between the steps there, or the slot of an empty list.
    pub marker: Rect,
    /// The frame the list is in, to light up.
    pub frame: Option<Loc>,
    /// The step would stay where it is.
    pub unchanged: bool,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Layout {
    pub start: Rect,
    pub items: Vec<Item>,
    pub labels: Vec<Label>,
    pub wires: Vec<Wire>,
    pub lists: Vec<ListRegion>,
    pub gaps: Vec<Gap>,
    pub width: f32,
    pub height: f32,
}

impl Layout {
    pub fn find(&self, loc: &Loc) -> Option<&Item> {
        self.items
            .iter()
            .find(|i| i.form != Form::Empty && i.loc == *loc)
    }

    /// The step under the point: the innermost box, so a step inside a frame and not the frame.
    pub fn hit(&self, p: [f32; 2]) -> Option<&Item> {
        self.items
            .iter()
            .filter(|i| i.form != Form::Empty && i.rect.has(p))
            .min_by(|a, b| {
                (a.rect.w * a.rect.h)
                    .partial_cmp(&(b.rect.w * b.rect.h))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Where the step at `from` would land if dropped at `p`: in the innermost list under the
    /// point that is not inside the step itself, at the position among its steps the point is at.
    pub fn drop_target(&self, from: &Loc, p: [f32; 2]) -> Option<Drop> {
        let depth = from.place.len();
        let region = self
            .lists
            .iter()
            .filter(|r| r.rect.has(p))
            .filter(|r| {
                // Not a list of the step's own, nor one inside it.
                !(r.place.len() > depth
                    && r.place[..depth] == from.place[..]
                    && r.place[depth].0 == from.index)
            })
            .min_by(|a, b| {
                (a.rect.w * a.rect.h)
                    .partial_cmp(&(b.rect.w * b.rect.h))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })?;
        let index = region
            .spans
            .iter()
            .filter(|s| (s[0] + s[1]) / 2.0 < p[1])
            .count();
        let marker = match (
            region.spans.first(),
            region.spans.get(index.wrapping_sub(1)),
            region.spans.get(index),
        ) {
            (None, _, _) => self
                .items
                .iter()
                .find(|i| i.form == Form::Empty && i.loc.place == region.place)
                .map(|i| i.rect)?,
            (Some(first), None, _) => Rect {
                x: region.rect.x + 8.0,
                y: first[0] - GAP / 2.0 - 1.5,
                w: region.rect.w - 16.0,
                h: 3.0,
            },
            (Some(_), Some(before), None) => Rect {
                x: region.rect.x + 8.0,
                y: before[1] + END_GAP - 1.5,
                w: region.rect.w - 16.0,
                h: 3.0,
            },
            (Some(_), Some(before), Some(after)) => Rect {
                x: region.rect.x + 8.0,
                y: (before[1] + after[0]) / 2.0 - 1.5,
                w: region.rect.w - 16.0,
                h: 3.0,
            },
        };
        let frame = region.place.split_last().map(|(&(step, _), rest)| Loc {
            place: rest.to_vec(),
            index: step,
        });
        let unchanged =
            region.place == from.place && (index == from.index || index == from.index + 1);
        Some(Drop {
            place: region.place.clone(),
            index,
            marker,
            frame,
            unchanged,
        })
    }
}

/// The text a step shows: what it does in a sentence, and under it what kind of step it is.
fn words(n: &Node) -> (String, String) {
    let step = n.to_step("", &mut Vec::new());
    let kind = KINDS
        .iter()
        .find(|k| k.kind == n.kind)
        .map_or("", |k| k.label);
    (plain(&step), kind.to_owned())
}

fn is_model(n: &Node) -> bool {
    n.kind == GENERATE || n.kind == DECIDE || (n.kind == "tool" && n.tool == crate::tools::DELEGATE)
}

/// What names a column: nothing when a step has one list, else the part's name.
fn label_of(node: &Node, block: usize) -> Option<String> {
    if node.blocks.len() < 2 {
        return None;
    }
    let b = &node.blocks[block];
    Some(match b.part {
        Part::Then => "Then".to_owned(),
        Part::Else | Part::Default => "Otherwise".to_owned(),
        Part::Case => format!("When {}", short(&b.when.text)),
        Part::Body => "Each round".to_owned(),
        Part::Try => "Try".to_owned(),
        Part::Catch => "If it still fails".to_owned(),
        Part::Branch => match b.when.text.trim() {
            "" => "(unnamed)".to_owned(),
            name => name.to_owned(),
        },
    })
}

fn short(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    let (cut, more) = crate::runs::clip(line, 18);
    if more { format!("{cut}…") } else { cut }
}

/// A step's size: a plain box, or the frame around its columns.
fn measure(node: &Node) -> (f32, f32) {
    if node.blocks.is_empty() {
        return (NODE_W, NODE_H);
    }
    let cols: Vec<(f32, f32)> = node.blocks.iter().map(|b| measure_list(&b.steps)).collect();
    let width: f32 = cols.iter().map(|c| c.0).sum::<f32>() + COL_GAP * (cols.len() - 1) as f32;
    let tallest = cols.iter().map(|c| c.1).fold(0.0, f32::max);
    let label = if node.blocks.len() > 1 { LABEL_H } else { 0.0 };
    (
        width.max(NODE_W) + 2.0 * PAD,
        HEAD + PAD + label + tallest + FOOT,
    )
}

fn measure_list(nodes: &[Node]) -> (f32, f32) {
    if nodes.is_empty() {
        return (NODE_W, EMPTY_H);
    }
    let sizes: Vec<(f32, f32)> = nodes.iter().map(measure).collect();
    (
        sizes.iter().map(|s| s.0).fold(0.0, f32::max),
        sizes.iter().map(|s| s.1).sum::<f32>() + GAP * (sizes.len() - 1) as f32,
    )
}

struct Pen {
    out: Layout,
}

impl Pen {
    /// Lays out the steps of the list at `place` down a spine `width` wide whose left edge is `x`
    /// and top is `y`. Gives the spine's first top and last bottom, as centre-line points.
    fn list(
        &mut self,
        nodes: &[Node],
        place: &Place,
        x: f32,
        y: f32,
        width: f32,
    ) -> ([f32; 2], [f32; 2]) {
        let centre = x + width / 2.0;
        if nodes.is_empty() {
            let rect = Rect {
                x: centre - NODE_W / 2.0,
                y,
                w: NODE_W,
                h: EMPTY_H,
            };
            self.out.lists.push(ListRegion {
                place: place.clone(),
                rect: Rect {
                    x,
                    y,
                    w: width,
                    h: EMPTY_H,
                },
                spans: Vec::new(),
            });
            self.out.items.push(Item {
                loc: Loc {
                    place: place.clone(),
                    index: 0,
                },
                form: Form::Empty,
                kind: String::new(),
                id: String::new(),
                title: "Nothing here yet".to_owned(),
                detail: String::new(),
                model: false,
                rect,
            });
            return ([centre, rect.y], [centre, rect.bottom()]);
        }
        let mut cursor = y;
        let mut previous: Option<[f32; 2]> = None;
        let mut first = [centre, y];
        let mut spans = Vec::with_capacity(nodes.len());
        for (index, node) in nodes.iter().enumerate() {
            let (w, h) = measure(node);
            let rect = Rect {
                x: centre - w / 2.0,
                y: cursor,
                w,
                h,
            };
            let top = [centre, rect.y];
            if let Some(from) = previous {
                self.out.wires.push(Wire { from, to: top });
                self.out.gaps.push(Gap {
                    place: place.clone(),
                    index,
                    at: [centre, from[1] + GAP / 2.0],
                });
            } else {
                first = top;
                // Before the first step there is room only on the top list; inside a frame the
                // column's label is there.
                if place.is_empty() {
                    self.out.gaps.push(Gap {
                        place: place.clone(),
                        index: 0,
                        at: [centre, y - GAP / 2.0],
                    });
                }
            }
            spans.push([rect.y, rect.bottom()]);
            let loc = Loc {
                place: place.clone(),
                index,
            };
            let (title, detail) = words(node);
            self.out.items.push(Item {
                loc: loc.clone(),
                form: if node.blocks.is_empty() {
                    Form::Step
                } else {
                    Form::Frame
                },
                kind: node.kind.clone(),
                id: node.id.text.trim().to_owned(),
                title,
                detail,
                model: is_model(node),
                rect,
            });
            if !node.blocks.is_empty() {
                self.columns(node, &loc, rect);
            }
            previous = Some([centre, rect.bottom()]);
            cursor = rect.bottom() + GAP;
        }
        let last = previous.unwrap_or(first);
        self.out.gaps.push(Gap {
            place: place.clone(),
            index: nodes.len(),
            at: [centre, last[1] + END_GAP],
        });
        self.out.lists.push(ListRegion {
            place: place.clone(),
            rect: Rect {
                x,
                y,
                w: width,
                h: last[1] - y,
            },
            spans,
        });
        (first, last)
    }

    /// The columns inside the frame of `node`, with the wires into and out of each.
    fn columns(&mut self, node: &Node, at: &Loc, frame: Rect) {
        let cols: Vec<(f32, f32)> = node.blocks.iter().map(|b| measure_list(&b.steps)).collect();
        let used: f32 = cols.iter().map(|c| c.0).sum::<f32>() + COL_GAP * (cols.len() - 1) as f32;
        let labelled = node.blocks.len() > 1;
        let label_h = if labelled { LABEL_H } else { 0.0 };
        let mut x = frame.x + PAD + (frame.w - 2.0 * PAD - used) / 2.0;
        let top = frame.y + HEAD + PAD + label_h;
        let enter = [frame.centre_x(), frame.y + HEAD];
        let leave = [frame.centre_x(), frame.bottom()];
        for (b, block) in node.blocks.iter().enumerate() {
            let w = cols[b].0;
            let mut place = at.place.clone();
            place.push((at.index, b));
            let (first, last) = self.list(&block.steps, &place, x, top, w);
            // A step dropped anywhere in the column goes in this list: from under the frame's
            // title bar to its foot.
            if let Some(region) = self.out.lists.iter_mut().rev().find(|r| r.place == place) {
                region.rect.x = x;
                region.rect.w = w;
                region.rect.y = frame.y + HEAD;
                region.rect.h = frame.bottom() - region.rect.y;
            }
            let centre = x + w / 2.0;
            if let Some(text) = label_of(node, b) {
                let chip = Rect {
                    x: centre - NODE_W / 2.0 + 20.0,
                    y: top - LABEL_H,
                    w: NODE_W - 40.0,
                    h: CHIP_H,
                };
                self.out.labels.push(Label { text, rect: chip });
                self.out.wires.push(Wire {
                    from: enter,
                    to: [centre, chip.y],
                });
                self.out.wires.push(Wire {
                    from: [centre, chip.bottom()],
                    to: first,
                });
            } else {
                self.out.wires.push(Wire {
                    from: enter,
                    to: first,
                });
            }
            // Straight down to a join line under the tallest column, then in to the middle, so
            // a short column's wire does not cut across the one beside it.
            let join = [centre, frame.bottom() - FOOT / 2.0];
            self.out.wires.push(Wire {
                from: last,
                to: join,
            });
            self.out.wires.push(Wire {
                from: join,
                to: leave,
            });
            x += w + COL_GAP;
        }
    }
}

/// The drawing of the editor's steps.
pub fn layout(ed: &Editor) -> Layout {
    let (spine, height) = measure_list(&ed.steps);
    let width = spine.max(START_W);
    let mut pen = Pen {
        out: Layout::default(),
    };
    let start = Rect {
        x: MARGIN + width / 2.0 - START_W / 2.0,
        y: MARGIN,
        w: START_W,
        h: START_H,
    };
    pen.out.start = start;
    let below = start.bottom() + GAP;
    let (first, _) = pen.list(&ed.steps, &Vec::new(), MARGIN, below, width);
    pen.out.wires.insert(
        0,
        Wire {
            from: [start.centre_x(), start.bottom()],
            to: first,
        },
    );
    pen.out.width = width + 2.0 * MARGIN;
    pen.out.height = below + height + MARGIN;
    // The top list is the whole drawing under the start, so a step dropped anywhere there that is
    // not in a frame goes in it.
    let (full_w, full_h) = (pen.out.width, pen.out.height);
    if let Some(root) = pen.out.lists.iter_mut().find(|r| r.place.is_empty()) {
        root.rect = Rect {
            x: 0.0,
            y: start.bottom(),
            w: full_w,
            h: full_h - start.bottom(),
        };
    }
    pen.out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

fn list_at<'a>(ed: &'a Editor, place: &Place) -> Option<&'a Vec<Node>> {
    let mut list = &ed.steps;
    for &(step, block) in place {
        list = &list.get(step)?.blocks.get(block)?.steps;
    }
    Some(list)
}

fn parent_of(at: &Loc) -> Option<Loc> {
    let (&(index, _), rest) = at.place.split_last()?;
    Some(Loc {
        place: rest.to_vec(),
        index,
    })
}

fn first_inside(node: &Node, place: &Place, index: usize) -> Option<Loc> {
    node.blocks.iter().enumerate().find_map(|(b, block)| {
        (!block.steps.is_empty()).then(|| {
            let mut inner = place.clone();
            inner.push((index, b));
            Loc {
                place: inner,
                index: 0,
            }
        })
    })
}

/// The step the arrow key `dir` moves to from `from`, if there is one. Up and down follow the
/// spine, and past the end of a frame go on from the frame itself; right goes into a frame, or
/// across to the next column, and left goes back across or out to the frame.
pub fn step_from(ed: &Editor, from: &Loc, dir: Dir) -> Option<Loc> {
    let list = list_at(ed, &from.place)?;
    let node = list.get(from.index)?;
    let sibling = |index: usize| {
        (index < list.len()).then(|| Loc {
            place: from.place.clone(),
            index,
        })
    };
    // The step across, in the next or previous column of the frame this one is in.
    let across = |by: isize| {
        let parent = parent_of(from)?;
        let &(_, block) = from.place.last()?;
        let to = block.checked_add_signed(by)?;
        let columns = &list_at(ed, &parent.place)?.get(parent.index)?.blocks;
        let target = &columns.get(to)?.steps;
        (!target.is_empty()).then(|| {
            let mut place = parent.place.clone();
            place.push((parent.index, to));
            Loc {
                place,
                index: from.index.min(target.len() - 1),
            }
        })
    };
    match dir {
        Dir::Up => from
            .index
            .checked_sub(1)
            .and_then(sibling)
            .or_else(|| parent_of(from)),
        Dir::Down => sibling(from.index + 1).or_else(|| {
            // Past the last step of a frame: on to what follows the frame.
            let mut at = parent_of(from)?;
            loop {
                if let Some(next) = step_from_sibling(ed, &at) {
                    return Some(next);
                }
                at = parent_of(&at)?;
            }
        }),
        Dir::Right => first_inside(node, &from.place, from.index).or_else(|| across(1)),
        Dir::Left => across(-1).or_else(|| parent_of(from)),
    }
}

/// The step after `at` in its own list.
fn step_from_sibling(ed: &Editor, at: &Loc) -> Option<Loc> {
    let list = list_at(ed, &at.place)?;
    (at.index + 1 < list.len()).then(|| Loc {
        place: at.place.clone(),
        index: at.index + 1,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::workflow::Saved;

    fn editor(steps: Value) -> Editor {
        let w = Saved {
            id: "w1".into(),
            agent_id: "a1".into(),
            name: "Flow".into(),
            goal: String::new(),
            definition: serde_json::from_value(json!({"steps": steps})).unwrap(),
            created_at: 0,
        };
        Editor::from_saved(&w)
    }

    fn sh(id: &str) -> Value {
        json!({"id": id, "kind": "tool", "tool": "shell", "args": {"command": format!("echo {id}")}})
    }

    fn at(place: &[(usize, usize)], index: usize) -> Loc {
        Loc {
            place: place.to_vec(),
            index,
        }
    }

    fn rich() -> Editor {
        editor(json!([
            sh("a"),
            {"id": "p", "kind": "parallel", "branches": [
                {"name": "left", "steps": [
                    {"id": "each", "kind": "for_each", "over": "[1, 2]",
                     "steps": [sh("inner"), {"id": "i", "kind": "if", "cond": "true",
                        "then": [sh("t")], "else": []}]},
                    sh("after-each")
                ]},
                {"name": "right", "steps": [sh("r")]}
            ]},
            {"id": "sw", "kind": "switch", "on": "1", "cases": [
                {"when": "1", "steps": [sh("c1")]}, {"when": "2", "steps": []}
            ], "default": [sh("d")]},
            {"id": "say", "kind": "generate", "instruction": "hi"},
            sh("last")
        ]))
    }

    #[test]
    fn a_plain_list_is_one_spine_with_a_wire_between_each() {
        let l = layout(&editor(json!([sh("a"), sh("b"), sh("c")])));
        let boxes: Vec<_> = l.items.iter().map(|i| i.rect).collect();
        assert_eq!(boxes.len(), 3);
        for (n, r) in boxes.iter().enumerate() {
            assert_eq!((r.w, r.h), (NODE_W, NODE_H));
            assert!(
                (r.centre_x() - boxes[0].centre_x()).abs() < 0.01,
                "one spine"
            );
            if n > 0 {
                assert!((r.y - boxes[n - 1].bottom() - GAP).abs() < 0.01);
            }
        }
        assert_eq!(
            l.wires.len(),
            3,
            "start to the first, then each to the next"
        );
        assert_eq!(l.wires[0].from, [l.start.centre_x(), l.start.bottom()]);
        assert_eq!(
            (l.width, l.items[0].kind.as_str()),
            (NODE_W + 2.0 * MARGIN, "tool")
        );
        assert_eq!(l.items[0].detail, "Use a tool");
        assert!(l.height > boxes[2].bottom());
    }

    #[test]
    fn a_branching_step_is_a_frame_with_its_lists_side_by_side_and_named() {
        let l = layout(&editor(json!([
            {"id": "i", "kind": "if", "cond": "true", "then": [sh("t1"), sh("t2")], "else": [sh("e")]},
            sh("after")
        ])));
        let frame = l.find(&at(&[], 0)).unwrap();
        assert_eq!(frame.form, Form::Frame);
        let then = l.find(&at(&[(0, 0)], 0)).unwrap().rect;
        let other = l.find(&at(&[(0, 1)], 0)).unwrap().rect;
        assert!(then.right() < other.x, "then is left of otherwise");
        assert!(frame.rect.contains(&then) && frame.rect.contains(&other));
        let names: Vec<_> = l.labels.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(names, ["Then", "Otherwise"]);
        let after = l.find(&at(&[], 1)).unwrap().rect;
        assert!((after.y - frame.rect.bottom() - GAP).abs() < 0.01);
        // Into each column (twice, through its chip) and out of each (twice, by the join line);
        // and start to the frame, the frame to after, and t1 to t2.
        assert_eq!(l.wires.len(), 1 + 2 * 4 + 1 + 1 /* t1 to t2 */);
    }

    #[test]
    fn a_loop_is_a_frame_with_one_list_and_no_label_and_an_empty_list_has_a_slot() {
        let l = layout(&editor(json!([
            {"id": "e", "kind": "for_each", "over": "[1]", "steps": [sh("x")]},
            {"id": "i", "kind": "if", "cond": "true", "then": [sh("t")], "else": []}
        ])));
        assert_eq!(l.labels.len(), 2, "only the if names its lists");
        let slot = l.items.iter().find(|i| i.form == Form::Empty).unwrap();
        assert_eq!(slot.loc, at(&[(1, 1)], 0));
        assert!(l.find(&at(&[(1, 1)], 0)).is_none(), "a slot is not a step");
        let frame = l.find(&at(&[], 1)).unwrap();
        assert!(frame.rect.contains(&slot.rect));
    }

    #[test]
    fn nothing_overlaps_and_every_step_is_inside_the_frame_it_belongs_to() {
        let e = rich();
        let l = layout(&e);
        for (n, a) in l.items.iter().enumerate() {
            assert!(a.rect.x >= 0.0 && a.rect.y >= 0.0, "{a:?}");
            assert!(
                a.rect.right() <= l.width && a.rect.bottom() <= l.height,
                "{a:?}"
            );
            for b in &l.items[n + 1..] {
                let nested = a.rect.contains(&b.rect) || b.rect.contains(&a.rect);
                assert!(nested || !a.rect.overlaps(&b.rect), "{a:?} overlaps {b:?}");
            }
            if let Some(parent) = crate::workflow::layout::parent_of(&a.loc) {
                let frame = l.find(&parent).unwrap();
                assert!(frame.rect.contains(&a.rect), "{a:?} is outside {frame:?}");
            }
        }
        for chip in &l.labels {
            for item in l.items.iter().filter(|i| i.form != Form::Frame) {
                assert!(!chip.rect.overlaps(&item.rect), "{chip:?} over {item:?}");
            }
        }
        assert_eq!(
            layout(&e),
            l,
            "the same steps are always drawn the same way"
        );
    }

    #[test]
    fn a_step_that_calls_a_model_is_marked_and_nothing_else_is() {
        let l = layout(&editor(json!([
            sh("a"),
            {"id": "g", "kind": "generate", "instruction": "x"},
            {"id": "d", "kind": "decide", "question": "q", "options": ["a", "b"], "input": "x", "bind": "r"},
            {"id": "h", "kind": "tool", "tool": "delegate", "args": {"agent": "Ann", "task": "t"}},
            {"id": "s", "kind": "set", "values": {"a": "1"}, "bind": "v"}
        ])));
        let marks: Vec<_> = l.items.iter().map(|i| i.model).collect();
        assert_eq!(marks, [false, true, true, true, false]);
        assert_eq!(l.items[2].detail, "Let the AI choose");
    }

    #[test]
    fn arrow_keys_follow_the_spine_into_and_out_of_frames_and_across_columns() {
        let e = rich();
        let go = |from: &Loc, dir| step_from(&e, from, dir);
        // Down the top spine: a, the parallel frame, the switch, generate, last.
        assert_eq!(go(&at(&[], 0), Dir::Down), Some(at(&[], 1)));
        assert_eq!(
            go(&at(&[], 4), Dir::Down),
            None,
            "the last step has nothing below"
        );
        assert_eq!(go(&at(&[], 1), Dir::Up), Some(at(&[], 0)));
        assert_eq!(go(&at(&[], 0), Dir::Up), None);
        // Right goes into the frame, to its first step; left comes back out.
        let each = at(&[(1, 0)], 0);
        assert_eq!(go(&at(&[], 1), Dir::Right), Some(each.clone()));
        assert_eq!(go(&each, Dir::Left), Some(at(&[], 1)));
        // Right from a frame goes into it; right from a plain step goes across to the next
        // column, to the same height or the nearest.
        assert_eq!(go(&each, Dir::Right), Some(at(&[(1, 0), (0, 0)], 0)));
        assert_eq!(
            go(&at(&[(1, 0)], 1), Dir::Right),
            Some(at(&[(1, 1)], 0)),
            "clamped to the one step"
        );
        assert_eq!(go(&at(&[(1, 1)], 0), Dir::Left), Some(at(&[(1, 0)], 0)));
        // Down past the last step of a frame goes on from the frame; up from the first goes to it.
        assert_eq!(go(&at(&[(1, 1)], 0), Dir::Down), Some(at(&[], 2)));
        assert_eq!(go(&at(&[(1, 0)], 0), Dir::Up), Some(at(&[], 1)));
        // Into a switch goes to its first list that has steps.
        assert_eq!(go(&at(&[], 2), Dir::Right), Some(at(&[(2, 0)], 0)));
    }

    fn centre_of(r: &Rect) -> [f32; 2] {
        [r.centre_x(), r.y + r.h / 2.0]
    }

    #[test]
    fn a_button_to_add_a_step_sits_on_each_wire_of_the_top_list_and_after_the_last_in_a_frame() {
        let l = layout(&editor(json!([sh("a"), sh("b"), sh("c")])));
        let mut top: Vec<usize> = l.gaps.iter().map(|g| g.index).collect();
        top.sort_unstable();
        assert_eq!(
            top,
            [0, 1, 2, 3],
            "before the first, between, and after the last"
        );
        let (a, b) = (
            l.find(&at(&[], 0)).unwrap().rect,
            l.find(&at(&[], 1)).unwrap().rect,
        );
        let between = l.gaps.iter().find(|g| g.index == 1).unwrap();
        assert!((between.at[1] - (a.bottom() + b.y) / 2.0).abs() < 0.01);
        // In a frame's column: after the last step, and between two, but not before the first,
        // where the column's label is.
        let l = layout(&editor(json!([
            {"id": "i", "kind": "if", "cond": "true", "then": [sh("t1"), sh("t2")], "else": []}
        ])));
        let then: Vec<usize> = l
            .gaps
            .iter()
            .filter(|g| g.place == [(0, 0)])
            .map(|g| g.index)
            .collect();
        assert_eq!(then, [1, 2]);
        assert!(
            l.gaps.iter().all(|g| g.place != [(0, 1)]),
            "an empty list has its slot, not a button"
        );
    }

    #[test]
    fn the_step_under_a_point_is_the_innermost_box() {
        let l = layout(&editor(json!([
            sh("a"),
            {"id": "i", "kind": "if", "cond": "true", "then": [sh("t")], "else": []}
        ])));
        let inner = l.find(&at(&[(1, 0)], 0)).unwrap();
        assert_eq!(l.hit(centre_of(&inner.rect)).unwrap().loc, at(&[(1, 0)], 0));
        let frame = l.find(&at(&[], 1)).unwrap();
        let padding = [frame.rect.x + 4.0, frame.rect.bottom() - 4.0];
        assert_eq!(
            l.hit(padding).unwrap().loc,
            at(&[], 1),
            "the frame, in its own padding"
        );
        assert!(l.hit([-5.0, -5.0]).is_none());
    }

    #[test]
    fn a_dragged_step_lands_in_the_list_under_the_pointer_at_the_place_between_its_steps() {
        let l = layout(&editor(json!([
            sh("a"), sh("b"), sh("c"),
            {"id": "i", "kind": "if", "cond": "true", "then": [sh("t1")], "else": []}
        ])));
        let rect = |place: &[(usize, usize)], index| l.find(&at(place, index)).unwrap().rect;
        let (a, b, c) = (rect(&[], 0), rect(&[], 1), rect(&[], 2));
        // Between b and c: a goes to position 2 of the top list (counted as the list is now).
        let p = [b.centre_x(), (b.bottom() + c.y) / 2.0];
        let drop = l.drop_target(&at(&[], 0), p).unwrap();
        assert_eq!(
            (drop.place.clone(), drop.index, drop.unchanged),
            (vec![], 2, false)
        );
        assert!((drop.marker.y + drop.marker.h / 2.0 - (b.bottom() + c.y) / 2.0).abs() < 0.01);
        assert_eq!(drop.frame, None);
        // Just under itself, or just over: it would stay.
        let under = [a.centre_x(), a.bottom() + 4.0];
        assert!(l.drop_target(&at(&[], 0), under).unwrap().unchanged);
        let over = [b.centre_x(), b.y - 4.0];
        assert!(l.drop_target(&at(&[], 1), over).unwrap().unchanged);
        // Above the first step, and below the last.
        assert_eq!(
            l.drop_target(&at(&[], 2), [a.centre_x(), a.y - 6.0])
                .unwrap()
                .index,
            0
        );
        // Into the frame's then column: that list, after t1, with the frame to light.
        let t1 = rect(&[(3, 0)], 0);
        let into = l
            .drop_target(&at(&[], 0), [t1.centre_x(), t1.bottom() + 3.0])
            .unwrap();
        assert_eq!((into.place.clone(), into.index), (vec![(3, 0)], 1));
        assert_eq!(into.frame, Some(at(&[], 3)));
        // Into the empty otherwise list: its slot is the marker.
        let slot = l.items.iter().find(|i| i.form == Form::Empty).unwrap().rect;
        let empty = l.drop_target(&at(&[], 0), centre_of(&slot)).unwrap();
        assert_eq!(
            (empty.place.clone(), empty.index, empty.marker),
            (vec![(3, 1)], 0, slot)
        );
        assert!(
            l.drop_target(&at(&[], 0), [-9.0, -9.0]).is_none(),
            "nowhere is nothing"
        );
    }

    #[test]
    fn a_frame_dragged_over_its_own_columns_lands_outside_it_and_never_inside_itself() {
        let l = layout(&editor(json!([
            sh("a"),
            {"id": "i", "kind": "if", "cond": "true", "then": [sh("t1")], "else": [sh("e1")]},
            sh("z")
        ])));
        let from = at(&[], 1);
        for item in l.items.iter().filter(|i| {
            i.loc.place.first() == Some(&(1, 0)) || i.loc.place.first() == Some(&(1, 1))
        }) {
            let drop = l.drop_target(&from, centre_of(&item.rect)).unwrap();
            assert!(
                !(!drop.place.is_empty() && drop.place[0].0 == 1),
                "{:?} landed inside the frame itself: {drop:?}",
                item.loc
            );
        }
    }
}
