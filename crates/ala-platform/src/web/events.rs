use std::{collections::HashMap, rc::Rc};

use ala::{
    Capslock, ClipboardEntry, ClipboardItem, ClipboardString, DispatchEventResult, GestureTuning,
    Image, ImageFormat, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, ModifiersChangedEvent,
    MouseButton, MouseDownEvent, MouseExitEvent, MouseMoveEvent, MouseUpEvent, NavigationDirection,
    Pixels, PlatformInput, Point, ScrollDelta, ScrollWheelEvent, TouchEvent, TouchId, TouchPhase,
    point, px,
};
use wasm_bindgen::prelude::*;

use crate::web::ime_mirror::ImeMirror;
use crate::web::window::WebWindowInner;

pub struct WebEventListeners {
    _handles: Vec<EventListenerHandle>,
}

pub(crate) struct EventListenerHandle {
    target: web_sys::EventTarget,
    event_name: &'static str,
    closure: Closure<dyn FnMut(JsValue)>,
}

impl EventListenerHandle {
    pub(crate) fn add(
        target: &web_sys::EventTarget,
        event_name: &'static str,
        handler: impl FnMut(JsValue) + 'static,
    ) -> Self {
        let closure = Closure::<dyn FnMut(JsValue)>::new(handler);
        target
            .add_event_listener_with_callback(event_name, closure.as_ref().unchecked_ref())
            .ok();
        Self {
            target: target.clone(),
            event_name,
            closure,
        }
    }

    pub(crate) fn add_non_passive(
        target: &web_sys::EventTarget,
        event_name: &'static str,
        handler: impl FnMut(JsValue) + 'static,
    ) -> Self {
        let closure = Closure::<dyn FnMut(JsValue)>::new(handler);
        let target_js: &JsValue = target.as_ref();
        let callback_js: &JsValue = closure.as_ref();
        let options = js_sys::Object::new();
        js_sys::Reflect::set(&options, &"passive".into(), &false.into()).ok();
        if let Ok(add_fn_val) = js_sys::Reflect::get(target_js, &"addEventListener".into()) {
            if let Ok(add_fn) = add_fn_val.dyn_into::<js_sys::Function>() {
                add_fn
                    .call3(target_js, &event_name.into(), callback_js, &options)
                    .ok();
            }
        }
        Self {
            target: target.clone(),
            event_name,
            closure,
        }
    }
}

impl Drop for EventListenerHandle {
    fn drop(&mut self) {
        self.target
            .remove_event_listener_with_callback(
                self.event_name,
                self.closure.as_ref().unchecked_ref(),
            )
            .ok();
    }
}

pub(crate) struct ClickState {
    last_position: Point<Pixels>,
    last_time: f64,
    current_count: usize,
}

#[derive(Default)]
pub(crate) struct TouchIds {
    next: u64,
    active: HashMap<i32, TouchId>,
}

impl TouchIds {
    fn start(&mut self, pointer_id: i32) -> Option<TouchId> {
        let next = self.next.checked_add(1)?;
        let touch_id = TouchId(self.next);
        self.next = next;
        self.active.insert(pointer_id, touch_id);
        Some(touch_id)
    }

    fn active(&self, pointer_id: i32) -> Option<TouchId> {
        self.active.get(&pointer_id).copied()
    }

    fn end(&mut self, pointer_id: i32) -> Option<TouchId> {
        self.active.remove(&pointer_id)
    }
}

impl Default for ClickState {
    fn default() -> Self {
        Self {
            last_position: Point::default(),
            last_time: 0.0,
            current_count: 0,
        }
    }
}

impl ClickState {
    fn register_click(&mut self, position: Point<Pixels>, time: f64) -> usize {
        let distance = ((f32::from(position.x) - f32::from(self.last_position.x)).powi(2)
            + (f32::from(position.y) - f32::from(self.last_position.y)).powi(2))
        .sqrt();

        if (time - self.last_time) < 400.0 && distance < 5.0 {
            self.current_count += 1;
        } else {
            self.current_count = 1;
        }

        self.last_position = position;
        self.last_time = time;
        self.current_count
    }
}

impl WebWindowInner {
    pub fn register_event_listeners(self: &Rc<Self>) -> WebEventListeners {
        let mut handles = vec![
            self.register_pointer_down(),
            self.register_pointer_up(),
            self.register_pointer_cancel(),
            self.register_touch_end(),
            self.register_pointer_move(),
            self.register_pointer_leave(),
            self.register_wheel(),
            self.register_context_menu(),
            self.register_dragover(),
            self.register_drop(),
            self.register_key_down(),
            self.register_key_up(),
            self.register_before_input(),
            self.register_input(),
            self.register_paste(),
            self.register_composition_start(),
            self.register_composition_update(),
            self.register_composition_end(),
            self.register_focus(),
            self.register_blur(),
            self.register_pointer_enter(),
        ];
        handles.extend(self.register_selection_change());
        handles.extend(self.register_visibility_change());
        handles.extend(self.register_appearance_change());
        handles.extend(self.register_fullscreen_change());
        handles.extend(self.register_viewport_changes());

        WebEventListeners { _handles: handles }
    }

    fn register_viewport_changes(self: &Rc<Self>) -> Vec<EventListenerHandle> {
        let mut targets: Vec<web_sys::EventTarget> = vec![self.browser_window.clone().into()];
        if let Some(viewport) = self.browser_window.visual_viewport() {
            targets.push(viewport.into());
        }
        targets
            .into_iter()
            .flat_map(|target| {
                ["resize", "scroll"].map(|event_name| {
                    let this = Rc::clone(self);
                    EventListenerHandle::add(&target, event_name, move |_| {
                        this.notify_viewport_changed()
                    })
                })
            })
            .collect()
    }

    fn listen(
        self: &Rc<Self>,
        event_name: &'static str,
        handler: impl FnMut(JsValue) + 'static,
    ) -> EventListenerHandle {
        EventListenerHandle::add(self.canvas.as_ref(), event_name, handler)
    }

    fn listen_input(
        self: &Rc<Self>,
        event_name: &'static str,
        handler: impl FnMut(JsValue) + 'static,
    ) -> EventListenerHandle {
        EventListenerHandle::add(self.ime_mirror.event_target(), event_name, handler)
    }

    fn listen_non_passive(
        self: &Rc<Self>,
        event_name: &'static str,
        handler: impl FnMut(JsValue) + 'static,
    ) -> EventListenerHandle {
        EventListenerHandle::add_non_passive(self.canvas.as_ref(), event_name, handler)
    }

    fn dispatch_input(&self, input: PlatformInput) -> Option<DispatchEventResult> {
        self.with_callback(|callbacks| &mut callbacks.input, |callback| callback(input))
    }

    fn update_modifiers(&self, modifiers: Modifiers, capslock: Capslock) -> bool {
        let mut current_state = self.state.borrow_mut();
        let changed = current_state.modifiers != modifiers || current_state.capslock != capslock;
        current_state.modifiers = modifiers;
        current_state.capslock = capslock;
        changed
    }

    fn register_pointer_down(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen("pointerdown", move |event: JsValue| {
            let event: web_sys::PointerEvent = event.unchecked_into();
            event.prevent_default();

            let pointer_type = event.pointer_type();
            let position = pointer_position_in_element(&event);
            this.gesture_start_visual_viewport_height
                .set(this.visual_viewport_height());

            this.canvas.set_pointer_capture(event.pointer_id()).ok();

            if pointer_type == "touch" {
                if this.keyboard_likely_dismissed() {
                    this.ime_mirror.set_virtual_keyboard_enabled(false);
                }
                let Some(touch_id) = this.touch_ids.borrow_mut().start(event.pointer_id()) else {
                    log::error!("exhausted touch identifiers");
                    return;
                };
                this.state.borrow_mut().mouse_position = position;
                if this.touch_tap_candidate.get().is_none() {
                    this.touch_tap_candidate
                        .set(Some((event.pointer_id(), position)));
                }
                this.dispatch_input(PlatformInput::Touch(TouchEvent {
                    id: touch_id,
                    phase: TouchPhase::Started,
                    position,
                    predicted_position: None,
                    force: None,
                }));
                return;
            }

            let button = dom_mouse_button_to_ala(event.button());
            let modifiers = modifiers_from_mouse_event(&event, this.is_mac);
            let time = js_sys::Date::now();

            this.pressed_button.set(Some(button));
            let click_count = this.click_state.borrow_mut().register_click(position, time);

            {
                let mut current_state = this.state.borrow_mut();
                current_state.mouse_position = position;
                current_state.modifiers = modifiers;
            }

            this.dispatch_input(PlatformInput::MouseDown(MouseDownEvent {
                button,
                position,
                modifiers,
                click_count,
                first_mouse: false,
            }));

            this.ime_mirror.focus();
        })
    }

    fn pointer_targets_text_input(&self, position: Point<Pixels>) -> bool {
        self.with_input_handler(|handler| {
            handler.query_accepts_text_input()
                && handler
                    .element_bounds()
                    .is_some_and(|bounds| bounds.contains(&position))
        })
        .unwrap_or(false)
    }

    fn register_pointer_up(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen("pointerup", move |event: JsValue| {
            let event: web_sys::PointerEvent = event.unchecked_into();
            event.prevent_default();

            let position = pointer_position_in_element(&event);

            if event.pointer_type() == "touch" {
                let Some(touch_id) = this.touch_ids.borrow_mut().end(event.pointer_id()) else {
                    return;
                };
                this.state.borrow_mut().mouse_position = position;
                let completes_tap = match this.touch_tap_candidate.get() {
                    Some((pointer_id, _)) if pointer_id == event.pointer_id() => {
                        this.touch_tap_candidate.set(None);
                        true
                    }
                    _ => false,
                };
                let dispatch_result = this.dispatch_input(PlatformInput::Touch(TouchEvent {
                    id: touch_id,
                    phase: TouchPhase::Ended,
                    position,
                    predicted_position: None,
                    force: None,
                }));

                let viewport_stable = this.gesture_start_visual_viewport_height.get()
                    == this.visual_viewport_height();
                if completes_tap
                    && viewport_stable
                    && dispatch_result.is_some_and(|result| !result.default_prevented)
                    && this.pointer_targets_text_input(position)
                {
                    this.show_virtual_keyboard();
                }
                this.schedule_ime_mirror_sync();
                return;
            }

            let button = dom_mouse_button_to_ala(event.button());
            let modifiers = modifiers_from_mouse_event(&event, this.is_mac);

            this.pressed_button.set(None);
            let click_count = this.click_state.borrow().current_count;

            {
                let mut current_state = this.state.borrow_mut();
                current_state.mouse_position = position;
                current_state.modifiers = modifiers;
            }

            this.dispatch_input(PlatformInput::MouseUp(MouseUpEvent {
                button,
                position,
                modifiers,
                click_count,
            }));

            this.schedule_ime_mirror_sync();
        })
    }

    fn visual_viewport_height(&self) -> f64 {
        self.browser_window
            .visual_viewport()
            .map_or(0.0, |viewport| viewport.height() * viewport.scale())
    }

    fn keyboard_likely_dismissed(&self) -> bool {
        if !self.touch_input {
            return false;
        }
        let Some(viewport) = self.browser_window.visual_viewport() else {
            return false;
        };
        let Some(document) = self.browser_window.document() else {
            return false;
        };
        if let Ok(Some(meta)) = document.query_selector("meta[name=viewport]")
            && let Some(content) = meta.get_attribute("content")
            && content.split(',').any(|directive| {
                directive.split_once('=').is_some_and(|(key, value)| {
                    key.trim().eq_ignore_ascii_case("interactive-widget")
                        && !value.trim().eq_ignore_ascii_case("resizes-visual")
                })
            })
        {
            return false;
        }
        let Some(root) = document.document_element() else {
            return false;
        };
        (viewport.scale() - 1.).abs() < 0.001
            && viewport.offset_top().abs() < 1.
            && viewport.offset_left().abs() < 1.
            && (viewport.height() - root.client_height() as f64).abs() < 1.
            && (viewport.width() - root.client_width() as f64).abs() < 1.
    }

    fn register_pointer_cancel(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen("pointercancel", move |event: JsValue| {
            let event: web_sys::PointerEvent = event.unchecked_into();
            if event.pointer_type() == "touch" {
                let Some(touch_id) = this.touch_ids.borrow_mut().end(event.pointer_id()) else {
                    return;
                };
                if let Some((pointer_id, _)) = this.touch_tap_candidate.get()
                    && pointer_id == event.pointer_id()
                {
                    this.touch_tap_candidate.set(None);
                }
                this.dispatch_input(PlatformInput::Touch(TouchEvent {
                    id: touch_id,
                    phase: TouchPhase::Cancelled,
                    position: pointer_position_in_element(&event),
                    predicted_position: None,
                    force: None,
                }));
            } else {
                this.pressed_button.set(None);
            }
        })
    }

    fn register_touch_end(self: &Rc<Self>) -> EventListenerHandle {
        self.listen_non_passive("touchend", move |event: JsValue| {
            let event: web_sys::Event = event.unchecked_into();
            event.prevent_default();
        })
    }

    fn schedule_ime_mirror_sync(self: &Rc<Self>) {
        ImeMirror::schedule_sync(self);
    }

    pub(crate) fn show_virtual_keyboard(self: &Rc<Self>) {
        if !self.touch_input {
            return;
        }
        let was_enabled = self.ime_mirror.virtual_keyboard_enabled();
        self.ime_mirror.set_virtual_keyboard_enabled(true);
        if !was_enabled || !self.ime_mirror.is_focused() || self.keyboard_likely_dismissed() {
            self.focus_ime_mirror();
        }
    }

    pub(crate) fn focus_ime_mirror(self: &Rc<Self>) {
        self.suppress_focus_status_events.set(true);
        if self.ime_mirror.is_focused() {
            self.ime_mirror.blur();
        }
        self.ime_mirror.focus();
        self.suppress_focus_status_events.set(false);

        let callback = wasm_bindgen::closure::Closure::once_into_js({
            let this = Rc::clone(self);
            move || {
                this.state.borrow_mut().is_active = true;
                *this.active_window.borrow_mut() = Some(this.handle);
                this.with_callback(
                    |callbacks| &mut callbacks.active_status_change,
                    |callback| callback(true),
                );
            }
        });
        if let Err(error) = self
            .browser_window
            .set_timeout_with_callback(callback.unchecked_ref())
        {
            log::warn!("failed to defer web window activation: {error:?}");
        }
    }

    fn dispatch_synthetic_keystroke(&self, key: &str, modifiers: Modifiers) {
        let keystroke = Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: None,
        };
        self.dispatch_input(PlatformInput::KeyDown(KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        }));
        self.dispatch_input(PlatformInput::KeyUp(KeyUpEvent { keystroke }));
    }

    fn register_pointer_move(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen("pointermove", move |event: JsValue| {
            let event: web_sys::PointerEvent = event.unchecked_into();
            event.prevent_default();

            let position = pointer_position_in_element(&event);

            if event.pointer_type() == "touch" {
                let Some(touch_id) = this.touch_ids.borrow().active(event.pointer_id()) else {
                    return;
                };
                this.state.borrow_mut().mouse_position = position;
                if let Some((pointer_id, start_position)) = this.touch_tap_candidate.get()
                    && pointer_id == event.pointer_id()
                    && (position - start_position).magnitude()
                        > f64::from(GestureTuning::default().touch_slop)
                {
                    this.touch_tap_candidate.set(None);
                }
                this.dispatch_input(PlatformInput::Touch(TouchEvent {
                    id: touch_id,
                    phase: TouchPhase::Moved,
                    position,
                    predicted_position: predicted_pointer_position(&event, position),
                    force: None,
                }));
                return;
            }

            let modifiers = modifiers_from_mouse_event(&event, this.is_mac);
            let current_pressed = this.pressed_button.get();

            {
                let mut current_state = this.state.borrow_mut();
                current_state.mouse_position = position;
                current_state.modifiers = modifiers;
            }

            this.dispatch_input(PlatformInput::MouseMove(MouseMoveEvent {
                position,
                pressed_button: current_pressed,
                modifiers,
            }));
        })
    }

    fn register_pointer_leave(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen("pointerleave", move |event: JsValue| {
            let event: web_sys::PointerEvent = event.unchecked_into();
            let mouse_event: &web_sys::MouseEvent = event.as_ref();
            if this.a11y.contains(mouse_event.related_target()) {
                return;
            }

            let position = pointer_position_in_element(&event);
            let modifiers = modifiers_from_mouse_event(&event, this.is_mac);
            let current_pressed = this.pressed_button.get();

            {
                let mut current_state = this.state.borrow_mut();
                current_state.mouse_position = position;
                current_state.modifiers = modifiers;
                current_state.is_hovered = false;
            }

            this.dispatch_input(PlatformInput::MouseExited(MouseExitEvent {
                position,
                pressed_button: current_pressed,
                modifiers,
            }));

            this.with_callback(
                |callbacks| &mut callbacks.hover_status_change,
                |callback| callback(false),
            );
        })
    }

    fn register_wheel(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen_non_passive("wheel", move |event: JsValue| {
            let event: web_sys::WheelEvent = event.unchecked_into();

            let mouse_event: &web_sys::MouseEvent = event.as_ref();
            let position = mouse_position_in_element(mouse_event);
            let modifiers = modifiers_from_wheel_event(mouse_event, this.is_mac);

            let delta_mode = event.delta_mode();
            let delta = if delta_mode == 1 {
                ScrollDelta::Lines(point(-event.delta_x() as f32, -event.delta_y() as f32))
            } else {
                ScrollDelta::Pixels(point(
                    px(-event.delta_x() as f32),
                    px(-event.delta_y() as f32),
                ))
            };

            {
                let mut current_state = this.state.borrow_mut();
                current_state.modifiers = modifiers;
            }

            let result = this.dispatch_input(PlatformInput::ScrollWheel(ScrollWheelEvent {
                position,
                delta,
                modifiers,
                touch_phase: TouchPhase::Moved,
            }));
            if this.mount.is_none() || result.is_none_or(|result| result.default_prevented) {
                event.prevent_default();
            }
        })
    }

    fn register_context_menu(self: &Rc<Self>) -> EventListenerHandle {
        self.listen("contextmenu", move |event: JsValue| {
            let event: web_sys::Event = event.unchecked_into();
            event.prevent_default();
        })
    }

    fn register_dragover(self: &Rc<Self>) -> EventListenerHandle {
        self.listen("dragover", move |event: JsValue| {
            let event: web_sys::DragEvent = event.unchecked_into();
            event.prevent_default();
        })
    }

    fn register_drop(self: &Rc<Self>) -> EventListenerHandle {
        self.listen("drop", move |event: JsValue| {
            let event: web_sys::DragEvent = event.unchecked_into();
            event.prevent_default();
        })
    }

    fn register_key_down(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen_input("keydown", move |event: JsValue| {
            let event: web_sys::KeyboardEvent = event.unchecked_into();

            let modifiers = modifiers_from_keyboard_event(&event, this.is_mac);
            let capslock = capslock_from_keyboard_event(&event);

            if this.update_modifiers(modifiers, capslock) {
                this.dispatch_input(PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                    modifiers,
                    capslock,
                }));
            }

            let key = dom_key_to_ala_key(&event);

            if is_modifier_only_key(&key) {
                return;
            }

            let is_held = event.repeat();
            let key_char = compute_key_char(&event, &key, &modifiers);

            let keystroke = Keystroke {
                modifiers,
                key,
                key_char: key_char.clone(),
            };

            let result = this.dispatch_input(PlatformInput::KeyDown(KeyDownEvent {
                keystroke,
                is_held,
                prefer_character_input: false,
            }));

            if let Some(result) = result {
                if !result.propagate {
                    event.prevent_default();
                    this.schedule_ime_mirror_sync();
                    return;
                }
            }

            if this.is_composing.get() || event.is_composing() {
                event.prevent_default();
                return;
            }

            if keystroke_inserts_text(&modifiers, this.is_mac)
                && let Some(text) = key_char
            {
                this.with_input_handler(|handler| {
                    handler.replace_text_in_range(None, &text);
                });
                event.prevent_default();
            }
            this.schedule_ime_mirror_sync();
        })
    }

    fn register_key_up(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen_input("keyup", move |event: JsValue| {
            let event: web_sys::KeyboardEvent = event.unchecked_into();

            let modifiers = modifiers_from_keyboard_event(&event, this.is_mac);
            let capslock = capslock_from_keyboard_event(&event);

            if this.update_modifiers(modifiers, capslock) {
                this.dispatch_input(PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                    modifiers,
                    capslock,
                }));
            }

            let key = dom_key_to_ala_key(&event);

            if is_modifier_only_key(&key) {
                return;
            }

            let key_char = compute_key_char(&event, &key, &modifiers);

            let keystroke = Keystroke {
                modifiers,
                key,
                key_char,
            };

            let result = this.dispatch_input(PlatformInput::KeyUp(KeyUpEvent { keystroke }));
            if let Some(result) = result {
                if !result.propagate {
                    event.prevent_default();
                }
            }
        })
    }

    fn register_input(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen_input("input", move |event: JsValue| {
            let event: web_sys::InputEvent = event.unchecked_into();

            if this.is_composing.get() || event.is_composing() {
                return;
            }

            let new_value = this.ime_mirror.value();
            let old_value = this.ime_mirror.stored_text();
            if new_value == old_value {
                return;
            }

            let old_units: Vec<u16> = old_value.encode_utf16().collect();
            let new_units: Vec<u16> = new_value.encode_utf16().collect();

            let post_edit_caret = this
                .ime_mirror
                .selection_start()
                .map(|caret| caret as usize);
            let anchored_suffix_length = post_edit_caret
                .map(|caret| new_units.len().saturating_sub(caret))
                .filter(|&suffix_length| {
                    suffix_length <= old_units.len()
                        && old_units[old_units.len() - suffix_length..]
                            == new_units[new_units.len() - suffix_length..]
                });
            let suffix_length = anchored_suffix_length.unwrap_or_else(|| {
                old_units
                    .iter()
                    .rev()
                    .zip(new_units.iter().rev())
                    .take_while(|(old_unit, new_unit)| old_unit == new_unit)
                    .count()
            });
            let prefix_length = old_units
                .iter()
                .zip(&new_units)
                .take_while(|(old_unit, new_unit)| old_unit == new_unit)
                .count()
                .min(old_units.len() - suffix_length)
                .min(new_units.len() - suffix_length);

            let inserted_text = String::from_utf16_lossy(
                &new_units[prefix_length..new_units.len() - suffix_length],
            );
            let replaced_old_end = old_units.len() - suffix_length;

            let (element_selection_start, element_selection_end) =
                this.ime_mirror.stored_selection();
            let removed_before_selection =
                (element_selection_start as usize).saturating_sub(prefix_length);
            let removed_after_selection =
                replaced_old_end.saturating_sub(element_selection_end as usize);

            let applied = this.with_input_handler(|handler| {
                let Some(selection) = handler.selected_text_range(false) else {
                    return false;
                };
                let range = selection
                    .range
                    .start
                    .saturating_sub(removed_before_selection)
                    ..selection.range.end + removed_after_selection;
                handler.replace_text_in_range(Some(range), &inserted_text);
                true
            });
            if applied != Some(true) {
                return;
            }

            this.ime_mirror.adopt_element_state();
        })
    }

    fn register_selection_change(self: &Rc<Self>) -> Option<EventListenerHandle> {
        let document = self.browser_window.document()?;
        let this = Rc::clone(self);
        Some(EventListenerHandle::add(
            document.as_ref(),
            "selectionchange",
            move |_event: JsValue| {
                if this.is_composing.get() || !this.ime_mirror.is_focused() {
                    return;
                }
                if this.ime_mirror.value() != this.ime_mirror.stored_text() {
                    return;
                }
                let Some(element_start) = this.ime_mirror.selection_start() else {
                    return;
                };
                let element_end = this
                    .ime_mirror
                    .element_selection_end()
                    .unwrap_or(element_start);
                let (stored_start, stored_end) = this.ime_mirror.stored_selection();
                if (element_start, element_end) == (stored_start, stored_end) {
                    return;
                }
                let applied = this.with_input_handler(|handler| {
                    let Some(selection) = handler.selected_text_range(false) else {
                        return false;
                    };
                    let alignment = selection.range.start.checked_sub(stored_start as usize);
                    if alignment.is_none()
                        || alignment != selection.range.end.checked_sub(stored_end as usize)
                    {
                        return false;
                    }
                    let alignment = alignment.unwrap();
                    handler.set_selected_text_range(
                        alignment + element_start as usize..alignment + element_end as usize,
                    );
                    true
                });
                if applied == Some(true) {
                    this.ime_mirror.adopt_element_state();
                } else {
                    this.ime_mirror.reject_selection_import();
                    this.schedule_ime_mirror_sync();
                }
            },
        ))
    }

    fn register_before_input(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen_input("beforeinput", move |event: JsValue| {
            let event: web_sys::InputEvent = event.unchecked_into();

            if this.is_composing.get() || event.is_composing() {
                return;
            }

            match event.input_type().as_str() {
                "insertLineBreak" | "insertParagraph" => {
                    event.prevent_default();
                    this.dispatch_synthetic_keystroke("enter", Modifiers::default());
                    this.schedule_ime_mirror_sync();
                }
                _ => {}
            }
        })
    }

    fn register_paste(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen_input("paste", move |event: JsValue| {
            let event: web_sys::ClipboardEvent = event.unchecked_into();
            let Some(clipboard_data) = event.clipboard_data() else {
                return;
            };
            let text = clipboard_data
                .get_data("text/plain")
                .ok()
                .filter(|text| !text.is_empty());

            let mut image_files = Vec::new();
            let items = clipboard_data.items();
            for index in 0..items.length() {
                let Some(item) = items.get(index) else {
                    continue;
                };
                if item.kind() != "file" {
                    continue;
                }
                let Some(format) = ImageFormat::from_mime_type(&item.type_()) else {
                    continue;
                };
                if let Ok(Some(file)) = item.get_as_file() {
                    image_files.push((format, file));
                }
            }

            if text.is_none() && image_files.is_empty() {
                return;
            }
            event.prevent_default();

            if image_files.is_empty() {
                if let Some(text) = text {
                    this.with_input_handler(|handler| {
                        handler.paste(ClipboardItem::new_string(text));
                    });
                }
                return;
            }

            let this = Rc::clone(&this);
            wasm_bindgen_futures::spawn_local(async move {
                let mut entries = Vec::new();
                if let Some(text) = text {
                    entries.push(ClipboardEntry::String(ClipboardString::new(text)));
                }
                for (format, file) in image_files {
                    match crate::web::platform::read_blob_bytes(&file).await {
                        Ok(bytes) => {
                            entries.push(ClipboardEntry::Image(Image::from_bytes(format, bytes)));
                        }
                        Err(error) => {
                            log::error!(
                                "failed to read pasted image: {}",
                                crate::web::platform::js_error_message(&error)
                            );
                        }
                    }
                }
                if entries.is_empty() {
                    return;
                }
                this.with_input_handler(|handler| {
                    handler.paste(ClipboardItem { entries });
                });
            });
        })
    }

    fn register_composition_start(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen_input("compositionstart", move |_event: JsValue| {
            this.is_composing.set(true);
        })
    }

    fn register_composition_update(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen_input("compositionupdate", move |event: JsValue| {
            let event: web_sys::CompositionEvent = event.unchecked_into();
            let data = event.data().unwrap_or_default();
            this.is_composing.set(true);
            this.with_input_handler(|handler| {
                handler.replace_and_mark_text_in_range(None, &data, None);
            });
        })
    }

    fn register_composition_end(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen_input("compositionend", move |event: JsValue| {
            let event: web_sys::CompositionEvent = event.unchecked_into();
            let data = event.data().unwrap_or_default();
            this.is_composing.set(false);
            this.with_input_handler(|handler| {
                if handler.marked_text_range().is_some() {
                    handler.replace_text_in_range(None, &data);
                }
                handler.unmark_text();
            });
            this.ime_mirror.adopt_element_state();
            this.schedule_ime_mirror_sync();
        })
    }

    fn register_focus(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen_input("focus", move |_event: JsValue| {
            if this.suppress_focus_status_events.get() {
                return;
            }
            {
                let mut state = this.state.borrow_mut();
                state.is_active = true;
            }
            *this.active_window.borrow_mut() = Some(this.handle);
            this.report_activity();
        })
    }

    fn register_blur(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen_input("blur", move |_event: JsValue| {
            if this.suppress_focus_status_events.get() {
                return;
            }
            {
                let mut state = this.state.borrow_mut();
                state.is_active = false;
            }
            this.report_activity();
        })
    }

    fn report_activity(self: &Rc<Self>) {
        let this = Rc::clone(self);
        wasm_bindgen_futures::spawn_local(async move {
            let active = this.state.borrow().is_active;
            this.with_callback(
                |callbacks| &mut callbacks.active_status_change,
                |callback| callback(active),
            );
        });
    }

    fn register_pointer_enter(self: &Rc<Self>) -> EventListenerHandle {
        let this = Rc::clone(self);
        self.listen("pointerenter", move |_event: JsValue| {
            {
                let mut state = this.state.borrow_mut();
                state.is_hovered = true;
            }
            this.with_callback(
                |callbacks| &mut callbacks.hover_status_change,
                |callback| callback(true),
            );
        })
    }
}

fn dom_key_to_ala_key(event: &web_sys::KeyboardEvent) -> String {
    let key = event.key();
    match key.as_str() {
        "Enter" => "enter".to_string(),
        "Backspace" => "backspace".to_string(),
        "Tab" => "tab".to_string(),
        "Escape" => "escape".to_string(),
        "Delete" => "delete".to_string(),
        " " => "space".to_string(),
        "ArrowLeft" => "left".to_string(),
        "ArrowRight" => "right".to_string(),
        "ArrowUp" => "up".to_string(),
        "ArrowDown" => "down".to_string(),
        "Home" => "home".to_string(),
        "End" => "end".to_string(),
        "PageUp" => "pageup".to_string(),
        "PageDown" => "pagedown".to_string(),
        "Insert" => "insert".to_string(),
        "Control" => "control".to_string(),
        "Alt" => "alt".to_string(),
        "Shift" => "shift".to_string(),
        "Meta" => "platform".to_string(),
        "CapsLock" => "capslock".to_string(),
        other => {
            if let Some(rest) = other.strip_prefix('F') {
                if let Ok(number) = rest.parse::<u8>() {
                    if (1..=35).contains(&number) {
                        return format!("f{number}");
                    }
                }
            }
            other.to_lowercase()
        }
    }
}

fn dom_mouse_button_to_ala(button: i16) -> MouseButton {
    match button {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        3 => MouseButton::Navigate(NavigationDirection::Back),
        4 => MouseButton::Navigate(NavigationDirection::Forward),
        _ => MouseButton::Left,
    }
}

fn modifiers_from_keyboard_event(event: &web_sys::KeyboardEvent, _is_mac: bool) -> Modifiers {
    Modifiers {
        control: event.ctrl_key(),
        alt: event.alt_key(),
        shift: event.shift_key(),
        platform: event.meta_key(),
        function: false,
    }
}

fn modifiers_from_mouse_event(event: &web_sys::PointerEvent, _is_mac: bool) -> Modifiers {
    let mouse_event: &web_sys::MouseEvent = event.as_ref();
    Modifiers {
        control: mouse_event.ctrl_key(),
        alt: mouse_event.alt_key(),
        shift: mouse_event.shift_key(),
        platform: mouse_event.meta_key(),
        function: false,
    }
}

fn modifiers_from_wheel_event(event: &web_sys::MouseEvent, _is_mac: bool) -> Modifiers {
    Modifiers {
        control: event.ctrl_key(),
        alt: event.alt_key(),
        shift: event.shift_key(),
        platform: event.meta_key(),
        function: false,
    }
}

fn capslock_from_keyboard_event(event: &web_sys::KeyboardEvent) -> Capslock {
    Capslock {
        on: event.get_modifier_state("CapsLock"),
    }
}

pub(crate) fn is_mac_platform(browser_window: &web_sys::Window) -> bool {
    let navigator = browser_window.navigator();

    #[allow(deprecated)]
    if let Ok(platform) = navigator.platform() {
        if platform.contains("Mac") {
            return true;
        }
    }

    if let Ok(user_agent) = navigator.user_agent() {
        return user_agent.contains("Mac");
    }

    false
}

fn is_modifier_only_key(key: &str) -> bool {
    matches!(
        key,
        "control" | "alt" | "shift" | "platform" | "capslock" | "compose" | "process"
    )
}

fn keystroke_inserts_text(modifiers: &Modifiers, is_mac: bool) -> bool {
    if is_mac {
        !modifiers.platform && !modifiers.control
    } else {
        modifiers.is_subset_of(&Modifiers::shift()) || (modifiers.control && modifiers.alt)
    }
}

fn compute_key_char(
    event: &web_sys::KeyboardEvent,
    ala_key: &str,
    modifiers: &Modifiers,
) -> Option<String> {
    if (modifiers.platform || modifiers.control) && !(modifiers.control && modifiers.alt) {
        return None;
    }

    if is_modifier_only_key(ala_key) {
        return None;
    }

    if ala_key == "space" {
        return Some(" ".to_string());
    }

    let raw_key = event.key();

    if raw_key.len() == 1 {
        return Some(raw_key);
    }

    None
}

fn pointer_position_in_element(event: &web_sys::PointerEvent) -> Point<Pixels> {
    let mouse_event: &web_sys::MouseEvent = event.as_ref();
    mouse_position_in_element(mouse_event)
}

const MAX_PREDICTION_LEAD_MS: f64 = 10.;

fn predicted_pointer_position(
    event: &web_sys::PointerEvent,
    position: Point<Pixels>,
) -> Option<Point<Pixels>> {
    let method = js_sys::Reflect::get(event, &JsValue::from_str("getPredictedEvents")).ok()?;
    let method = method.dyn_ref::<js_sys::Function>()?;
    let predicted_events: js_sys::Array = method.call0(event).ok()?.dyn_into().ok()?;
    let mut best: Option<(f64, web_sys::PointerEvent)> = None;
    for predicted in predicted_events.iter() {
        let Ok(predicted) = predicted.dyn_into::<web_sys::PointerEvent>() else {
            continue;
        };
        let lead = predicted.time_stamp() - event.time_stamp();
        if lead <= 0. {
            continue;
        }
        let distance_to_cap = (lead - MAX_PREDICTION_LEAD_MS).abs();
        if best
            .as_ref()
            .is_none_or(|(best_distance, _)| distance_to_cap < *best_distance)
        {
            best = Some((distance_to_cap, predicted));
        }
    }
    let (_, predicted) = best?;
    let lead = predicted.time_stamp() - event.time_stamp();
    let scale = (MAX_PREDICTION_LEAD_MS / lead).min(1.) as f32;
    let event: &web_sys::MouseEvent = event.as_ref();
    let predicted: &web_sys::MouseEvent = predicted.as_ref();
    Some(point(
        position.x + px((predicted.client_x() - event.client_x()) as f32 * scale),
        position.y + px((predicted.client_y() - event.client_y()) as f32 * scale),
    ))
}

fn mouse_position_in_element(event: &web_sys::MouseEvent) -> Point<Pixels> {
    point(px(event.offset_x() as f32), px(event.offset_y() as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_pointer_id_reuse_gets_a_new_touch_id() {
        let mut touch_ids = TouchIds::default();
        let first = touch_ids.start(7).expect("first touch id");
        let concurrent = touch_ids.start(8).expect("concurrent touch id");

        assert_ne!(first, concurrent);
        assert_eq!(touch_ids.active(7), Some(first));
        assert_eq!(touch_ids.end(7), Some(first));
        assert_eq!(touch_ids.active(7), None);

        let reused = touch_ids.start(7).expect("reused pointer touch id");
        assert_ne!(reused, first);
        assert_ne!(reused, concurrent);
    }
}
