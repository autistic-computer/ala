#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct Cursor {
    pub line: usize,
    pub index: usize,
    pub affinity: Affinity,
}

impl Cursor {
    pub const fn new(line: usize, index: usize) -> Self {
        Self::new_with_affinity(line, index, Affinity::Before)
    }

    pub const fn new_with_affinity(line: usize, index: usize, affinity: Affinity) -> Self {
        Self {
            line,
            index,
            affinity,
        }
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub enum Affinity {
    #[default]
    Before,
    After,
}

impl Affinity {
    pub fn before(&self) -> bool {
        *self == Self::Before
    }

    pub fn after(&self) -> bool {
        *self == Self::After
    }

    pub const fn from_before(before: bool) -> Self {
        if before {
            Self::Before
        } else {
            Self::After
        }
    }

    pub const fn from_after(after: bool) -> Self {
        if after {
            Self::After
        } else {
            Self::Before
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct LayoutCursor {
    pub line: usize,
    pub layout: usize,
    pub glyph: usize,
}

impl LayoutCursor {
    pub const fn new(line: usize, layout: usize, glyph: usize) -> Self {
        Self {
            line,
            layout,
            glyph,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Motion {
    LayoutCursor(LayoutCursor),
    Previous,
    Next,
    Left,
    Right,
    Up,
    Down,
    Home,
    SoftHome,
    End,
    ParagraphStart,
    ParagraphEnd,
    PageUp,
    PageDown,
    Vertical(i32),
    PreviousWord,
    NextWord,
    LeftWord,
    RightWord,
    BufferStart,
    BufferEnd,
    GotoLine(usize),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Scroll {
    pub line: usize,
    pub vertical: f32,
    pub horizontal: f32,
}

impl Scroll {
    pub const fn new(line: usize, vertical: f32, horizontal: f32) -> Self {
        Self {
            line,
            vertical,
            horizontal,
        }
    }
}
