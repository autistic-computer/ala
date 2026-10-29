use crate::scheduler::Instant;
use std::{cell::RefCell, rc::Rc, time::Duration};

use crate::{
    AbsoluteLength, AnyElement, App, Bounds, BoxShadow, CornerRadiusMode, Corners, DispatchPhase,
    Div, Element, ElementId, GlassMaterial, GlassShape, GlobalElementId, Hitbox, HitboxBehavior,
    InspectorElementId, InteractiveElement, Interactivity, Interpolate, IntoElement, LayoutId,
    LiquidRect, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Point, SpringConfig, SpringState, Stateful, StatefulInteractiveElement, StyleRefinement,
    Styled, Window, div, point, size,
};

pub fn liquid_glass(id: impl Into<ElementId>, material: GlassMaterial) -> LiquidGlass {
    let id = id.into();
    LiquidGlass {
        id: id.clone(),
        content: Some(div().id(id)),
        material,
        pressed: None,
        hovered: None,
        press_scale: 1.12,
        hover_scale: 1.0,
        spring: SpringConfig::new(320., 20., 1.),
        appear: true,
        shown: true,
        morph: None,
        lift: None,
        lift_scale: 1.0,
        drag_flex: Pixels(0.),
        light_follows_pointer: false,
        shadows: Vec::new(),
        corner_radii: Corners::default(),
        corner_radius_mode: CornerRadiusMode::default(),
        corner_smoothing: None,
    }
}

pub struct LiquidGlass {
    id: ElementId,
    content: Option<Stateful<Div>>,
    material: GlassMaterial,
    pressed: Option<GlassMaterial>,
    hovered: Option<GlassMaterial>,
    press_scale: f32,
    hover_scale: f32,
    spring: SpringConfig,
    appear: bool,
    shown: bool,
    morph: Option<SpringConfig>,
    lift: Option<GlassMaterial>,
    lift_scale: f32,
    drag_flex: Pixels,
    light_follows_pointer: bool,
    shadows: Vec<BoxShadow>,
    corner_radii: Corners<AbsoluteLength>,
    corner_radius_mode: CornerRadiusMode,
    corner_smoothing: Option<f32>,
}

impl LiquidGlass {
    pub fn pressed_material(mut self, material: GlassMaterial) -> Self {
        self.pressed = Some(material);
        self
    }

    pub fn hovered_material(mut self, material: GlassMaterial) -> Self {
        self.hovered = Some(material);
        self
    }

    pub fn press_scale(mut self, scale: f32) -> Self {
        self.press_scale = scale;
        self
    }

    pub fn hover_scale(mut self, scale: f32) -> Self {
        self.hover_scale = scale;
        self
    }

    pub fn spring(mut self, spring: SpringConfig) -> Self {
        self.spring = spring;
        self
    }

    pub fn appear(mut self, appear: bool) -> Self {
        self.appear = appear;
        self
    }

    pub fn shown(mut self, shown: bool) -> Self {
        self.shown = shown;
        self
    }

    pub fn morph(mut self, spring: SpringConfig) -> Self {
        self.morph = Some(spring);
        self
    }

    pub fn lift_material(mut self, material: GlassMaterial) -> Self {
        self.lift = Some(material);
        self
    }

    pub fn lift_scale(mut self, scale: f32) -> Self {
        self.lift_scale = scale;
        self
    }

    pub fn drag_flex(mut self, flex: impl Into<Pixels>) -> Self {
        self.drag_flex = flex.into();
        self
    }

    pub fn light_follows_pointer(mut self, follows: bool) -> Self {
        self.light_follows_pointer = follows;
        self
    }

    pub fn glass_shadow(mut self, shadows: Vec<BoxShadow>) -> Self {
        self.shadows = shadows;
        self
    }

    fn content(&mut self) -> &mut Stateful<Div> {
        self.content
            .as_mut()
            .expect("liquid glass content is taken only when laid out")
    }
}

impl Styled for LiquidGlass {
    fn style(&mut self) -> &mut StyleRefinement {
        self.content().style()
    }
}

impl ParentElement for LiquidGlass {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.content().extend(elements);
    }
}

impl InteractiveElement for LiquidGlass {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.content().interactivity()
    }
}

impl StatefulInteractiveElement for LiquidGlass {}

impl IntoElement for LiquidGlass {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[derive(Clone, Copy, Default)]
struct Toggle {
    spring: SpringState,
    on: bool,
}

impl Toggle {
    fn step(&mut self, config: SpringConfig, delta: f32, snap: bool) -> bool {
        let target = if self.on { 1. } else { 0. };
        if !snap {
            self.spring = config.step(self.spring, target, delta);
        }
        if snap || config.is_settled(self.spring, target, 0.001) {
            self.spring = SpringState {
                position: target,
                velocity: 0.,
            };
            return false;
        }
        true
    }

    fn phase(&self) -> f32 {
        self.spring.position
    }
}

struct LiquidGlassState {
    press: Toggle,
    hover: Toggle,
    presence: Toggle,
    touch: Point<f32>,
    body: Option<LiquidRect>,
    lift: Toggle,
    press_origin: Point<Pixels>,
    pointer: Point<Pixels>,
    drag: [SpringState; 2],
    updated_at: Instant,
}

pub struct LiquidGlassFrame {
    state: Rc<RefCell<LiquidGlassState>>,
    hitbox: Hitbox,
    shape: GlassShape,
    material: GlassMaterial,
    shadows: Vec<BoxShadow>,
    presence: f32,
    grouped: bool,
}

impl Element for LiquidGlass {
    type RequestLayoutState = AnyElement;
    type PrepaintState = LiquidGlassFrame;

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
        let mut content = self.content.take().expect("liquid glass is laid out once");
        let style = content.style();
        let radii = &style.corner_radii;
        self.corner_radii = Corners {
            top_left: radii.top_left.unwrap_or_default(),
            top_right: radii.top_right.unwrap_or_default(),
            bottom_right: radii.bottom_right.unwrap_or_default(),
            bottom_left: radii.bottom_left.unwrap_or_default(),
        };
        self.corner_radius_mode = style.corner_radius_mode.unwrap_or_default();
        self.corner_smoothing = style.corner_smoothing;
        let mut content = content.into_any_element();
        (content.request_layout(window, cx), content)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        content: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        window.with_opacity_set_at_paint(|window| content.prepaint(window, cx));
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        let state = window.with_element_state(
            id.expect("liquid glass has an id"),
            |state: Option<Rc<RefCell<LiquidGlassState>>>, _| {
                let state = state.unwrap_or_else(|| {
                    Rc::new(RefCell::new(LiquidGlassState {
                        press: Toggle::default(),
                        hover: Toggle::default(),
                        presence: Toggle {
                            spring: SpringState {
                                position: if self.appear || !self.shown { 0. } else { 1. },
                                velocity: 0.,
                            },
                            on: true,
                        },
                        touch: point(0.5, 0.5),
                        body: None,
                        lift: Toggle::default(),
                        press_origin: Point::default(),
                        pointer: Point::default(),
                        drag: Default::default(),
                        updated_at: cx.background_executor().now(),
                    }))
                });
                (state.clone(), state)
            },
        );

        let (press, hover, presence, touch, lift, body, drag) = {
            let mut state = state.borrow_mut();
            let now = cx.background_executor().now();
            let delta = now.duration_since(state.updated_at).as_secs_f32().min(0.1);
            state.updated_at = now;
            state.hover.on = hitbox.is_hovered(window);
            state.presence.on = self.shown;
            let snap = cx.reduce_motion();
            let config = self.spring;
            let mut moving = state.press.step(config, delta, snap);
            moving |= state.hover.step(config, delta, snap);
            moving |= state.presence.step(config, delta, snap);
            let body = match self.morph {
                Some(spring) => {
                    let body = state
                        .body
                        .get_or_insert_with(|| LiquidRect::new(bounds, spring));
                    body.config = spring;
                    body.set_target(bounds);
                    if snap {
                        body.snap(bounds);
                    } else {
                        moving |= body.step(Duration::from_secs_f32(delta));
                    }
                    let velocity = body.velocity();
                    let speed = velocity.x.as_f32().hypot(velocity.y.as_f32());
                    let stretched = body.stretched(0.12, 0.35);
                    state.lift.on = speed > 120.;
                    moving |= state.lift.step(config, delta, snap);
                    stretched
                }
                None => {
                    state.body = None;
                    bounds
                }
            };
            let flex = self.drag_flex.as_f32();
            let pull = if flex > 0. && state.press.on {
                let reach = state.pointer - state.press_origin;
                let length = reach.x.as_f32().hypot(reach.y.as_f32());
                if length > 0. {
                    let banded = flex * (length / (flex * 3.)).tanh();
                    [
                        reach.x.as_f32() / length * banded,
                        reach.y.as_f32() / length * banded,
                    ]
                } else {
                    [0., 0.]
                }
            } else {
                [0., 0.]
            };
            for (axis, target) in state.drag.iter_mut().zip(pull) {
                if snap {
                    *axis = SpringState {
                        position: target,
                        velocity: 0.,
                    };
                    continue;
                }
                *axis = config.step(*axis, target, delta);
                if config.is_settled(*axis, target, 0.01) {
                    *axis = SpringState {
                        position: target,
                        velocity: 0.,
                    };
                } else {
                    moving = true;
                }
            }
            if moving {
                window.request_animation_frame();
            }
            (
                state.press.phase(),
                state.hover.phase(),
                state.presence.phase(),
                state.touch,
                state.lift.phase(),
                body,
                point(state.drag[0].position, state.drag[1].position),
            )
        };

        let rest = Interpolate::interpolate(self.material.vanished(), self.material, presence);
        let hovered = self.hovered.unwrap_or_else(|| {
            rest.specular(rest.specular * 1.4)
                .brightness(rest.brightness + 0.03)
        });
        let longest = bounds.size.width.max(bounds.size.height);
        let pressed = self.pressed.unwrap_or_else(|| {
            rest.glow(0.22)
                .glow_radius(longest * 0.9)
                .refraction(rest.refraction * 1.2)
        });
        let pressed = pressed.glow_center(touch);
        let mut material = Interpolate::interpolate(
            Interpolate::interpolate(rest, hovered, hover),
            pressed,
            press,
        );
        if let Some(lifted) = self.lift {
            material = Interpolate::interpolate(material, lifted, lift);
        }
        let attention = hover.max(press).clamp(0., 1.);
        if self.light_follows_pointer && attention > 0.001 {
            let toward = window.mouse_position() - body.center();
            if toward.x != Pixels(0.) || toward.y != Pixels(0.) {
                let angle = toward.y.as_f32().atan2(toward.x.as_f32());
                material.light_angle = turn_toward(material.light_angle, angle, attention);
            }
        }

        let scale = (1. + (self.hover_scale - 1.) * hover + (self.press_scale - 1.) * press)
            * (1. + (self.lift_scale - 1.) * lift)
            * (0.92 + 0.08 * presence);
        let (pull_x, pull_y) = (drag.x.abs(), drag.y.abs());
        let grown = size(
            body.size.width * scale + Pixels(1.2 * pull_x - 0.6 * pull_y),
            body.size.height * scale + Pixels(1.2 * pull_y - 0.6 * pull_x),
        );
        let center = body.center() + point(Pixels(drag.x), Pixels(drag.y));
        let requested = self.corner_radii.to_pixels(window.rem_size());
        let radii = match self.corner_radius_mode {
            CornerRadiusMode::Inherit => window.resolve_corner_radii(requested, bounds.size),
            CornerRadiusMode::Fixed => requested.clamp_radii_for_quad_size(bounds.size),
        };
        let shape = GlassShape {
            bounds: Bounds::new(center - point(grown.width / 2., grown.height / 2.), grown),
            corner_radii: Corners {
                top_left: radii.top_left * scale,
                top_right: radii.top_right * scale,
                bottom_right: radii.bottom_right * scale,
                bottom_left: radii.bottom_left * scale,
            },
        };

        let shadows: Vec<BoxShadow> = self
            .shadows
            .iter()
            .map(|shadow| BoxShadow {
                color: shadow.color.opacity(presence.clamp(0., 1.)),
                ..shadow.clone()
            })
            .collect();
        let grouped = presence > 0.001
            && window.glass_groups.last().is_some_and(|group| {
                group.borrow_mut().push(GroupedGlass {
                    shape,
                    press,
                    touch,
                    shadows: shadows.clone(),
                    corner_smoothing: self.corner_smoothing,
                });
                true
            });

        LiquidGlassFrame {
            state,
            hitbox,
            shape,
            material,
            shadows,
            presence,
            grouped,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        content: &mut Self::RequestLayoutState,
        frame: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let presence = frame.presence;
        if presence > 0.001 && !frame.grouped {
            window.paint_glass_with_shadows(
                frame.shape,
                &frame.material,
                self.corner_smoothing,
                &frame.shadows,
            );
        }
        if self.shown || presence > 0.001 {
            window.with_element_opacity(Some(presence.clamp(0., 1.)), |window| {
                content.paint(window, cx)
            });
        }

        let view = window.current_view();
        let down_state = frame.state.clone();
        let down_hitbox = frame.hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && down_hitbox.is_hovered(window)
                && down_state.borrow().presence.on
            {
                let mut state = down_state.borrow_mut();
                state.press.on = true;
                state.press_origin = event.position;
                state.pointer = event.position;
                state.touch = point(
                    ((event.position.x - bounds.origin.x) / bounds.size.width).clamp(0., 1.),
                    ((event.position.y - bounds.origin.y) / bounds.size.height).clamp(0., 1.),
                );
                cx.notify(view);
            }
        });
        let up_state = frame.state.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                let mut state = up_state.borrow_mut();
                if state.press.on {
                    state.press.on = false;
                    cx.notify(view);
                }
            }
        });
        let move_state = frame.state.clone();
        let move_hitbox = frame.hitbox.clone();
        let tracks_pointer = self.drag_flex > Pixels(0.) || self.light_follows_pointer;
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let hovered = move_hitbox.is_hovered(window);
            let mut state = move_state.borrow_mut();
            if state.press.on {
                state.pointer = event.position;
            }
            if hovered != state.hover.on || (tracks_pointer && (hovered || state.press.on)) {
                cx.notify(view);
            }
        });
    }
}

pub type GlassGroupShapes = Rc<RefCell<Vec<GroupedGlass>>>;

#[derive(Clone, Debug)]
pub struct GroupedGlass {
    shape: GlassShape,
    press: f32,
    touch: Point<f32>,
    shadows: Vec<BoxShadow>,
    corner_smoothing: Option<f32>,
}

fn turn_toward(from: f32, to: f32, phase: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let delta = (to - from + PI).rem_euclid(TAU) - PI;
    from + delta * phase
}

pub fn glass_group(id: impl Into<ElementId>, material: GlassMaterial) -> GlassGroup {
    let id = id.into();
    GlassGroup {
        id: id.clone(),
        content: Some(div().id(id)),
        material,
    }
}

pub struct GlassGroup {
    id: ElementId,
    content: Option<Stateful<Div>>,
    material: GlassMaterial,
}

impl GlassGroup {
    fn content(&mut self) -> &mut Stateful<Div> {
        self.content
            .as_mut()
            .expect("glass group content is taken only when laid out")
    }
}

impl Styled for GlassGroup {
    fn style(&mut self) -> &mut StyleRefinement {
        self.content().style()
    }
}

impl ParentElement for GlassGroup {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.content().extend(elements);
    }
}

impl InteractiveElement for GlassGroup {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.content().interactivity()
    }
}

impl StatefulInteractiveElement for GlassGroup {}

impl IntoElement for GlassGroup {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GlassGroup {
    type RequestLayoutState = AnyElement;
    type PrepaintState = Vec<GroupedGlass>;

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
        let mut content = self
            .content
            .take()
            .expect("glass group is laid out once")
            .into_any_element();
        (content.request_layout(window, cx), content)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        content: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        window.glass_groups.push(GlassGroupShapes::default());
        content.prepaint(window, cx);
        let shapes = window
            .glass_groups
            .pop()
            .expect("the group pushed its own shapes");
        shapes.take()
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        content: &mut Self::RequestLayoutState,
        grouped: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let supports_glass = window.supports_backdrop_sampling();
        if !supports_glass {
            for glass in grouped.iter() {
                window.paint_drop_shadows_with_smoothing(
                    glass.shape.bounds,
                    glass.shape.corner_radii,
                    &glass.shadows,
                    glass.corner_smoothing,
                    false,
                );
            }
        }
        for body in grouped.chunks(crate::GLASS_MAX_SHAPES) {
            let shapes: Vec<GlassShape> = body.iter().map(|glass| glass.shape).collect();
            let mut material = self.material;
            if let Some(pressed) = body
                .iter()
                .max_by(|a, b| a.press.total_cmp(&b.press))
                .filter(|glass| glass.press > 0.001)
            {
                let union = shapes
                    .iter()
                    .map(|shape| shape.bounds)
                    .reduce(|a, b| a.union(&b))
                    .unwrap_or_default();
                let bounds = pressed.shape.bounds;
                let at = bounds.origin
                    + point(
                        bounds.size.width * pressed.touch.x,
                        bounds.size.height * pressed.touch.y,
                    );
                let longest = bounds.size.width.max(bounds.size.height);
                material = Interpolate::interpolate(
                    material,
                    material
                        .glow(0.22)
                        .glow_radius(longest * 0.9)
                        .glow_center(point(
                            (at.x - union.origin.x) / union.size.width.max(Pixels(1.)),
                            (at.y - union.origin.y) / union.size.height.max(Pixels(1.)),
                        )),
                    pressed.press,
                );
            }
            let corner_smoothing = body.first().and_then(|glass| glass.corner_smoothing);
            window.paint_glass_shapes_with_smoothing(&shapes, &material, corner_smoothing);
            if supports_glass && body.iter().any(|glass| !glass.shadows.is_empty()) {
                let glass = window.glass_primitive(&shapes, &material, corner_smoothing);
                for child in body {
                    window.paint_glass_shadows(
                        &glass,
                        child.shape,
                        &child.shadows,
                        child.corner_smoothing,
                    );
                }
            }
        }
        content.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{Context, Modifiers, Render, TestAppContext, VisualTestContext, px};

    struct Button;

    impl Render for Button {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                liquid_glass("button", GlassMaterial::regular())
                    .appear(false)
                    .absolute()
                    .left(px(20.))
                    .top(px(20.))
                    .size(px(40.))
                    .rounded_full(),
            )
        }
    }

    fn glass_width(cx: &mut VisualTestContext) -> f32 {
        cx.update(|window, _| {
            window
                .painted_glasses()
                .iter()
                .map(|glass| glass.shape_bounds().size.width.0)
                .fold(0., f32::max)
        })
    }

    fn next_frame(cx: &mut VisualTestContext, after: Duration) {
        cx.executor().advance_clock(after);
        cx.update(|window, cx| window.simulate_next_frame(cx));
        cx.run_until_parked();
    }

    #[crate::test]
    fn pressing_swells_the_glass_and_releasing_settles_it(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| Button);
        cx.run_until_parked();
        let rest = glass_width(cx);
        let scale = cx.update(|window, _| window.scale_factor());
        assert_eq!(rest, 40. * scale);

        let center = point(px(40.), px(40.));
        cx.simulate_mouse_move(center, None, Modifiers::none());
        cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::none());
        for _ in 0..30 {
            next_frame(cx, Duration::from_millis(16));
        }
        let pressed = glass_width(cx);
        assert!(pressed > rest * 1.08, "pressed glass is {pressed} wide");

        cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::none());
        for _ in 0..120 {
            next_frame(cx, Duration::from_millis(16));
        }
        assert_eq!(glass_width(cx), rest);
    }

    struct Gel;

    impl Render for Gel {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                liquid_glass("gel", GlassMaterial::regular())
                    .appear(false)
                    .press_scale(1.)
                    .drag_flex(px(10.))
                    .absolute()
                    .left(px(20.))
                    .top(px(20.))
                    .size(px(40.))
                    .rounded_full(),
            )
        }
    }

    fn glass_center_x(cx: &mut VisualTestContext) -> f32 {
        cx.update(|window, _| {
            let glass = window.painted_glasses()[0].shape_bounds();
            (glass.origin.x.0 + glass.size.width.0 / 2.) / window.scale_factor()
        })
    }

    #[crate::test]
    fn held_presses_pull_the_glass_toward_the_pointer_like_gel(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| Gel);
        cx.run_until_parked();
        assert_eq!(glass_center_x(cx), 40.);

        let start = point(px(40.), px(40.));
        let pulled = point(px(240.), px(40.));
        cx.simulate_mouse_move(start, None, Modifiers::none());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(pulled, MouseButton::Left, Modifiers::none());
        for _ in 0..60 {
            next_frame(cx, Duration::from_millis(16));
        }
        let center = glass_center_x(cx);
        assert!(
            center > 45. && center < 50.,
            "pulled glass sits at {center}"
        );

        cx.simulate_mouse_up(pulled, MouseButton::Left, Modifiers::none());
        for _ in 0..120 {
            next_frame(cx, Duration::from_millis(16));
        }
        assert_eq!(glass_center_x(cx), 40.);
    }

    struct Hidden;

    impl Render for Hidden {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                liquid_glass("hidden", GlassMaterial::regular())
                    .appear(false)
                    .shown(false)
                    .size(px(40.)),
            )
        }
    }

    #[crate::test]
    fn glass_that_starts_hidden_paints_nothing(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| Hidden);
        cx.run_until_parked();
        assert!(cx.update(|window, _| window.painted_glasses().is_empty()));
    }

    #[test]
    fn materials_ride_springs() {
        use crate::{AnimationExt as _, SpringAnimation};
        div().id("glass").with_spring(
            "frost",
            SpringAnimation::new(SpringConfig::new(300., 30., 1.)).to(true),
            |element, phase| {
                element.glass(phase.interpolate(GlassMaterial::clear(), GlassMaterial::frosted()))
            },
        );
    }

    #[test]
    fn angles_turn_the_short_way_round() {
        use std::f32::consts::PI;
        let turned = turn_toward(0.9 * PI, -0.9 * PI, 0.5);
        assert!((turned.abs() - PI).abs() < 1e-4, "{turned}");
        assert!((turn_toward(0., 1., 0.25) - 0.25).abs() < 1e-6);
    }

    struct Row {
        grouped: bool,
    }

    impl Render for Row {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let buttons = (0..3usize).map(|index| {
                liquid_glass(("button", index), GlassMaterial::regular())
                    .appear(false)
                    .size(px(40.))
                    .rounded_full()
            });
            let row = div().flex().gap(px(4.));
            if self.grouped {
                glass_group("row", GlassMaterial::regular())
                    .child(row.children(buttons))
                    .into_any_element()
            } else {
                row.children(buttons).into_any_element()
            }
        }
    }

    #[crate::test]
    fn groups_paint_their_glass_children_as_one_body(cx: &mut TestAppContext) {
        let bodies = |grouped: bool, cx: &mut TestAppContext| {
            let (_, cx) = cx.add_window_view(move |_, _| Row { grouped });
            cx.run_until_parked();
            cx.update(|window, _| {
                window
                    .painted_glasses()
                    .iter()
                    .map(|glass| glass.shape_count)
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(bodies(false, cx), vec![1, 1, 1]);
        assert_eq!(bodies(true, cx), vec![3]);
    }

    struct Shadowed {
        kind: u8,
    }

    impl Render for Shadowed {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let shadow = BoxShadow {
                color: crate::black(),
                offset: point(px(2.), px(3.)),
                blur_radius: px(4.),
                spread_radius: px(1.),
                inset: false,
            };
            let material = GlassMaterial::regular().merge(px(12.));
            if self.kind == 0 {
                return div()
                    .size(px(40.))
                    .rounded_full()
                    .glass(material)
                    .shadow(vec![shadow])
                    .into_any_element();
            }
            let buttons = (0usize..if self.kind == 1 { 1 } else { 3 }).map(|index| {
                liquid_glass(("shadowed", index), material)
                    .appear(false)
                    .size(px(40.))
                    .rounded_full()
                    .glass_shadow(vec![shadow.clone()])
            });
            let row = div().flex().gap(px(4.)).children(buttons);
            if self.kind == 2 {
                glass_group("shadowed-group", material)
                    .child(row)
                    .into_any_element()
            } else {
                row.into_any_element()
            }
        }
    }

    #[crate::test]
    fn glass_surfaces_paint_their_own_shadows_after_the_clean_body(cx: &mut TestAppContext) {
        for kind in 0..3 {
            let (_, cx) = cx.add_window_view(move |_, _| Shadowed { kind });
            cx.run_until_parked();
            let glasses = cx.update(|window, _| window.painted_glasses());
            let count = if kind == 2 { 3 } else { 1 };
            assert_eq!(glasses.len(), count + 1);
            let body = &glasses[0];
            assert!(body.shadow.is_none());
            assert_eq!(body.shape_count as usize, count);
            for shadow in &glasses[1..] {
                assert!(shadow.shadow.is_some());
                assert!(!shadow.reads_backdrop());
                assert_eq!(shadow.shape_count, body.shape_count);
                assert_eq!(shadow.shapes, body.shapes);
                assert!(shadow.order >= body.order);
            }
        }
    }
}
