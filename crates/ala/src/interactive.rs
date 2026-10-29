use crate::{
    Bounds, Capslock, Context, Empty, IntoElement, Keystroke, LongPressEvent, Modifiers, Pixels,
    Point, Render, TouchDragEvent, Window, point, seal::Sealed,
};
use smallvec::SmallVec;
use std::{any::Any, fmt::Debug, ops::Deref, path::PathBuf};

pub trait InputEvent: Sealed + 'static {
    fn to_platform_input(self) -> PlatformInput;
}

pub trait KeyEvent: InputEvent {}

pub trait MouseEvent: InputEvent {}

pub trait GestureEvent: InputEvent {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyDownEvent {
    pub keystroke: Keystroke,

    pub is_held: bool,

    pub prefer_character_input: bool,
}

impl Sealed for KeyDownEvent {}
impl InputEvent for KeyDownEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::KeyDown(self)
    }
}
impl KeyEvent for KeyDownEvent {}

#[derive(Clone, Debug)]
pub struct KeyUpEvent {
    pub keystroke: Keystroke,
}

impl Sealed for KeyUpEvent {}
impl InputEvent for KeyUpEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::KeyUp(self)
    }
}
impl KeyEvent for KeyUpEvent {}

#[derive(Clone, Debug, Default)]
pub struct ModifiersChangedEvent {
    pub modifiers: Modifiers,
    pub capslock: Capslock,
}

impl Sealed for ModifiersChangedEvent {}
impl InputEvent for ModifiersChangedEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::ModifiersChanged(self)
    }
}
impl KeyEvent for ModifiersChangedEvent {}

impl Deref for ModifiersChangedEvent {
    type Target = Modifiers;

    fn deref(&self) -> &Self::Target {
        &self.modifiers
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TouchPhase {
    Started,
    #[default]
    Moved,
    Ended,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TouchId(pub u64);

#[derive(Clone, Debug, Default)]
pub struct TouchEvent {
    pub id: TouchId,
    pub phase: TouchPhase,
    pub position: Point<Pixels>,
    pub predicted_position: Option<Point<Pixels>>,
    pub force: Option<f32>,
}

impl Sealed for TouchEvent {}
impl InputEvent for TouchEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::Touch(self)
    }
}

#[derive(Clone, Debug, Default)]
pub struct MouseDownEvent {
    pub button: MouseButton,

    pub position: Point<Pixels>,

    pub modifiers: Modifiers,

    pub click_count: usize,

    pub first_mouse: bool,
}

impl Sealed for MouseDownEvent {}
impl InputEvent for MouseDownEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::MouseDown(self)
    }
}
impl MouseEvent for MouseDownEvent {}

impl MouseDownEvent {
    pub fn is_focusing(&self) -> bool {
        match self.button {
            MouseButton::Left => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MouseUpEvent {
    pub button: MouseButton,

    pub position: Point<Pixels>,

    pub modifiers: Modifiers,

    pub click_count: usize,
}

impl Sealed for MouseUpEvent {}
impl InputEvent for MouseUpEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::MouseUp(self)
    }
}

impl MouseEvent for MouseUpEvent {}

impl MouseUpEvent {
    pub fn is_focusing(&self) -> bool {
        match self.button {
            MouseButton::Left => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MouseClickEvent {
    pub down: MouseDownEvent,

    pub up: MouseUpEvent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PressureStage {
    #[default]
    Zero,
    Normal,
    Force,
}

#[derive(Debug, Clone, Default)]
pub struct MousePressureEvent {
    pub pressure: f32,
    pub stage: PressureStage,
    pub position: Point<Pixels>,
    pub modifiers: Modifiers,
}

impl Sealed for MousePressureEvent {}
impl InputEvent for MousePressureEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::MousePressure(self)
    }
}
impl MouseEvent for MousePressureEvent {}

#[derive(Clone, Debug, Default)]
pub struct KeyboardClickEvent {
    pub button: KeyboardButton,

    pub bounds: Bounds<Pixels>,
}

#[derive(Clone, Debug, Default)]
pub struct TouchClickEvent {
    pub position: Point<Pixels>,
    pub tap_count: usize,
    pub long_press: bool,
}

#[derive(Clone, Debug)]
pub enum ClickEvent {
    Mouse(MouseClickEvent),
    Keyboard(KeyboardClickEvent),
    Touch(TouchClickEvent),
}

impl Default for ClickEvent {
    fn default() -> Self {
        ClickEvent::Keyboard(KeyboardClickEvent::default())
    }
}

impl ClickEvent {
    pub fn modifiers(&self) -> Modifiers {
        match self {
            ClickEvent::Keyboard(_) => Modifiers::default(),
            ClickEvent::Mouse(event) => event.up.modifiers,
            ClickEvent::Touch(_) => Modifiers::default(),
        }
    }

    pub fn position(&self) -> Point<Pixels> {
        match self {
            ClickEvent::Keyboard(event) => event.bounds.bottom_left(),
            ClickEvent::Mouse(event) => event.up.position,
            ClickEvent::Touch(event) => event.position,
        }
    }

    pub fn mouse_position(&self) -> Option<Point<Pixels>> {
        match self {
            ClickEvent::Keyboard(_) => None,
            ClickEvent::Mouse(event) => Some(event.up.position),
            ClickEvent::Touch(_) => None,
        }
    }

    pub fn is_right_click(&self) -> bool {
        match self {
            ClickEvent::Keyboard(_) => false,
            ClickEvent::Mouse(event) => {
                event.down.button == MouseButton::Right && event.up.button == MouseButton::Right
            }
            ClickEvent::Touch(_) => false,
        }
    }

    pub fn is_middle_click(&self) -> bool {
        match self {
            ClickEvent::Keyboard(_) => false,
            ClickEvent::Mouse(event) => {
                event.down.button == MouseButton::Middle && event.up.button == MouseButton::Middle
            }
            ClickEvent::Touch(_) => false,
        }
    }

    pub fn is_secondary(&self) -> bool {
        match self {
            ClickEvent::Keyboard(_) => false,
            ClickEvent::Mouse(event) => {
                event.down.button == MouseButton::Right && event.up.button == MouseButton::Right
            }
            ClickEvent::Touch(event) => event.long_press,
        }
    }

    pub fn standard_click(&self) -> bool {
        match self {
            ClickEvent::Keyboard(_) => true,
            ClickEvent::Mouse(event) => {
                event.down.button == MouseButton::Left && event.up.button == MouseButton::Left
            }
            ClickEvent::Touch(event) => !event.long_press,
        }
    }

    pub fn first_focus(&self) -> bool {
        match self {
            ClickEvent::Keyboard(_) => false,
            ClickEvent::Mouse(event) => event.down.first_mouse,
            ClickEvent::Touch(_) => false,
        }
    }

    pub fn click_count(&self) -> usize {
        match self {
            ClickEvent::Keyboard(_) => 1,
            ClickEvent::Mouse(event) => event.up.click_count,
            ClickEvent::Touch(event) => event.tap_count,
        }
    }

    pub fn is_keyboard(&self) -> bool {
        match self {
            ClickEvent::Mouse(_) | ClickEvent::Touch(_) => false,
            ClickEvent::Keyboard(_) => true,
        }
    }
}

#[derive(Hash, PartialEq, Eq, Copy, Clone, Debug, Default)]
pub enum KeyboardButton {
    #[default]
    Enter,
    Space,
}

#[derive(Hash, Default, PartialEq, Eq, Copy, Clone, Debug)]
pub enum MouseButton {
    #[default]
    Left,

    Right,

    Middle,

    Navigate(NavigationDirection),
}

impl MouseButton {
    pub fn all() -> Vec<Self> {
        vec![
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Navigate(NavigationDirection::Back),
            MouseButton::Navigate(NavigationDirection::Forward),
        ]
    }
}

#[derive(Hash, Default, PartialEq, Eq, Copy, Clone, Debug)]
pub enum NavigationDirection {
    #[default]
    Back,

    Forward,
}

#[derive(Clone, Debug, Default)]
pub struct MouseMoveEvent {
    pub position: Point<Pixels>,

    pub pressed_button: Option<MouseButton>,

    pub modifiers: Modifiers,
}

impl Sealed for MouseMoveEvent {}
impl InputEvent for MouseMoveEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::MouseMove(self)
    }
}
impl MouseEvent for MouseMoveEvent {}

impl MouseMoveEvent {
    pub fn dragging(&self) -> bool {
        self.pressed_button == Some(MouseButton::Left)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScrollWheelEvent {
    pub position: Point<Pixels>,

    pub delta: ScrollDelta,

    pub modifiers: Modifiers,

    pub touch_phase: TouchPhase,
}

impl Sealed for ScrollWheelEvent {}
impl InputEvent for ScrollWheelEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::ScrollWheel(self)
    }
}
impl MouseEvent for ScrollWheelEvent {}

impl Deref for ScrollWheelEvent {
    type Target = Modifiers;

    fn deref(&self) -> &Self::Target {
        &self.modifiers
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ScrollDelta {
    Pixels(Point<Pixels>),
    Lines(Point<f32>),
}

impl Default for ScrollDelta {
    fn default() -> Self {
        Self::Lines(Default::default())
    }
}

#[derive(Clone, Debug, Default)]
pub struct PinchEvent {
    pub position: Point<Pixels>,

    pub delta: f32,

    pub modifiers: Modifiers,

    pub phase: TouchPhase,
}

impl Sealed for PinchEvent {}
impl InputEvent for PinchEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::Pinch(self)
    }
}
impl GestureEvent for PinchEvent {}
impl MouseEvent for PinchEvent {}

impl Deref for PinchEvent {
    type Target = Modifiers;

    fn deref(&self) -> &Self::Target {
        &self.modifiers
    }
}

impl ScrollDelta {
    pub fn precise(&self) -> bool {
        match self {
            ScrollDelta::Pixels(_) => true,
            ScrollDelta::Lines(_) => false,
        }
    }

    pub fn pixel_delta(&self, line_height: Pixels) -> Point<Pixels> {
        match self {
            ScrollDelta::Pixels(delta) => *delta,
            ScrollDelta::Lines(delta) => point(line_height * delta.x, line_height * delta.y),
        }
    }

    pub fn coalesce(self, other: ScrollDelta) -> ScrollDelta {
        match (self, other) {
            (ScrollDelta::Pixels(a), ScrollDelta::Pixels(b)) => {
                let x = if a.x.signum() == b.x.signum() {
                    a.x + b.x
                } else {
                    b.x
                };

                let y = if a.y.signum() == b.y.signum() {
                    a.y + b.y
                } else {
                    b.y
                };

                ScrollDelta::Pixels(point(x, y))
            }

            (ScrollDelta::Lines(a), ScrollDelta::Lines(b)) => {
                let x = if a.x.signum() == b.x.signum() {
                    a.x + b.x
                } else {
                    b.x
                };

                let y = if a.y.signum() == b.y.signum() {
                    a.y + b.y
                } else {
                    b.y
                };

                ScrollDelta::Lines(point(x, y))
            }

            _ => other,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MouseExitEvent {
    pub position: Point<Pixels>,
    pub pressed_button: Option<MouseButton>,
    pub modifiers: Modifiers,
}

impl Sealed for MouseExitEvent {}
impl InputEvent for MouseExitEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::MouseExited(self)
    }
}

impl MouseEvent for MouseExitEvent {}

impl Deref for MouseExitEvent {
    type Target = Modifiers;

    fn deref(&self) -> &Self::Target {
        &self.modifiers
    }
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct ExternalPaths(pub SmallVec<[PathBuf; 2]>);

impl ExternalPaths {
    pub fn paths(&self) -> &[PathBuf] {
        &self.0
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ExternalDragPayload {
    Files(FileDragPaths),
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct FileDragPaths(SmallVec<[(PathBuf, bool); 2]>);

impl FileDragPaths {
    pub fn new(entries: impl IntoIterator<Item = (PathBuf, bool)>) -> Self {
        Self(entries.into_iter().collect())
    }

    pub fn entries(&self) -> &[(PathBuf, bool)] {
        &self.0
    }
}

impl Render for ExternalPaths {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Debug, Clone)]
pub enum FileDropEvent {
    Entered {
        position: Point<Pixels>,
        paths: ExternalPaths,
    },
    Pending {
        position: Point<Pixels>,
    },
    Submit {
        position: Point<Pixels>,
    },
    Exited,
    Ended,
}

impl Sealed for FileDropEvent {}
impl InputEvent for FileDropEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::FileDrop(self)
    }
}
impl MouseEvent for FileDropEvent {}

#[derive(Clone, Debug)]
pub enum PlatformInput {
    KeyDown(KeyDownEvent),
    KeyUp(KeyUpEvent),
    ModifiersChanged(ModifiersChangedEvent),
    MouseDown(MouseDownEvent),
    MouseUp(MouseUpEvent),
    MousePressure(MousePressureEvent),
    MouseMove(MouseMoveEvent),
    MouseExited(MouseExitEvent),
    ScrollWheel(ScrollWheelEvent),
    Pinch(PinchEvent),
    LongPress(LongPressEvent),
    TouchDrag(TouchDragEvent),
    FileDrop(FileDropEvent),
    Touch(TouchEvent),
}

impl PlatformInput {
    pub(crate) fn mouse_event(&self) -> Option<&dyn Any> {
        match self {
            PlatformInput::KeyDown { .. } => None,
            PlatformInput::KeyUp { .. } => None,
            PlatformInput::ModifiersChanged { .. } => None,
            PlatformInput::MouseDown(event) => Some(event),
            PlatformInput::MouseUp(event) => Some(event),
            PlatformInput::MouseMove(event) => Some(event),
            PlatformInput::MousePressure(event) => Some(event),
            PlatformInput::MouseExited(event) => Some(event),
            PlatformInput::ScrollWheel(event) => Some(event),
            PlatformInput::Pinch(event) => Some(event),
            PlatformInput::LongPress(event) => Some(event),
            PlatformInput::TouchDrag(event) => Some(event),
            PlatformInput::FileDrop(event) => Some(event),
            PlatformInput::Touch(_) => None,
        }
    }

    pub(crate) fn keyboard_event(&self) -> Option<&dyn Any> {
        match self {
            PlatformInput::KeyDown(event) => Some(event),
            PlatformInput::KeyUp(event) => Some(event),
            PlatformInput::ModifiersChanged(event) => Some(event),
            PlatformInput::MouseDown(_) => None,
            PlatformInput::MouseUp(_) => None,
            PlatformInput::MouseMove(_) => None,
            PlatformInput::MousePressure(_) => None,
            PlatformInput::MouseExited(_) => None,
            PlatformInput::ScrollWheel(_) => None,
            PlatformInput::Pinch(_) => None,
            PlatformInput::LongPress(_) => None,
            PlatformInput::TouchDrag(_) => None,
            PlatformInput::FileDrop(_) => None,
            PlatformInput::Touch(_) => None,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            PlatformInput::KeyDown(_) => "key_down",
            PlatformInput::KeyUp(_) => "key_up",
            PlatformInput::ModifiersChanged(_) => "modifiers_changed",
            PlatformInput::MouseDown(_) => "mouse_down",
            PlatformInput::MouseUp(_) => "mouse_up",
            PlatformInput::MousePressure(_) => "mouse_pressure",
            PlatformInput::MouseMove(_) => "mouse_move",
            PlatformInput::MouseExited(_) => "mouse_exited",
            PlatformInput::ScrollWheel(_) => "scroll_wheel",
            PlatformInput::Pinch(_) => "pinch",
            PlatformInput::LongPress(_) => "long_press",
            PlatformInput::TouchDrag(_) => "touch_drag",
            PlatformInput::FileDrop(_) => "file_drop",
            PlatformInput::Touch(_) => "touch",
        }
    }

    pub fn touch_event(&self) -> Option<&TouchEvent> {
        match self {
            PlatformInput::Touch(event) => Some(event),
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {

    use crate::{
        self as ala, AppContext as _, Context, FocusHandle, InteractiveElement, IntoElement,
        KeyBinding, Keystroke, Modifiers, ParentElement, Render, TestAppContext, Window, div,
    };

    struct TestView {
        saw_key_down: bool,
        saw_action: bool,
        focus_handle: FocusHandle,
    }

    actions!(test_only, [TestAction]);

    impl Render for TestView {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().id("testview").child(
                div()
                    .key_context("parent")
                    .on_key_down(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.saw_key_down = true
                    }))
                    .on_action(cx.listener(|this: &mut TestView, _: &TestAction, _, _| {
                        this.saw_action = true
                    }))
                    .child(
                        div()
                            .key_context("nested")
                            .track_focus(&self.focus_handle)
                            .into_element(),
                    ),
            )
        }
    }

    #[ala::test]
    fn test_on_events(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| TestView {
                    saw_key_down: false,
                    saw_action: false,
                    focus_handle: cx.focus_handle(),
                })
            })
            .unwrap()
        });

        cx.update(|cx| {
            cx.bind_keys(vec![KeyBinding::new("ctrl-g", TestAction, Some("parent"))]);
        });

        window
            .update(cx, |test_view, window, cx| {
                window.focus(&test_view.focus_handle, cx)
            })
            .unwrap();

        cx.dispatch_keystroke(*window, Keystroke::parse("a").unwrap());
        cx.dispatch_keystroke(*window, Keystroke::parse("ctrl-g").unwrap());

        window
            .update(cx, |test_view, _, _| {
                assert!(test_view.saw_key_down || test_view.saw_action);
                assert!(test_view.saw_key_down);
                assert!(test_view.saw_action);
            })
            .unwrap();
    }

    #[ala::test]
    fn test_multi_modifier_gesture_does_not_dispatch_standalone_modifier_binding(
        cx: &mut TestAppContext,
    ) {
        let (test_view, cx) = cx.add_window_view(|_, cx| TestView {
            saw_key_down: false,
            saw_action: false,
            focus_handle: cx.focus_handle(),
        });

        cx.update(|_, cx| {
            cx.bind_keys(vec![KeyBinding::new("shift", TestAction, None)]);
        });
        test_view.update_in(cx, |test_view, window, cx| {
            window.focus(&test_view.focus_handle, cx);
        });

        cx.simulate_modifiers_change(Modifiers::alt());
        cx.simulate_modifiers_change(Modifiers::alt() | Modifiers::shift());
        cx.simulate_modifiers_change(Modifiers::shift());
        cx.simulate_modifiers_change(Modifiers::none());
        assert!(!test_view.read_with(cx, |test_view, _| test_view.saw_action));

        cx.simulate_modifiers_change(Modifiers::shift());
        cx.simulate_modifiers_change(Modifiers::none());
        assert!(test_view.read_with(cx, |test_view, _| test_view.saw_action));
    }
}
