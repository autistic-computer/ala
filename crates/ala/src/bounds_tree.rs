use crate::{Bounds, Half};
use std::{
    fmt::Debug,
    ops::{Add, Sub},
};

const CELL_SIZE: f64 = 64.;

const MAX_CELLS_PER_AXIS: usize = 256;

const NONE: u32 = u32::MAX;

#[derive(Debug)]
pub(crate) struct BoundsTree<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    grid: Grid<U>,
    max: Option<(Bounds<U>, u32)>,
    recorded: Vec<(Bounds<U>, u32)>,
}

#[derive(Debug)]
struct Grid<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    columns: usize,
    rows: usize,
    cells: Vec<Cell>,
    entries: Vec<Entry<U>>,
    degenerate: Vec<(Bounds<U>, u32)>,
}

#[derive(Clone, Copy, Debug)]
struct Cell {
    head: u32,
    cover: u32,
    max: u32,
}

impl Cell {
    const EMPTY: Cell = Cell {
        head: NONE,
        cover: 0,
        max: 0,
    };
}

#[derive(Clone, Debug)]
struct Entry<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    bounds: Bounds<U>,
    order: u32,
    rest_max: u32,
    next: u32,
}

#[derive(Clone, Copy)]
struct Span {
    start: f64,
    end: f64,
    first_cell: usize,
    last_cell: usize,
}

impl Span {
    fn new(start: f64, end: f64, cells: usize) -> Self {
        Span {
            start,
            end,
            first_cell: cell_at(start, cells),
            last_cell: cell_at(end, cells),
        }
    }

    fn enters(&self, cell: usize, cells: usize) -> bool {
        self.start < cell_end(cell, cells) && self.end > cell_start(cell)
    }

    fn covers(&self, cell: usize, cells: usize) -> bool {
        self.start <= cell_start(cell) && self.end >= cell_end(cell, cells)
    }
}

fn cell_at(coordinate: f64, cells: usize) -> usize {
    ((coordinate * (1. / CELL_SIZE)) as isize).clamp(0, cells as isize - 1) as usize
}

fn cell_start(cell: usize) -> f64 {
    if cell == 0 {
        f64::NEG_INFINITY
    } else {
        cell as f64 * CELL_SIZE
    }
}

fn cell_end(cell: usize, cells: usize) -> f64 {
    if cell + 1 == cells {
        f64::INFINITY
    } else {
        (cell + 1) as f64 * CELL_SIZE
    }
}

impl<U> Grid<U>
where
    U: Clone
        + Debug
        + PartialEq
        + PartialOrd
        + Add<U, Output = U>
        + Sub<Output = U>
        + Half
        + Default
        + Into<f64>,
{
    fn clear(&mut self) {
        self.cells.fill(Cell::EMPTY);
        self.entries.clear();
        self.degenerate.clear();
    }

    #[allow(clippy::eq_op)]
    fn has_area(bounds: &Bounds<U>) -> bool {
        bounds.size.width > U::default()
            && bounds.size.height > U::default()
            && bounds.origin.x == bounds.origin.x
            && bounds.origin.y == bounds.origin.y
    }

    fn spans(&self, bounds: &Bounds<U>) -> (Span, Span) {
        let right = bounds.origin.x.clone() + bounds.size.width.clone();
        let bottom = bounds.origin.y.clone() + bounds.size.height.clone();
        (
            Span::new(bounds.origin.x.clone().into(), right.into(), self.columns),
            Span::new(bounds.origin.y.clone().into(), bottom.into(), self.rows),
        )
    }

    fn needs(&self, bounds: &Bounds<U>) -> Option<(usize, usize)> {
        let right: f64 = (bounds.origin.x.clone() + bounds.size.width.clone()).into();
        let bottom: f64 = (bounds.origin.y.clone() + bounds.size.height.clone()).into();
        if right <= self.columns as f64 * CELL_SIZE && bottom <= self.rows as f64 * CELL_SIZE {
            return None;
        }
        let needed = |end: f64| {
            ((end / CELL_SIZE).ceil() as isize).clamp(1, MAX_CELLS_PER_AXIS as isize) as usize
        };
        let (columns, rows) = (
            needed(right).max(self.columns),
            needed(bottom).max(self.rows),
        );
        (columns > self.columns || rows > self.rows).then_some((columns, rows))
    }

    fn resize(&mut self, columns: usize, rows: usize) {
        self.columns = columns;
        self.rows = rows;
        self.cells.clear();
        self.cells.resize(columns * rows, Cell::EMPTY);
        self.entries.clear();
        self.degenerate.clear();
    }

    fn add(&mut self, bounds: &Bounds<U>, order: u32) {
        if !Self::has_area(bounds) {
            self.degenerate.push((bounds.clone(), order));
            return;
        }
        let (x, y) = self.spans(bounds);
        for row in y.first_cell..=y.last_cell {
            let covers_row = y.covers(row, self.rows);
            for column in x.first_cell..=x.last_cell {
                let cell = &mut self.cells[row * self.columns + column];
                cell.max = cell.max.max(order);
                if covers_row && x.covers(column, self.columns) {
                    cell.cover = cell.cover.max(order);
                } else {
                    let rest_max = match self.entries.get(cell.head as usize) {
                        Some(next) => next.rest_max.max(order),
                        None => order,
                    };
                    let entry = self.entries.len() as u32;
                    self.entries.push(Entry {
                        bounds: bounds.clone(),
                        order,
                        rest_max,
                        next: cell.head,
                    });
                    cell.head = entry;
                }
            }
        }
    }

    fn max_intersecting(&self, query: &Bounds<U>) -> u32 {
        let mut max = self
            .degenerate
            .iter()
            .filter(|(bounds, _)| bounds.intersects(query))
            .map(|(_, order)| *order)
            .max()
            .unwrap_or(0);
        let (x, y) = self.spans(query);
        for row in y.first_cell..=y.last_cell {
            let enters_row = y.enters(row, self.rows);
            for column in x.first_cell..=x.last_cell {
                let cell = &self.cells[row * self.columns + column];
                if cell.max <= max {
                    continue;
                }
                if cell.cover > max && enters_row && x.enters(column, self.columns) {
                    max = cell.cover;
                }
                let mut entry_ix = cell.head;
                while let Some(entry) = self.entries.get(entry_ix as usize) {
                    if entry.rest_max <= max {
                        break;
                    }
                    if entry.order > max && entry.bounds.intersects(query) {
                        max = entry.order;
                    }
                    entry_ix = entry.next;
                }
            }
        }
        max
    }
}

impl<U> BoundsTree<U>
where
    U: Clone
        + Debug
        + PartialEq
        + PartialOrd
        + Add<U, Output = U>
        + Sub<Output = U>
        + Half
        + Default
        + Into<f64>,
{
    pub fn clear(&mut self) {
        self.grid.clear();
        self.max = None;
        self.recorded.clear();
    }

    pub fn insert(&mut self, new_bounds: Bounds<U>) -> u32 {
        let ordering = self.find_max_ordering(&new_bounds) + 1;
        self.add(&new_bounds, ordering);
        self.recorded.push((new_bounds, ordering));
        ordering
    }

    fn add(&mut self, bounds: &Bounds<U>, ordering: u32) {
        if Grid::has_area(bounds)
            && let Some((columns, rows)) = self.grid.needs(bounds)
        {
            self.grid.resize(columns, rows);
            for (recorded, recorded_ordering) in &self.recorded {
                self.grid.add(recorded, *recorded_ordering);
            }
        }
        self.grid.add(bounds, ordering);
        if self.max.as_ref().is_none_or(|(_, max)| *max < ordering) {
            self.max = Some((bounds.clone(), ordering));
        }
    }

    fn find_max_ordering(&self, query: &Bounds<U>) -> u32 {
        if let Some((max_bounds, max)) = &self.max
            && query.intersects(max_bounds)
        {
            return *max;
        }
        if Grid::has_area(query) {
            self.grid.max_intersecting(query)
        } else {
            self.recorded
                .iter()
                .filter(|(bounds, _)| bounds.intersects(query))
                .map(|(_, ordering)| *ordering)
                .max()
                .unwrap_or(0)
        }
    }
}

impl<U> Default for BoundsTree<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    fn default() -> Self {
        BoundsTree {
            grid: Grid {
                columns: 1,
                rows: 1,
                cells: vec![Cell::EMPTY],
                entries: Vec::new(),
                degenerate: Vec::new(),
            },
            max: None,
            recorded: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BoundsTree, CELL_SIZE, MAX_CELLS_PER_AXIS};
    use crate::{Bounds, Point, Size};
    use rand::{Rng, SeedableRng};

    #[test]
    fn test_insert() {
        let mut tree = BoundsTree::<f32>::default();
        let bounds1 = Bounds {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds2 = Bounds {
            origin: Point { x: 5.0, y: 5.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds3 = Bounds {
            origin: Point { x: 10.0, y: 10.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };

        assert_eq!(tree.insert(bounds1), 1);
        assert_eq!(tree.insert(bounds2), 2);
        assert_eq!(tree.insert(bounds3), 3);

        let bounds4 = Bounds {
            origin: Point { x: 20.0, y: 20.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds5 = Bounds {
            origin: Point { x: 40.0, y: 40.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds6 = Bounds {
            origin: Point { x: 25.0, y: 25.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        assert_eq!(tree.insert(bounds4), 1);
        assert_eq!(tree.insert(bounds5), 1);
        assert_eq!(tree.insert(bounds6), 2);
    }

    #[test]
    fn test_random_iterations() {
        let max_bounds = 100;
        for seed in 1..=1000 {
            let mut tree = BoundsTree::default();
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed as u64);
            let mut expected_quads: Vec<(Bounds<f32>, u32)> = Vec::new();

            let num_bounds = rng.random_range(1..=max_bounds);
            for _ in 0..num_bounds {
                let min_x: f32 = rng.random_range(-100.0..100.0);
                let min_y: f32 = rng.random_range(-100.0..100.0);
                let width: f32 = rng.random_range(0.0..50.0);
                let height: f32 = rng.random_range(0.0..50.0);
                let bounds = Bounds {
                    origin: Point { x: min_x, y: min_y },
                    size: Size { width, height },
                };

                let expected_ordering = expected_quads
                    .iter()
                    .filter_map(|quad| quad.0.intersects(&bounds).then_some(quad.1))
                    .max()
                    .unwrap_or(0)
                    + 1;
                expected_quads.push((bounds, expected_ordering));

                let actual_ordering = tree.insert(bounds);
                assert_eq!(actual_ordering, expected_ordering);
            }
        }
    }

    #[test]
    fn test_large_random_iterations() {
        for seed in 1..=10 {
            let mut tree = BoundsTree::default();
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed as u64);
            let mut expected_quads: Vec<(Bounds<f32>, u32)> = Vec::new();
            for _ in 0..2000 {
                let bounds = Bounds {
                    origin: Point {
                        x: rng.random_range(-1000.0..1000.0),
                        y: rng.random_range(-1000.0..1000.0),
                    },
                    size: Size {
                        width: rng.random_range(0.0..80.0),
                        height: rng.random_range(0.0..80.0),
                    },
                };
                let expected_ordering = expected_quads
                    .iter()
                    .filter_map(|quad| quad.0.intersects(&bounds).then_some(quad.1))
                    .max()
                    .unwrap_or(0)
                    + 1;
                expected_quads.push((bounds, expected_ordering));
                assert_eq!(tree.insert(bounds), expected_ordering);
            }
        }
    }

    #[test]
    fn awkward_bounds_are_ordered_as_comparing_them_with_all_would() {
        let edge = CELL_SIZE as f32;
        let coordinate = |rng: &mut rand::rngs::StdRng| match rng.random_range(0..12) {
            0 => f32::INFINITY,
            1 => f32::NEG_INFINITY,
            2 => f32::NAN,
            3 => rng.random_range(-4..(MAX_CELLS_PER_AXIS as i32 + 4)) as f32 * edge,
            4..8 => rng.random_range(-3..8) as f32 * edge,
            _ => rng.random_range(-2.0 * edge..6.0 * edge),
        };
        let length = |rng: &mut rand::rngs::StdRng| match rng.random_range(0..10) {
            0 => 0.,
            1 => -rng.random_range(0.0..edge),
            2 => f32::INFINITY,
            3..6 => rng.random_range(0..4) as f32 * edge,
            _ => rng.random_range(0.0..3.0 * edge),
        };
        for seed in 1..=400 {
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            let mut tree = BoundsTree::default();
            for _ in 0..3 {
                let frame: Vec<_> = (0..rng.random_range(1..200))
                    .map(|_| Bounds {
                        origin: Point {
                            x: coordinate(&mut rng),
                            y: coordinate(&mut rng),
                        },
                        size: Size {
                            width: length(&mut rng),
                            height: length(&mut rng),
                        },
                    })
                    .collect();
                tree.clear();
                let mut inserted: Vec<(Bounds<f32>, u32)> = Vec::new();
                for bounds in &frame {
                    let expected = inserted
                        .iter()
                        .filter_map(|(other, order)| other.intersects(bounds).then_some(*order))
                        .max()
                        .unwrap_or(0)
                        + 1;
                    assert_eq!(tree.insert(*bounds), expected, "seed {seed}: {bounds:?}");
                    inserted.push((*bounds, expected));
                }
            }
        }
    }
}
