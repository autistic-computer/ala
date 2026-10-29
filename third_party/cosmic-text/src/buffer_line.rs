#![allow(clippy::too_many_arguments)]

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};
use core::mem;

use crate::{
    Align, Attrs, AttrsList, Cached, Ellipsize, FontSystem, Hinting, LayoutLine, LayoutRunIter,
    LineEnding, ShapeLine, Shaping, Wrap,
};

#[derive(Clone, Debug)]
pub struct BufferLine {
    text: String,
    ending: LineEnding,
    attrs_list: AttrsList,
    align: Option<Align>,
    shape_opt: Cached<ShapeLine>,
    layout_opt: Cached<Vec<LayoutLine>>,
    shaping: Shaping,
    metadata: Option<usize>,
}

impl BufferLine {
    pub fn new<T: Into<String>>(
        text: T,
        ending: LineEnding,
        attrs_list: AttrsList,
        shaping: Shaping,
    ) -> Self {
        Self {
            text: text.into(),
            ending,
            attrs_list,
            align: None,
            shape_opt: Cached::Empty,
            layout_opt: Cached::Empty,
            shaping,
            metadata: None,
        }
    }

    pub fn reset_new<T: Into<String>>(
        &mut self,
        text: T,
        ending: LineEnding,
        attrs_list: AttrsList,
        shaping: Shaping,
    ) {
        self.text = text.into();
        self.ending = ending;
        self.attrs_list = attrs_list;
        self.align = None;
        self.shape_opt.set_unused();
        self.layout_opt.set_unused();
        self.shaping = shaping;
        self.metadata = None;
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text<T: AsRef<str>>(
        &mut self,
        text: T,
        ending: LineEnding,
        attrs_list: AttrsList,
    ) -> bool {
        let text = text.as_ref();
        if text != self.text || ending != self.ending || attrs_list != self.attrs_list {
            self.text.clear();
            self.text.push_str(text);
            self.ending = ending;
            self.attrs_list = attrs_list;
            self.reset();
            true
        } else {
            false
        }
    }

    pub fn into_text(self) -> String {
        self.text
    }

    pub const fn ending(&self) -> LineEnding {
        self.ending
    }

    pub fn set_ending(&mut self, ending: LineEnding) -> bool {
        if ending != self.ending {
            self.ending = ending;
            self.reset_shaping();
            true
        } else {
            false
        }
    }

    pub const fn attrs_list(&self) -> &AttrsList {
        &self.attrs_list
    }

    pub fn set_attrs_list(&mut self, attrs_list: AttrsList) -> bool {
        if attrs_list != self.attrs_list {
            self.attrs_list = attrs_list;
            self.reset_shaping();
            true
        } else {
            false
        }
    }

    pub const fn align(&self) -> Option<Align> {
        self.align
    }

    pub fn set_align(&mut self, align: Option<Align>) -> bool {
        if align != self.align {
            self.align = align;
            self.reset_layout();
            true
        } else {
            false
        }
    }

    pub fn append(&mut self, other: &Self) {
        let len = self.text.len();
        self.text.push_str(other.text());

        self.ending = other.ending();

        if other.attrs_list.defaults() != self.attrs_list.defaults() {
            self.attrs_list
                .add_span(len..len + other.text().len(), &other.attrs_list.defaults());
        }

        for (other_range, attrs) in other.attrs_list.spans_iter() {
            let range = other_range.start + len..other_range.end + len;
            self.attrs_list.add_span(range, &attrs.as_attrs());
        }

        self.reset();
    }

    pub fn split_off(&mut self, index: usize) -> Self {
        let text = self.text.split_off(index);
        let attrs_list = self.attrs_list.split_off(index);
        self.reset();

        let mut new = Self::new(text, self.ending, attrs_list, self.shaping);
        self.ending = LineEnding::None;
        new.align = self.align;
        new
    }

    pub fn reset(&mut self) {
        self.metadata = None;
        self.reset_shaping();
    }

    pub fn reset_shaping(&mut self) {
        self.shape_opt.set_unused();
        self.reset_layout();
    }

    pub fn reset_layout(&mut self) {
        self.layout_opt.set_unused();
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn shape(&mut self, font_system: &mut FontSystem, tab_width: u16) -> &ShapeLine {
        if self.shape_opt.is_unused() {
            let mut line = self
                .shape_opt
                .take_unused()
                .unwrap_or_else(ShapeLine::empty);
            line.build(
                font_system,
                &self.text,
                &self.attrs_list,
                self.shaping,
                tab_width,
            );
            self.shape_opt.set_used(line);
            self.layout_opt.set_unused();
        }
        self.shape_opt.get().expect("shape not found")
    }

    pub const fn shape_opt(&self) -> Option<&ShapeLine> {
        self.shape_opt.get()
    }

    pub const fn needs_reshaping(&self) -> bool {
        self.shape_opt.is_invalidated() || self.layout_opt.is_invalidated()
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn layout(
        &mut self,
        font_system: &mut FontSystem,
        font_size: f32,
        width_opt: Option<f32>,
        wrap: Wrap,
        ellipsize: Ellipsize,
        match_mono_width: Option<f32>,
        tab_width: u16,
        hinting: Hinting,
    ) -> &[LayoutLine] {
        if self.layout_opt.is_unused() {
            let align = self.align;
            let mut layout = self
                .layout_opt
                .take_unused()
                .unwrap_or_else(|| Vec::with_capacity(1));
            let shape = self.shape(font_system, tab_width);
            shape.layout_to_buffer(
                &mut font_system.shape_buffer,
                font_size,
                width_opt,
                wrap,
                ellipsize,
                align,
                &mut layout,
                match_mono_width,
                hinting,
            );
            self.layout_opt.set_used(layout);
        }
        self.layout_opt.get().expect("layout not found")
    }

    pub const fn layout_opt(&self) -> Option<&Vec<LayoutLine>> {
        self.layout_opt.get()
    }

    pub fn layout_runs(&self, height_opt: Option<f32>, line_height: f32) -> LayoutRunIter<'_> {
        LayoutRunIter::from_lines(core::slice::from_ref(self), height_opt, line_height, 0.0, 0)
    }

    pub const fn metadata(&self) -> Option<usize> {
        self.metadata
    }

    pub fn set_metadata(&mut self, metadata: usize) {
        self.metadata = Some(metadata);
    }

    pub(crate) fn empty() -> Self {
        Self {
            text: String::default(),
            ending: LineEnding::None,
            attrs_list: AttrsList::new(&Attrs::new()),
            align: None,
            shape_opt: Cached::Empty,
            layout_opt: Cached::Empty,
            shaping: Shaping::Advanced,
            metadata: None,
        }
    }

    pub(crate) fn reclaim_attrs(&mut self) -> AttrsList {
        mem::replace(&mut self.attrs_list, AttrsList::new(&Attrs::new()))
    }

    pub(crate) fn reclaim_text(&mut self) -> String {
        let mut text = mem::take(&mut self.text);
        text.clear();
        text
    }
}
