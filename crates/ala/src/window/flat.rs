mod text;

use super::Window;
use crate::refineable::Refineable as _;
use crate::{
    A11ySubtreeBuilder, App, AvailableSpace, BorderStyle, Bounds, ContentMask, Context, Corners,
    CursorStyle, DispatchPhase, Edges, Element, ElementId, Entity, EntityId, Font, GlobalElementId,
    Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Role,
    ScrollWheelEvent, SharedString, Size, Style, StyleRefinement, Styled, TextRun,
    TransformationMatrix, WindowTextSystem, phi, point, px, quad, size,
};
use anyhow::Result;
use std::{cell::RefCell, mem, rc::Rc, sync::Arc};
use text::Paragraphs;

const NONE: u32 = u32::MAX;
const SWEEP_EVERY: u64 = 256;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
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
    pub align: Align,
    pub justify: Align,
    pub fill: Option<Hsla>,
    pub border: Pixels,
    pub border_color: Hsla,
    pub radius: Pixels,
    pub corner_share: Option<f32>,
    pub corner_smoothing: Option<f32>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub len: usize,
    pub style: TextStyleId,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    pub key: u64,
    pub button: MouseButton,
    pub count: usize,
    pub modifiers: Modifiers,
    pub position: Point<Pixels>,
}

struct ResolvedText {
    run: TextRun,
    size: Pixels,
    line_height: Pixels,
    wrap: Wrap,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Box,
    Text,
    Icon,
}

#[derive(Clone, Copy)]
struct Node {
    parent: u32,
    end: u32,
    children: u32,
    payload: u32,
    target: u32,
    style: u16,
    kind: Kind,
    shift: f32,
    fit: f32,
    main: f32,
    cross: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

struct Icon {
    path: SharedString,
    color: Hsla,
    size: Pixels,
}

#[derive(Clone, Copy)]
struct Target {
    key: u64,
    cursor: CursorStyle,
}

struct Label {
    node: u32,
    key: u64,
    role: Role,
    text: SharedString,
}

#[derive(Clone, Copy)]
struct TargetRect {
    bounds: Bounds<Pixels>,
    key: u64,
    cursor: CursorStyle,
}

#[derive(Default)]
struct Pointer {
    rects: Vec<TargetRect>,
    hovered: Option<u64>,
    pressed: Option<(u64, MouseButton)>,
    focused: Option<u64>,
    clicks: Vec<Click>,
    scroll: Point<Pixels>,
}

impl Pointer {
    fn pick(&self, position: Point<Pixels>) -> Option<TargetRect> {
        self.rects
            .iter()
            .rev()
            .find(|rect| rect.bounds.contains(&position))
            .copied()
    }

    fn step_focus(&mut self, forward: bool) {
        let count = self.rects.len();
        if count == 0 {
            self.focused = None;
            return;
        }
        let current = self
            .focused
            .and_then(|key| self.rects.iter().position(|rect| rect.key == key));
        let next = match (current, forward) {
            (Some(index), true) => (index + 1) % count,
            (Some(index), false) => (index + count - 1) % count,
            (None, true) => 0,
            (None, false) => count - 1,
        };
        self.focused = Some(self.rects[next].key);
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
    icons: Vec<Icon>,
    targets: Vec<Target>,
    labels: Vec<Label>,
    clips: Vec<(u32, ContentMask<Pixels>)>,
    paragraphs: Paragraphs,
    pointer: Rc<RefCell<Pointer>>,
    scrollable: bool,
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
            icons: Vec::new(),
            targets: Vec::new(),
            labels: Vec::new(),
            clips: Vec::new(),
            paragraphs: Paragraphs::default(),
            pointer: Rc::default(),
            scrollable: false,
            frame: 0,
        }
    }

    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
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
        self.icons.clear();
        self.targets.clear();
        self.labels.clear();
    }

    fn push(&mut self, kind: Kind, style: u16, payload: u32, shift: Pixels) -> u32 {
        let parent = self.open.last().copied().unwrap_or(NONE);
        if parent != NONE {
            self.nodes[parent as usize].children += 1;
        }
        let index = self.nodes.len() as u32;
        self.nodes.push(Node {
            parent,
            end: index + 1,
            children: 0,
            payload,
            target: NONE,
            style,
            kind,
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
        let index = self.push(Kind::Box, style.0, NONE, shift);
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
        self.spans(key, version, style, text, &[])
    }

    pub fn spans(
        &mut self,
        key: u64,
        version: u64,
        style: TextStyleId,
        text: &SharedString,
        spans: &[Span],
    ) -> NodeId {
        let (slot, new) = self.paragraphs.slot((key, style.0));
        let paragraph = &mut self.paragraphs.slots[slot as usize];
        if new || paragraph.version != version {
            paragraph.version = version;
            paragraph.shape(
                text,
                spans,
                &self.text_styles[style.0 as usize],
                &self.text_styles,
                &self.text_system,
            );
        }
        paragraph.used = self.frame;
        NodeId(self.push(Kind::Text, style.0, slot, Pixels::ZERO))
    }

    pub fn icon(&mut self, path: &SharedString, size: Pixels, color: Hsla) -> NodeId {
        self.icons.push(Icon {
            path: path.clone(),
            color,
            size,
        });
        NodeId(self.push(Kind::Icon, 0, self.icons.len() as u32 - 1, Pixels::ZERO))
    }

    pub fn target(&mut self, node: NodeId, key: u64, cursor: CursorStyle) {
        self.targets.push(Target { key, cursor });
        self.nodes[node.0 as usize].target = self.targets.len() as u32 - 1;
    }

    pub fn describe(&mut self, node: NodeId, key: u64, role: Role, text: &SharedString) {
        self.labels.push(Label {
            node: node.0,
            key,
            role,
            text: text.clone(),
        });
    }

    pub fn hovered(&self) -> Option<u64> {
        self.pointer.borrow().hovered
    }

    pub fn pressed(&self) -> Option<u64> {
        self.pointer.borrow().pressed.map(|(key, _)| key)
    }

    pub fn focused(&self) -> Option<u64> {
        self.pointer.borrow().focused
    }

    pub fn focus(&mut self, key: Option<u64>) {
        self.pointer.borrow_mut().focused = key;
    }

    pub fn focus_next(&mut self) {
        self.pointer.borrow_mut().step_focus(true);
    }

    pub fn focus_previous(&mut self) {
        self.pointer.borrow_mut().step_focus(false);
    }

    pub fn clicks(&mut self) -> Vec<Click> {
        mem::take(&mut self.pointer.borrow_mut().clicks)
    }

    pub fn set_scrollable(&mut self, scrollable: bool) {
        self.scrollable = scrollable;
    }

    pub fn scrolled(&mut self) -> Point<Pixels> {
        mem::take(&mut self.pointer.borrow_mut().scroll)
    }

    pub fn target_bounds(&self, key: u64) -> Option<Bounds<Pixels>> {
        let pointer = self.pointer.borrow();
        pointer
            .rects
            .iter()
            .find(|rect| rect.key == key)
            .map(|rect| rect.bounds)
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
            icons,
            scale_factor,
            ..
        } = self;
        if nodes.is_empty() {
            return;
        }

        for index in (0..nodes.len()).rev() {
            let node = nodes[index];
            let fit = match node.kind {
                Kind::Text => match text_styles[node.style as usize].wrap {
                    Wrap::Words => 0.,
                    Wrap::Never => paragraphs.slots[node.payload as usize].width.0,
                },
                Kind::Icon => icons[node.payload as usize].size.0,
                Kind::Box => {
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
            if node.kind != Kind::Box {
                continue;
            }
            let style = &box_styles[node.style as usize];
            let inner = (node.w - style.padding.left.0 - style.padding.right.0).max(0.);
            let extent = |child: &Node| match child.kind {
                Kind::Text => match text_styles[child.style as usize].wrap {
                    Wrap::Words => Extent::Grow,
                    Wrap::Never => Extent::Fit,
                },
                Kind::Icon => Extent::Fit,
                Kind::Box => box_styles[child.style as usize].width,
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
            let height = match node.kind {
                Kind::Text => {
                    let text = &text_styles[node.style as usize];
                    let paragraph = &mut paragraphs.slots[node.payload as usize];
                    paragraph.wrap(match text.wrap {
                        Wrap::Words => Some(px(node.w)),
                        Wrap::Never => None,
                    });
                    (text.line_height * paragraph.visual_line_count()).0
                }
                Kind::Icon => icons[node.payload as usize].size.0,
                Kind::Box => {
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

        let scale = *scale_factor;
        nodes[0].x = 0.;
        nodes[0].y = 0.;
        for index in 0..nodes.len() {
            let node = nodes[index];
            if node.kind != Kind::Box {
                continue;
            }
            let style = &box_styles[node.style as usize];
            let inner_width = (node.w - style.padding.left.0 - style.padding.right.0).max(0.);
            let inner_height = (node.h - style.padding.top.0 - style.padding.bottom.0).max(0.);
            let grows = |child: &Node| {
                child.kind == Kind::Box && box_styles[child.style as usize].height == Extent::Grow
            };
            let place = |align: Align, free: f32| match align {
                Align::Start => 0.,
                Align::Center => (free / 2. * scale).round() / scale,
                Align::End => free,
            };
            let mut used = style.gap.0 * node.children.saturating_sub(1) as f32;
            let mut growing = 0;
            let mut child = index + 1;
            while child < node.end as usize {
                match style.flow {
                    Flow::Column if grows(&nodes[child]) => growing += 1,
                    Flow::Column => used += nodes[child].h,
                    Flow::Row => used += nodes[child].w,
                }
                child = nodes[child].end as usize;
            }
            let free = match style.flow {
                Flow::Column => inner_height - used,
                Flow::Row => inner_width - used,
            }
            .max(0.);
            let grow_share = if growing > 0 {
                free / growing as f32
            } else {
                0.
            };
            let lead = if growing > 0 {
                0.
            } else {
                place(style.justify, free)
            };
            let mut x = node.x + style.padding.left.0;
            let mut y = node.y + style.padding.top.0 + node.shift;
            match style.flow {
                Flow::Column => y += lead,
                Flow::Row => x += lead,
            }
            let mut child = index + 1;
            while child < node.end as usize {
                if grows(&nodes[child]) {
                    nodes[child].h = match style.flow {
                        Flow::Row => inner_height,
                        Flow::Column => nodes[child].h.max(grow_share),
                    };
                }
                match style.flow {
                    Flow::Column => {
                        nodes[child].x = x + place(style.align, inner_width - nodes[child].w);
                        nodes[child].y = y;
                        y += nodes[child].h + style.gap.0;
                    }
                    Flow::Row => {
                        nodes[child].x = x;
                        nodes[child].y = y + place(style.align, inner_height - nodes[child].h);
                        x += nodes[child].w + style.gap.0;
                    }
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

    pub fn paint(&mut self, origin: Point<Pixels>, window: &mut Window, cx: &App) -> Result<()> {
        let bounds = Bounds::new(origin, self.size());
        let result = window.paint_layer(bounds, |window| {
            self.paint_fills(origin, window, cx)?;
            self.paint_text(origin, window)
        });
        self.frame += 1;
        if self.frame.is_multiple_of(SWEEP_EVERY) {
            self.paragraphs.sweep(self.frame);
        }
        result
    }

    fn paint_fills(&mut self, origin: Point<Pixels>, window: &mut Window, cx: &App) -> Result<()> {
        let mut clips = mem::take(&mut self.clips);
        clips.clear();
        let mut pointer = self.pointer.borrow_mut();
        pointer.rects.clear();
        let mut result = Ok(());
        for (index, node) in self.nodes.iter().enumerate() {
            while clips.last().is_some_and(|(end, _)| index as u32 >= *end) {
                clips.pop();
            }
            let bounds = Bounds::new(
                origin + point(px(node.x), px(node.y)),
                size(px(node.w), px(node.h)),
            );
            let mask = clips.last().map(|(_, mask)| *mask);
            if node.target != NONE {
                let target = self.targets[node.target as usize];
                let bounds = mask.map_or(bounds, |mask| bounds.intersect(&mask.bounds));
                if !bounds.is_empty() {
                    pointer.rects.push(TargetRect {
                        bounds,
                        key: target.key,
                        cursor: target.cursor,
                    });
                }
            }
            match node.kind {
                Kind::Text => {}
                Kind::Icon => {
                    let icon = &self.icons[node.payload as usize];
                    result = window.with_content_mask(mask, |window| {
                        window.paint_svg(
                            bounds,
                            icon.path.clone(),
                            None,
                            TransformationMatrix::unit(),
                            icon.color,
                            cx,
                        )
                    });
                    if result.is_err() {
                        break;
                    }
                }
                Kind::Box => {
                    let style = &self.box_styles[node.style as usize];
                    let bordered = style.border > Pixels::ZERO && style.border_color.a > 0.;
                    if style.fill.is_some() || bordered {
                        window.with_content_mask(mask, |window| {
                            let radii = Corners::all(style.radius);
                            let radii = match style.corner_share {
                                Some(share) => radii.share_of_quad_size(bounds.size, share),
                                None => window.resolve_corner_radii(radii, bounds.size),
                            };
                            let (radii, smoothing) =
                                window.drawn_corners(radii, bounds.size, style.corner_smoothing);
                            if let Some(fill) = style.fill.filter(|fill| fill.a > 0.) {
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
                            }
                            if bordered {
                                let mut border = quad(
                                    bounds,
                                    radii,
                                    style.border_color.opacity(0.),
                                    Edges::all(style.border),
                                    style.border_color,
                                    BorderStyle::default(),
                                );
                                border.corner_smoothing = smoothing;
                                window.paint_quad(border);
                            }
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
            }
        }
        drop(pointer);
        self.clips = clips;
        result
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
            match node.kind {
                Kind::Icon => {}
                Kind::Text => {
                    let style = &self.text_styles[node.style as usize];
                    let paragraph = &mut self.paragraphs.slots[node.payload as usize];
                    let mask = clips.last().map(|(_, mask)| *mask);
                    result =
                        window.with_content_mask(mask, |window| paragraph.paint(style, at, window));
                    if result.is_err() {
                        break;
                    }
                }
                Kind::Box => {
                    if self.box_styles[node.style as usize].clip == Clip::Bounds {
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
            }
        }
        self.clips = clips;
        result
    }

    pub fn listen(&mut self, hitbox: &Hitbox, view: EntityId, window: &mut Window) {
        let inside = hitbox.is_hovered(window);
        let under = inside
            .then(|| self.pointer.borrow().pick(window.mouse_position()))
            .flatten();
        let stale = {
            let mut pointer = self.pointer.borrow_mut();
            let key = under.map(|rect| rect.key);
            let stale = pointer.hovered != key;
            pointer.hovered = key;
            stale
        };
        if stale {
            window.on_next_frame(move |_, cx| cx.notify(view));
        }
        if let Some(rect) = under
            && rect.cursor != CursorStyle::Arrow
        {
            window.set_cursor_style(rect.cursor, hitbox);
        }

        let pointer = self.pointer.clone();
        let area = hitbox.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            let mut pointer = pointer.borrow_mut();
            let key = area
                .is_hovered(window)
                .then(|| pointer.pick(event.position))
                .flatten()
                .map(|rect| rect.key);
            if pointer.hovered != key {
                pointer.hovered = key;
                cx.notify(view);
            }
        });

        let pointer = self.pointer.clone();
        let area = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !area.is_hovered(window) {
                return;
            }
            let mut pointer = pointer.borrow_mut();
            if let Some(rect) = pointer.pick(event.position) {
                pointer.pressed = Some((rect.key, event.button));
                pointer.focused = Some(rect.key);
                cx.notify(view);
                window.prevent_default();
                cx.stop_propagation();
            }
        });

        let pointer = self.pointer.clone();
        let area = hitbox.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let mut pointer = pointer.borrow_mut();
            let Some((key, button)) = pointer.pressed.take() else {
                return;
            };
            let released = area
                .is_hovered(window)
                .then(|| pointer.pick(event.position))
                .flatten()
                .map(|rect| rect.key);
            if button == event.button && released == Some(key) {
                pointer.clicks.push(Click {
                    key,
                    button,
                    count: event.click_count,
                    modifiers: event.modifiers,
                    position: event.position,
                });
            }
            cx.notify(view);
        });

        if self.scrollable {
            let pointer = self.pointer.clone();
            let area = hitbox.clone();
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !area.should_handle_scroll(window) {
                    return;
                }
                let delta = event.delta.pixel_delta(window.line_height());
                let mut pointer = pointer.borrow_mut();
                pointer.scroll = pointer.scroll + delta;
                cx.notify(view);
                cx.stop_propagation();
            });
        }
    }

    fn described(&self, origin: Point<Pixels>) -> Vec<Described> {
        self.labels
            .iter()
            .map(|label| {
                let node = &self.nodes[label.node as usize];
                Described {
                    key: label.key,
                    role: label.role,
                    text: label.text.clone(),
                    bounds: Bounds::new(
                        origin + point(px(node.x), px(node.y)),
                        size(px(node.w), px(node.h)),
                    ),
                }
            })
            .collect()
    }
}

struct Described {
    key: u64,
    role: Role,
    text: SharedString,
    bounds: Bounds<Pixels>,
}

pub trait Flat: 'static + Sized {
    fn ui(&mut self) -> &mut Ui;

    fn build(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>);

    fn clicked(&mut self, _click: Click, _window: &mut Window, _cx: &mut Context<Self>) {}
}

pub struct Surface<V> {
    id: ElementId,
    view: Entity<V>,
    style: StyleRefinement,
    fit: bool,
    behavior: HitboxBehavior,
}

pub fn surface<V: Flat>(id: impl Into<ElementId>, view: &Entity<V>) -> Surface<V> {
    Surface {
        id: id.into(),
        view: view.clone(),
        style: StyleRefinement::default(),
        fit: false,
        behavior: HitboxBehavior::Normal,
    }
}

impl<V: Flat> Surface<V> {
    pub fn fit_content(mut self) -> Self {
        self.fit = true;
        self
    }

    pub fn occlude(mut self) -> Self {
        self.behavior = HitboxBehavior::BlockMouse;
        self
    }
}

pub struct SurfaceState {
    hitbox: Hitbox,
    described: Vec<Described>,
    scale_factor: f32,
}

impl<V: Flat> IntoElement for Surface<V> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<V: Flat> Styled for Surface<V> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<V: Flat> Element for Surface<V> {
    type RequestLayoutState = ();
    type PrepaintState = SurfaceState;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        if !self.fit {
            return (window.request_layout(style, [], cx), ());
        }
        let view = self.view.clone();
        let layout = window.request_measured_layout(style, move |known, available, window, cx| {
            let width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(width) => width,
                AvailableSpace::MinContent => Pixels::ZERO,
                AvailableSpace::MaxContent => Pixels::MAX,
            });
            view.update(cx, |view, cx| {
                view.build(
                    Bounds::new(Point::default(), size(width, Pixels::ZERO)),
                    window,
                    cx,
                );
                view.ui().size()
            })
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, self.behavior);
        let described = self.view.update(cx, |view, cx| {
            view.build(bounds, window, cx);
            if window.a11y.is_active() {
                view.ui().described(bounds.origin)
            } else {
                Vec::new()
            }
        });
        SurfaceState {
            hitbox,
            described,
            scale_factor: window.scale_factor(),
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let view = self.view.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            view.update(cx, |view, cx| {
                for click in view.ui().clicks() {
                    view.clicked(click, window, cx);
                }
            });
        });
        self.view.update(cx, |view, cx| {
            let ui = view.ui();
            if let Err(error) = ui.paint(bounds.origin, window, cx) {
                log::error!("flat surface paint failed: {error:#}");
            }
            ui.listen(&prepaint.hitbox, cx.entity_id(), window);
        });
    }

    fn a11y_role(&self) -> Option<Role> {
        Some(Role::Group)
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut A11ySubtreeBuilder,
    ) {
        let scale = f64::from(prepaint.scale_factor);
        for item in &prepaint.described {
            let mut node = accesskit::Node::new(item.role);
            node.set_label(item.text.to_string());
            node.set_bounds(accesskit::Rect {
                x0: f64::from(item.bounds.left().0) * scale,
                y0: f64::from(item.bounds.top().0) * scale,
                x1: f64::from(item.bounds.right().0) * scale,
                y1: f64::from(item.bounds.bottom().0) * scale,
            });
            builder.push_child(builder.synthetic_node_id(item.key), node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_painted_target_under_the_pointer_wins() {
        let rect = |x: f32, key| TargetRect {
            bounds: Bounds::new(point(px(x), px(0.)), size(px(20.), px(10.))),
            key,
            cursor: CursorStyle::Arrow,
        };
        let mut pointer = Pointer {
            rects: vec![rect(0., 1), rect(10., 2)],
            ..Default::default()
        };
        let pick =
            |pointer: &Pointer, x: f32| pointer.pick(point(px(x), px(5.))).map(|rect| rect.key);
        assert_eq!(pick(&pointer, 15.), Some(2));
        assert_eq!(pick(&pointer, 5.), Some(1));
        assert_eq!(pick(&pointer, 35.), None);
        pointer.step_focus(false);
        assert_eq!(pointer.focused, Some(2));
        pointer.step_focus(true);
        assert_eq!(pointer.focused, Some(1));
    }
}
