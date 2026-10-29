use alloc::sync::Arc;
#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};
use core::cmp;
use unicode_segmentation::UnicodeSegmentation;

use crate::{AttrsList, BorrowedWithFontSystem, Buffer, Cursor, FontSystem, Motion};

pub use self::editor::*;
mod editor;

#[cfg(feature = "syntect")]
pub use self::syntect::*;
#[cfg(feature = "syntect")]
mod syntect;

#[cfg(feature = "vi")]
pub use self::vi::*;
#[cfg(feature = "vi")]
mod vi;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Motion(Motion),
    Escape,
    Insert(char),
    Enter,
    Backspace,
    Delete,
    Indent,
    Unindent,
    Click { x: i32, y: i32 },
    DoubleClick { x: i32, y: i32 },
    TripleClick { x: i32, y: i32 },
    Drag { x: i32, y: i32 },
    Scroll { pixels: f32 },
}

#[derive(Debug)]
pub enum BufferRef<'buffer> {
    Owned(Buffer),
    Borrowed(&'buffer mut Buffer),
    Arc(Arc<Buffer>),
}

impl Clone for BufferRef<'_> {
    fn clone(&self) -> Self {
        match self {
            Self::Owned(buffer) => Self::Owned(buffer.clone()),
            Self::Borrowed(buffer) => Self::Owned((*buffer).clone()),
            Self::Arc(buffer) => Self::Arc(buffer.clone()),
        }
    }
}

impl From<Buffer> for BufferRef<'_> {
    fn from(buffer: Buffer) -> Self {
        Self::Owned(buffer)
    }
}

impl<'buffer> From<&'buffer mut Buffer> for BufferRef<'buffer> {
    fn from(buffer: &'buffer mut Buffer) -> Self {
        Self::Borrowed(buffer)
    }
}

impl From<Arc<Buffer>> for BufferRef<'_> {
    fn from(arc: Arc<Buffer>) -> Self {
        Self::Arc(arc)
    }
}

#[derive(Clone, Debug)]
pub struct ChangeItem {
    pub start: Cursor,
    pub end: Cursor,
    pub text: String,
    pub insert: bool,
}

impl ChangeItem {
    pub fn reverse(&mut self) {
        self.insert = !self.insert;
    }
}

#[derive(Clone, Debug, Default)]
pub struct Change {
    pub items: Vec<ChangeItem>,
}

impl Change {
    pub fn reverse(&mut self) {
        self.items.reverse();
        for item in &mut self.items {
            item.reverse();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Selection {
    None,
    Normal(Cursor),
    Line(Cursor),
    Word(Cursor),
}

pub trait Edit<'buffer> {
    fn borrow_with<'font_system>(
        &'font_system mut self,
        font_system: &'font_system mut FontSystem,
    ) -> BorrowedWithFontSystem<'font_system, Self>
    where
        Self: Sized,
    {
        BorrowedWithFontSystem {
            inner: self,
            font_system,
        }
    }

    fn buffer_ref(&self) -> &BufferRef<'buffer>;

    fn buffer_ref_mut(&mut self) -> &mut BufferRef<'buffer>;

    fn with_buffer<F: FnOnce(&Buffer) -> T, T>(&self, f: F) -> T {
        match self.buffer_ref() {
            BufferRef::Owned(buffer) => f(buffer),
            BufferRef::Borrowed(buffer) => f(buffer),
            BufferRef::Arc(buffer) => f(buffer),
        }
    }

    fn with_buffer_mut<F: FnOnce(&mut Buffer) -> T, T>(&mut self, f: F) -> T {
        match self.buffer_ref_mut() {
            BufferRef::Owned(buffer) => f(buffer),
            BufferRef::Borrowed(buffer) => f(buffer),
            BufferRef::Arc(buffer) => f(Arc::make_mut(buffer)),
        }
    }

    fn redraw(&self) -> bool {
        self.with_buffer(super::buffer::Buffer::redraw)
    }

    fn set_redraw(&mut self, redraw: bool) {
        self.with_buffer_mut(|buffer| buffer.set_redraw(redraw));
    }

    fn cursor(&self) -> Cursor;

    fn set_cursor(&mut self, cursor: Cursor);

    fn selection(&self) -> Selection;

    fn set_selection(&mut self, selection: Selection);

    fn selection_bounds(&self) -> Option<(Cursor, Cursor)> {
        self.with_buffer(|buffer| {
            let cursor = self.cursor();
            match self.selection() {
                Selection::None => None,
                Selection::Normal(select) => match select.line.cmp(&cursor.line) {
                    cmp::Ordering::Greater => Some((cursor, select)),
                    cmp::Ordering::Less => Some((select, cursor)),
                    cmp::Ordering::Equal => {
                        if select.index < cursor.index {
                            Some((select, cursor))
                        } else {
                            Some((cursor, select))
                        }
                    }
                },
                Selection::Line(select) => {
                    let start_line = cmp::min(select.line, cursor.line);
                    let end_line = cmp::max(select.line, cursor.line);
                    let end_index = buffer.lines[end_line].text().len();
                    Some((Cursor::new(start_line, 0), Cursor::new(end_line, end_index)))
                }
                Selection::Word(select) => {
                    let (mut start, mut end) = match select.line.cmp(&cursor.line) {
                        cmp::Ordering::Greater => (cursor, select),
                        cmp::Ordering::Less => (select, cursor),
                        cmp::Ordering::Equal => {
                            if select.index < cursor.index {
                                (select, cursor)
                            } else {
                                (cursor, select)
                            }
                        }
                    };

                    {
                        let line = &buffer.lines[start.line];
                        start.index = line
                            .text()
                            .unicode_word_indices()
                            .rev()
                            .map(|(i, _)| i)
                            .find(|&i| i < start.index)
                            .unwrap_or(0);
                    }

                    {
                        let line = &buffer.lines[end.line];
                        end.index = line
                            .text()
                            .unicode_word_indices()
                            .map(|(i, word)| i + word.len())
                            .find(|&i| i > end.index)
                            .unwrap_or_else(|| line.text().len());
                    }

                    Some((start, end))
                }
            }
        })
    }

    fn auto_indent(&self) -> bool;

    fn set_auto_indent(&mut self, auto_indent: bool);

    fn tab_width(&self) -> u16;

    fn set_tab_width(&mut self, tab_width: u16);

    fn shape_as_needed(&mut self, font_system: &mut FontSystem, prune: bool);

    fn delete_range(&mut self, start: Cursor, end: Cursor);

    fn insert_at(&mut self, cursor: Cursor, data: &str, attrs_list: Option<AttrsList>) -> Cursor;

    fn copy_selection(&self) -> Option<String>;

    fn delete_selection(&mut self) -> bool;

    fn insert_string(&mut self, data: &str, attrs_list: Option<AttrsList>) {
        self.delete_selection();
        let new_cursor = self.insert_at(self.cursor(), data, attrs_list);
        self.set_cursor(new_cursor);
    }

    fn apply_change(&mut self, change: &Change) -> bool;

    fn start_change(&mut self);

    fn finish_change(&mut self) -> Option<Change>;

    fn action(&mut self, font_system: &mut FontSystem, action: Action);

    fn cursor_position(&self) -> Option<(i32, i32)>;
}

impl<'buffer, E: Edit<'buffer>> BorrowedWithFontSystem<'_, E> {
    pub fn with_buffer_mut<F: FnOnce(&mut BorrowedWithFontSystem<Buffer>) -> T, T>(
        &mut self,
        f: F,
    ) -> T {
        self.inner.with_buffer_mut(|buffer| {
            let mut borrowed = BorrowedWithFontSystem {
                inner: buffer,
                font_system: self.font_system,
            };
            f(&mut borrowed)
        })
    }

    pub fn set_tab_width(&mut self, tab_width: u16) {
        self.inner.set_tab_width(tab_width);
    }

    pub fn shape_as_needed(&mut self, prune: bool) {
        self.inner.shape_as_needed(self.font_system, prune);
    }

    pub fn action(&mut self, action: Action) {
        self.inner.action(self.font_system, action);
    }
}
