use super::{Window, quantize_glyph_origin};
use crate::collections::FxHashMap;
use crate::{
    AtlasTile, BorderStyle, Bounds, ContentMask, Corners, DrawOrder, Edges, Font, FontId, GlyphId,
    Hsla, LineLayout, MonochromeSprite, Pixels, Point, RenderGlyphParams, SUBPIXEL_VARIANTS_Y,
    ScaledPixels, SharedString, Size, TextRun, TransformationMatrix, WindowTextSystem, phi, point,
    px, quad, size,
};
use anyhow::Result;
use std::{mem, ops::Range, sync::Arc};

const _: () = assert!(SUBPIXEL_VARIANTS_Y == 1);

const NONE: u32 = u32::MAX;
const SWEEP_EVERY: u64 = 256;
const KEEP_FRAMES: u64 = 512;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flow {
    #[default]
    Column,
    Row,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Extent {
    #[default]
    Fit,
    Grow,
    Fixed(Pixels),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Clip {
    #[default]
    Visible,
    Bounds,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Wrap {
    #[default]
    Words,
    Never,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: Pixels,
    pub top: Pixels,
    pub right: Pixels,
    pub bottom: Pixels,
}

impl Insets {
    pub const fn all(value: Pixels) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoxStyle {
    pub flow: Flow,
    pub width: Extent,
    pub height: Extent,
    pub padding: Insets,
    pub gap: Pixels,
    pub fill: Option<Hsla>,
    pub radius: Pixels,
    pub clip: Clip,
}

#[derive(Clone, Debug)]
pub struct TextStyle {
    pub font: Font,
    pub size: Pixels,
    pub color: Hsla,
    pub wrap: Wrap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BoxStyleId(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextStyleId(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

struct ResolvedText {
    run: TextRun,
    size: Pixels,
    line_height: Pixels,
    wrap: Wrap,
}

#[derive(Clone, Copy)]
struct Node {
    parent: u32,
    end: u32,
    children: u32,
    paragraph: u32,
    style: u16,
    shift: f32,
    fit: f32,
    main: f32,
    cross: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Node {
    fn is_text(&self) -> bool {
        self.paragraph != NONE
    }
}

#[derive(Clone, Copy)]
struct Glyph {
    x: Pixels,
    y: Pixels,
    id: GlyphId,
    run: u16,
    is_emoji: bool,
    placed_at: Pixels,
    device_x: ScaledPixels,
    raster_y: ScaledPixels,
    size: Size<ScaledPixels>,
    tile: Option<AtlasTile>,
}

struct Run {
    font_id: FontId,
    bounding_box: Size<Pixels>,
}

struct SourceLine {
    layout: Option<Arc<LineLayout>>,
    text: Range<usize>,
    glyphs: Range<u32>,
    baseline: Pixels,
}

struct VisualLine {
    glyphs: Range<u32>,
    start_x: Pixels,
    baseline: Pixels,
}

#[derive(Clone, Copy, PartialEq)]
enum Wrapped {
    Stale,
    At(Option<Pixels>),
}

struct Paragraph {
    key: Option<(u64, u16)>,
    version: u64,
    used: u64,
    text: SharedString,
    lines: Vec<SourceLine>,
    runs: Vec<Run>,
    glyphs: Vec<Glyph>,
    visual_lines: Vec<VisualLine>,
    wrapped: Wrapped,
    width: Pixels,
    generations: (usize, u64, u32),
    has_emoji: bool,
    has_offsets: bool,
}

impl Paragraph {
    fn new(key: Option<(u64, u16)>) -> Self {
        Self {
            key,
            version: 0,
            used: 0,
            text: SharedString::default(),
            lines: Vec::new(),
            runs: Vec::new(),
            glyphs: Vec::new(),
            visual_lines: Vec::new(),
            wrapped: Wrapped::Stale,
            width: Pixels::ZERO,
            generations: (0, 0, 0),
            has_emoji: false,
            has_offsets: false,
        }
    }

    fn shape(&mut self, text: &SharedString, style: &mut ResolvedText, system: &WindowTextSystem) {
        self.text = text.clone();
        self.lines.clear();
        self.runs.clear();
        self.glyphs.clear();
        self.wrapped = Wrapped::Stale;
        self.width = Pixels::ZERO;
        self.has_emoji = false;
        self.has_offsets = false;
        let mut start = 0;
        for line in text.split('\n') {
            style.run.len = line.len();
            let layout =
                system.layout_line(line, style.size, std::slice::from_ref(&style.run), None);
            let first = self.glyphs.len() as u32;
            for shaped in &layout.runs {
                let run = match self
                    .runs
                    .iter()
                    .position(|run| run.font_id == shaped.font_id)
                {
                    Some(run) => run,
                    None => {
                        self.runs.push(Run {
                            font_id: shaped.font_id,
                            bounding_box: system.bounding_box(shaped.font_id, style.size).size,
                        });
                        self.runs.len() - 1
                    }
                };
                for glyph in &shaped.glyphs {
                    self.has_emoji |= glyph.is_emoji;
                    self.has_offsets |= glyph.position.y != Pixels::ZERO;
                    self.glyphs.push(Glyph {
                        x: glyph.position.x,
                        y: glyph.position.y,
                        id: glyph.id,
                        run: run as u16,
                        is_emoji: glyph.is_emoji,
                        placed_at: px(f32::NAN),
                        device_x: ScaledPixels(0.),
                        raster_y: ScaledPixels(0.),
                        size: Size::default(),
                        tile: None,
                    });
                }
            }
            let padding_top = (style.line_height - layout.ascent - layout.descent) / 2.;
            self.width = self.width.max(layout.width);
            self.lines.push(SourceLine {
                baseline: padding_top + layout.ascent,
                text: start..start + line.len(),
                glyphs: first..self.glyphs.len() as u32,
                layout: (style.wrap == Wrap::Words).then_some(layout),
            });
            start += line.len() + 1;
        }
    }

    fn wrap(&mut self, width: Option<Pixels>) {
        if self.wrapped == Wrapped::At(width) {
            return;
        }
        self.wrapped = Wrapped::At(width);
        self.visual_lines.clear();
        for line in &self.lines {
            let mut start = line.glyphs.start;
            let mut start_x = Pixels::ZERO;
            if let (Some(width), Some(layout)) = (width, &line.layout) {
                let boundaries =
                    layout.compute_wrap_boundaries(&self.text[line.text.clone()], width, None);
                let mut run = 0;
                let mut run_start = line.glyphs.start;
                for boundary in boundaries {
                    while run < boundary.run_ix {
                        run_start += layout.runs[run].glyphs.len() as u32;
                        run += 1;
                    }
                    let glyph = run_start + boundary.glyph_ix as u32;
                    self.visual_lines.push(VisualLine {
                        glyphs: start..glyph,
                        start_x,
                        baseline: line.baseline,
                    });
                    start = glyph;
                    start_x = self.glyphs[glyph as usize].x;
                }
            }
            self.visual_lines.push(VisualLine {
                glyphs: start..line.glyphs.end,
                start_x,
                baseline: line.baseline,
            });
        }
    }

    fn paint(
        &mut self,
        style: &ResolvedText,
        origin: Point<Pixels>,
        window: &mut Window,
    ) -> Result<()> {
        let direct = window.text_fade.is_none()
            && !self.has_emoji
            && !self.has_offsets
            && self
                .runs
                .iter()
                .all(|run| !window.should_use_subpixel_rendering(run.font_id, style.size));
        let Some((order, mut sprites)) = direct
            .then(|| window.next_frame.scene.take_layer_sprites())
            .flatten()
        else {
            return self.paint_glyphs(style, origin, window);
        };
        let first = sprites.len();
        let result = self.paint_sprites(style, origin, order, &mut sprites, window);
        window
            .next_frame
            .scene
            .restore_layer_sprites(sprites, first);
        result
    }

    fn first_visible_line(
        &self,
        style: &ResolvedText,
        origin: Point<Pixels>,
        mask: &Bounds<Pixels>,
    ) -> usize {
        let tallest = self.runs.iter().fold(Pixels::ZERO, |tallest, run| {
            tallest.max(run.bounding_box.height)
        });
        ((mask.top() - tallest - origin.y) / style.line_height)
            .floor()
            .max(0.) as usize
    }

    fn paint_glyphs(
        &self,
        style: &ResolvedText,
        origin: Point<Pixels>,
        window: &mut Window,
    ) -> Result<()> {
        let mask = window.content_mask().bounds;
        let first = self.first_visible_line(style, origin, &mask);
        for (index, line) in self.visual_lines.iter().enumerate().skip(first) {
            let top = origin.y + style.line_height * index;
            if top >= mask.bottom() {
                break;
            }
            let mut x = origin.x;
            let mut previous_x = line.start_x;
            for glyph in &self.glyphs[line.glyphs.start as usize..line.glyphs.end as usize] {
                x += glyph.x - previous_x;
                previous_x = glyph.x;
                let run = &self.runs[glyph.run as usize];
                let visible = Bounds {
                    origin: point(x, top),
                    size: run.bounding_box,
                }
                .intersects(&mask);
                if !visible {
                    continue;
                }
                let at = point(x, top) + point(px(0.), line.baseline) + point(px(0.), glyph.y);
                if glyph.is_emoji {
                    window.paint_emoji(at, run.font_id, glyph.id, style.size)?;
                } else {
                    window.paint_glyph(at, run.font_id, glyph.id, style.size, style.run.color)?;
                }
            }
        }
        Ok(())
    }

    fn paint_sprites(
        &mut self,
        style: &ResolvedText,
        origin: Point<Pixels>,
        order: DrawOrder,
        sprites: &mut Vec<MonochromeSprite>,
        window: &Window,
    ) -> Result<()> {
        let scale_factor = window.scale_factor();
        let (font_generation, atlas_generation) = window.glyph_run_generations();
        let generations = (font_generation, atlas_generation, scale_factor.to_bits());
        if self.generations != generations {
            self.generations = generations;
            for glyph in &mut self.glyphs {
                glyph.placed_at = px(f32::NAN);
            }
        }
        let mask = window.content_mask().bounds;
        let content_mask = window.snapped_content_mask();
        let color = style.run.color.opacity(window.element_opacity());
        let dilation = window
            .text_system()
            .glyph_dilation_for_color(style.run.color);
        let font_smoothing_strength = match dilation {
            0 => 0,
            1 => 64,
            2 => 128,
            3 => 191,
            _ => 255,
        };
        let first = self.first_visible_line(style, origin, &mask);
        for (index, line) in self.visual_lines.iter().enumerate().skip(first) {
            let top = origin.y + style.line_height * index;
            if top >= mask.bottom() {
                break;
            }
            let baseline = top + line.baseline;
            let device_y = quantize_glyph_origin(point(px(0.), baseline).scale(scale_factor))
                .0
                .y;
            let mut x = origin.x;
            let mut previous_x = line.start_x;
            for glyph in &mut self.glyphs[line.glyphs.start as usize..line.glyphs.end as usize] {
                x += glyph.x - previous_x;
                previous_x = glyph.x;
                let run = &self.runs[glyph.run as usize];
                let visible = Bounds {
                    origin: point(x, top),
                    size: run.bounding_box,
                }
                .intersects(&mask);
                if !visible {
                    continue;
                }
                if glyph.placed_at != x {
                    let (integer_origin, variant) =
                        quantize_glyph_origin(point(x, baseline).scale(scale_factor));
                    let params = RenderGlyphParams {
                        font_id: run.font_id,
                        glyph_id: glyph.id,
                        font_size: style.size,
                        subpixel_variant: variant,
                        scale_factor,
                        is_emoji: false,
                        subpixel_rendering: false,
                        font_smoothing: dilation > 0,
                        font_smoothing_strength,
                        synthetic_bold: false,
                        synthetic_italic: false,
                    };
                    let resolved = window.glyph_tile(&params)?;
                    glyph.placed_at = x;
                    glyph.tile = None;
                    if let Some((raster_bounds, tile)) = resolved {
                        let raster_origin: Point<ScaledPixels> =
                            raster_bounds.origin.map(Into::into);
                        glyph.device_x = integer_origin.x + raster_origin.x;
                        glyph.raster_y = raster_origin.y;
                        glyph.size = tile.bounds.size.map(Into::into);
                        glyph.tile = Some(tile);
                    }
                }
                let Some(tile) = glyph.tile else {
                    continue;
                };
                let bounds = Bounds {
                    origin: point(glyph.device_x, device_y + glyph.raster_y),
                    size: glyph.size,
                };
                if bounds.intersect(&content_mask.bounds).is_empty() {
                    continue;
                }
                sprites.push(MonochromeSprite {
                    order,
                    pad: 0,
                    bounds,
                    content_mask,
                    color,
                    tile,
                    transformation: TransformationMatrix::unit(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct Paragraphs {
    slots: Vec<Paragraph>,
    index: FxHashMap<(u64, u16), u32>,
    free: Vec<u32>,
}

impl Paragraphs {
    fn slot(&mut self, key: (u64, u16)) -> (u32, bool) {
        if let Some(slot) = self.index.get(&key) {
            return (*slot, false);
        }
        let slot = match self.free.pop() {
            Some(slot) => {
                self.slots[slot as usize].key = Some(key);
                slot
            }
            None => {
                self.slots.push(Paragraph::new(Some(key)));
                self.slots.len() as u32 - 1
            }
        };
        self.index.insert(key, slot);
        (slot, true)
    }

    fn sweep(&mut self, frame: u64) {
        for (slot, paragraph) in self.slots.iter_mut().enumerate() {
            let Some(key) = paragraph.key else {
                continue;
            };
            if paragraph.used + KEEP_FRAMES < frame {
                self.index.remove(&key);
                *paragraph = Paragraph::new(None);
                self.free.push(slot as u32);
            }
        }
    }
}

pub struct Ui {
    text_system: Arc<WindowTextSystem>,
    rem_size: Pixels,
    scale_factor: f32,
    box_styles: Vec<BoxStyle>,
    text_styles: Vec<ResolvedText>,
    nodes: Vec<Node>,
    open: Vec<u32>,
    clips: Vec<(u32, ContentMask<Pixels>)>,
    paragraphs: Paragraphs,
    frame: u64,
}

impl Ui {
    pub fn new(window: &Window) -> Self {
        Self {
            text_system: window.text_system().clone(),
            rem_size: window.rem_size(),
            scale_factor: window.scale_factor(),
            box_styles: Vec::new(),
            text_styles: Vec::new(),
            nodes: Vec::new(),
            open: Vec::new(),
            clips: Vec::new(),
            paragraphs: Paragraphs::default(),
            frame: 0,
        }
    }

    pub fn box_style(&mut self, style: BoxStyle) -> BoxStyleId {
        self.box_styles.push(style);
        BoxStyleId(self.box_styles.len() as u16 - 1)
    }

    pub fn text_style(&mut self, style: TextStyle) -> TextStyleId {
        let line_height = phi().to_pixels(style.size.into(), self.rem_size);
        let line_height = px(
            crate::util::round_to_device_pixel(line_height.0, self.scale_factor)
                / self.scale_factor,
        );
        self.text_styles.push(ResolvedText {
            run: TextRun {
                len: 0,
                font: style.font,
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            },
            size: style.size,
            line_height,
            wrap: style.wrap,
        });
        TextStyleId(self.text_styles.len() as u16 - 1)
    }

    pub fn line_height(&self, style: TextStyleId) -> Pixels {
        self.text_styles[style.0 as usize].line_height
    }

    pub fn begin(&mut self) {
        self.nodes.clear();
        self.open.clear();
    }

    fn push(&mut self, style: u16, paragraph: u32, shift: Pixels) -> u32 {
        let parent = self.open.last().copied().unwrap_or(NONE);
        if parent != NONE {
            self.nodes[parent as usize].children += 1;
        }
        let index = self.nodes.len() as u32;
        self.nodes.push(Node {
            parent,
            end: index + 1,
            children: 0,
            paragraph,
            style,
            shift: shift.0,
            fit: 0.,
            main: 0.,
            cross: 0.,
            x: 0.,
            y: 0.,
            w: 0.,
            h: 0.,
        });
        index
    }

    pub fn open(&mut self, style: BoxStyleId) -> NodeId {
        self.open_shifted(style, Pixels::ZERO)
    }

    pub fn open_shifted(&mut self, style: BoxStyleId, shift: Pixels) -> NodeId {
        let index = self.push(style.0, NONE, shift);
        self.open.push(index);
        NodeId(index)
    }

    pub fn close(&mut self) {
        let index = self.open.pop().expect("close without a matching open");
        self.nodes[index as usize].end = self.nodes.len() as u32;
    }

    pub fn text(
        &mut self,
        key: u64,
        version: u64,
        style: TextStyleId,
        text: &SharedString,
    ) -> NodeId {
        let (slot, new) = self.paragraphs.slot((key, style.0));
        let paragraph = &mut self.paragraphs.slots[slot as usize];
        if new || paragraph.version != version {
            paragraph.version = version;
            paragraph.shape(
                text,
                &mut self.text_styles[style.0 as usize],
                &self.text_system,
            );
        }
        paragraph.used = self.frame;
        NodeId(self.push(style.0, slot, Pixels::ZERO))
    }

    pub fn layout(&mut self, available: Size<Pixels>) {
        assert!(
            self.open.is_empty(),
            "layout with {} open boxes",
            self.open.len()
        );
        let Self {
            nodes,
            box_styles,
            text_styles,
            paragraphs,
            ..
        } = self;
        if nodes.is_empty() {
            return;
        }

        for index in (0..nodes.len()).rev() {
            let node = nodes[index];
            let fit = if node.is_text() {
                match text_styles[node.style as usize].wrap {
                    Wrap::Words => 0.,
                    Wrap::Never => paragraphs.slots[node.paragraph as usize].width.0,
                }
            } else {
                let style = &box_styles[node.style as usize];
                match style.width {
                    Extent::Fixed(width) => width.0,
                    Extent::Fit | Extent::Grow => {
                        let inner = match style.flow {
                            Flow::Row => {
                                node.main + style.gap.0 * node.children.saturating_sub(1) as f32
                            }
                            Flow::Column => node.cross,
                        };
                        inner + style.padding.left.0 + style.padding.right.0
                    }
                }
            };
            nodes[index].fit = fit;
            if node.parent != NONE {
                let parent = &mut nodes[node.parent as usize];
                match box_styles[parent.style as usize].flow {
                    Flow::Row => parent.main += fit,
                    Flow::Column => parent.cross = parent.cross.max(fit),
                }
            }
        }

        let root_style = &box_styles[nodes[0].style as usize];
        nodes[0].w = match root_style.width {
            Extent::Fixed(width) => width.0,
            Extent::Fit => nodes[0].fit.min(available.width.0),
            Extent::Grow => available.width.0,
        };
        for index in 0..nodes.len() {
            let node = nodes[index];
            if node.is_text() {
                continue;
            }
            let style = &box_styles[node.style as usize];
            let inner = (node.w - style.padding.left.0 - style.padding.right.0).max(0.);
            let extent = |child: &Node| {
                if child.is_text() {
                    match style.flow {
                        Flow::Column => Extent::Grow,
                        Flow::Row => Extent::Fit,
                    }
                } else {
                    box_styles[child.style as usize].width
                }
            };
            let mut grow_share = 0.;
            if style.flow == Flow::Row {
                let mut taken = style.gap.0 * node.children.saturating_sub(1) as f32;
                let mut growing = 0;
                let mut child = index + 1;
                while child < node.end as usize {
                    match extent(&nodes[child]) {
                        Extent::Grow => growing += 1,
                        Extent::Fit | Extent::Fixed(_) => taken += nodes[child].fit,
                    }
                    child = nodes[child].end as usize;
                }
                if growing > 0 {
                    grow_share = ((inner - taken) / growing as f32).max(0.);
                }
            }
            let mut child = index + 1;
            while child < node.end as usize {
                let width = match (extent(&nodes[child]), style.flow) {
                    (Extent::Fixed(width), _) => width.0,
                    (Extent::Fit, _) => nodes[child].fit.min(inner),
                    (Extent::Grow, Flow::Column) => inner,
                    (Extent::Grow, Flow::Row) => grow_share,
                };
                nodes[child].w = width;
                child = nodes[child].end as usize;
            }
            nodes[index].main = 0.;
            nodes[index].cross = 0.;
        }

        for index in (0..nodes.len()).rev() {
            let node = nodes[index];
            let height = if node.is_text() {
                let text = &text_styles[node.style as usize];
                let paragraph = &mut paragraphs.slots[node.paragraph as usize];
                paragraph.wrap(match text.wrap {
                    Wrap::Words => Some(px(node.w)),
                    Wrap::Never => None,
                });
                (text.line_height * paragraph.visual_lines.len()).0
            } else {
                let style = &box_styles[node.style as usize];
                match style.height {
                    Extent::Fixed(height) => height.0,
                    Extent::Grow if index == 0 => available.height.0,
                    Extent::Fit | Extent::Grow => {
                        let inner = match style.flow {
                            Flow::Column => {
                                node.main + style.gap.0 * node.children.saturating_sub(1) as f32
                            }
                            Flow::Row => node.cross,
                        };
                        inner + style.padding.top.0 + style.padding.bottom.0
                    }
                }
            };
            nodes[index].h = height;
            if node.parent != NONE {
                let parent = &mut nodes[node.parent as usize];
                match box_styles[parent.style as usize].flow {
                    Flow::Column => parent.main += height,
                    Flow::Row => parent.cross = parent.cross.max(height),
                }
            }
        }

        nodes[0].x = 0.;
        nodes[0].y = 0.;
        for index in 0..nodes.len() {
            let node = nodes[index];
            if node.is_text() {
                continue;
            }
            let style = &box_styles[node.style as usize];
            let mut x = node.x + style.padding.left.0;
            let mut y = node.y + style.padding.top.0 + node.shift;
            let mut child = index + 1;
            while child < node.end as usize {
                nodes[child].x = x;
                nodes[child].y = y;
                match style.flow {
                    Flow::Column => y += nodes[child].h + style.gap.0,
                    Flow::Row => x += nodes[child].w + style.gap.0,
                }
                child = nodes[child].end as usize;
            }
        }
    }

    pub fn size(&self) -> Size<Pixels> {
        self.nodes
            .first()
            .map_or(Size::default(), |root| size(px(root.w), px(root.h)))
    }

    pub fn bounds(&self, node: NodeId) -> Bounds<Pixels> {
        let node = &self.nodes[node.0 as usize];
        Bounds::new(point(px(node.x), px(node.y)), size(px(node.w), px(node.h)))
    }

    pub fn paint(&mut self, origin: Point<Pixels>, window: &mut Window) -> Result<()> {
        let bounds = Bounds::new(origin, self.size());
        let result = window.paint_layer(bounds, |window| {
            self.paint_fills(origin, window);
            self.paint_text(origin, window)
        });
        self.frame += 1;
        if self.frame.is_multiple_of(SWEEP_EVERY) {
            self.paragraphs.sweep(self.frame);
        }
        result
    }

    fn paint_fills(&mut self, origin: Point<Pixels>, window: &mut Window) {
        let mut clips = mem::take(&mut self.clips);
        clips.clear();
        for (index, node) in self.nodes.iter().enumerate() {
            while clips.last().is_some_and(|(end, _)| index as u32 >= *end) {
                clips.pop();
            }
            if node.is_text() {
                continue;
            }
            let style = &self.box_styles[node.style as usize];
            let bounds = Bounds::new(
                origin + point(px(node.x), px(node.y)),
                size(px(node.w), px(node.h)),
            );
            if let Some(fill) = style.fill {
                let mask = clips.last().map(|(_, mask)| *mask);
                window.with_content_mask(mask, |window| {
                    let radii =
                        window.resolve_corner_radii(Corners::all(style.radius), bounds.size);
                    let (radii, smoothing) = window.drawn_corners(radii, bounds.size, None);
                    let mut background = quad(
                        bounds,
                        radii,
                        fill,
                        Edges::default(),
                        fill.opacity(0.),
                        BorderStyle::default(),
                    );
                    background.corner_smoothing = smoothing;
                    window.paint_quad(background);
                });
            }
            if style.clip == Clip::Bounds {
                let mask = ContentMask { bounds };
                let mask = match clips.last() {
                    Some((_, outer)) => mask.intersect(outer),
                    None => mask,
                };
                clips.push((node.end, mask));
            }
        }
        self.clips = clips;
    }

    fn paint_text(&mut self, origin: Point<Pixels>, window: &mut Window) -> Result<()> {
        let mut clips = mem::take(&mut self.clips);
        clips.clear();
        let mut result = Ok(());
        for (index, node) in self.nodes.iter().enumerate() {
            while clips.last().is_some_and(|(end, _)| index as u32 >= *end) {
                clips.pop();
            }
            let at = origin + point(px(node.x), px(node.y));
            if node.is_text() {
                let style = &self.text_styles[node.style as usize];
                let paragraph = &mut self.paragraphs.slots[node.paragraph as usize];
                let mask = clips.last().map(|(_, mask)| *mask);
                result =
                    window.with_content_mask(mask, |window| paragraph.paint(style, at, window));
                if result.is_err() {
                    break;
                }
            } else if self.box_styles[node.style as usize].clip == Clip::Bounds {
                let mask = ContentMask {
                    bounds: Bounds::new(at, size(px(node.w), px(node.h))),
                };
                let mask = match clips.last() {
                    Some((_, outer)) => mask.intersect(outer),
                    None => mask,
                };
                clips.push((node.end, mask));
            }
        }
        self.clips = clips;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swept_slot_is_freed_once() {
        let mut paragraphs = Paragraphs::default();
        let (kept, _) = paragraphs.slot((0, 0));
        let (stale, _) = paragraphs.slot((1, 0));
        paragraphs.slots[kept as usize].used = KEEP_FRAMES + 2;
        paragraphs.sweep(KEEP_FRAMES + 1);
        paragraphs.sweep(KEEP_FRAMES + 2);
        assert_eq!(paragraphs.free, vec![stale]);
        assert_eq!(paragraphs.slot((0, 0)), (kept, false));
        assert_eq!(paragraphs.slot((2, 0)), (stale, true));
        assert_eq!(paragraphs.slot((3, 0)), (2, true));
    }
}
