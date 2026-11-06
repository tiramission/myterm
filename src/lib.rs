//! myterm — a Windows terminal: tabs with split panes, profiles, find,
//! selection and copy/paste, command palette, context menu.

mod grid;
mod keys;
pub mod profiles;
mod session;
pub mod settings;
mod term;
pub mod ui;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use nsh_base::{menu, menu_item};
use nsh_core::TouchPhase;
use nsh_core::element::TouchEvent;
use nsh_core::keymap::When;
use nsh_core::prelude::*;
use ui::{
    accent, accent_foreground, background, border, foreground, muted_foreground, popover, primary,
    radius, success, surface, surface_foreground, warning,
};

const START_COLS: u16 = 100;
const START_ROWS: u16 = 28;
const TAB_HEIGHT: f32 = 34.0;
const SEPARATOR: f32 = 1.0;

#[derive(Clone, PartialEq, Debug)]
enum PaletteAction {
    NewTab,
    NewTabProfile(usize),
    SplitRight,
    SplitDown,
    Find,
    Copy,
    Paste,
    SelectAll,
    CloseTab,
    ScrollTop,
    ScrollBottom,
    Settings,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SplitAxis {
    Row,
    Column,
}

/// How a tab's panes are arranged.
#[derive(Clone, PartialEq, Debug)]
enum Layout {
    Leaf(u64),
    Split {
        axis: SplitAxis,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

/// One terminal: a session, its frame and its per-view state.
struct Pane {
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
    focus: ElementRef,
    anchor: Cell<Option<(u16, u32)>>,
    dragging: Cell<bool>,
    bounds: Cell<(f32, f32)>,
    /// Where the terminal cursor is in points inside the pane: the IME
    /// anchors its composition and candidate windows there.
    ime_at: Signal<(f32, f32)>,
}

impl Pane {
    fn session(&self) -> Option<Rc<session::Session>> {
        self.session.clone()
    }
}

/// A tab: one or more panes and how they are arranged.
struct Tab {
    id: u64,
    panes: RefCell<Vec<Rc<Pane>>>,
    layout: Signal<Layout>,
    active: Signal<u64>,
    title: Signal<String>,
    exited: Signal<Option<i32>>,
}

impl Tab {
    fn pane(&self, id: u64) -> Option<Rc<Pane>> {
        self.panes
            .borrow()
            .iter()
            .find(|pane| pane.id == id)
            .cloned()
    }

    fn active_pane(&self) -> Option<Rc<Pane>> {
        self.pane(self.active.get())
    }

    fn panes_in_order(&self) -> Vec<Rc<Pane>> {
        let mut order = Vec::new();
        flatten(&self.layout.get(), &mut order);
        order.into_iter().filter_map(|id| self.pane(id)).collect()
    }
}

fn flatten(layout: &Layout, out: &mut Vec<u64>) {
    match layout {
        Layout::Leaf(id) => out.push(*id),
        Layout::Split { first, second, .. } => {
            flatten(first, out);
            flatten(second, out);
        }
    }
}

/// The layout with `target`'s leaf replaced by `replacement`.
fn replace_leaf(layout: &Layout, target: u64, replacement: &Layout) -> Layout {
    match layout {
        Layout::Leaf(id) if *id == target => replacement.clone(),
        Layout::Leaf(id) => Layout::Leaf(*id),
        Layout::Split {
            axis,
            first,
            second,
        } => Layout::Split {
            axis: *axis,
            first: Box::new(replace_leaf(first, target, replacement)),
            second: Box::new(replace_leaf(second, target, replacement)),
        },
    }
}

/// The layout without `target`; `None` when nothing is left.
fn remove_leaf(layout: &Layout, target: u64) -> Option<Layout> {
    match layout {
        Layout::Leaf(id) => (*id != target).then_some(layout.clone()),
        Layout::Split {
            axis,
            first,
            second,
        } => match (remove_leaf(first, target), remove_leaf(second, target)) {
            (None, None) => None,
            (None, Some(survivor)) => Some(survivor),
            (Some(survivor), None) => Some(survivor),
            (Some(a), Some(b)) => Some(Layout::Split {
                axis: *axis,
                first: Box::new(a),
                second: Box::new(b),
            }),
        },
    }
}

struct AppState {
    tabs: Signal<Vec<TabInfo>>,
    active: Signal<u64>,
    runtimes: RefCell<Vec<Rc<Tab>>>,
    next_tab: Cell<u64>,
    next_pane: Cell<u64>,
    profiles: Vec<profiles::Profile>,
    palette_open: Signal<bool>,
    palette_query: Signal<String>,
    palette_items: Signal<Vec<PaletteItem>>,
    palette_all: Vec<PaletteItem>,
    palette_focus: ElementRef,
    settings: Signal<settings::Settings>,
    metrics: Signal<grid::Metrics>,
    scheme: Signal<settings::Scheme>,
    settings_open: Signal<bool>,
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

    fn active_pane(&self) -> Option<Rc<Pane>> {
        self.active_tab().and_then(|tab| tab.active_pane())
    }

    fn activate(&self, id: u64) {
        if self.active.get() != id {
            log::info!("activate tab {id}");
            self.active.set(id);
            self.focus_active_pane();
        }
    }

    fn focus_active_pane(&self) {
        if let Some(pane) = self.active_pane() {
            let focus = pane.focus;
            set_timeout(Duration::from_millis(0), move || focus.focus());
        }
    }

    fn activate_pane(&self, tab: &Rc<Tab>, pane_id: u64) {
        if tab.active.get() != pane_id {
            tab.active.set(pane_id);
            self.focus_active_pane();
        }
    }

    fn cycle_pane(&self, offset: isize) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let panes = tab.panes_in_order();
        if panes.len() < 2 {
            return;
        }
        let current = panes
            .iter()
            .position(|pane| pane.id == tab.active.get())
            .unwrap_or(0) as isize;
        let len = panes.len() as isize;
        let next = ((current + offset) % len + len) % len;
        self.activate_pane(&tab, panes[next as usize].id);
    }

    fn close(&self, id: u64) {
        log::info!("close tab {id}");
        let index = self.runtimes.borrow().iter().position(|tab| tab.id == id);
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
                    self.focus_active_pane();
                }
                None => self.active.set(0),
            }
        }
    }

    fn close_active_pane(&self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let pane_id = tab.active.get();
        self.close_pane(&tab, pane_id);
    }

    fn close_pane(&self, tab: &Rc<Tab>, pane_id: u64) {
        if tab.panes.borrow().len() <= 1 {
            self.close(tab.id);
            return;
        }
        let Some(layout) = remove_leaf(&tab.layout.get(), pane_id) else {
            self.close(tab.id);
            return;
        };
        log::info!("close pane {pane_id} of tab {}", tab.id);
        tab.panes.borrow_mut().retain(|pane| pane.id != pane_id);
        tab.layout.set(layout);
        if tab.active.get() == pane_id {
            if let Some(next) = tab.panes_in_order().first() {
                tab.active.set(next.id);
            }
        }
        self.focus_active_pane();
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

fn new_pane(state: &Rc<AppState>, profile: &profiles::Profile) -> Rc<Pane> {
    let id = state.next_pane.get() + 1;
    state.next_pane.set(id);
    let metrics = state.metrics.get();
    let spawned = session::Session::spawn(
        profile,
        START_COLS,
        START_ROWS,
        metrics.cell_w as u32,
        metrics.cell_h as u32,
    );
    let (session, error) = match spawned {
        Ok(session) => {
            let session = Rc::new(session);
            // The scheme is applied before the first frame, so the grid never
            // flashes libghostty's default black.
            session.set_scheme(&state.scheme.get());
            (Some(session), None)
        }
        Err(error) => (None, Some(error)),
    };
    Rc::new(Pane {
        id,
        profile: profile.clone(),
        session,
        error,
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
        focus: element_ref(),
        anchor: Cell::new(None),
        dragging: Cell::new(false),
        bounds: Cell::new((0.0, 0.0)),
        ime_at: signal((0.0_f32, 0.0_f32)),
    })
}

fn spawn_tab(state: &Rc<AppState>, profile: Option<usize>) -> Option<u64> {
    // An explicit index wins, then the settings' default profile, then the
    // first one that was found.
    let preferred = state.settings.get().default_profile;
    let profile = profile
        .and_then(|index| state.profiles.get(index).cloned())
        .or_else(|| {
            state
                .profiles
                .iter()
                .find(|profile| profile.name == preferred)
                .cloned()
        })
        .or_else(|| state.profiles.first().cloned())?;

    let pane = new_pane(state, &profile);
    if pane.error.is_some() {
        log::warn!("{} failed to start: {:?}", profile.name, pane.error);
    } else {
        log::info!("new pane {} ({})", pane.id, profile.name);
    }
    let pane_id = pane.id;

    let id = state.next_tab.get() + 1;
    state.next_tab.set(id);
    let title = if pane.error.is_some() {
        format!("{} · 启动失败", profile.name)
    } else {
        profile.name.clone()
    };
    let tab = Rc::new(Tab {
        id,
        panes: RefCell::new(vec![pane]),
        layout: signal(Layout::Leaf(pane_id)),
        active: signal(pane_id),
        title: signal(title.clone()),
        exited: signal(None::<i32>),
    });
    state.runtimes.borrow_mut().push(tab);

    let mut tabs = state.tabs.get();
    tabs.push(TabInfo {
        id,
        title,
        exited: None,
    });
    state.tabs.set(tabs);
    state.active.set(id);
    state.focus_active_pane();
    Some(id)
}

fn split_active(state: &Rc<AppState>, axis: SplitAxis) {
    let Some(tab) = state.active_tab() else {
        return;
    };
    let Some(current) = tab.active_pane() else {
        return;
    };
    let pane = new_pane(state, &current.profile);
    let pane_id = pane.id;
    let target = current.id;
    let layout = replace_leaf(
        &tab.layout.get(),
        target,
        &Layout::Split {
            axis,
            first: Box::new(Layout::Leaf(target)),
            second: Box::new(Layout::Leaf(pane_id)),
        },
    );
    tab.panes.borrow_mut().push(pane);
    tab.layout.set(layout);
    tab.active.set(pane_id);
    log::info!("split pane {target} -> {pane_id}");
    state.focus_active_pane();
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
        PaletteItem {
            title: "向右分屏".to_owned(),
            hint: "Ctrl+Shift+D".to_owned(),
            keywords: "split right pane",
            action: PaletteAction::SplitRight,
        },
        PaletteItem {
            title: "向下分屏".to_owned(),
            hint: "Ctrl+Shift+E".to_owned(),
            keywords: "split down pane",
            action: PaletteAction::SplitDown,
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
            title: "关闭窗格".to_owned(),
            hint: "Ctrl+Shift+W".to_owned(),
            keywords: "close pane tab",
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
        PaletteItem {
            title: "打开设置".to_owned(),
            hint: "Ctrl+,".to_owned(),
            keywords: "settings preferences options config",
            action: PaletteAction::Settings,
        },
    ]);
    let palette_items = palette_all.clone();
    let defaults = load_settings();
    let metrics = grid::Metrics::new(
        defaults.font_size,
        defaults.line_height,
        defaults.padding,
        &defaults.font_family,
    );

    let state = Rc::new(AppState {
        tabs: signal(Vec::<TabInfo>::new()),
        active: signal(0_u64),
        runtimes: RefCell::new(Vec::new()),
        next_tab: Cell::new(0),
        next_pane: Cell::new(0),
        profiles: profile_list,
        palette_open: signal(false),
        palette_query: signal(String::new()),
        palette_items: signal(palette_items),
        palette_all,
        palette_focus: element_ref(),
        settings: signal(defaults.clone()),
        metrics: signal(metrics),
        scheme: signal(settings::scheme(&defaults.scheme)),
        settings_open: signal(false),
    });

    spawn_tab(&state, None);
    {
        let state = state.clone();
        set_timeout(Duration::from_millis(250), move || {
            state.focus_active_pane()
        });
    }

    // Poll every pane: newest frame, title, exit status, find result.
    let poll = state.clone();
    set_interval(Duration::from_millis(8), move || {
        for tab in poll.runtimes.borrow().iter() {
            for pane in tab.panes.borrow().iter() {
                let Some(session) = pane.session.as_ref() else {
                    continue;
                };
                if let Some(frame) = session.take_frame() {
                    // The IME anchors to the terminal's cursor, so its cell
                    // is kept where the pane can read it.
                    let metrics = poll.metrics.get();
                    let at = match frame.cursor {
                        Some((col, row)) => (
                            metrics.pad + f32::from(col) * metrics.cell_w,
                            metrics.pad + f32::from(row) * metrics.cell_h,
                        ),
                        None => (0.0, 0.0),
                    };
                    if pane.ime_at.get() != at {
                        pane.ime_at.set(at);
                    }
                    *pane.frame.borrow_mut() = Some(frame);
                    pane.epoch.update(|value| *value += 1);
                }
                if let Some(title) = session.title() {
                    let usable = !title.is_empty()
                        && title.chars().count() <= 40
                        && !title.contains(".exe")
                        && !title.contains('/');
                    if usable && pane.title.get() != title {
                        pane.title.set(title);
                    }
                }
                if pane.exit.get().is_none() {
                    if let Some(code) = session.poll_exit() {
                        pane.exit.set(Some(code));
                    }
                }
                let found = session.find_result();
                if pane.find_count.get() != found {
                    pane.find_count.set(found);
                }
            }

            let title = tab
                .active_pane()
                .map(|pane| pane.title.get())
                .unwrap_or_default();
            if tab.title.get() != title {
                tab.title.set(title.clone());
            }
            let mut all_exited = true;
            let mut code = Some(0);
            for pane in tab.panes.borrow().iter() {
                match pane.exit.get() {
                    Some(status) => code = Some(status),
                    None => all_exited = false,
                }
            }
            let exited = all_exited.then_some(code.unwrap_or(-1));
            if tab.exited.get() != exited {
                tab.exited.set(exited);
            }
        }

        let mut tabs = poll.tabs.get();
        let mut dirty = false;
        for info in tabs.iter_mut() {
            if let Some(tab) = poll.tab(info.id) {
                let base = tab.title.get();
                let exited = tab.exited.get();
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
    let status_name = state.clone();
    let status_dims = state.clone();
    let status_settings = state.clone();
    let pane_state = state.clone();
    let area_w = signal(0.0_f32);
    let area_h = signal(0.0_f32);

    view()
        .flex_1()
        .bg(background())
        // The tab strip is window chrome: it stays put while panes change.
        .child(
            view()
                .h(px(TAB_HEIGHT))
                .w_full()
                .flex_row()
                .items_center()
                .gap_1()
                .px_2()
                .bg(surface())
                .child(each(
                    state.tabs,
                    |info| info.id,
                    move |info, _| tab_button(bar_state.clone(), info),
                ))
                .child(ui::icon_button("+", "新建标签", {
                    let state = state.clone();
                    move || {
                        spawn_tab(&state, None);
                    }
                }))
                .child(profile_menu(menu_state))
                .child(view().flex_1())
                .child(
                    text("Ctrl+Shift+P 命令面板")
                        .text_xs()
                        .text_color(muted_foreground())
                        .px_2(),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .child(dynamic(move || {
                    let _ = pane_state.tabs.get();
                    let Some(tab) = pane_state.active_tab() else {
                        return empty_state(pane_state.clone()).into_element();
                    };
                    let layout = tab.layout.get();
                    render_layout(
                        pane_state.clone(),
                        tab,
                        &layout,
                        (area_w.get(), area_h.get()),
                    )
                    .into_element()
                }))
                .on_layout({
                    let area_w = area_w;
                    let area_h = area_h;
                    move |bounds: &Bounds<Points>| {
                        area_w.set(bounds.size.width.0);
                        area_h.set(bounds.size.height.0);
                    }
                }),
        )
        // The status bar: what the active pane is, how big it is, and the
        // way into the settings.
        .child(
            view()
                .h(px(26.0))
                .w_full()
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .border_t_1()
                .border_color(border())
                .bg(surface())
                .child(
                    text(move || {
                        status_name
                            .active_pane()
                            .map(|pane| pane.profile.name.clone())
                            .unwrap_or_default()
                    })
                    .text_xs()
                    .text_color(surface_foreground()),
                )
                .child(view().flex_1())
                .child(
                    text(move || {
                        let Some(pane) = status_dims.active_pane() else {
                            return String::new();
                        };
                        let (cols, rows) = pane.dims.get();
                        format!("{cols}×{rows}")
                    })
                    .text_xs()
                    .text_color(muted_foreground()),
                )
                .child(ui::text_button("设置…", move || {
                    status_settings.settings_open.set(true);
                })),
        )
        // A press outside the overlay dismisses it, as Escape does. The
        // backdrop comes first, so the overlay's own controls stay reachable.
        .child(when(
            {
                let state = state.clone();
                move || state.settings_open.get() || state.palette_open.get()
            },
            {
                let press = state.clone();
                move || {
                    view()
                        .absolute()
                        .left(px(0.0))
                        .top(px(0.0))
                        .w_full()
                        .h_full()
                        .on_press({
                            let state = press.clone();
                            move || {
                                if state.palette_open.get() {
                                    state.palette_open.set(false);
                                    state.palette_query.set(String::new());
                                    state.palette_items.set(state.palette_all.clone());
                                } else {
                                    state.settings_open.set(false);
                                }
                            }
                        })
                }
            },
        ))
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
        .child(when(
            {
                let state = state.clone();
                move || state.settings_open.get()
            },
            {
                let state = state.clone();
                move || settings_overlay(state.clone())
            },
        ))
}

fn render_layout(
    state: Rc<AppState>,
    tab: Rc<Tab>,
    layout: &Layout,
    area: (f32, f32),
) -> AnyElement {
    let mut rects = Vec::new();
    pane_rects(layout, (0.0, 0.0), area, &mut rects);
    // One canvas paints every pane at its own offset: NSH 0.3 draws a canvas
    // in a nested view without the view's own offset, so per-pane canvases
    // land in the wrong place when a tab is split.
    let metrics = state.metrics.get();
    let metrics = Rc::new(metrics);
    let draw_tab = tab.clone();
    let draw_rects = rects.clone();
    let draw_metrics = metrics.clone();
    let draw_active = tab.active;

    let mut container = view()
        .absolute()
        .left(px(0.0))
        .top(px(0.0))
        .w_full()
        .h_full()
        .child(
            canvas(move || {
                let mut painter = Painter::new();
                for (pane_id, (x, y, w, h)) in draw_rects.iter().copied() {
                    let Some(pane) = draw_tab.pane(pane_id) else {
                        continue;
                    };
                    let _ = pane.epoch.get();
                    if let Some(frame) = &*pane.frame.borrow() {
                        painter = grid::draw_into(painter, frame, &draw_metrics, x, y, (w, h));
                    }
                    let _ = draw_active.get();
                }
                painter
            })
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w_full()
            .h_full(),
        );

    for (pane_id, rect) in rects {
        container = container.child(pane_container(state.clone(), tab.clone(), pane_id, rect));
    }
    container.into_element()
}

fn pane_rects(
    layout: &Layout,
    origin: (f32, f32),
    area: (f32, f32),
    out: &mut Vec<(u64, (f32, f32, f32, f32))>,
) {
    match layout {
        Layout::Leaf(id) => out.push((*id, (origin.0, origin.1, area.0, area.1))),
        Layout::Split {
            axis,
            first,
            second,
        } => {
            if matches!(axis, SplitAxis::Row) {
                let w = ((area.0 - SEPARATOR) / 2.0).max(1.0);
                pane_rects(first, origin, (w, area.1), out);
                pane_rects(
                    second,
                    (origin.0 + w + SEPARATOR, origin.1),
                    ((area.0 - w - SEPARATOR).max(1.0), area.1),
                    out,
                );
            } else {
                let h = ((area.1 - SEPARATOR) / 2.0).max(1.0);
                pane_rects(first, origin, (area.0, h), out);
                pane_rects(
                    second,
                    (origin.0, origin.1 + h + SEPARATOR),
                    (area.0, (area.1 - h - SEPARATOR).max(1.0)),
                    out,
                );
            }
        }
    }
}

/// The tab's panes: one canvas paints every pane at its own offset, and a
/// layer per pane holds its chrome (find bar, exit strip, menu) and input.
fn pane_container(
    state: Rc<AppState>,
    tab: Rc<Tab>,
    pane_id: u64,
    rect: (f32, f32, f32, f32),
) -> AnyElement {
    let Some(pane) = tab.pane(pane_id) else {
        return view().into_element();
    };
    let (x, y, w, h) = rect;
    let pane_metrics = Rc::new(state.metrics.get());
    let active = tab.active;
    let activate_state = state.clone();
    let activate_tab = tab.clone();
    let focus_ref = pane.focus;
    let menu_pane = pane.clone();
    let find_condition = pane.clone();
    let find_body = pane.clone();
    let find_state = state.clone();
    let exit_condition = pane.clone();
    let exit_body = pane.clone();
    let exit_state = state.clone();
    let menu_condition = pane.clone();
    let menu_body = pane.clone();
    let menu_state = state.clone();
    let key_pane = pane.clone();
    let change_pane = pane.clone();
    let composing = Rc::new(Cell::new(false));
    let key_composing = composing.clone();
    let change_composing = composing.clone();
    let composition = composing.clone();
    let error_condition = pane.clone();
    let error_body = pane.clone();
    let error_state = state.clone();
    let dismiss_pane = pane.clone();
    let ime_at = pane.ime_at;

    view()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(w))
        .h(px(h))
        .overflow_hidden()
        .border_1()
        .border_color(move || {
            if active.get() == pane_id {
                primary()
            } else {
                background()
            }
        })
        .label("终端窗格")
        .on_press(move || {
            activate_state.activate_pane(&activate_tab, pane_id);
            focus_ref.focus();
        })
        .child(when(
            move || find_condition.find_open.get(),
            move || find_bar(find_state.clone(), find_body.clone()),
        ))
        .child(when(
            move || exit_condition.exit.get().is_some(),
            move || exit_strip(exit_state.clone(), exit_body.clone()),
        ))
        .child(when(
            move || error_condition.error.is_some(),
            move || error_panel(error_body.clone(), error_state.clone()),
        ))
        .child(
            nsh_base::input()
                .element_ref(&pane.focus)
                .value(move || String::new())
                .caret(false)
                .auto_focus(true)
                .paste_behavior(PasteBehavior::Report)
                .on_key_press(move |key: &str| {
                    if key_composing.get() || key == "Backspace" || key == "Enter" {
                        return;
                    }
                    if !key.is_empty() {
                        if let Some(session) = key_pane.session.as_ref() {
                            session.send_text(key);
                        }
                    }
                })
                .on_change(move |text: &str| {
                    if change_composing.get() && !text.is_empty() {
                        if let Some(session) = change_pane.session.as_ref() {
                            session.send_text(text);
                        }
                    }
                })
                .on_composition_change(move |event: &Composition| {
                    composition.set(event.composing);
                })
                // The IME places its composition and candidate windows at
                // this input's caret, so the field sits on the terminal's
                // own cursor instead of the pane's corner: Chinese input
                // then appears where the text will go.
                .absolute()
                .left(move || px(ime_at.get().0))
                .top(move || px(ime_at.get().1))
                .size(px(1.0))
                .opacity(0.0),
        )
        .child(
            view()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .w_full()
                .h_full()
                .on_context_menu(move |event: &ContextMenuEvent| {
                    menu_pane
                        .menu_at
                        .set((event.position.x.0, event.position.y.0));
                    menu_pane.menu_open.set(true);
                })
                .on_press({
                    let pane = dismiss_pane.clone();
                    move || pane.menu_open.set(false)
                })
                .on_touch({
                    let pane = pane.clone();
                    let metrics = pane_metrics.clone();
                    move |event: &TouchEvent| {
                        let Some(point) = event.points.iter().find(|point| point.changed) else {
                            return;
                        };
                        let col = ((point.location.x.0 - metrics.pad) / metrics.cell_w)
                            .floor()
                            .max(0.0) as u16;
                        let row = ((point.location.y.0 - metrics.pad) / metrics.cell_h)
                            .floor()
                            .max(0.0) as u16;
                        let (cols, rows) = pane.grid.get();
                        let cell = (
                            col.min(cols.saturating_sub(1)),
                            u32::from(row.min(rows.saturating_sub(1))),
                        );
                        match event.phase {
                            TouchPhase::Start => {
                                pane.anchor.set(Some(cell));
                                pane.dragging.set(false);
                                if let Some(session) = pane.session.as_ref() {
                                    session.clear_selection();
                                }
                            }
                            TouchPhase::Move => {
                                if let Some(anchor) = pane.anchor.get() {
                                    if anchor != cell || pane.dragging.get() {
                                        pane.dragging.set(true);
                                        if let Some(session) = pane.session.as_ref() {
                                            session.select(anchor, cell);
                                        }
                                    }
                                }
                            }
                            TouchPhase::End | TouchPhase::Cancel => {
                                pane.anchor.set(None);
                            }
                        }
                    }
                })
                .on_layout({
                    let pane = pane.clone();
                    let metrics = pane_metrics.clone();
                    move |bounds: &Bounds<Points>| {
                        pane.bounds.set((bounds.size.width.0, bounds.size.height.0));
                        let cols = metrics.cols(bounds.size.width.0);
                        let rows = metrics.rows(bounds.size.height.0);
                        if cols == 0 || rows == 0 {
                            return;
                        }
                        if pane.grid.get() != (cols, rows) {
                            pane.grid.set((cols, rows));
                            if pane.dims.get() != (cols, rows) {
                                pane.dims.set((cols, rows));
                            }
                            if let Some(session) = pane.session.as_ref() {
                                session.resize(
                                    cols,
                                    rows,
                                    metrics.cell_w as u32,
                                    metrics.cell_h as u32,
                                );
                            }
                        }
                    }
                }),
        )
        // The menu is the last child, so it sits above the full-size touch
        // layer: otherwise that layer takes the clicks meant for its rows.
        .child(when(
            move || menu_condition.menu_open.get(),
            move || context_menu_view(menu_state.clone(), menu_body.clone()),
        ))
        .into_element()
}

fn error_panel(pane: Rc<Pane>, state: Rc<AppState>) -> impl IntoElement {
    let message = pane.error.clone().unwrap_or_default();
    let name = pane.profile.name.clone();
    let close_state = state.clone();
    let retry_state = state.clone();
    let retry_profile = pane.profile.clone();
    let id = pane.id;
    let command = format!("{} {}", pane.profile.program, pane.profile.args.join(" "));

    view()
        .flex_1()
        .bg(background())
        .items_center()
        .justify_center()
        .child(
            view()
                .w(px(520.0))
                .gap_4()
                .p_4()
                .rounded(radius())
                .border_1()
                .border_color(border())
                .child(
                    text(format!("无法启动 {name}"))
                        .text_base()
                        .text_color(foreground()),
                )
                .child(text(message).text_sm().text_color(warning()))
                .child(text(command).text_xs().text_color(muted_foreground()))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(ui::primary_button("重试", move || {
                            let index = retry_state
                                .profiles
                                .iter()
                                .position(|p| p.name == retry_profile.name);
                            spawn_tab(&retry_state, index);
                            if let Some(tab) = retry_state.active_tab() {
                                retry_state.close_pane(&tab, id);
                            }
                        }))
                        .child(ui::text_button("关闭窗格", move || {
                            if let Some(tab) = close_state.active_tab() {
                                close_state.close_pane(&tab, id);
                            }
                        })),
                ),
        )
}

fn exit_strip(state: Rc<AppState>, pane: Rc<Pane>) -> impl IntoElement {
    let restart_state = state.clone();
    let restart_pane = pane.clone();
    let close_state = state.clone();
    let text_pane = pane.clone();
    let color_pane = pane.clone();
    let close_target = pane.clone();

    view()
        .h(px(28.0))
        .w_full()
        .flex_row()
        .items_center()
        .gap_3()
        .px_3()
        .border_t_1()
        .border_color(border())
        .bg(popover())
        .child(
            text(move || {
                let code = text_pane.exit.get().unwrap_or(-1);
                if code == 0 {
                    "进程已退出，代码 0".to_owned()
                } else {
                    format!("进程已退出，代码 {code}")
                }
            })
            .text_xs()
            .text_color(move || {
                if color_pane.exit.get() == Some(0) {
                    success()
                } else {
                    warning()
                }
            }),
        )
        .child(view().flex_1())
        .child(ui::text_button("重新启动", move || {
            let profile = restart_pane.profile.name.clone();
            let index = restart_state
                .profiles
                .iter()
                .position(|p| p.name == profile);
            spawn_tab(&restart_state, index);
            let tab = restart_state
                .runtimes
                .borrow()
                .iter()
                .find(|tab| tab.pane(restart_pane.id).is_some())
                .cloned();
            if let Some(tab) = tab {
                restart_state.close_pane(&tab, restart_pane.id);
            }
        }))
        .child(ui::text_button("关闭窗格", move || {
            if let Some(tab) = close_state
                .runtimes
                .borrow()
                .iter()
                .find(|tab| tab.pane(close_target.id).is_some())
                .cloned()
            {
                close_state.close_pane(&tab, close_target.id);
            }
        }))
}

fn find_bar(_state: Rc<AppState>, pane: Rc<Pane>) -> impl IntoElement {
    let query_pane = pane.clone();
    let change_query_pane = pane.clone();
    let count_pane = pane.clone();
    let next_pane = pane.clone();
    let prev_pane = pane.clone();
    let close_pane = pane.clone();
    let focus = pane.focus;

    view()
        .h(px(32.0))
        .w_full()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .border_b_1()
        .border_color(border())
        .bg(popover())
        .child(
            nsh_base::input()
                .element_ref(&pane.find_focus)
                .value(move || query_pane.find_query.get())
                .placeholder("查找…")
                .caret(true)
                .auto_focus(true)
                .on_change(move |text: &str| {
                    change_query_pane.find_query.set(text.to_owned());
                    change_query_pane.find_skip.set(0);
                    change_query_pane.find_count.set(None);
                    if let Some(session) = change_query_pane.session.as_ref() {
                        session.find(text, 1, 0);
                    }
                })
                .w(px(300.0))
                .h(px(24.0))
                .px_2()
                .text_sm()
                .text_color(foreground())
                .bg(background())
                .rounded(px(4.0)),
        )
        .child(
            text(move || match count_pane.find_count.get() {
                Some((0, _)) => "0/0".to_owned(),
                Some((total, selected)) => format!("{selected}/{total}"),
                None => "…".to_owned(),
            })
            .text_xs()
            .text_color(muted_foreground())
            .w(px(70.0)),
        )
        .child(ui::text_button("↑", move || {
            let skip = prev_pane.find_skip.get();
            let query = prev_pane.find_query.get();
            if let Some(session) = prev_pane.session.as_ref() {
                session.find(&query, -1, skip);
            }
        }))
        .child(ui::text_button("↓", move || {
            let skip = next_pane.find_skip.get() + 1;
            next_pane.find_skip.set(skip);
            let query = next_pane.find_query.get();
            if let Some(session) = next_pane.session.as_ref() {
                session.find(&query, 1, skip);
            }
        }))
        .child(ui::text_button("×", move || {
            close_pane.find_open.set(false);
            if let Some(session) = close_pane.session.as_ref() {
                session.clear_selection();
            }
            let focus = focus;
            set_timeout(Duration::from_millis(0), move || focus.focus());
        }))
}

fn empty_state(state: Rc<AppState>) -> impl IntoElement {
    view()
        .flex_1()
        .bg(background())
        .items_center()
        .justify_center()
        .gap_3()
        .child(text("没有打开的会话").text_base().text_color(foreground()))
        .child(
            text("按 Ctrl+Shift+T 新建一个标签")
                .text_sm()
                .text_color(muted_foreground()),
        )
        .child(ui::primary_button("新建标签", move || {
            spawn_tab(&state, None);
        }))
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

    bind("ctrl+shift+t", {
        let state = state.clone();
        Box::new(move || {
            spawn_tab(&state, None);
        })
    });
    bind("ctrl+shift+d", {
        let state = state.clone();
        Box::new(move || split_active(&state, SplitAxis::Row))
    });
    bind("ctrl+shift+e", {
        let state = state.clone();
        Box::new(move || split_active(&state, SplitAxis::Column))
    });
    bind("ctrl+shift+w", {
        let state = state.clone();
        Box::new(move || state.close_active_pane())
    });
    bind("ctrl+,", {
        let state = state.clone();
        Box::new(move || state.settings_open.set(true))
    });
    bind("ctrl+tab", {
        let state = state.clone();
        Box::new(move || state.activate_offset(1))
    });
    bind("ctrl+shift+tab", {
        let state = state.clone();
        Box::new(move || state.activate_offset(-1))
    });
    bind("alt+left", {
        let state = state.clone();
        Box::new(move || state.cycle_pane(-1))
    });
    bind("alt+up", {
        let state = state.clone();
        Box::new(move || state.cycle_pane(-1))
    });
    bind("alt+right", {
        let state = state.clone();
        Box::new(move || state.cycle_pane(1))
    });
    bind("alt+down", {
        let state = state.clone();
        Box::new(move || state.cycle_pane(1))
    });
    bind("ctrl+shift+c", {
        let state = state.clone();
        Box::new(move || copy_selection(&state))
    });
    bind("ctrl+shift+v", {
        let state = state.clone();
        Box::new(move || paste_clipboard(&state))
    });
    bind("ctrl+shift+f", {
        let state = state.clone();
        Box::new(move || toggle_find(&state))
    });
    bind("ctrl+shift+p", {
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
    });
    bind("ctrl+shift+u", {
        let state = state.clone();
        Box::new(move || {
            if let Some(pane) = state.active_pane() {
                if let Some(session) = pane.session.as_ref() {
                    session.select_all();
                }
            }
        })
    });
    bind("shift+pageup", {
        let state = state.clone();
        Box::new(move || {
            if let Some(session) = state.active_pane().and_then(|pane| pane.session()) {
                session.scroll(-20);
            }
        })
    });
    bind("shift+pagedown", {
        let state = state.clone();
        Box::new(move || {
            if let Some(session) = state.active_pane().and_then(|pane| pane.session()) {
                session.scroll(20);
            }
        })
    });

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

    // Terminal keys go to the active pane.
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
                let active = state.active_pane();
                if keystroke == &"enter" {
                    if state.palette_open.get() {
                        if let Some(item) = state.palette_items.get().first() {
                            run_palette(&state, item.action.clone());
                        }
                        return;
                    }
                    if let Some(pane) = active.as_ref() {
                        if pane.find_open.get() {
                            let skip = pane.find_skip.get() + 1;
                            pane.find_skip.set(skip);
                            let query = pane.find_query.get();
                            if let Some(session) = pane.session.as_ref() {
                                session.find(&query, 1, skip);
                            }
                            return;
                        }
                    }
                }
                if keystroke == &"escape" {
                    if state.settings_open.get() {
                        state.settings_open.set(false);
                        state.focus_active_pane();
                        return;
                    }
                    if state.palette_open.get() {
                        state.palette_open.set(false);
                        state.palette_query.set(String::new());
                        state.palette_items.set(state.palette_all.clone());
                        state.focus_active_pane();
                        return;
                    }
                    if let Some(pane) = active.as_ref() {
                        if pane.menu_open.get() {
                            pane.menu_open.set(false);
                            return;
                        }
                        if pane.find_open.get() {
                            close_find(pane);
                            return;
                        }
                    }
                }
                if let Some(session) = active.and_then(|pane| pane.session()) {
                    session.send_key(key, mods);
                }
            },
        );
    }
}

fn toggle_find(state: &Rc<AppState>) {
    let Some(pane) = state.active_pane() else {
        return;
    };
    if pane.find_open.get() {
        close_find(&pane);
    } else {
        pane.find_open.set(true);
        let focus = pane.find_focus;
        set_timeout(Duration::from_millis(0), move || focus.focus());
    }
}

fn close_find(pane: &Rc<Pane>) {
    pane.find_open.set(false);
    pane.find_count.set(None);
    pane.find_query.set(String::new());
    if let Some(session) = pane.session.as_ref() {
        session.clear_selection();
    }
    let focus = pane.focus;
    set_timeout(Duration::from_millis(0), move || focus.focus());
}

fn open_find(state: &Rc<AppState>) {
    let Some(pane) = state.active_pane() else {
        return;
    };
    pane.find_open.set(true);
    let focus = pane.find_focus;
    set_timeout(Duration::from_millis(0), move || focus.focus());
}

fn copy_selection(state: &Rc<AppState>) {
    if let Some(pane) = state.active_pane() {
        if let Some(session) = pane.session.as_ref() {
            let text = session.selection_text();
            if !text.is_empty() {
                set_clipboard(text);
            }
        }
    }
}

fn paste_clipboard(state: &Rc<AppState>) {
    let Some(pane) = state.active_pane() else {
        return;
    };
    let Some(session) = pane.session() else {
        return;
    };
    spawn(async move {
        let text = clipboard_string().await;
        if !text.is_empty() {
            session.send_text(&text);
        }
    });
}

fn context_menu_view(state: Rc<AppState>, pane: Rc<Pane>) -> impl IntoElement {
    let (x, y) = pane.menu_at.get();
    let close = {
        let pane = pane.clone();
        move || pane.menu_open.set(false)
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
    let close_pane = pane.clone();

    view()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(184.0))
        .p_1()
        .rounded(radius())
        .border_1()
        .border_color(border())
        .bg(popover())
        .shadow_lg()
        .child(ui::menu_row("复制", move || {
            copy_selection(&state_copy);
            close_copy();
        }))
        .child(ui::menu_row("粘贴", move || {
            paste_clipboard(&state_paste);
            close_paste();
        }))
        .child(ui::menu_row("全选", move || {
            if let Some(pane) = state_all.active_pane() {
                if let Some(session) = pane.session.as_ref() {
                    session.select_all();
                }
            }
            close_all();
        }))
        .child(ui::menu_row("查找…", move || {
            open_find(&state_find);
            close_find();
        }))
        .child(view().h(px(1.0)).my_1().bg(border()))
        .child(ui::menu_row("关闭窗格", move || {
            if let Some(tab) = state_close
                .runtimes
                .borrow()
                .iter()
                .find(|tab| tab.pane(close_pane.id).is_some())
                .cloned()
            {
                state_close.close_pane(&tab, close_pane.id);
            }
            close_close();
        }))
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
                accent()
            } else {
                surface()
            }
        })
        .transition_colors()
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
                        .text_sm()
                        .number_of_lines(1)
                        .text_color(move || {
                            if fg_state.active.get() == id {
                                accent_foreground()
                            } else {
                                muted_foreground()
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
                .label("关闭标签")
                .hover(|s| s.bg(accent()))
                .on_press(move || close_state.close(id))
                .child(
                    text("×")
                        .text_sm()
                        .text_color(muted_foreground())
                        .hover(|s| s.text_color(accent_foreground())),
                ),
        )
}

fn profile_menu(state: Rc<AppState>) -> impl IntoElement {
    let mut menu_view = menu()
        .offset(px(4.0))
        .popup_style(|s| ui::popup_surface(s).min_w(px(180.0)))
        .trigger(
            view()
                .label("新建标签（选择配置文件）")
                .size(px(22.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(|s| s.bg(accent()))
                .active(|s| s.bg(accent()))
                .child(text("⌄").text_xs().text_color(muted_foreground())),
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
                .py_2()
                .rounded(radius())
                .hover(|s| s.bg(accent()))
                .child(text(name).text_sm().text_color(foreground())),
        );
    }
    menu_view
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
        PaletteAction::SplitRight => split_active(state, SplitAxis::Row),
        PaletteAction::SplitDown => split_active(state, SplitAxis::Column),
        PaletteAction::Find => open_find(state),
        PaletteAction::Copy => copy_selection(state),
        PaletteAction::Paste => paste_clipboard(state),
        PaletteAction::SelectAll => {
            if let Some(pane) = state.active_pane() {
                if let Some(session) = pane.session.as_ref() {
                    session.select_all();
                }
            }
        }
        PaletteAction::CloseTab => state.close_active_pane(),
        PaletteAction::ScrollTop => {
            if let Some(session) = state.active_pane().and_then(|pane| pane.session()) {
                session.scroll(-100_000);
            }
        }
        PaletteAction::ScrollBottom => {
            if let Some(session) = state.active_pane().and_then(|pane| pane.session()) {
                session.scroll(100_000);
            }
        }
        PaletteAction::Settings => state.settings_open.set(true),
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
        .rounded(radius())
        .border_1()
        .border_color(border())
        .bg(popover())
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
                .text_sm()
                .text_color(foreground())
                .bg(background())
                .rounded(px(4.0)),
        )
        .child(view().gap_1().child(each(
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
                    .py_2()
                    .rounded(radius())
                    .hover(|s| s.bg(accent()))
                    .active(|s| s.bg(accent()))
                    .on_press(move || {
                        run_palette(&row_state, action.clone());
                    })
                    .child(
                        text(move || item.get().title.clone())
                            .text_sm()
                            .text_color(foreground()),
                    )
                    .child(view().flex_1())
                    .child(
                        text(move || item.get().hint.clone())
                            .text_xs()
                            .text_color(muted_foreground()),
                    )
            },
        )))
}

fn settings_overlay(state: Rc<AppState>) -> impl IntoElement {
    let current = state.settings.get();
    let close = {
        let state = state.clone();
        move || state.settings_open.set(false)
    };
    let close_btn = close.clone();

    let size_up = {
        let state = state.clone();
        move || change_font_size(&state, 1.0)
    };
    let size_down = {
        let state = state.clone();
        move || change_font_size(&state, -1.0)
    };
    let size_text = {
        let state = state.clone();
        move || format!("{} pt", state.settings.get().font_size)
    };

    let mut family_rows = view().gap_1();
    for family in settings::FONT_FAMILIES {
        let state_row = state.clone();
        let selected = current.font_family == family;
        family_rows = family_rows.child(ui::setting_row(family, selected, move || {
            let mut next = state_row.settings.get();
            next.font_family = family.to_owned();
            save_settings(&state_row, next);
        }));
    }

    let mut scheme_rows = view().gap_1();
    for candidate in settings::schemes() {
        let selected = candidate.name == current.scheme;
        let swatch = scheme_swatch(&candidate);
        let name = candidate.name.clone();
        scheme_rows = scheme_rows.child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(radius())
                .hover(|s| s.bg(accent()))
                .active(|s| s.bg(accent()))
                .on_press({
                    let state = state.clone();
                    let name = candidate.name.clone();
                    move || apply_scheme(&state, &name)
                })
                .child(swatch)
                .child(text(name).text_sm().text_color(foreground()))
                .child(view().flex_1())
                .child(
                    text(if selected { "✓" } else { "" })
                        .text_sm()
                        .text_color(primary()),
                ),
        );
    }

    let mut profile_rows = view().gap_1();
    for profile in state.profiles.iter() {
        let name = profile.name.clone();
        let selected = state.settings.get().default_profile == name;
        profile_rows = profile_rows.child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(radius())
                .hover(|s| s.bg(accent()))
                .active(|s| s.bg(accent()))
                .on_press({
                    let state = state.clone();
                    let name = profile.name.clone();
                    move || {
                        let mut next = state.settings.get();
                        next.default_profile = name.clone();
                        save_settings(&state, next);
                    }
                })
                .child(text(name).text_sm().text_color(foreground()))
                .child(view().flex_1())
                .child(
                    text(if selected { "默认" } else { "" })
                        .text_xs()
                        .text_color(primary()),
                ),
        );
    }

    view()
        .absolute()
        .left(px(120.0))
        .top(px(56.0))
        .w(px(560.0))
        .p_4()
        .gap_4()
        .rounded(radius())
        .border_1()
        .border_color(border())
        .bg(popover())
        .shadow_lg()
        .child(
            view()
                .flex_row()
                .items_center()
                .child(text("设置").text_base().text_color(foreground()))
                .child(view().flex_1())
                .child(ui::text_button("关闭", move || close_btn())),
        )
        .child(
            view().gap_2().child(ui::section_label("字号")).child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(step_button("−", "减小字号", size_down))
                    .child(
                        text(size_text)
                            .text_sm()
                            .text_color(foreground())
                            .w(px(60.0)),
                    )
                    .child(step_button("+", "增大字号", size_up)),
            ),
        )
        .child(
            view()
                .gap_2()
                .child(ui::section_label("字体"))
                .child(family_rows),
        )
        .child(
            view()
                .gap_2()
                .child(ui::section_label("配色方案"))
                .child(scheme_rows),
        )
        .child(
            view()
                .gap_2()
                .child(ui::section_label("默认配置文件"))
                .child(profile_rows),
        )
}

/// A scheme's own colours, as the sample that identifies it: its background
/// and three ANSI colours, which are data rather than theme tokens.
fn scheme_swatch(scheme: &settings::Scheme) -> impl IntoElement {
    view()
        .flex_row()
        .size(px(28.0))
        .rounded(px(4.0))
        .overflow_hidden()
        .border_1()
        .border_color(border())
        .bg(rgb(scheme.background))
        .child(view().flex_1().h_full().bg(rgb(scheme.palette[1])))
        .child(view().flex_1().h_full().bg(rgb(scheme.palette[2])))
        .child(view().flex_1().h_full().bg(rgb(scheme.palette[4])))
}

fn step_button(
    label: &'static str,
    accessible: &'static str,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .label(accessible)
        .size(px(26.0))
        .items_center()
        .justify_center()
        .rounded(radius())
        .border_1()
        .border_color(border())
        .hover(|s| s.bg(accent()))
        .active(|s| s.bg(accent()))
        .on_press(on_press)
        .child(text(label).text_sm().text_color(foreground()))
}

fn change_font_size(state: &Rc<AppState>, delta: f32) {
    let mut next = state.settings.get();
    let size = (next.font_size + delta).clamp(9.0, 32.0);
    if (size - next.font_size).abs() < f32::EPSILON {
        return;
    }
    next.font_size = size;
    save_settings(state, next);
}

fn apply_scheme(state: &Rc<AppState>, name: &str) {
    let mut next = state.settings.get();
    next.scheme = name.to_owned();
    save_settings(state, next);
}

/// Applies settings everywhere they reach: metrics, the PTY sizes and every
/// running terminal's palette.
fn save_settings(state: &Rc<AppState>, next: settings::Settings) {
    let scheme = settings::scheme(&next.scheme);
    state.metrics.set(grid::Metrics::new(
        next.font_size,
        next.line_height,
        next.padding,
        &next.font_family,
    ));
    state.scheme.set(scheme.clone());
    state.settings.set(next.clone());

    let metrics = state.metrics.get();
    for tab in state.runtimes.borrow().iter() {
        for pane in tab.panes.borrow().iter() {
            let (width, height) = pane.bounds.get();
            let cols = metrics.cols(width);
            let rows = metrics.rows(height);
            if cols > 0 && rows > 0 {
                pane.grid.set((cols, rows));
                pane.dims.set((cols, rows));
            }
            if let Some(session) = pane.session.as_ref() {
                session.set_scheme(&scheme);
                if cols > 0 && rows > 0 {
                    session.resize(cols, rows, metrics.cell_w as u32, metrics.cell_h as u32);
                }
            }
        }
    }

    let _ =
        nsh_core::storage::Storage::set_item(settings::Settings::storage_key(), next.serialize());
}

/// Reads the stored settings; anything unreadable falls back to the defaults,
/// so a corrupt entry never keeps the app from starting.
fn load_settings() -> settings::Settings {
    nsh_core::storage::Storage::get_item(settings::Settings::storage_key())
        .map(|text| settings::Settings::parse(&text))
        .unwrap_or_default()
}
