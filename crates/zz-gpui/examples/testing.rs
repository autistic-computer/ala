#![cfg_attr(target_family = "wasm", no_main)]

#[path = "example_support/fonts.rs"]
mod example_support;

use zz_gpui::{
    App, Bounds, Context, FocusHandle, Focusable, Render, Task, Window, WindowBounds,
    WindowOptions, actions, div, prelude::*, px, rgb, size,
};
use zz_gpui_platform::application;

actions!(counter, [Increment, Decrement]);

struct Counter {
    count: i32,
    focus_handle: FocusHandle,
    _subscription: zz_gpui::Subscription,
}

struct CounterEvent;

impl zz_gpui::EventEmitter<CounterEvent> for Counter {}

impl Counter {
    fn new(cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe_self(|this: &mut Self, _event: &CounterEvent, _cx| {
            this.count = 999;
        });

        Self {
            count: 0,
            focus_handle: cx.focus_handle(),
            _subscription: subscription,
        }
    }

    fn increment(&mut self, _: &Increment, _window: &mut Window, cx: &mut Context<Self>) {
        self.count += 1;
        cx.notify();
    }

    fn decrement(&mut self, _: &Decrement, _window: &mut Window, cx: &mut Context<Self>) {
        self.count -= 1;
        cx.notify();
    }

    fn load(&self, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            this.update(cx, |counter, _| {
                counter.count = 100;
            })
            .ok();
        })
    }

    fn reload(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            this.update(cx, |counter, _| {
                counter.count += 50;
            })
            .ok();
        })
        .detach();
    }
}

impl Focusable for Counter {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("counter")
            .key_context("Counter")
            .on_action(cx.listener(Self::increment))
            .on_action(cx.listener(Self::decrement))
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x1e1e2e))
            .size_full()
            .justify_center()
            .items_center()
            .child(
                div()
                    .text_3xl()
                    .text_color(rgb(0xcdd6f4))
                    .child(format!("{}", self.count)),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("decrement")
                            .px_4()
                            .py_2()
                            .bg(rgb(0x313244))
                            .hover(|s| s.bg(rgb(0x45475a)))
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(rgb(0xcdd6f4))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.decrement(&Decrement, window, cx)
                            }))
                            .child("−"),
                    )
                    .child(
                        div()
                            .id("increment")
                            .px_4()
                            .py_2()
                            .bg(rgb(0x313244))
                            .hover(|s| s.bg(rgb(0x45475a)))
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(rgb(0xcdd6f4))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.increment(&Increment, window, cx)
                            }))
                            .child("+"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("load")
                            .px_4()
                            .py_2()
                            .bg(rgb(0x313244))
                            .hover(|s| s.bg(rgb(0x45475a)))
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(rgb(0xcdd6f4))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load(cx).detach();
                            }))
                            .child("Load"),
                    )
                    .child(
                        div()
                            .id("reload")
                            .px_4()
                            .py_2()
                            .bg(rgb(0x313244))
                            .hover(|s| s.bg(rgb(0x45475a)))
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(rgb(0xcdd6f4))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.reload(cx);
                            }))
                            .child("Reload"),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x6c7086))
                    .child("Press ↑/↓ or click buttons"),
            )
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        if !example_support::load_fonts(cx) {
            return;
        }
        cx.bind_keys([
            zz_gpui::KeyBinding::new("up", Increment, Some("Counter")),
            zz_gpui::KeyBinding::new("down", Decrement, Some("Counter")),
        ]);

        let bounds = Bounds::centered(None, size(px(300.), px(200.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let counter = cx.new(|cx| Counter::new(cx));
                counter.focus_handle(cx).focus(window, cx);
                counter
            },
        )
        .unwrap();
    });
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run_example();
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    zz_gpui_platform::web_init();
    run_example();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::prelude::*;
    use zz_gpui::{TestAppContext, VisualTestContext};

    #[zz_gpui::test]
    fn basic_testing(cx: &mut TestAppContext) {
        let counter = cx.new(|cx| Counter::new(cx));

        counter.update(cx, |counter, _| {
            counter.count = 42;
        });

        let updated = counter.read_with(cx, |counter, _| counter.count);
        assert_eq!(updated, 42);

        counter.update(cx, |_, cx| {
            cx.emit(CounterEvent);
        });

        let count_after_update = counter.read_with(cx, |counter, _| counter.count);
        assert_eq!(
            count_after_update, 999,
            "Side effects should run after update completes"
        );
    }

    #[zz_gpui::test]
    fn test_counter_in_window(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| cx.new(|cx| Counter::new(cx)))
                .unwrap()
        });

        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let counter = window.root(&mut cx).unwrap();

        let focus_handle = counter.read_with(&cx, |counter, _| counter.focus_handle.clone());
        cx.update(|window, cx| {
            focus_handle.dispatch_action(&Increment, window, cx);
        });

        let count_after = counter.read_with(&cx, |counter, _| counter.count);
        assert_eq!(
            count_after, 1,
            "Action dispatched via focus handle should increment"
        );
    }

    #[zz_gpui::test]
    async fn test_async_operations(cx: &mut TestAppContext) {
        let counter = cx.new(|cx| Counter::new(cx));

        counter.update(cx, |counter, cx| counter.load(cx)).await;

        let count = counter.read_with(cx, |counter, _| counter.count);
        assert_eq!(count, 100, "Load task should have set count to 100");

        counter.update(cx, |counter, cx| counter.reload(cx));

        let count = counter.read_with(cx, |counter, _| counter.count);
        assert_eq!(count, 100, "Detached reload task shouldn't have run yet");

        cx.run_until_parked();

        let count = counter.read_with(cx, |counter, _| counter.count);
        assert_eq!(count, 150, "Reload task should have run after parking");
    }

    #[zz_gpui::test]
    async fn test_allow_parking(cx: &mut TestAppContext) {
        cx.executor().allow_parking();

        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(5));
            tx.send(42).ok();
        });

        let result = rx.await.unwrap();
        assert_eq!(result, 42);
    }

    #[zz_gpui::test(iterations = 10)]
    fn test_counter_random_operations(cx: &mut TestAppContext, mut rng: StdRng) {
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| cx.new(|cx| Counter::new(cx)))
                .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);

        let counter = cx.new(|cx| Counter::new(cx));

        let mut expected = 0i32;
        for _ in 0..100 {
            if rng.random_bool(0.5) {
                expected += 1;
                counter.update_in(&mut cx, |counter, window, cx| {
                    counter.increment(&Increment, window, cx)
                });
            } else {
                expected -= 1;
                counter.update_in(&mut cx, |counter, window, cx| {
                    counter.decrement(&Decrement, window, cx)
                });
            }
        }

        let actual = counter.read_with(&cx, |counter, _| counter.count);
        assert_eq!(
            actual, expected,
            "Counter should match expected after random ops"
        );
    }

    mod distributed_systems {
        use std::sync::{Arc, Mutex};

        struct MockNetworkState {
            ordering: Vec<i32>,
            a_to_b: Vec<i32>,
            b_to_a: Vec<i32>,
        }

        #[derive(Clone)]
        struct MockNetwork {
            state: Arc<Mutex<MockNetworkState>>,
        }

        impl MockNetwork {
            fn new() -> Self {
                Self {
                    state: Arc::new(Mutex::new(MockNetworkState {
                        ordering: Vec::new(),
                        a_to_b: Vec::new(),
                        b_to_a: Vec::new(),
                    })),
                }
            }

            fn a_client(&self) -> NetworkClient {
                NetworkClient {
                    network: self.clone(),
                    is_a: true,
                }
            }

            fn b_client(&self) -> NetworkClient {
                NetworkClient {
                    network: self.clone(),
                    is_a: false,
                }
            }
        }

        #[derive(Clone)]
        struct NetworkClient {
            network: MockNetwork,
            is_a: bool,
        }

        impl NetworkClient {
            fn send(&self, value: i32) {
                let mut network = self.network.state.lock().unwrap();
                network.ordering.push(value);
                if self.is_a {
                    network.b_to_a.push(value);
                } else {
                    network.a_to_b.push(value);
                }
            }

            fn receive_all(&self) -> Vec<i32> {
                let mut network = self.network.state.lock().unwrap();
                if self.is_a {
                    network.a_to_b.drain(..).collect()
                } else {
                    network.b_to_a.drain(..).collect()
                }
            }
        }

        use zz_gpui::Context;

        struct NetworkedCounter {
            count: i32,
            client: NetworkClient,
        }

        impl NetworkedCounter {
            fn new(client: NetworkClient) -> Self {
                Self { count: 0, client }
            }

            fn increment(&mut self, delta: i32, cx: &mut Context<Self>) {
                self.count += delta;

                cx.background_spawn({
                    let client = self.client.clone();
                    async move {
                        client.send(delta);
                    }
                })
                .detach();
            }

            fn sync(&mut self) {
                for delta in self.client.receive_all() {
                    self.count += delta;
                }
            }
        }

        use super::*;

        #[zz_gpui::test]
        fn test_app_sync(cx_a: &mut TestAppContext, cx_b: &mut TestAppContext) {
            let network = MockNetwork::new();

            let a = cx_a.new(|_| NetworkedCounter::new(network.a_client()));
            let b = cx_b.new(|_| NetworkedCounter::new(network.b_client()));

            b.update(cx_b, |b, cx| b.increment(42, cx));
            b.read_with(cx_b, |b, _| assert_eq!(b.count, 42));
            a.read_with(cx_a, |a, _| assert_eq!(a.count, 0));

            cx_b.run_until_parked();
            a.update(cx_a, |a, _| a.sync());

            b.read_with(cx_b, |b, _| assert_eq!(b.count, 42));
            a.read_with(cx_a, |a, _| assert_eq!(a.count, 42));
        }

        #[zz_gpui::test(iterations = 10)]
        fn test_random_interleaving(
            cx_a: &mut TestAppContext,
            cx_b: &mut TestAppContext,
            mut rng: StdRng,
        ) {
            let network = MockNetwork::new();

            let mut original_order = Vec::new();
            let a = cx_a.new(|_| NetworkedCounter::new(MockNetwork::a_client(&network)));
            let b = cx_b.new(|_| NetworkedCounter::new(MockNetwork::b_client(&network)));

            let num_operations: usize = rng.random_range(3..8);

            for i in 0..num_operations {
                let i = i as i32;
                let which = rng.random_bool(0.5);

                original_order.push(i);
                if which {
                    b.update(cx_b, |b, cx| b.increment(i, cx));
                } else {
                    a.update(cx_a, |a, cx| a.increment(i, cx));
                }
            }

            cx_a.run_until_parked();

            a.update(cx_a, |a, _| a.sync());
            b.update(cx_b, |b, _| b.sync());

            let a_count = a.read_with(cx_a, |a, _| a.count);
            let b_count = b.read_with(cx_b, |b, _| b.count);

            assert_eq!(a_count, b_count, "A and B should have the same count");

            let actual = network.state.lock().unwrap().ordering.clone();
            let spawned: Vec<_> = original_order.iter().map(|n| format!("{}", n)).collect();
            let ran: Vec<_> = actual.iter().map(|n| format!("{}", n)).collect();
            let diff: Vec<_> = original_order
                .iter()
                .zip(actual.iter())
                .map(|(o, a)| {
                    if o == a {
                        " ".to_string()
                    } else {
                        "^".to_string()
                    }
                })
                .collect();
            println!("spawned: [{}]", spawned.join(", "));
            println!("ran:     [{}]", ran.join(", "));
            println!("         [{}]", diff.join(", "));
        }
    }
}
