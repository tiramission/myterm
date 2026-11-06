//! myterm — M1: tabs, profiles, selection and copy/paste, exit states.

mod grid;
mod keys;
pub mod profiles;
mod session;
mod term;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use nsh_base::{menu, menu_item};
use nsh_core::element::TouchEvent;
use nsh_core::keymap::When;
use nsh_core::prelude::*;
use nsh_core::TouchPhase;

const FONT_SIZE: f32 = 13.0;
const START_COLS: u16 = 100;
const START_ROWS: u16 = 28;
const TAB_HEIGHT: f32 = 34.0;

#[derive(Clone, PartialEq, Debug)]
enum PaletteAction {
    NewTab,
    NewTabProfile(usize),
    Find,
    Copy,
    Paste,
    SelectAll,
    CloseTab,
    ScrollTop,
    ScrollBottom,
}

#[derive(Clone, PartialEq)]
struct PaletteItem {
    title: String,
    hint: String,
    keywords: &'static str,
    action: PaletteAction,
}

#[derive(Clone, PartialEq)]
struct TabInfo {
    id: u64,
    title: String,
    exited: Option<i32>,
}

struct Tab {
    id: u64,
    profile: profiles::Profile,
    session: Option<Rc<session::Session>>,
    error: Option<String>,
    frame: RefCell<Option<Arc<term::Frame>>>,
    epoch: Signal<u64>,
    grid: Cell<(u16, u16)>,
    dims: Signal<(u16, u16)>,
    exit: Signal<Option<i32>>,
    title: Signal<String>,
    find_open: Signal<bool>,
    find_query: Signal<String>,
    find_skip: Cell<usize>,
    find_count: Signal<Option<(usize, usize)>>,
    find_focus: ElementRef,
    menu_open: Signal<bool>,
    menu_at: Signal<(f32, f32)>,
}

impl Tab {
    fn session(&self) -> Option<Rc<session::Session>> {
        self.session.clone()
    }
}

struct AppState {
    tabs: Signal<Vec<TabInfo>>,
    active: Signal<u64>,
    runtimes: RefCell<Vec<Rc<Tab>>>,
    next_id: Cell<u64>,
    profiles: Vec<profiles::Profile>,
    focus: ElementRef,
    anchor: Cell<Option<(u16, u32)>>,
    dragging: Cell<bool>,
    palette_open: Signal<bool>,
    palette_query: Signal<String>,
    palette_items: Signal<Vec<PaletteItem>>,
    palette_all: Vec<PaletteItem>,
    palette_focus: ElementRef,
}

impl AppState {
    fn tab(&self, id: u64) -> Option<Rc<Tab>> {
        self.runtimes
            .borrow()
            .iter()
            .find(|tab| tab.id == id)
            .cloned()
    }

    fn active_tab(&self) -> Option<Rc<Tab>> {
        self.tab(self.active.get())
    }

    fn activate(&self, id: u64) {
        if self.active.get() != id {
            log::info!("activate tab {id}");
            self.active.set(id);
            let focus = self.focus;
            set_timeout(Duration::from_millis(0), move || focus.focus());
        }
    }

    fn close(&self, id: u64) {
        log::info!("close tab {id}");
        let index = self
            .runtimes
            .borrow()
            .iter()
            .position(|tab| tab.id == id);
        let Some(index) = index else {
            return;
        };
        self.runtimes.borrow_mut().remove(index);
        let remaining: Vec<TabInfo> = self
            .tabs
            .get()
            .into_iter()
            .filter(|info| info.id != id)
            .collect();
        let was_active = self.active.get() == id;
        self.tabs.set(remaining.clone());
        if was_active {
            let next = remaining
                .get(index.saturating_sub(1))
                .or_else(|| remaining.first());
            match next {
                Some(info) => {
                    self.active.set(info.id);
                    let focus = self.focus;
                    set_timeout(Duration::from_millis(0), move || focus.focus());
                }
                None => self.active.set(0),
            }
        }
    }

    fn activate_offset(&self, offset: isize) {
        let tabs = self.tabs.get();
        if tabs.is_empty() {
            return;
        }
        let current = tabs
            .iter()
            .position(|info| info.id == self.active.get())
            .unwrap_or(0) as isize;
        let len = tabs.len() as isize;
        let next = ((current + offset) % len + len) % len;
        self.activate(tabs[next as usize].id);
    }
}

fn spawn_tab(state: &Rc<AppState>, profile: Option<usize>) -> Option<u64> {
    let profile = profile
        .and_then(|index| state.profiles.get(index).cloned())
        .or_else(|| state.profiles.first().cloned())?;
    let id = state.next_id.get() + 1;
    state.next_id.set(id);

    let metrics = grid::Metrics::new(FONT_SIZE);
    let spawned = session::Session::spawn(
        &profile,
        START_COLS,
        START_ROWS,
        metrics.cell_w as u32,
        metrics.cell_h as u32,
    );
    let (session, error) = match spawned {
        Ok(session) => (Some(Rc::new(session)), None),
        Err(error) => (None, Some(error)),
    };
    log::info!("tab {id}: {}{}", profile.name, if error.is_some() { " (failed)" } else { "" });

    let tab = Rc::new(Tab {
        id,
        profile: profile.clone(),
        session,
        error: error.clone(),
        frame: RefCell::new(None),
        epoch: signal(0_u64),
        grid: Cell::new((START_COLS, START_ROWS)),
        dims: signal((START_COLS, START_ROWS)),
        exit: signal(None::<i32>),
        title: signal(profile.name.clone()),
        find_open: signal(false),
        find_query: signal(String::new()),
        find_skip: Cell::new(0),
        find_count: signal(None),
        find_focus: element_ref(),
        menu_open: signal(false),
        menu_at: signal((0.0_f32, 0.0_f32)),
    });
    state.runtimes.borrow_mut().push(tab);

    let title = if error.is_some() {
        format!("{} · 启动失败", profile.name)
    } else {
        profile.name
    };
    let mut tabs = state.tabs.get();
    tabs.push(TabInfo {
        id,
        title,
        exited: None,
    });
    state.tabs.set(tabs);
    state.active.set(id);
    let focus = state.focus;
    set_timeout(Duration::from_millis(0), move || focus.focus());
    Some(id)
}

/// The app's UI set (`nsh.json` `"rust"`).
#[native_module("myterm/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

#[component]
pub fn App() -> impl IntoElement {
    let profile_list = profiles::discover();
    let mut palette_all: Vec<PaletteItem> = vec![
        PaletteItem {
            title: "新建标签".to_owned(),
            hint: "Ctrl+Shift+T".to_owned(),
            keywords: "new tab",
            action: PaletteAction::NewTab,
        },
    ];
    for (index, profile) in profile_list.iter().enumerate() {
        palette_all.push(PaletteItem {
            title: format!("新建 {} 标签", profile.name),
            hint: String::new(),
            keywords: "new tab profile",
            action: PaletteAction::NewTabProfile(index),
        });
    }
    palette_all.extend([
        PaletteItem {
            title: "查找…".to_owned(),
            hint: "Ctrl+Shift+F".to_owned(),
            keywords: "find search",
            action: PaletteAction::Find,
        },
        PaletteItem {
            title: "复制选区".to_owned(),
            hint: "Ctrl+Shift+C".to_owned(),
            keywords: "copy",
            action: PaletteAction::Copy,
        },
        PaletteItem {
            title: "粘贴".to_owned(),
            hint: "Ctrl+Shift+V".to_owned(),
            keywords: "paste",
            action: PaletteAction::Paste,
        },
        PaletteItem {
            title: "全选".to_owned(),
            hint: "Ctrl+Shift+U".to_owned(),
            keywords: "select all",
            action: PaletteAction::SelectAll,
        },
        PaletteItem {
            title: "关闭标签".to_owned(),
            hint: "Ctrl+Shift+W".to_owned(),
            keywords: "close tab",
            action: PaletteAction::CloseTab,
        },
        PaletteItem {
            title: "滚动到顶部".to_owned(),
            hint: "Shift+PageUp".to_owned(),
            keywords: "scroll top",
            action: PaletteAction::ScrollTop,
        },
        PaletteItem {
            title: "滚动到底部".to_owned(),
            hint: "Shift+PageDown".to_owned(),
            keywords: "scroll bottom",
            action: PaletteAction::ScrollBottom,
        },
    ]);
    let palette_items = palette_all.clone();

    let state = Rc::new(AppState {
        tabs: signal(Vec::<TabInfo>::new()),
        active: signal(0_u64),
        runtimes: RefCell::new(Vec::new()),
        next_id: Cell::new(0),
        profiles: profile_list,
        focus: element_ref(),
        anchor: Cell::new(None),
        dragging: Cell::new(false),
        palette_open: signal(false),
        palette_query: signal(String::new()),
        palette_items: signal(palette_items),
        palette_all,
        palette_focus: element_ref(),
    });

    spawn_tab(&state, None);

    // Poll every tab: newest frame, title, exit status.
    let poll = state.clone();
    set_interval(Duration::from_millis(8), move || {
        for tab in poll.runtimes.borrow().iter() {
            if let Some(session) = tab.session.as_ref() {
                if let Some(frame) = session.take_frame() {
                    *tab.frame.borrow_mut() = Some(frame);
                    tab.epoch.update(|value| *value += 1);
                }
                if let Some(title) = session.title() {
                    let usable = !title.is_empty()
                        && title.chars().count() <= 40
                        && !title.contains(".exe")
                        && !title.contains('/');
                    if usable && tab.title.get() != title {
                        tab.title.set(title);
                    }
                }
                if tab.exit.get().is_none() {
                    if let Some(code) = session.poll_exit() {
                        tab.exit.set(Some(code));
                    }
                }
                let found = session.find_result();
                if tab.find_count.get() != found {
                    tab.find_count.set(found);
                }
            }
        }

        let mut tabs = poll.tabs.get();
        let mut dirty = false;
        for info in tabs.iter_mut() {
            if let Some(tab) = poll.tab(info.id) {
                let base = tab.title.get();
                let exited = tab.exit.get();
                let display = match exited {
                    Some(_) => format!("{base} · 已退出"),
                    None => base,
                };
                if info.title != display || info.exited != exited {
                    info.title = display;
                    info.exited = exited;
                    dirty = true;
                }
            }
        }
        if dirty {
            poll.tabs.set(tabs);
        }
    });

    register_app_shortcuts(&state);

    let bar_state = state.clone();
    let menu_state = state.clone();
    let status_state = state.clone();
    let pane_state = state.clone();

    view()
        .flex_1()
        .bg(rgb(0x101216))
        .child(
            view()
                .h(px(TAB_HEIGHT))
                .w_full()
                .flex_row()
                .items_center()
                .gap_1()
                .px_2()
                .bg(rgb(0x15181d))
                .child(each(state.tabs, |info| info.id, move |info, _| {
                    tab_button(bar_state.clone(), info)
                }))
                .child(icon_button("+", {
                    let state = state.clone();
                    move || {
                        spawn_tab(&state, None);
                    }
                }))
                .child(profile_menu(menu_state))
                .child(
                    text(move || {
                        let Some(tab) = status_state.active_tab() else {
                            return String::new();
                        };
                        let (cols, rows) = tab.dims.get();
                        format!("{cols}×{rows}")
                    })
                    .text_size(px(11.0))
                    .text_color(rgb(0x6b7280))
                    .px_2(),
                ),
        )
        .child(view().flex_1().child(dynamic(move || {
            let _ = pane_state.tabs.get();
            if pane_state.runtimes.borrow().is_empty() {
                return empty_state(pane_state.clone()).into_element();
            }
            let id = pane_state.active.get();
            pane(pane_state.clone(), id).into_element()
        })))
        .child(when(
            {
                let state = state.clone();
                move || state.palette_open.get()
            },
            {
                let state = state.clone();
                move || palette_overlay(state.clone())
            },
        ))
}

fn icon_button(label: &'static str, on_press: impl Fn() + 'static) -> impl IntoElement {
    view()
        .size(px(26.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .active(|s| s.bg(rgb(0x2a3038)))
        .on_press(on_press)
        .child(
            text(label)
                .text_size(px(15.0))
                .text_color(rgb(0xaab2be)),
        )
}

fn tab_button(state: Rc<AppState>, info: ReadSignal<TabInfo>) -> impl IntoElement {
    let id = info.get().id;
    let bg_state = state.clone();
    let fg_state = state.clone();
    let activate_state = state.clone();
    let close_state = state.clone();
    let title = move || info.get().title.clone();

    view()
        .h(px(28.0))
        .max_w(px(220.0))
        .min_w(px(110.0))
        .flex_row()
        .items_center()
        .rounded(px(6.0))
        .bg(move || {
            if bg_state.active.get() == id {
                rgb(0x1e232b)
            } else {
                rgb(0x15181d)
            }
        })
        .child(
            view()
                .flex_1()
                .h_full()
                .flex_row()
                .items_center()
                .pl_3()
                .pr_2()
                .on_press(move || activate_state.activate(id))
                .child(
                    text(title)
                        .text_size(px(12.0))
                        .number_of_lines(1)
                        .text_color(move || {
                            if fg_state.active.get() == id {
                                rgb(0xf2f4f8)
                            } else {
                                rgb(0x9aa2ae)
                            }
                        }),
                ),
        )
        .child(
            view()
                .w(px(20.0))
                .h_full()
                .items_center()
                .justify_center()
                .on_press(move || close_state.close(id))
                .child(
                    text("×")
                        .text_size(px(13.0))
                        .text_color(rgb(0x9aa2ae))
                        .hover(|s| s.text_color(rgb(0xf2f4f8))),
                ),
        )
}

fn profile_menu(state: Rc<AppState>) -> impl IntoElement {
    let mut menu_view = menu()
        .offset(px(4.0))
        .popup_style(|s| {
            s.min_w(px(180.0))
                .p_1()
                .rounded(px(8.0))
                .border_1()
                .border_color(rgb(0x2a3038))
                .bg(rgb(0x1b1f26))
                .shadow_lg()
        })
        .trigger(
            view()
                .size(px(22.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .active(|s| s.bg(rgb(0x2a3038)))
                .child(text("⌄").text_size(px(11.0)).text_color(rgb(0xaab2be))),
        );

    for (index, profile) in state.profiles.iter().enumerate() {
        let name = profile.name.clone();
        let state = state.clone();
        menu_view = menu_view.child(
            menu_item()
                .on_select(move || {
                    spawn_tab(&state, Some(index));
                })
                .px_3()
                .py(px(6.0))
                .rounded(px(6.0))
                .hover(|s| s.bg(rgb(0x2a3038)))
                .child(text(name).text_size(px(12.0)).text_color(rgb(0xe6e8ec))),
        );
    }
    menu_view
}

fn pane(state: Rc<AppState>, id: u64) -> impl IntoElement {
    let Some(tab) = state.tab(id) else {
        return view();
    };

    let error_condition = tab.clone();
    let error_body = tab.clone();
    let error_state = state.clone();
    let surface_condition = tab.clone();
    let surface_body = tab.clone();
    let surface_state = state.clone();

    view()
        .flex_1()
        .child(when(move || error_condition.error.is_some(), move || {
            error_panel(error_body.clone(), error_state.clone())
        }))
        .child(when(move || surface_condition.error.is_none(), move || {
            terminal_surface(surface_state.clone(), surface_body.clone())
        }))
}

fn error_panel(tab: Rc<Tab>, state: Rc<AppState>) -> impl IntoElement {
    let message = tab.error.clone().unwrap_or_default();
    let name = tab.profile.name.clone();
    let close_state = state.clone();
    let id = tab.id;
    let command = format!("{} {}", tab.profile.program, tab.profile.args.join(" "));

    view()
        .flex_1()
        .bg(rgb(0x101216))
        .items_center()
        .justify_center()
        .child(
            view()
                .w(px(520.0))
                .gap_3()
                .child(
                    text(format!("无法启动 {name}"))
                        .text_size(px(15.0))
                        .text_color(rgb(0xf2f4f8)),
                )
                .child(
                    text(message)
                        .text_size(px(12.0))
                        .text_color(rgb(0xf2b544)),
                )
                .child(
                    text(command)
                        .text_size(px(11.0))
                        .text_color(rgb(0x8b929e)),
                )
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(
                            view()
                                .px_3()
                                .py_2()
                                .rounded(px(6.0))
                                .bg(rgb(0x2563eb))
                                .on_press(move || close_state.close(id))
                                .child(
                                    text("关闭标签")
                                        .text_size(px(12.0))
                                        .text_color(rgb(0xffffff)),
                                ),
                        )
                        .child(
                            view()
                                .px_3()
                                .py_2()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(rgb(0x2a3038))
                                .child(
                                    text("检查配置文件…")
                                        .text_size(px(12.0))
                                        .text_color(rgb(0xc3c8d1)),
                                ),
                        ),
                ),
        )
}

fn terminal_surface(state: Rc<AppState>, tab: Rc<Tab>) -> impl IntoElement {
    let metrics = Rc::new(grid::Metrics::new(FONT_SIZE));

    let draw_tab = tab.clone();
    let draw_metrics = metrics.clone();
    let layout_tab = tab.clone();
    let layout_metrics = metrics.clone();
    let touch_state = state.clone();
    let touch_tab = tab.clone();
    let touch_metrics = metrics.clone();
    let key_tab = tab.clone();
    let change_tab = tab.clone();
    let composing = Rc::new(Cell::new(false));
    let key_composing = composing.clone();
    let change_composing = composing.clone();
    let composition = composing.clone();
    let focus_ref = state.focus;
    let find_condition = tab.clone();
    let menu_tab = tab.clone();
    let menu_condition = tab.clone();
    let menu_body = tab.clone();
    let menu_state = state.clone();
    let find_body = tab.clone();
    let find_state = state.clone();

    view()
        .flex_1()
        .child(when(
            move || find_condition.find_open.get(),
            move || find_bar(find_state.clone(), find_body.clone()),
        ))
        .child(when(
            {
                let tab = tab.clone();
                move || tab.exit.get().is_some()
            },
            {
                let state = state.clone();
                let tab = tab.clone();
                move || exit_strip(state.clone(), tab.clone())
            },
        ))
        .child(
            canvas({
                let metrics = draw_metrics.clone();
                move || {
                    let _ = draw_tab.epoch.get();
                    let painter = match &*draw_tab.frame.borrow() {
                        Some(frame) => grid::draw(frame, &metrics),
                        None => Painter::new(),
                    };
                    painter
                }
            })
            .flex_1()
            .min_h(px(0.0))
            .on_layout(move |bounds: &Bounds<Points>| {
                let cols = layout_metrics.cols(bounds.size.width.0);
                let rows = layout_metrics.rows(bounds.size.height.0);
                if cols == 0 || rows == 0 {
                    return;
                }
                if layout_tab.grid.get() != (cols, rows) {
                    layout_tab.grid.set((cols, rows));
                    if layout_tab.dims.get() != (cols, rows) {
                        layout_tab.dims.set((cols, rows));
                    }
                    if let Some(session) = layout_tab.session.as_ref() {
                        session.resize(
                            cols,
                            rows,
                            layout_metrics.cell_w as u32,
                            layout_metrics.cell_h as u32,
                        );
                    }
                }
            })
            .on_press(move || focus_ref.focus())
            .on_touch(move |event: &TouchEvent| {
                let Some(point) = event.points.iter().find(|point| point.changed) else {
                    return;
                };
                let col = ((point.location.x.0 - touch_metrics.pad) / touch_metrics.cell_w)
                    .floor()
                    .max(0.0) as u16;
                let row = ((point.location.y.0 - touch_metrics.pad) / touch_metrics.cell_h)
                    .floor()
                    .max(0.0) as u16;
                let (cols, rows) = touch_tab.grid.get();
                let cell = (
                    col.min(cols.saturating_sub(1)),
                    u32::from(row.min(rows.saturating_sub(1))),
                );
                match event.phase {
                    TouchPhase::Start => {
                        touch_state.anchor.set(Some(cell));
                        touch_state.dragging.set(false);
                        if let Some(session) = touch_tab.session.as_ref() {
                            session.clear_selection();
                        }
                    }
                    TouchPhase::Move => {
                        if let Some(anchor) = touch_state.anchor.get() {
                            if anchor != cell || touch_state.dragging.get() {
                                touch_state.dragging.set(true);
                                if let Some(session) = touch_tab.session.as_ref() {
                                    session.select(anchor, cell);
                                }
                            }
                        }
                    }
                    TouchPhase::End | TouchPhase::Cancel => {
                        touch_state.anchor.set(None);
                    }
                }
                })
                .on_context_menu(move |event: &ContextMenuEvent| {
                    menu_tab.menu_at.set((event.position.x.0, event.position.y.0));
                    menu_tab.menu_open.set(true);
                }),
        )
        .child(when(
            move || menu_condition.menu_open.get(),
            move || context_menu_view(menu_state.clone(), menu_body.clone()),
        ))
        .child(
            nsh_base::input()
                .element_ref(&state.focus)
                .value(move || String::new())
                .caret(false)
                .auto_focus(true)
                .paste_behavior(PasteBehavior::Report)
                .on_key_press(move |key: &str| {
                    if key_composing.get() || key == "Backspace" || key == "Enter" {
                        return;
                    }
                    if !key.is_empty() {
                        if let Some(session) = key_tab.session.as_ref() {
                            session.send_text(key);
                        }
                    }
                })
                .on_change(move |text: &str| {
                    // Only an IME commit carries text the key presses did not.
                    if change_composing.get() && !text.is_empty() {
                        if let Some(session) = change_tab.session.as_ref() {
                            session.send_text(text);
                        }
                    }
                })
                .on_composition_change(move |event: &Composition| {
                    composition.set(event.composing);
                })
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .size(px(1.0))
                .opacity(0.0),
        )
}

fn exit_strip(state: Rc<AppState>, tab: Rc<Tab>) -> impl IntoElement {
    let restart_state = state.clone();
    let restart_tab = tab.clone();
    let close_state = state.clone();
    let text_tab = tab.clone();
    let color_tab = tab.clone();
    let id = tab.id;

    view()
        .h(px(28.0))
        .w_full()
        .flex_row()
        .items_center()
        .gap_3()
        .px_3()
        .bg(rgb(0x1b1f26))
        .child(
            text(move || {
                let code = text_tab.exit.get().unwrap_or(-1);
                if code == 0 {
                    "进程已退出，代码 0".to_owned()
                } else {
                    format!("进程已退出，代码 {code}")
                }
            })
            .text_size(px(11.0))
            .text_color(move || {
                if color_tab.exit.get() == Some(0) {
                    rgb(0x3fb950)
                } else {
                    rgb(0xf2b544)
                }
            }),
        )
        .child(
            view()
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .active(|s| s.bg(rgb(0x2a3038)))
                .on_press(move || {
                    let profile = restart_tab.profile.name.clone();
                    let index = restart_state
                        .profiles
                        .iter()
                        .position(|p| p.name == profile);
                    spawn_tab(&restart_state, index);
                    restart_state.close(restart_tab.id);
                })
                .child(
                    text("重新启动")
                        .text_size(px(11.0))
                        .text_color(rgb(0xe6e8ec)),
                ),
        )
        .child(
            view()
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .active(|s| s.bg(rgb(0x2a3038)))
                .on_press(move || close_state.close(id))
                .child(
                    text("关闭窗格")
                        .text_size(px(11.0))
                        .text_color(rgb(0xc3c8d1)),
                ),
        )
}

fn empty_state(state: Rc<AppState>) -> impl IntoElement {
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            text("没有打开的会话")
                .text_size(px(15.0))
                .text_color(rgb(0xe6e8ec)),
        )
        .child(
            text("按 Ctrl+Shift+T 新建一个标签")
                .text_size(px(12.0))
                .text_color(rgb(0x8b929e)),
        )
        .child(
            view()
                .px_4()
                .py_2()
                .rounded(px(6.0))
                .bg(rgb(0x2563eb))
                .on_press(move || {
                    spawn_tab(&state, None);
                })
                .child(
                    text("新建标签")
                        .text_size(px(12.0))
                        .text_color(rgb(0xffffff)),
                ),
        )
}

fn register_app_shortcuts(state: &Rc<AppState>) {
    let bind = |keystroke: &'static str, handler: Box<dyn Fn()>| {
        on_key_with(
            keystroke,
            KeyOptions {
                when: When::Always,
                repeat: false,
            },
            move |_| handler(),
        );
    };

    bind(
        "ctrl+shift+t",
        {
            let state = state.clone();
            Box::new(move || {
                spawn_tab(&state, None);
            })
        },
    );
    bind(
        "ctrl+shift+w",
        {
            let state = state.clone();
            Box::new(move || state.close(state.active.get()))
        },
    );
    bind(
        "ctrl+tab",
        {
            let state = state.clone();
            Box::new(move || state.activate_offset(1))
        },
    );
    bind(
        "ctrl+shift+tab",
        {
            let state = state.clone();
            Box::new(move || state.activate_offset(-1))
        },
    );
    bind(
        "ctrl+shift+c",
        {
            let state = state.clone();
            Box::new(move || {
                copy_selection(&state);
            })
        },
    );
    bind(
        "ctrl+shift+v",
        {
            let state = state.clone();
            Box::new(move || {
                paste_clipboard(&state);
            })
        },
    );
    bind(
        "ctrl+shift+f",
        {
            let state = state.clone();
            Box::new(move || {
                let Some(tab) = state.active_tab() else {
                    return;
                };
                let open = !tab.find_open.get();
                tab.find_open.set(open);
                if open {
                    let focus = tab.find_focus;
                    set_timeout(Duration::from_millis(0), move || focus.focus());
                } else {
                    tab.find_count.set(None);
                    tab.find_query.set(String::new());
                    if let Some(session) = tab.session.as_ref() {
                        session.clear_selection();
                    }
                    let focus = state.focus;
                    set_timeout(Duration::from_millis(0), move || focus.focus());
                }
            })
        },
    );
    bind(
        "ctrl+shift+p",
        {
            let state = state.clone();
            Box::new(move || {
                let open = !state.palette_open.get();
                state.palette_open.set(open);
                if open {
                    let focus = state.palette_focus;
                    set_timeout(Duration::from_millis(0), move || focus.focus());
                } else {
                    state.palette_query.set(String::new());
                    state.palette_items.set(state.palette_all.clone());
                }
            })
        },
    );
    // Ctrl+Shift+A is an editing shortcut the focused input keeps, so the
    // select-all lives on Ctrl+Shift+U.
    bind(
        "ctrl+shift+u",
        {
            let state = state.clone();
            Box::new(move || {
                if let Some(tab) = state.active_tab() {
                    if let Some(session) = tab.session.as_ref() {
                        session.select_all();
                    }
                }
            })
        },
    );
    bind(
        "shift+pageup",
        {
            let state = state.clone();
            Box::new(move || {
                if let Some(session) = state.active_tab().and_then(|tab| tab.session()) {
                    session.scroll(-20);
                }
            })
        },
    );
    bind(
        "shift+pagedown",
        {
            let state = state.clone();
            Box::new(move || {
                if let Some(session) = state.active_tab().and_then(|tab| tab.session()) {
                    session.scroll(20);
                }
            })
        },
    );

    for (index, keystroke) in [
        "ctrl+1", "ctrl+2", "ctrl+3", "ctrl+4", "ctrl+5", "ctrl+6", "ctrl+7", "ctrl+8", "ctrl+9",
    ]
    .into_iter()
    .enumerate()
    {
        let state = state.clone();
        on_key_with(
            keystroke,
            KeyOptions {
                when: When::Always,
                repeat: false,
            },
            move |_| {
                if let Some(info) = state.tabs.get().get(index) {
                    state.activate(info.id);
                }
            },
        );
    }

    // Terminal keys go to the active tab.
    for keystroke in keys::TERMINAL_KEYS {
        let Some((key, mods)) = keys::parse(keystroke) else {
            continue;
        };
        let state = state.clone();
        on_key_with(
            keystroke,
            KeyOptions {
                when: When::Always,
                repeat: true,
            },
            move |_| {
                let active = state.active_tab();
                if keystroke == &"enter" {
                    if state.palette_open.get() {
                        if let Some(item) = state.palette_items.get().first() {
                            run_palette(&state, item.action.clone());
                        }
                        return;
                    }
                    if let Some(tab) = active.as_ref() {
                        if tab.find_open.get() {
                            let skip = tab.find_skip.get() + 1;
                            tab.find_skip.set(skip);
                            let query = tab.find_query.get();
                            if let Some(session) = tab.session.as_ref() {
                                session.find(&query, 1, skip);
                            }
                            return;
                        }
                    }
                }
                if keystroke == &"escape" {
                    if state.palette_open.get() {
                        state.palette_open.set(false);
                        state.palette_query.set(String::new());
                        state.palette_items.set(state.palette_all.clone());
                        let focus = state.focus;
                        set_timeout(Duration::from_millis(0), move || focus.focus());
                        return;
                    }
                    if let Some(tab) = active.as_ref() {
                        if tab.menu_open.get() {
                            tab.menu_open.set(false);
                            return;
                        }
                        if tab.find_open.get() {
                            tab.find_open.set(false);
                            tab.find_count.set(None);
                            tab.find_query.set(String::new());
                            if let Some(session) = tab.session.as_ref() {
                                session.clear_selection();
                            }
                            let focus = state.focus;
                            set_timeout(Duration::from_millis(0), move || focus.focus());
                            return;
                        }
                    }
                }
                if let Some(session) = active.and_then(|tab| tab.session()) {
                    session.send_key(key, mods);
                }
            },
        );
    }
}

fn find_bar(state: Rc<AppState>, tab: Rc<Tab>) -> impl IntoElement {
    let query_tab = tab.clone();
    let count_tab = tab.clone();
    let next_tab = tab.clone();
    let prev_tab = tab.clone();
    let close_tab = tab.clone();
    let focus = state.focus;

    view()
        .h(px(32.0))
        .w_full()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .bg(rgb(0x1b1f26))
        .child(
            nsh_base::input()
                .element_ref(&tab.find_focus)
                .value(move || query_tab.find_query.get())
                .placeholder("查找…")
                .caret(true)
                .auto_focus(true)
                .on_change(move |text: &str| {
                    tab.find_query.set(text.to_owned());
                    tab.find_skip.set(0);
                    tab.find_count.set(None);
                    if let Some(session) = tab.session.as_ref() {
                        session.find(text, 1, 0);
                    }
                })
                .w(px(300.0))
                .h(px(24.0))
                .px_2()
                .text_size(px(12.0))
                .text_color(rgb(0xe6e8ec))
                .bg(rgb(0x101216))
                .rounded(px(4.0)),
        )
        .child(
            text(move || match count_tab.find_count.get() {
                Some((0, _)) => "0/0".to_owned(),
                Some((total, selected)) => format!("{selected}/{total}"),
                None => "…".to_owned(),
            })
            .text_size(px(11.0))
            .text_color(rgb(0x8b929e))
            .w(px(70.0)),
        )
        .child(
            view()
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .active(|s| s.bg(rgb(0x2a3038)))
                .on_press(move || {
                    let skip = prev_tab.find_skip.get();
                    let query = prev_tab.find_query.get();
                    if let Some(session) = prev_tab.session.as_ref() {
                        session.find(&query, -1, skip);
                    }
                })
                .child(text("↑").text_size(px(12.0)).text_color(rgb(0xc3c8d1))),
        )
        .child(
            view()
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .active(|s| s.bg(rgb(0x2a3038)))
                .on_press(move || {
                    let skip = next_tab.find_skip.get() + 1;
                    next_tab.find_skip.set(skip);
                    let query = next_tab.find_query.get();
                    if let Some(session) = next_tab.session.as_ref() {
                        session.find(&query, 1, skip);
                    }
                })
                .child(text("↓").text_size(px(12.0)).text_color(rgb(0xc3c8d1))),
        )
        .child(
            view()
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .active(|s| s.bg(rgb(0x2a3038)))
                .on_press(move || {
                    close_tab.find_open.set(false);
                    if let Some(session) = close_tab.session.as_ref() {
                        session.clear_selection();
                    }
                    let focus = focus;
                    set_timeout(Duration::from_millis(0), move || focus.focus());
                })
                .child(text("×").text_size(px(12.0)).text_color(rgb(0xc3c8d1))),
        )
}

fn copy_selection(state: &Rc<AppState>) {
    if let Some(tab) = state.active_tab() {
        if let Some(session) = tab.session.as_ref() {
            let text = session.selection_text();
            if !text.is_empty() {
                set_clipboard(text);
            }
        }
    }
}

fn paste_clipboard(state: &Rc<AppState>) {
    let Some(tab) = state.active_tab() else {
        return;
    };
    let Some(session) = tab.session() else {
        return;
    };
    spawn(async move {
        let text = clipboard_string().await;
        if !text.is_empty() {
            session.send_text(&text);
        }
    });
}

fn open_find(state: &Rc<AppState>) {
    let Some(tab) = state.active_tab() else {
        return;
    };
    tab.find_open.set(true);
    let focus = tab.find_focus;
    set_timeout(Duration::from_millis(0), move || focus.focus());
}

fn context_menu_view(state: Rc<AppState>, tab: Rc<Tab>) -> impl IntoElement {
    let (x, y) = tab.menu_at.get();
    let close = {
        let tab = tab.clone();
        move || tab.menu_open.set(false)
    };
    let close_copy = close.clone();
    let close_paste = close.clone();
    let close_all = close.clone();
    let close_find = close.clone();
    let close_close = close.clone();
    let state_copy = state.clone();
    let state_paste = state.clone();
    let state_all = state.clone();
    let state_find = state.clone();
    let state_close = state.clone();

    view()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(184.0))
        .p_1()
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(0x2a3038))
        .bg(rgb(0x1b1f26))
        .shadow_lg()
        .child(menu_row("复制", move || {
            copy_selection(&state_copy);
            close_copy();
        }))
        .child(menu_row("粘贴", move || {
            paste_clipboard(&state_paste);
            close_paste();
        }))
        .child(menu_row("全选", move || {
            if let Some(tab) = state_all.active_tab() {
                if let Some(session) = tab.session.as_ref() {
                    session.select_all();
                }
            }
            close_all();
        }))
        .child(menu_row("查找…", move || {
            open_find(&state_find);
            close_find();
        }))
        .child(view().h(px(1.0)).my_1().bg(rgb(0x2a3038)))
        .child(menu_row("关闭窗格", move || {
            state_close.close(state_close.active.get());
            close_close();
        }))
}

fn menu_row(label: &'static str, on_press: impl Fn() + 'static) -> impl IntoElement {
    view()
        .px_3()
        .py(px(6.0))
        .rounded(px(6.0))
        .active(|s| s.bg(rgb(0x2a3038)))
        .on_press(on_press)
        .child(text(label).text_size(px(12.0)).text_color(rgb(0xe6e8ec)))
}

fn filter_palette(all: &[PaletteItem], query: &str) -> Vec<PaletteItem> {
    let needle = query.trim().to_lowercase();
    all.iter()
        .filter(|item| {
            needle.is_empty()
                || item.title.to_lowercase().contains(&needle)
                || item.keywords.contains(&needle)
        })
        .cloned()
        .collect()
}

fn run_palette(state: &Rc<AppState>, action: PaletteAction) {
    match action {
        PaletteAction::NewTab => {
            spawn_tab(state, None);
        }
        PaletteAction::NewTabProfile(index) => {
            spawn_tab(state, Some(index));
        }
        PaletteAction::Find => open_find(state),
        PaletteAction::Copy => copy_selection(state),
        PaletteAction::Paste => paste_clipboard(state),
        PaletteAction::SelectAll => {
            if let Some(tab) = state.active_tab() {
                if let Some(session) = tab.session.as_ref() {
                    session.select_all();
                }
            }
        }
        PaletteAction::CloseTab => state.close(state.active.get()),
        PaletteAction::ScrollTop => {
            if let Some(session) = state.active_tab().and_then(|tab| tab.session()) {
                session.scroll(-100_000);
            }
        }
        PaletteAction::ScrollBottom => {
            if let Some(session) = state.active_tab().and_then(|tab| tab.session()) {
                session.scroll(100_000);
            }
        }
    }
    state.palette_open.set(false);
    state.palette_query.set(String::new());
    state.palette_items.set(state.palette_all.clone());
}

fn palette_overlay(state: Rc<AppState>) -> impl IntoElement {
    let query_state = state.clone();
    let change_state = state.clone();
    let row_state = state.clone();
    let all = Rc::new(state.palette_all.clone());

    view()
        .absolute()
        .top(px(64.0))
        .left(px(140.0))
        .w(px(520.0))
        .p_2()
        .gap_2()
        .rounded(px(10.0))
        .border_1()
        .border_color(rgb(0x2a3038))
        .bg(rgb(0x1b1f26))
        .shadow_lg()
        .child(
            nsh_base::input()
                .element_ref(&state.palette_focus)
                .value(move || query_state.palette_query.get())
                .placeholder("输入命令…")
                .auto_focus(true)
                .on_change(move |text: &str| {
                    let items = filter_palette(&all, text);
                    change_state.palette_query.set(text.to_owned());
                    change_state.palette_items.set(items);
                })
                .h(px(30.0))
                .w_full()
                .px_2()
                .text_size(px(13.0))
                .text_color(rgb(0xe6e8ec))
                .bg(rgb(0x101216))
                .rounded(px(6.0)),
        )
        .child(
            view()
                .gap_1()
                .child(each(
                    state.palette_items,
                    |item| item.title.clone(),
                    move |item, _| {
                        let row_state = row_state.clone();
                        let action = item.get().action.clone();
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .active(|s| s.bg(rgb(0x2a3038)))
                            .on_press(move || {
                                run_palette(&row_state, action.clone());
                            })
                            .child(
                                text(move || item.get().title.clone())
                                    .text_size(px(12.0))
                                    .text_color(rgb(0xe6e8ec)),
                            )
                            .child(
                                text(move || item.get().hint.clone())
                                    .text_size(px(11.0))
                                    .text_color(rgb(0x6b7280)),
                            )
                    },
                )),
        )
}
