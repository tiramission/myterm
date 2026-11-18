use super::*;

pub(super) fn menu_row(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    theme: &gpui_kit::component::Theme,
) -> Stateful<Div> {
    let hover = theme.muted;
    let text = if enabled {
        theme.foreground
    } else {
        theme.muted_foreground
    };

    div()
        .id(ElementId::Name(id.into()))
        .flex()
        .flex_row()
        .items_center()
        .h(px(28.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .text_size(px(12.0))
        .text_color(text)
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |this| this.bg(hover))
        })
        .child(label)
}

#[derive(Clone, Copy)]
pub struct InstallState {
    /// This process is the installed copy.
    pub installed: bool,
    /// A copy exists at the install location, but this is not it.
    pub present: bool,
    /// An install or uninstall is running.
    pub busy: bool,
}

impl InstallState {
    pub(super) fn summary(self) -> &'static str {
        if self.busy {
            return "处理中…";
        }
        match (self.installed, self.present) {
            (true, _) => "已安装",
            (false, true) => "已安装（当前不是它）",
            (false, false) => "未安装",
        }
    }
}

/// The button that installs, or removes, this app.
pub(super) fn install_action(
    theme: &gpui_kit::component::Theme,
    install: InstallState,
    entity: Entity<Workspace>,
) -> AnyElement {
    if install.busy {
        return div().into_any_element();
    }

    let accent = theme.ring;
    let muted = theme.muted_foreground;
    let danger = theme.danger;
    let (label, color, destructive) = if install.installed {
        ("卸载", danger, true)
    } else if install.present {
        // A copy exists but this is not it. Offering "install" would silently
        // overwrite it; the honest action is to remove what is there first.
        ("重新安装", accent, false)
    } else {
        ("安装到本机", accent, false)
    };

    div()
        .id("install-action")
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .h(px(28.0))
        .w_full()
        .rounded(px(7.0))
        // A fill rather than an outline, like every other button in the
        // window: the card has no rules left, and a bordered button would be
        // the one rectangle drawn in it.
        .bg(color.opacity(0.12))
        .text_size(px(11.5))
        .text_color(color)
        .cursor_pointer()
        .hover(move |this| this.bg(color.opacity(0.2)))
        .on_click(move |_: &ClickEvent, _, cx| {
            // Called on the workspace directly rather than dispatched: the card
            // lives in a popover, which is painted in a deferred layer outside
            // the window's element tree, so an action dispatched here would
            // bubble through nodes that never see it.
            let _ = (destructive, muted);
            entity.update(cx, |workspace, cx| workspace.run_install(cx));
        })
        .child(label)
        .into_any_element()
}

/// The version as a `&'static str`, so the rows above can take plain strings
/// rather than each owning a `String`.
pub(super) fn version_label() -> &'static str {
    // `current_version` is `env!`-derived, so this is one short string per
    // process; the leak is deliberate and bounded.
    Box::leak(format!("v{}", update::current_version()).into_boxed_str())
}

/// The terminal library's version, read from the crate that links it.
///
/// The about card used to spell this out by hand, and it drifted: it still said
/// 1.0.0 while the vendored archive was 1.0.1. `vt-sys` publishes the number its
/// `build.rs` builds against, so this row now describes the library that is
/// actually linked into this binary.
pub(super) fn ghostty_label() -> &'static str {
    Box::leak(format!("libghostty-vt {}", vt::GHOSTTY_VERSION).into_boxed_str())
}

/// A small control in a pane header.
///
/// Ghost by default: the glyph is drawn straight on the header and only gains a
/// surface under the pointer, which is the rule the title bar's buttons follow.
/// They used to be filled chips, and four grey blocks in an otherwise quiet row
/// read as a segmented control — a group of related choices — when they are
/// four unrelated commands.
///
/// `destructive` gives the close button the same red wash the window's own
/// close button uses, so the one action that ends the pane is not the same
/// colour as the three that rearrange it.
pub(super) fn pane_action(
    id: impl Into<ElementId>,
    icon: &'static [u8],
    theme: &gpui_kit::component::Theme,
    destructive: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    pane_action_marked(id, icon, theme, destructive, false, on_click)
}

/// The same button, with the "this mode is on" fill the zoom wears while it is
/// zoomed.
pub(super) fn pane_action_marked(
    id: impl Into<ElementId>,
    icon: &'static [u8],
    theme: &gpui_kit::component::Theme,
    destructive: bool,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let wash = if destructive {
        theme.danger.opacity(0.16)
    } else {
        theme.muted
    };
    let accent_wash = theme.ring.opacity(0.13);

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(24.0))
        // The same rounded square as the title bar's buttons, one step tighter:
        // 24px with a 6px corner. A circle inside a rounded card was the one
        // shape in the window that belonged to no family.
        .rounded(px(6.0))
        // Nothing at rest: the header already carries the name, the ordinal, the
        // title and the state, and a row of four tinted blocks was the heaviest
        // thing in it. The surface appears when the pointer is on the button,
        // which is also the only moment the button is revealed at all.
        .cursor_pointer()
        // A mode that is on says so with a fill, the way the palette's selected
        // row does — the button is the only thing in the header that has a
        // state to be in.
        .when(active, |this| this.bg(accent_wash))
        .when(!active, |this| this.hover(move |this| this.bg(wash)))
        .on_click(on_click)
        .child(icon_data(
            icon,
            16.0,
            if active {
                theme.ring
            } else {
                theme.muted_foreground
            },
        ))
}

/// A choice inside the settings sheet.
///
/// Selected is carried by fill, border and weight together rather than by a
/// single colour: the sheet has to stay readable to someone who cannot tell the
/// accent green from the muted grey.
pub(super) fn setting_row(
    label: &'static str,
    hint: &'static str,
    theme: &gpui_kit::component::Theme,
    control: AnyElement,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .py(px(11.0))
        .border_b_1()
        .border_color(theme.border.opacity(0.45))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .min_w_0()
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(theme.foreground)
                        .child(label),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(theme.muted_foreground)
                        .child(hint),
                ),
        )
        .child(control)
}

/// A rounded icon-only button.
///
/// Icon-only controls carry no visible label, so the caller must pass an id that
/// also reads as a name. `size` is the frame, and the glyph follows it: the title
/// bar uses 28, the sidebar's shorter heading uses 24.
pub(super) fn icon_button(
    id: &'static str,
    icon: &'static [u8],
    size: f32,
    theme: &gpui_kit::component::Theme,
    _cx: &mut Context<Workspace>,
) -> Stateful<Div> {
    // A glyph is about four sevenths of its frame, which is the ratio the title
    // bar's buttons already use: 16 px inside 28.
    let glyph = icon_data(icon, (size * 0.57).round(), theme.muted_foreground);

    // A rounded square, not a circle. The circle was left over from a bar whose
    // other controls were pills; with tabs gone from the bar there is nothing to
    // echo, and a 28px circle at this size reads as a dot rather than as a
    // button. Eight pixels is the same corner the app mark wears one button
    // away, so the row looks like one row.
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(move |this| this.bg(theme.muted))
        .child(glyph)
}

/// Draws an inline SVG, for glyphs the bundled assets cannot provide.
///
/// The size is a parameter because the same drawing appears at different
/// scales: a toolbar glyph is sixteen pixels, while the cross inside a tab has
/// to be smaller than that or it fills the circle it sits in.
pub(super) fn icon_data(svg: &'static [u8], size: f32, color: Hsla) -> AnyElement {
    gpui_kit::component::Icon::default()
        .data(svg)
        .with_size(px(size))
        .text_color(color)
        .into_any_element()
}

/// One of the three caption buttons at the trailing edge of the title bar.
///
/// These keep their own shape: a caption button is a rectangle flush to the
/// corner in Windows, and rounding it would make the window feel like it is
/// missing its edge. Only the tint follows the palette — the close button gets a
/// red wash on hover, the others the same grey the rest of the bar uses.
pub(super) fn window_button(
    id: &'static str,
    icon: &'static [u8],
    area: WindowControlArea,
    destructive: bool,
    theme: &gpui_kit::component::Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let color = theme.muted_foreground;
    let wash = if destructive {
        theme.danger.opacity(0.14)
    } else {
        theme.muted
    };
    // The cross stays the same grey as its neighbours and only the wash turns
    // red: a permanently red close button draws the eye to the one control the
    // user is least likely to want.
    let glyph = icon_data(icon, 16.0, color);

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(42.0))
        .h(px(32.0))
        .rounded(px(7.0))
        .window_control_area(area)
        .cursor_pointer()
        .hover(move |this| this.bg(wash))
        .on_click(on_click)
        .child(glyph)
}

pub(super) fn key_input(keystroke: &Keystroke) -> Option<vt::KeyInput<'_>> {
    let key = map_key(&keystroke.key)?;
    let mut mods = 0u16;
    if keystroke.modifiers.shift {
        mods |= vt::MODS_SHIFT;
    }
    if keystroke.modifiers.control {
        mods |= vt::MODS_CTRL;
    }
    if keystroke.modifiers.alt {
        mods |= vt::MODS_ALT;
    }

    Some(vt::KeyInput {
        key,
        mods,
        utf8: keystroke.key_char.as_deref(),
    })
}

/// Maps GPUI's key names onto the parser's key codes.
pub(super) fn map_key(key: &str) -> Option<i32> {
    use vt::key;

    let code = match key {
        "enter" => key::ENTER,
        "tab" => key::TAB,
        "backspace" => key::BACKSPACE,
        "delete" => key::DELETE,
        "escape" => key::ESCAPE,
        "up" => key::ARROW_UP,
        "down" => key::ARROW_DOWN,
        "left" => key::ARROW_LEFT,
        "right" => key::ARROW_RIGHT,
        "home" => key::HOME,
        "end" => key::END,
        "pageup" => key::PAGE_UP,
        "pagedown" => key::PAGE_DOWN,
        "insert" => key::INSERT,
        "space" => key::SPACE,
        "f1" => key::F1,
        "f2" => key::F2,
        "f3" => key::F3,
        "f4" => key::F4,
        "f5" => key::F5,
        "f6" => key::F6,
        "f7" => key::F7,
        "f8" => key::F8,
        "f9" => key::F9,
        "f10" => key::F10,
        "f11" => key::F11,
        "f12" => key::F12,
        other => {
            let mut chars = other.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            match ch.to_ascii_lowercase() {
                'a' => key::A,
                'b' => key::B,
                'c' => key::C,
                'd' => key::D,
                'e' => key::E,
                'f' => key::F,
                'g' => key::G,
                'h' => key::H,
                'i' => key::I,
                'j' => key::J,
                'k' => key::K,
                'l' => key::L,
                'm' => key::M,
                'n' => key::N,
                'o' => key::O,
                'p' => key::P,
                'q' => key::Q,
                'r' => key::R,
                's' => key::S,
                't' => key::T,
                'u' => key::U,
                'v' => key::V,
                'w' => key::W,
                'x' => key::X,
                'y' => key::Y,
                'z' => key::Z,
                '0' => key::DIGIT_0,
                '1' => key::DIGIT_1,
                '2' => key::DIGIT_2,
                '3' => key::DIGIT_3,
                '4' => key::DIGIT_4,
                '5' => key::DIGIT_5,
                '6' => key::DIGIT_6,
                '7' => key::DIGIT_7,
                '8' => key::DIGIT_8,
                '9' => key::DIGIT_9,
                '-' => key::MINUS,
                '=' => key::EQUAL,
                ',' => key::COMMA,
                '.' => key::PERIOD,
                '/' => key::SLASH,
                ';' => key::SEMICOLON,
                '\'' => key::QUOTE,
                '[' => key::BRACKET_LEFT,
                ']' => key::BRACKET_RIGHT,
                '\\' => key::BACKSLASH,
                '`' => key::BACKQUOTE,
                _ => return None,
            }
        }
    };
    Some(code)
}

/// The title a pane wants its tab to show.
///
/// A shell usually sets one through OSC, but Windows shells often set the full
/// executable path, which is not a useful tab label. Only the final component is
/// kept, and the generic name is used when the shell says nothing.
pub(super) fn tab_title(pane: &Pane) -> Option<String> {
    let title = pane.session.frame().title.trim();
    if title.is_empty() {
        return None;
    }

    let name = title
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(title)
        .trim_end_matches(".exe");

    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

/// The directory a shell is working in, when it reports one.
///
/// There is no process inspection here: the shell reports it through OSC.
pub(super) fn pane_cwd(pane: &Pane) -> Option<String> {
    // The announced directory is the real one; the title is only a guess that
    // happens to work for shells which title themselves with a path.
    if let Some(pwd) = pane.session.pwd() {
        return Some(shorten_path(&pwd));
    }

    let title = pane.session.frame().title.trim();
    if title.is_empty() || title.to_ascii_lowercase().ends_with(".exe") {
        return None;
    }
    let looks_like_path = title.contains('\\') || title.contains('/') || title.starts_with('~');
    if !looks_like_path {
        return None;
    }
    Some(shorten_path(title))
}

/// Replaces the user's home directory with `~`, which is how every shell prompt
/// already writes it.
pub(super) fn shorten_path(path: &str) -> String {
    static HOME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let home = HOME.get_or_init(|| std::env::var("USERPROFILE").ok());
    match home {
        Some(home) if !home.is_empty() => {
            let lower = path.to_ascii_lowercase();
            let home_lower = home.to_ascii_lowercase();
            if lower.starts_with(&home_lower) {
                let rest = &path[home.len()..];
                return if rest.is_empty() {
                    "~".to_owned()
                } else {
                    format!("~{}", rest.replace('\\', "/"))
                };
            }
            path.replace('\\', "/")
        }
        _ => path.to_owned(),
    }
}

/// The two lines a row in the column shows.
///
/// One concise sidebar name: a real shell title, otherwise the current folder.
pub(super) fn pane_row_title(pane: &Pane) -> String {
    let cwd = pane_cwd(pane);
    let program = pane_label(pane);
    let distribution = pty_command::wsl_distribution_name(&pane.command);

    // A title worth showing is one the shell chose: not the program's own name,
    // and not a path (Windows shells title themselves with the exe's path).
    let reported = pane.session.frame().title.trim();
    let is_path = reported.contains('\\') || reported.contains('/');
    let interesting = !reported.is_empty()
        && !is_path
        && !reported.eq_ignore_ascii_case(&program)
        && !distribution
            .as_deref()
            .is_some_and(|name| reported.eq_ignore_ascii_case(name))
        && !reported.to_ascii_lowercase().ends_with(".exe");

    match (interesting, cwd) {
        (true, _) => reported
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(reported)
            .trim_end_matches(".exe")
            .to_owned(),
        (false, Some(cwd)) => directory_name(&cwd),
        (false, None) => distribution.map_or(program, |instance| format!("WSL · {instance}")),
    }
}

/// The last segment of a shortened path — what a person would call the place.
pub(super) fn directory_name(path: &str) -> String {
    // `~` on its own is home, and its last segment is the whole string.
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit('/').next() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => trimmed.to_owned(),
    }
}

/// The one line of context a pane header carries: where the shell is, or the
/// title it set for itself when that title is not just its own path.
pub(super) fn pane_meta(
    pane: &Pane,
    branches: &BTreeMap<String, Option<String>>,
) -> Option<String> {
    let Some(cwd) = pane_cwd(pane) else {
        return pane_detail(pane).filter(|detail| !detail.to_ascii_lowercase().ends_with(".exe"));
    };
    Some(match branches.get(&cwd).cloned().flatten() {
        // The branch rides with the directory, the way the prototype writes it:
        // `~/myterm · main`. It is the one line that tells two shells in the
        // same checkout apart from two shells in different ones.
        Some(branch) => format!("{cwd} · {branch}"),
        None => cwd,
    })
}

/// The branch a directory is on, when it is inside a git checkout.
///
/// This reads `.git/HEAD` and walks up a few levels, which is the whole of what
/// the pill in the bar needs. It does not shell out to git: spawning a process
/// every couple of seconds to paint one word is a poor trade, and a checkout
/// without git installed still has a HEAD file.
pub(super) fn git_branch(directory: &str) -> Option<String> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let start = if let Some(rest) = directory.strip_prefix('~') {
        format!("{home}{rest}")
    } else {
        directory.to_owned()
    };

    let mut path = std::path::PathBuf::from(start.replace('/', "\\"));
    for _ in 0..8 {
        let head = path.join(".git").join("HEAD");
        if let Ok(text) = std::fs::read_to_string(&head) {
            let text = text.trim();
            if let Some(reference) = text.strip_prefix("ref:") {
                let name = reference.trim().rsplit('/').next().unwrap_or_default();
                return (!name.is_empty()).then(|| name.to_owned());
            }
            // A detached HEAD holds a bare commit id; the short form is what
            // every prompt shows for it.
            return Some(text.chars().take(7).collect());
        }
        if !path.pop() {
            break;
        }
    }
    None
}

/// The name a pane shows in its header.
///
/// The shell's OSC title when it sets one, otherwise the program it was started
/// with. Both end up with any path stripped, because a pane is named by what it
/// is, not by where it lives.
/// Anything the pane's title says beyond its name.
///
/// A shell sets the title to a path often enough that the difference is worth
/// showing; when it only repeats the program name, there is nothing to add and
/// the header keeps its space.
pub(super) fn pane_detail(pane: &Pane) -> Option<String> {
    let title = pane.session.frame().title.trim();
    let name = pane_label(pane);

    (!title.is_empty() && title != name).then(|| title.to_owned())
}

pub(super) fn pane_label(pane: &Pane) -> String {
    tab_title(pane).unwrap_or_else(|| pty_command::display_name(&pane.command))
}

/// The grid size a freshly opened window should start with.
pub(super) fn initial_grid(window: &Window) -> (u16, u16) {
    let bounds = window.bounds();
    // The first real resize corrects this; it only has to be large enough that
    // the shell starts with a usable width.
    let cols = (bounds.size.width.as_f32() / 8.0).max(20.0) as u16;
    let rows = (bounds.size.height.as_f32() / 18.0).max(6.0) as u16;
    (cols, rows)
}

/// The window and taskbar icon, decoded from the bundled PNG.
///
/// A missing or unreadable file leaves the window with the default icon rather
/// than refusing to start, since the icon is decoration and the shell is not.
pub(super) fn window_icon() -> Option<std::sync::Arc<image::RgbaImage>> {
    const PNG: &[u8] = include_bytes!("../../../icon.png");
    match image::load_from_memory_with_format(PNG, image::ImageFormat::Png) {
        Ok(image) => Some(std::sync::Arc::new(image.to_rgba8())),
        Err(error) => {
            log::warn!("could not decode the window icon: {error}");
            None
        }
    }
}

/// Applies the Mica material and the dark frame to the window.
///
/// Two things have to line up for the glass to read correctly. Mica takes its
/// tint from the window's light/dark mode, and GPUI decides that from the OS
/// setting at creation time, so a user on a light desktop running this dark
/// terminal would get a pale bar over a black grid. And `window_background` is
/// carried through `WindowOptions` but not applied by the Windows backend, so
/// the material has to be requested directly.
#[cfg(target_os = "windows")]
pub(super) fn apply_window_material(window: &Window, light: bool) {
    use raw_window_handle::RawWindowHandle;
    use std::ffi::c_void;

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
    /// `DWMSBT_MAINWINDOW`: the Mica backdrop.
    const DWMSBT_MAINWINDOW: i32 = 2;

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut c_void,
            attribute: u32,
            value: *const c_void,
            size: u32,
        ) -> i32;
    }

    // `Window` has its own `window_handle`, so the trait method is named in full.
    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as *mut c_void;

    // SAFETY: the handle comes from the live window and each value outlives its
    // call. A failure only costs the material or the frame tint, never the app.
    unsafe {
        let dark: i32 = if light { 0 } else { 1 };
        let status = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
        if status != 0 {
            log::debug!("could not force the dark window frame: {status}");
        }

        let backdrop = DWMSBT_MAINWINDOW;
        let status = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &backdrop as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
        if status != 0 {
            log::debug!("could not apply the Mica backdrop: {status}");
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub(super) fn apply_window_material(_window: &Window, _light: bool) {}

/// Whether the desktop is using the light theme.
///
/// Read before the event loop starts, because the first shell is spawned there
/// and needs the palette it will be drawn with. The registry key is the one
/// Settings writes; a missing key means light, which is the Windows default.
#[cfg(target_os = "windows")]
pub(super) fn system_prefers_light() -> bool {
    use std::ffi::c_void;

    const HKEY_CURRENT_USER: *mut c_void = 0x8000_0001u32 as isize as *mut c_void;
    const RRF_RT_REG_DWORD: u32 = 0x0000_0010;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            key: *mut c_void,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut c_void,
            size: *mut u32,
        ) -> i32;
    }

    // Both strings are NUL-terminated and live for the whole call.
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
        .encode_utf16()
        .collect();
    let value: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();

    let mut data: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: the buffers are sized as declared and outlive the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut c_void,
            &mut size,
        )
    };

    status != 0 || data != 0
}

#[cfg(not(target_os = "windows"))]
pub(super) fn system_prefers_light() -> bool {
    true
}

/// The font the terminal grid is drawn with.
///
/// Maple Mono NF CN is a programming face with CJK coverage and Nerd Font
/// glyphs, so box drawing, powerline separators and Chinese text all come from
/// one font instead of falling back mid-cell.
pub(super) const TERMINAL_FONT: &str = "Maple Mono NF CN";

/// Points the theme's monospace family at [`TERMINAL_FONT`].
///
/// Both registered theme configs are rewritten, not just the live family: a
/// mode change re-applies the config, and a config that says nothing about the
/// mono family would put the old one back.
/// Applies the terminal's font family, and its size when one was chosen.
///
/// Kept in one place because a theme change re-reads the library's theme file:
/// anything set here has to be re-applied after every mode switch, or the grid
/// drifts back to the library's monospace.
pub(super) fn apply_terminal_font(cx: &mut App, size: Option<f32>) {
    gpui_kit::component::Theme::update(cx, |theme| {
        // The library's theme files store the mono size as points, while the
        // resolved theme carries pixels. Both are written here so the two
        // cannot disagree after a mode change.
        let mut dark = (*theme.dark_theme).clone();
        dark.mono_font_family = Some(TERMINAL_FONT.into());
        if let Some(size) = size {
            dark.mono_font_size = Some(size);
        }
        theme.dark_theme = std::rc::Rc::new(dark);

        let mut light = (*theme.light_theme).clone();
        light.mono_font_family = Some(TERMINAL_FONT.into());
        if let Some(size) = size {
            light.mono_font_size = Some(size);
        }
        theme.light_theme = std::rc::Rc::new(light);

        theme.mono_font_family = TERMINAL_FONT.into();
        if let Some(size) = size {
            theme.mono_font_size = gpui_kit::px(size);
        }
    });
}
