use super::{ResolvedText, Span, Wrap};
use crate::collections::FxHashMap;
use crate::window::{Window, quantize_glyph_origin};
use crate::{
    AtlasTile, Bounds, DrawOrder, FontId, GlyphId, Hsla, LineLayout, MonochromeSprite, Pixels,
    Point, RenderGlyphParams, SUBPIXEL_VARIANTS_Y, ScaledPixels, SharedString, Size, TextRun,
    TransformationMatrix, WindowTextSystem, point, px,
};
use anyhow::Result;
use smallvec::SmallVec;
use std::{ops::Range, sync::Arc};

const _: () = assert!(SUBPIXEL_VARIANTS_Y == 1);

const KEEP_FRAMES: u64 = 512;

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
    color: Hsla,
    bounding_box: Size<Pixels>,
}

struct SourceLine {
    layout: Option<Arc<LineLayout>>,
    text: Range<usize>,
    glyphs: Range<u32>,
    first_visual: u32,
    baseline: Pixels,
    width: Pixels,
}

struct VisualLine {
    glyphs: Range<u32>,
    start_x: Pixels,
    baseline: Pixels,
}

struct Ink {
    color: Hsla,
    smoothing: bool,
    strength: u8,
}

pub(super) struct Paragraph {
    key: Option<(u64, u16)>,
    pub(super) version: u64,
    pub(super) used: u64,
    text: SharedString,
    spans: Vec<Span>,
    lines: Vec<SourceLine>,
    runs: Vec<Run>,
    glyphs: Vec<Glyph>,
    visual_lines: Vec<VisualLine>,
    wrapped: Option<Option<Pixels>>,
    wrap_from: usize,
    pub(super) width: Pixels,
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
            spans: Vec::new(),
            lines: Vec::new(),
            runs: Vec::new(),
            glyphs: Vec::new(),
            visual_lines: Vec::new(),
            wrapped: None,
            wrap_from: 0,
            width: Pixels::ZERO,
            generations: (0, 0, 0),
            has_emoji: false,
            has_offsets: false,
        }
    }

    pub(super) fn visual_line_count(&self) -> usize {
        self.visual_lines.len()
    }

    fn kept_lines(&self, text: &str, spans: &[Span]) -> usize {
        let Some(last) = self.lines.last() else {
            return 0;
        };
        let kept = last.text.start;
        if kept == 0
            || self.wrap_from < self.lines.len()
            || text.len() < kept
            || text.as_bytes()[..kept] != self.text.as_bytes()[..kept]
        {
            return 0;
        }
        let mut covered = 0;
        let mut compared = 0;
        for (old, new) in self.spans.iter().zip(spans) {
            if covered >= kept {
                break;
            }
            if old.style != new.style
                || (old.len != new.len && covered + old.len.min(new.len) < kept)
            {
                return 0;
            }
            covered += old.len;
            compared += 1;
        }
        if covered < kept && (compared < self.spans.len() || compared < spans.len()) {
            return 0;
        }
        self.lines.len() - 1
    }

    pub(super) fn shape(
        &mut self,
        text: &SharedString,
        spans: &[Span],
        base: &ResolvedText,
        styles: &[ResolvedText],
        system: &WindowTextSystem,
    ) {
        let kept = self.kept_lines(text, spans);
        let mut start = 0;
        if kept == 0 {
            self.lines.clear();
            self.runs.clear();
            self.glyphs.clear();
            self.visual_lines.clear();
            self.has_emoji = false;
            self.has_offsets = false;
        } else {
            let line = &self.lines[kept];
            start = line.text.start;
            self.glyphs.truncate(line.glyphs.start as usize);
            self.visual_lines.truncate(line.first_visual as usize);
            self.lines.truncate(kept);
        }
        self.wrap_from = kept;
        self.text = text.clone();
        self.spans.clear();
        self.spans.extend_from_slice(spans);

        let mut span = 0;
        let mut span_start = 0;
        let mut runs: SmallVec<[TextRun; 4]> = SmallVec::new();
        let mut colors: SmallVec<[(usize, Hsla); 4]> = SmallVec::new();
        for line in text[start..].split('\n') {
            let end = start + line.len();
            while span < spans.len() && span_start + spans[span].len <= start {
                span_start += spans[span].len;
                span += 1;
            }
            runs.clear();
            colors.clear();
            let mut at = start;
            let mut next = span;
            let mut next_start = span_start;
            loop {
                let (style, stop) = match spans.get(next) {
                    Some(span) => (
                        &styles[span.style.0 as usize],
                        (next_start + span.len).min(end),
                    ),
                    None => (base, end),
                };
                if stop > at || (stop >= end && runs.is_empty()) {
                    let mut run = style.run.clone();
                    run.len = stop - at;
                    colors.push((stop - start, run.color));
                    runs.push(run);
                    at = stop;
                }
                if at >= end {
                    break;
                }
                next_start += spans[next].len;
                next += 1;
            }
            let layout = system.layout_line(line, base.size, &runs, None);
            let first = self.glyphs.len() as u32;
            for shaped in &layout.runs {
                let mut run = u16::MAX;
                let mut run_color = None;
                for glyph in &shaped.glyphs {
                    let color = colors
                        .iter()
                        .find(|(stop, _)| glyph.index < *stop)
                        .or(colors.last())
                        .map_or(base.run.color, |(_, color)| *color);
                    if run_color != Some(color) {
                        run_color = Some(color);
                        run = self.run(shaped.font_id, color, base.size, system);
                    }
                    self.has_emoji |= glyph.is_emoji;
                    self.has_offsets |= glyph.position.y != Pixels::ZERO;
                    self.glyphs.push(Glyph {
                        x: glyph.position.x,
                        y: glyph.position.y,
                        id: glyph.id,
                        run,
                        is_emoji: glyph.is_emoji,
                        placed_at: px(f32::NAN),
                        device_x: ScaledPixels(0.),
                        raster_y: ScaledPixels(0.),
                        size: Size::default(),
                        tile: None,
                    });
                }
            }
            let padding_top = (base.line_height - layout.ascent - layout.descent) / 2.;
            self.lines.push(SourceLine {
                baseline: padding_top + layout.ascent,
                width: layout.width,
                text: start..end,
                glyphs: first..self.glyphs.len() as u32,
                first_visual: 0,
                layout: (base.wrap == Wrap::Words).then_some(layout),
            });
            start = end + 1;
        }
        self.width = self
            .lines
            .iter()
            .fold(Pixels::ZERO, |width, line| width.max(line.width));
    }

    fn run(
        &mut self,
        font_id: FontId,
        color: Hsla,
        size: Pixels,
        system: &WindowTextSystem,
    ) -> u16 {
        let found = self
            .runs
            .iter()
            .position(|run| run.font_id == font_id && run.color == color);
        let run = found.unwrap_or_else(|| {
            self.runs.push(Run {
                font_id,
                color,
                bounding_box: system.bounding_box(font_id, size).size,
            });
            self.runs.len() - 1
        });
        run as u16
    }

    pub(super) fn wrap(&mut self, width: Option<Pixels>) {
        let from = if self.wrapped == Some(width) {
            self.wrap_from
        } else {
            0
        };
        if from >= self.lines.len() {
            return;
        }
        self.wrapped = Some(width);
        self.wrap_from = self.lines.len();
        if from == 0 {
            self.visual_lines.clear();
        }
        for line in &mut self.lines[from..] {
            line.first_visual = self.visual_lines.len() as u32;
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

    pub(super) fn paint(
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
                    window.paint_glyph(at, run.font_id, glyph.id, style.size, run.color)?;
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
        out: &mut Vec<MonochromeSprite>,
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
        let opacity = window.element_opacity();
        let inks: SmallVec<[Ink; 4]> = self
            .runs
            .iter()
            .map(|run| {
                let dilation = window.text_system().glyph_dilation_for_color(run.color);
                Ink {
                    color: run.color.opacity(opacity),
                    smoothing: dilation > 0,
                    strength: match dilation {
                        0 => 0,
                        1 => 64,
                        2 => 128,
                        3 => 191,
                        _ => 255,
                    },
                }
            })
            .collect();
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
                let ink = &inks[glyph.run as usize];
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
                        font_smoothing: ink.smoothing,
                        font_smoothing_strength: ink.strength,
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
                out.push(MonochromeSprite {
                    order,
                    pad: 0,
                    bounds,
                    content_mask,
                    color: ink.color,
                    tile,
                    transformation: TransformationMatrix::unit(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Paragraphs {
    pub(super) slots: Vec<Paragraph>,
    index: FxHashMap<(u64, u16), u32>,
    free: Vec<u32>,
}

impl Paragraphs {
    pub(super) fn slot(&mut self, key: (u64, u16)) -> (u32, bool) {
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

    pub(super) fn sweep(&mut self, frame: u64) {
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
