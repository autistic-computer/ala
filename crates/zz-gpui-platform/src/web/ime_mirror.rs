use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use zz_gpui::{Autocapitalize, TextInputAction, TextInputConfiguration, TextInputMode};

use crate::web::window::WebWindowInner;

const CONTEXT_CHARS: usize = 512;

const MIN_EDGE_CHARS: usize = 64;

pub(crate) struct ImeMirror {
    element: web_sys::HtmlTextAreaElement,
    text: RefCell<String>,
    selection: Cell<(u32, u32)>,
    window_hint: Cell<usize>,
    sync_scheduled: Cell<bool>,
    selection_import_rejected: Cell<bool>,
}

impl ImeMirror {
    pub(crate) fn new(
        document: &web_sys::Document,
        body: &web_sys::HtmlElement,
        touch_input: bool,
        focus: bool,
    ) -> anyhow::Result<Self> {
        let element: web_sys::HtmlTextAreaElement = document
            .create_element("textarea")
            .map_err(|e| anyhow::anyhow!("Failed to create textarea element: {e:?}"))?
            .dyn_into()
            .map_err(|e| anyhow::anyhow!("Created element is not a textarea: {e:?}"))?;
        let style = element.style();
        style.set_property("position", "fixed").ok();
        style.set_property("top", "0").ok();
        style.set_property("left", "0").ok();
        style.set_property("width", "1px").ok();
        style.set_property("height", "1px").ok();
        style.set_property("opacity", "0").ok();
        style.set_property("font-size", "16px").ok();
        body.append_child(&element)
            .map_err(|e| anyhow::anyhow!("Failed to append input to body: {e:?}"))?;
        if focus {
            Self::focus_element(&element);
        }
        if touch_input {
            element
                .set_attribute("inputmode", "none")
                .map_err(|error| anyhow::anyhow!("Failed to configure input mode: {error:?}"))?;
        }

        let this = Self {
            element,
            text: RefCell::new(String::new()),
            selection: Cell::new((0, 0)),
            window_hint: Cell::new(0),
            sync_scheduled: Cell::new(false),
            selection_import_rejected: Cell::new(false),
        };
        this.apply_configuration(&TextInputConfiguration::default());
        Ok(this)
    }

    pub(crate) fn apply_configuration(&self, configuration: &TextInputConfiguration) {
        let element: &web_sys::Element = self.element.as_ref();
        self.element.set_spellcheck(configuration.suggestions);
        let on_off = |enabled: bool| if enabled { "on" } else { "off" };
        element
            .set_attribute("autocomplete", on_off(configuration.suggestions))
            .ok();
        element
            .set_attribute("autocorrect", on_off(configuration.autocorrect))
            .ok();
        element
            .set_attribute(
                "autocapitalize",
                match configuration.autocapitalize {
                    Autocapitalize::None => "off",
                    Autocapitalize::Words => "words",
                    Autocapitalize::Sentences => "sentences",
                    Autocapitalize::Characters => "characters",
                },
            )
            .ok();
        let enter_key_hint = match configuration.input_action {
            TextInputAction::Unspecified => None,
            TextInputAction::Enter => Some("enter"),
            TextInputAction::Done => Some("done"),
            TextInputAction::Go => Some("go"),
            TextInputAction::Next => Some("next"),
            TextInputAction::Previous => Some("previous"),
            TextInputAction::Search => Some("search"),
            TextInputAction::Send => Some("send"),
        };
        match enter_key_hint {
            Some(hint) => element.set_attribute("enterkeyhint", hint).ok(),
            None => element.remove_attribute("enterkeyhint").ok(),
        };
        let input_mode = match configuration.input_mode {
            TextInputMode::Text => None,
            TextInputMode::None => Some("none"),
            TextInputMode::Decimal => Some("decimal"),
            TextInputMode::Numeric => Some("numeric"),
            TextInputMode::Tel => Some("tel"),
            TextInputMode::Search => Some("search"),
            TextInputMode::Email => Some("email"),
            TextInputMode::Url => Some("url"),
        };
        match input_mode {
            Some(mode) => element.set_attribute("inputmode", mode).ok(),
            None => element.remove_attribute("inputmode").ok(),
        };
    }

    pub(crate) fn event_target(&self) -> &web_sys::EventTarget {
        self.element.as_ref()
    }

    pub(crate) fn focus(&self) {
        Self::focus_element(&self.element);
    }

    fn focus_element(element: &web_sys::HtmlTextAreaElement) {
        let options = web_sys::FocusOptions::new();
        options.set_prevent_scroll(true);
        if let Err(error) = element.focus_with_options(&options) {
            log::warn!("Failed to focus IME mirror: {error:?}");
        }
    }

    pub(crate) fn is_focused(&self) -> bool {
        let element: &web_sys::Element = self.element.as_ref();
        web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element())
            .is_some_and(|active| &active == element)
    }

    pub(crate) fn blur(&self) {
        self.element.blur().ok();
    }

    pub(crate) fn virtual_keyboard_enabled(&self) -> bool {
        self.element.get_attribute("inputmode").as_deref() != Some("none")
    }

    pub(crate) fn set_read_only(&self, read_only: bool) {
        self.element.set_read_only(read_only);
    }

    pub(crate) fn set_virtual_keyboard_enabled(&self, enabled: bool) {
        if let Err(error) = self
            .element
            .set_attribute("inputmode", if enabled { "text" } else { "none" })
        {
            log::warn!("Failed to configure input mode: {error:?}");
        }
    }

    pub(crate) fn remove(&self) {
        let element: &web_sys::Element = self.element.as_ref();
        element.remove();
    }

    pub(crate) fn value(&self) -> String {
        self.element.value()
    }

    pub(crate) fn selection_start(&self) -> Option<u32> {
        self.element.selection_start().ok().flatten()
    }

    pub(crate) fn element_selection_end(&self) -> Option<u32> {
        self.element.selection_end().ok().flatten()
    }

    pub(crate) fn stored_text(&self) -> String {
        self.text.borrow().clone()
    }

    pub(crate) fn stored_selection(&self) -> (u32, u32) {
        self.selection.get()
    }

    pub(crate) fn adopt_element_state(&self) {
        *self.text.borrow_mut() = self.element.value();
        let selection_start = self.selection_start().unwrap_or(0);
        let selection_end = self
            .element
            .selection_end()
            .ok()
            .flatten()
            .unwrap_or(selection_start);
        self.selection.set((selection_start, selection_end));
    }

    pub(crate) fn reject_selection_import(&self) {
        self.selection_import_rejected.set(true);
    }

    pub(crate) fn schedule_sync(window: &Rc<WebWindowInner>) {
        if window.ime_mirror.sync_scheduled.replace(true) {
            return;
        }
        let closure = wasm_bindgen::closure::Closure::once_into_js({
            let window = Rc::clone(window);
            move || {
                window.ime_mirror.sync_scheduled.set(false);
                sync(&window);
            }
        });
        window
            .browser_window
            .set_timeout_with_callback(closure.unchecked_ref())
            .ok();

        fn sync(window: &WebWindowInner) {
            if window.is_composing.get() {
                return;
            }
            let mirror = &window.ime_mirror;
            if !mirror.selection_import_rejected.replace(false)
                && *mirror.text.borrow() == mirror.element.value()
            {
                let live_start = mirror.selection_start().unwrap_or(0);
                let live_end = mirror.element_selection_end().unwrap_or(live_start);
                if (live_start, live_end) != mirror.selection.get() {
                    return;
                }
            }
            let selection = window
                .with_input_handler(|handler| handler.selected_text_range(false))
                .flatten();
            let Some(selection) = selection else {
                if !mirror.text.borrow().is_empty() {
                    mirror.element.set_value("");
                    mirror.text.borrow_mut().clear();
                }
                mirror.selection.set((0, 0));
                return;
            };
            let editable_range = window
                .with_input_handler(|handler| handler.text_input_editable_range())
                .flatten();

            if is_consistent(
                window,
                &selection.range,
                editable_range.as_ref(),
                MIN_EDGE_CHARS,
            ) {
                return;
            }

            if move_selection_within_window(
                window,
                &selection.range,
                editable_range.as_ref(),
                MIN_EDGE_CHARS,
            ) {
                return;
            }

            let mut window_range = selection.range.start.saturating_sub(CONTEXT_CHARS)
                ..selection.range.end + CONTEXT_CHARS;
            if let Some(editable_range) = &editable_range {
                window_range.start = window_range.start.max(editable_range.start);
                window_range.end = window_range
                    .end
                    .min(editable_range.end)
                    .max(window_range.start);
            }
            let mut adjusted = None;
            let text = window
                .with_input_handler(|handler| {
                    handler.text_for_range(window_range.clone(), &mut adjusted)
                })
                .flatten()
                .unwrap_or_default();
            let window_start = adjusted.unwrap_or(window_range).start;

            if *mirror.text.borrow() != text || mirror.element.value() != text {
                mirror.element.set_value(&text);
                *mirror.text.borrow_mut() = text;
            }

            mirror.window_hint.set(window_start);
            let selection_start = selection.range.start.saturating_sub(window_start) as u32;
            let selection_end = selection.range.end.saturating_sub(window_start) as u32;
            if mirror.element.selection_start().ok().flatten() != Some(selection_start)
                || mirror.element.selection_end().ok().flatten() != Some(selection_end)
            {
                mirror
                    .element
                    .set_selection_range(selection_start, selection_end)
                    .ok();
            }
            let actual_start = mirror.element.selection_start().ok().flatten();
            let actual_end = mirror.element.selection_end().ok().flatten();
            mirror.selection.set((
                actual_start.unwrap_or(selection_start),
                actual_end.unwrap_or(selection_end),
            ));
        }
    }
}

fn move_selection_within_window(
    window: &WebWindowInner,
    app_selection: &std::ops::Range<usize>,
    editable_range: Option<&std::ops::Range<usize>>,
    min_edge: usize,
) -> bool {
    let mirror = &window.ime_mirror;
    let stored_text = mirror.text.borrow().clone();
    let stored_length = stored_text.encode_utf16().count();
    if stored_length == 0 || mirror.element.value() != stored_text {
        return false;
    }
    let window_start = mirror.window_hint.get();

    if let Some(range) = editable_range
        && (window_start < range.start || window_start + stored_length > range.end)
    {
        return false;
    }
    let left_boundary = editable_range.map_or(0, |range| range.start);
    let Some(selection_start) = app_selection.start.checked_sub(window_start) else {
        return false;
    };
    let selection_end = selection_start + (app_selection.end - app_selection.start);
    if selection_end > stored_length {
        return false;
    }
    if selection_start < min_edge && window_start > left_boundary {
        return false;
    }

    let mut adjusted = None;
    let document_text = window
        .with_input_handler(|handler| {
            handler.text_for_range(
                window_start..window_start + stored_length + 1,
                &mut adjusted,
            )
        })
        .flatten()
        .unwrap_or_default();
    let document_text_length = document_text.encode_utf16().count();
    let window_at_right_boundary = document_text_length == stored_length
        || editable_range.is_some_and(|range| window_start + stored_length >= range.end);
    if selection_end + min_edge > stored_length && !window_at_right_boundary {
        return false;
    }
    if !document_text.starts_with(stored_text.as_str()) || document_text_length > stored_length + 1
    {
        return false;
    }

    mirror
        .element
        .set_selection_range(selection_start as u32, selection_end as u32)
        .ok();
    let actual_start = mirror.element.selection_start().ok().flatten();
    let actual_end = mirror.element.selection_end().ok().flatten();
    if actual_start != Some(selection_start as u32) || actual_end != Some(selection_end as u32) {
        return false;
    }
    mirror
        .selection
        .set((selection_start as u32, selection_end as u32));
    true
}

fn is_consistent(
    window: &WebWindowInner,
    app_selection: &std::ops::Range<usize>,
    editable_range: Option<&std::ops::Range<usize>>,
    min_edge: usize,
) -> bool {
    let mirror = &window.ime_mirror;
    let (element_selection_start, element_selection_end) = mirror.selection.get();
    let element_selection_start = element_selection_start as usize;
    let element_selection_end = element_selection_end as usize;
    let stored_text = mirror.text.borrow().clone();
    let stored_length = stored_text.encode_utf16().count();

    if stored_length == 0 {
        return false;
    }
    if mirror.element.selection_start().ok().flatten() != Some(element_selection_start as u32)
        || mirror.element.selection_end().ok().flatten() != Some(element_selection_end as u32)
    {
        return false;
    }
    let app_window_start = match app_selection.start.checked_sub(element_selection_start) {
        Some(start) => start,
        None => return false,
    };
    if let Some(range) = editable_range
        && (app_window_start < range.start || app_window_start + stored_length > range.end)
    {
        return false;
    }
    let left_boundary = editable_range.map_or(0, |range| range.start);
    let has_left_context = element_selection_start >= min_edge || app_window_start <= left_boundary;
    let right_context = stored_length.saturating_sub(element_selection_end);
    if !has_left_context || right_context < min_edge {
        let window_end = app_window_start + stored_length;
        let at_right_boundary = if let Some(range) = editable_range {
            window_end >= range.end
        } else {
            let mut adjusted = None;
            window
                .with_input_handler(|handler| {
                    handler.text_for_range(window_end..window_end + 1, &mut adjusted)
                })
                .flatten()
                .unwrap_or_default()
                .is_empty()
        };
        if !has_left_context || !at_right_boundary {
            return false;
        }
    }
    let mut adjusted = None;
    let document_text = window
        .with_input_handler(|handler| {
            handler.text_for_range(
                app_window_start..app_window_start + stored_length,
                &mut adjusted,
            )
        })
        .flatten()
        .unwrap_or_default();
    if document_text != stored_text {
        return false;
    }
    if mirror.element.value() != stored_text {
        return false;
    }
    app_selection.end.checked_sub(app_window_start) == Some(element_selection_end)
}
