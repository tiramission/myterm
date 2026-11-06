//! myterm — M0 spike: a ConPTY shell rendered through libghostty-vt into an
//! NSH canvas, typed into through a hidden input plus key bindings.

mod grid;
mod keys;
mod session;
mod term;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nsh_core::keymap::When;
use nsh_core::prelude::*;

const FONT_SIZE: f32 = 13.0;
const START_COLS: u16 = 100;
const START_ROWS: u16 = 28;

/// The app's UI set (`nsh.json` `"rust"`).
#[native_module("myterm/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

#[component]
pub fn App() -> impl IntoElement {
    let metrics = Rc::new(grid::Metrics::new(FONT_SIZE));

    let session = match session::Session::spawn(
        START_COLS,
        START_ROWS,
        metrics.cell_w as u32,
        metrics.cell_h as u32,
    ) {
        Ok(session) => Some(Rc::new(session)),
        Err(error) => {
            log::error!("session: {error}");
            None
        }
    };

    let frame_slot: Rc<RefCell<Option<Arc<term::Frame>>>> = Rc::new(RefCell::new(None));
    let draw_ms: Rc<Cell<f32>> = Rc::new(Cell::new(0.0_f32));
    let last_grid: Rc<Cell<(u16, u16)>> = Rc::new(Cell::new((START_COLS, START_ROWS)));
    let epoch = signal(0_u64);
    let status = signal("starting…".to_owned());
    let input_buffer = signal(String::new());

    if let Some(current) = session.clone() {
        let slot = frame_slot.clone();
        let draw_ms = draw_ms.clone();
        let profile = current.profile.clone();
        let poll = current.clone();
        set_interval(Duration::from_millis(8), move || {
            let Some(frame) = poll.take_frame() else {
                return;
            };
            let cols = frame.cols;
            let rows = frame.rows;
            *slot.borrow_mut() = Some(frame);
            epoch.update(|value| *value += 1);
            let title = poll.title().unwrap_or_default();
            status.set(format!(
                "{profile} · {cols}×{rows} · vt {:.2} ms · draw {:.2} ms · {title}",
                poll.frame_ms(),
                draw_ms.get()
            ));
        });

        // Keys that are not text: encoded by libghostty for the terminal's
        // modes, and consumed before the hidden input sees them.
        for keystroke in keys::TERMINAL_KEYS {
            let Some((key, mods)) = keys::parse(keystroke) else {
                continue;
            };
            let current = current.clone();
            let clears_buffer = keystroke == &"enter";
            on_key_with(
                keystroke,
                KeyOptions {
                    when: When::Always,
                    repeat: true,
                },
                move |_| {
                    current.send_key(key, mods);
                    if clears_buffer {
                        input_buffer.set(String::new());
                    }
                },
            );
        }
    }

    let input_ref = element_ref();

    let canvas_slot = frame_slot.clone();
    let canvas_draw_ms = draw_ms.clone();
    let canvas_metrics = metrics.clone();
    let layout_metrics = metrics.clone();
    let canvas_session = session.clone();
    let canvas_grid = last_grid.clone();
    let input_session = session.clone();

    view()
        .flex_1()
        .bg(rgb(0x101216))
        // The status strip. Keep it above the canvas: on Windows the canvas
        // clears its region and would hide anything drawn at the bottom.
        .child(
            view()
                .h(px(22.0))
                .w_full()
                .flex_row()
                .items_center()
                .px_3()
                .bg(rgb(0x15181d))
                .child(
                    text(move || status.get())
                        .text_size(px(11.0))
                        .text_color(rgb(0x8b929e)),
                ),
        )
        .child(
            canvas({
                let canvas_metrics = canvas_metrics.clone();
                move || {
                    let _ = epoch.get();
                    let started = Instant::now();
                    let painter = match &*canvas_slot.borrow() {
                        Some(frame) => grid::draw(frame, &canvas_metrics),
                        None => Painter::new(),
                    };
                    canvas_draw_ms.set(started.elapsed().as_secs_f32() * 1000.0);
                    painter
                }
            })
            .flex_1()
            .min_h(px(0.0))
            .on_layout(move |bounds: &Bounds<Points>| {
                let Some(current) = canvas_session.as_ref() else {
                    return;
                };
                let cols = layout_metrics.cols(bounds.size.width.0);
                let rows = layout_metrics.rows(bounds.size.height.0);
                if cols == 0 || rows == 0 {
                    return;
                }
                if canvas_grid.get() != (cols, rows) {
                    canvas_grid.set((cols, rows));
                    current.resize(
                        cols,
                        rows,
                        layout_metrics.cell_w as u32,
                        layout_metrics.cell_h as u32,
                    );
                }
            })
            .on_press(move || input_ref.focus()),
        )
        .child(
            nsh_base::input()
                .value(move || input_buffer.get())
                .caret(false)
                .auto_focus(true)
                .on_change(move |text: &str| {
                    let previous = input_buffer.get();
                    if let Some(delta) = text.strip_prefix(previous.as_str()) {
                        if !delta.is_empty() {
                            if let Some(current) = input_session.as_ref() {
                                current.send_text(delta);
                            }
                        }
                    }
                    input_buffer.set(text.to_owned());
                })
                .on_composition_change(move |composition: &Composition| {
                    log::info!(
                        "ime composing={} text={:?}",
                        composition.composing,
                        composition.text
                    );
                })
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .size(px(1.0))
                .opacity(0.0),
        )
}
