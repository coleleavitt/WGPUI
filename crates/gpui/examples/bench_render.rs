#![cfg_attr(target_family = "wasm", no_main)]

use std::time::{Duration, Instant};

use gpui::*;
use gpui_platform::application;

const ROW_COUNT: usize = 50;
const COLUMN_COUNT: usize = 20;
const ELEMENT_COUNT: usize = ROW_COUNT * COLUMN_COUNT;
const FRAME_LOG_INTERVAL: u64 = 60;
const WINDOW_WIDTH: Pixels = px(1200.0);
const WINDOW_HEIGHT: Pixels = px(800.0);
const CELL_WIDTH: Pixels = px(60.0);
const CELL_HEIGHT: Pixels = px(40.0);
const LIST_ITEM_COUNT: usize = 10_000;
const LIST_ITEM_HEIGHT: Pixels = px(30.0);
const ANIMATED_ELEMENT_COUNT: usize = 200;

#[derive(Clone, Copy, Eq, PartialEq)]
enum BenchScenario {
    Grid,
    List,
    Animated,
}

impl BenchScenario {
    fn name(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::List => "list",
            Self::Animated => "animated",
        }
    }

    fn element_count(self) -> usize {
        match self {
            Self::Grid => ELEMENT_COUNT,
            Self::List => LIST_ITEM_COUNT,
            Self::Animated => ANIMATED_ELEMENT_COUNT,
        }
    }
}

struct BenchApp {
    scenario: BenchScenario,
    frame_count: u64,
    last_frame_at: Instant,
    _redraw_task: Task<()>,
}

impl BenchApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let redraw_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;

                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
        });

        Self {
            scenario: BenchScenario::Grid,
            frame_count: 0,
            last_frame_at: Instant::now(),
            _redraw_task: redraw_task,
        }
    }

    fn set_scenario(&mut self, scenario: BenchScenario, cx: &mut Context<Self>) {
        if self.scenario != scenario {
            self.scenario = scenario;
            self.frame_count = 0;
            self.last_frame_at = Instant::now();
            cx.notify();
        }
    }

    fn cell_color(row: usize, column: usize) -> Rgba {
        let red = ((row * 5 + column * 11) % 256) as u32;
        let green = ((row * 13 + column * 7) % 256) as u32;
        let blue = ((row * 3 + column * 17) % 256) as u32;

        rgb((red << 16) | (green << 8) | blue)
    }

    fn render_grid(&self) -> AnyElement {
        let mut grid = div()
            .flex()
            .flex_wrap()
            .w(WINDOW_WIDTH)
            .h(WINDOW_HEIGHT)
            .bg(rgb(0x111111));

        for row in 0..ROW_COUNT {
            for column in 0..COLUMN_COUNT {
                let label = SharedString::from(format!("{}:{}", row + 1, column + 1));
                let cell = div()
                    .w(CELL_WIDTH)
                    .h(CELL_HEIGHT)
                    .bg(Self::cell_color(row, column))
                    .border_1()
                    .border_color(rgb(0x202020))
                    .p(px(4.0))
                    .text_size(px(10.0))
                    .text_color(rgb(0xffffff))
                    .child(label);

                grid = grid.child(cell);
            }
        }

        grid.into_any_element()
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        uniform_list(
            "bench-list",
            LIST_ITEM_COUNT,
            cx.processor(|_this, range: std::ops::Range<usize>, _window, _cx| {
                range
                    .map(|index| {
                        let shade = ((index * 17) % 64) as u32;
                        div()
                            .id(index)
                            .w_full()
                            .h(LIST_ITEM_HEIGHT)
                            .bg(rgb(0x202020 + shade * 0x010101))
                            .border_b_1()
                            .border_color(rgb(0x303030))
                            .px(px(8.0))
                            .flex()
                            .items_center()
                            .text_size(px(12.0))
                            .text_color(rgb(0xffffff))
                            .child(SharedString::from(format!("Item {index}")))
                    })
                    .collect()
            }),
        )
        .size_full()
        .into_any_element()
    }

    fn render_animated(&self) -> AnyElement {
        let mut grid = div()
            .flex()
            .flex_wrap()
            .w(WINDOW_WIDTH)
            .h(WINDOW_HEIGHT)
            .bg(rgb(0x111111));

        for index in 0..ANIMATED_ELEMENT_COUNT {
            let value = (((self.frame_count + index as u64) * 7) % 256) as u32;
            let cell = div()
                .w(CELL_WIDTH)
                .h(CELL_HEIGHT)
                .bg(rgb(value * 0x010101))
                .border_1()
                .border_color(rgb(0x202020))
                .p(px(4.0))
                .text_size(px(10.0))
                .text_color(rgb(0xffffff))
                .child(SharedString::from(format!("{index}")));

            grid = grid.child(cell);
        }

        grid.into_any_element()
    }
}

impl Render for BenchApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame_count += 1;
        let now = Instant::now();
        let total_frame_time = now.duration_since(self.last_frame_at);
        self.last_frame_at = now;

        if self.frame_count % FRAME_LOG_INTERVAL == 0 {
            println!(
                "scenario {} frame {}: total frame time {:?}; elements {}",
                self.scenario.name(),
                self.frame_count,
                total_frame_time,
                self.scenario.element_count()
            );
        }

        let content = match self.scenario {
            BenchScenario::Grid => self.render_grid(),
            BenchScenario::List => self.render_list(cx),
            BenchScenario::Animated => self.render_animated(),
        };

        div().size_full().bg(rgb(0x080808)).child(content)
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(WINDOW_WIDTH, WINDOW_HEIGHT));
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: true,
                ..Default::default()
            },
            |_, cx| cx.new(BenchApp::new),
        );

        match window {
            Ok(window) => match window.update(cx, |_, _, cx| cx.entity()) {
                Ok(view) => {
                    cx.observe_keystrokes(move |event, _window, cx| {
                        let scenario = match event.keystroke.key.as_str() {
                            "1" => Some(BenchScenario::Grid),
                            "2" => Some(BenchScenario::List),
                            "3" => Some(BenchScenario::Animated),
                            _ => None,
                        };

                        if let Some(scenario) = scenario {
                            cx.stop_propagation();
                            view.update(cx, |view, cx| {
                                view.set_scenario(scenario, cx);
                            });
                        }
                    })
                    .detach();
                }
                Err(err) => eprintln!("failed to initialize bench_render keyboard shortcuts: {err}"),
            },
            Err(err) => eprintln!("failed to open bench_render window: {err}"),
        }
        cx.activate(true);
    });
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run_example();
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    run_example();
}
