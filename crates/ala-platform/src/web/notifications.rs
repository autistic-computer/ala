use std::{cell::RefCell, collections::HashMap, rc::Rc};

use ala::{ForegroundExecutor, SharedString, SystemNotification, SystemNotificationResponse, Task};
use futures::{StreamExt as _, channel::mpsc};
use wasm_bindgen::{JsCast as _, prelude::*};
use web_sys::{Notification, NotificationOptions, NotificationPermission};

type ResponseCallback = Rc<RefCell<Option<Box<dyn FnMut(SystemNotificationResponse)>>>>;

const PERMISSION_GESTURES: [&str; 3] = ["pointerdown", "pointerup", "keydown"];

pub(crate) struct WebNotifications {
    browser_window: web_sys::Window,
    shown: HashMap<SharedString, Shown>,
    sender: Option<mpsc::UnboundedSender<SharedString>>,
    callback: ResponseCallback,
    permission_listeners: Vec<Closure<dyn FnMut(web_sys::Event)>>,
    _response_task: Option<Task<()>>,
}

struct Shown {
    notification: Notification,
    _on_click: Closure<dyn FnMut()>,
}

impl WebNotifications {
    pub(crate) fn new(browser_window: web_sys::Window) -> Self {
        Self {
            browser_window,
            shown: HashMap::new(),
            sender: None,
            callback: Rc::new(RefCell::new(None)),
            permission_listeners: Vec::new(),
            _response_task: None,
        }
    }

    fn supported(&self) -> bool {
        js_sys::Reflect::has(&self.browser_window, &"Notification".into()).unwrap_or(false)
    }

    pub(crate) fn show(&mut self, executor: &ForegroundExecutor, notification: SystemNotification) {
        if !self.supported() {
            return;
        }
        match Notification::permission() {
            NotificationPermission::Granted => {}
            NotificationPermission::Default => {
                self.ask_on_next_gesture();
                return;
            }
            _ => return,
        }
        let sender = self.sender(executor);
        let options = NotificationOptions::new();
        options.set_body(&notification.body);
        options.set_tag(&notification.tag);
        let Ok(shown) = Notification::new_with_options(&notification.title, &options) else {
            return;
        };
        let browser_window = self.browser_window.clone();
        let tag = notification.tag.clone();
        let clicked = shown.clone();
        let on_click = Closure::<dyn FnMut()>::new(move || {
            browser_window.focus().ok();
            clicked.close();
            sender.unbounded_send(tag.clone()).ok();
        });
        shown.set_onclick(Some(on_click.as_ref().unchecked_ref()));
        if let Some(previous) = self.shown.insert(
            notification.tag,
            Shown {
                notification: shown,
                _on_click: on_click,
            },
        ) {
            previous.notification.set_onclick(None);
        }
    }

    pub(crate) fn dismiss(&mut self, tag: &str) {
        if let Some(shown) = self.shown.remove(tag) {
            shown.notification.set_onclick(None);
            shown.notification.close();
        }
    }

    pub(crate) fn on_response(
        &mut self,
        executor: &ForegroundExecutor,
        callback: Box<dyn FnMut(SystemNotificationResponse)>,
    ) {
        self.sender(executor);
        *self.callback.borrow_mut() = Some(callback);
    }

    fn sender(&mut self, executor: &ForegroundExecutor) -> mpsc::UnboundedSender<SharedString> {
        if let Some(sender) = &self.sender {
            return sender.clone();
        }
        let (sender, mut receiver) = mpsc::unbounded::<SharedString>();
        let callback = self.callback.clone();
        self._response_task = Some(executor.spawn(async move {
            while let Some(tag) = receiver.next().await {
                let taken = callback.borrow_mut().take();
                if let Some(mut taken) = taken {
                    taken(SystemNotificationResponse {
                        tag,
                        action_id: None,
                    });
                    callback.borrow_mut().get_or_insert(taken);
                }
            }
        }));
        self.sender = Some(sender.clone());
        sender
    }

    fn ask_on_next_gesture(&mut self) {
        if !self.permission_listeners.is_empty() {
            return;
        }
        let target: &web_sys::EventTarget = self.browser_window.as_ref();
        for name in PERMISSION_GESTURES {
            let listener = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
                if !activates(&event)
                    || Notification::permission() != NotificationPermission::Default
                {
                    return;
                }
                Notification::request_permission().ok();
            });
            target
                .add_event_listener_with_callback_and_bool(
                    name,
                    listener.as_ref().unchecked_ref(),
                    true,
                )
                .ok();
            self.permission_listeners.push(listener);
        }
    }
}

fn activates(event: &web_sys::Event) -> bool {
    let Some(pointer) = event.dyn_ref::<web_sys::PointerEvent>() else {
        return event.type_() == "keydown";
    };
    let mouse = pointer.pointer_type() == "mouse";
    match event.type_().as_str() {
        "pointerdown" => mouse,
        "pointerup" => !mouse,
        _ => false,
    }
}
