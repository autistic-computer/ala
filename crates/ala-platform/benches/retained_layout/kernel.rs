//! Taffy kernel experiment, not an end-to-end port of GPUI Fast.
//! Stable node identities are given by this fixture. Reconciliation, closures, input, and
//! frame bookkeeping are deliberately excluded; a kernel win is not an ALA frame-time claim.

use super::{arg, measure, number};
use serde_json::{Value, json};
use taffy::{TaffyTree, prelude::*};

const PANELS: usize = 60;
const LABELS: usize = 64;
const LEAVES: usize = PANELS * LABELS;

struct Tree {
    tree: TaffyTree<()>,
    root: NodeId,
    leaves: Vec<NodeId>,
    panels: Vec<NodeId>,
    styles: Vec<Style>,
}

fn style(index: usize, frame: usize, dirty: usize) -> Style {
    Style {
        size: Size {
            width: length(160.),
            height: length(if index < dirty {
                16. + (frame % 2) as f32
            } else {
                16.
            }),
        },
        flex_shrink: 0.,
        ..Style::default()
    }
}
impl Tree {
    fn new(frame: usize, dirty: usize) -> Self {
        let mut tree = TaffyTree::new();
        tree.disable_rounding();
        let root = tree.new_leaf(Style::default()).unwrap();
        let mut state = Self {
            tree,
            root,
            leaves: Vec::new(),
            panels: Vec::new(),
            styles: Vec::new(),
        };
        state.rebuild(frame, dirty);
        state
    }
    fn rebuild(&mut self, frame: usize, dirty: usize) {
        // Match ALA: clear the existing tree and reuse its storage, rather than allocating
        // a fresh TaffyTree for every baseline frame.
        self.tree.clear();
        self.styles.clear();
        self.styles
            .extend((0..LEAVES).map(|ix| style(ix, frame, dirty)));
        self.leaves.clear();
        self.leaves.extend(
            self.styles
                .iter()
                .map(|style| self.tree.new_leaf(style.clone()).unwrap()),
        );
        self.panels = self
            .leaves
            .chunks(LABELS)
            .map(|children| {
                self.tree
                    .new_with_children(
                        Style {
                            display: Display::Flex,
                            flex_direction: FlexDirection::Column,
                            size: Size {
                                width: length(180.),
                                height: auto(),
                            },
                            ..Style::default()
                        },
                        children,
                    )
                    .unwrap()
            })
            .collect();
        self.root = self
            .tree
            .new_with_children(
                Style {
                    display: Display::Flex,
                    flex_wrap: FlexWrap::Wrap,
                    size: Size {
                        width: length(1080.),
                        height: auto(),
                    },
                    ..Style::default()
                },
                &self.panels,
            )
            .unwrap();
    }
    fn update(&mut self, frame: usize, dirty: usize, write_all: bool) {
        for index in 0..LEAVES {
            let next = style(index, frame, dirty);
            if write_all || self.styles[index] != next {
                self.tree
                    .set_style(self.leaves[index], next.clone())
                    .unwrap();
                self.styles[index] = next;
            }
        }
    }
    fn compute(&mut self) {
        self.tree
            .compute_layout(
                self.root,
                Size {
                    width: AvailableSpace::Definite(1080.),
                    height: AvailableSpace::MaxContent,
                },
            )
            .unwrap();
    }
    fn geometry(&self) -> Vec<[f32; 4]> {
        self.leaves
            .iter()
            .chain(self.panels.iter())
            .chain([&self.root])
            .map(|&id| {
                let layout = self.tree.layout(id).unwrap();
                [
                    layout.location.x,
                    layout.location.y,
                    layout.size.width,
                    layout.size.height,
                ]
            })
            .collect()
    }
}

pub fn verify() -> Value {
    let mut comparisons = 0;
    for dirty in [0, 1, 64, LEAVES] {
        let mut retained = Tree::new(0, dirty);
        for frame in 0..32 {
            let mut baseline = Tree::new(frame, dirty);
            baseline.compute();
            retained.update(frame, dirty, false);
            retained.compute();
            assert_eq!(
                baseline.geometry(),
                retained.geometry(),
                "layout mismatch dirty={dirty} frame={frame}"
            );
            comparisons += LEAVES + PANELS + 1;
        }
    }
    json!({"frames":128, "exact_node_geometry_comparisons":comparisons,
        "scope":"Taffy kernel with fixture-provided stable identities"})
}

pub fn run() -> Value {
    let mode = arg("--mode", "rebuild");
    let dirty = number("--dirty", 1);
    assert!(dirty <= LEAVES);
    let mut state = Tree::new(0, dirty);
    state.compute();
    let warmup = number("--warmup", 30);
    let mut samples = Vec::new();
    for frame in 0..warmup + number("--samples", 150) {
        let (_, timing) = measure(|| {
            if mode == "rebuild" {
                state.rebuild(frame, dirty);
            } else {
                state.update(frame, dirty, mode == "write-all");
            }
            state.compute();
            std::hint::black_box(state.tree.layout(state.root).unwrap());
        });
        if frame >= warmup {
            samples.push(timing);
        }
    }
    let checksum: f64 = state.geometry().iter().flatten().map(|&v| v as f64).sum();
    json!({"case":"layout", "scope":"Taffy kernel only", "mode":mode,
        "leaves":LEAVES,"dirty_leaves":dirty,"geometry_checksum":checksum,"samples":samples})
}
