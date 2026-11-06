//! The app's styled layer: one place that turns the theme's tokens into the
//! controls and surfaces the rest of the app composes. Colors come from the
//! theme by role; nothing here names a literal color, so both appearances and
//! an embedding app's tokens reach every screen.

use nsh_base::theme;
use nsh_core::element::{AdaptiveColor, StyleRefinement};
use nsh_core::prelude::*;

/// The window and pane background.
pub fn background() -> AdaptiveColor {
    theme().background
}

/// Primary text.
pub fn foreground() -> AdaptiveColor {
    theme().foreground
}

/// Secondary text: hints, captions, placeholders.
pub fn muted_foreground() -> AdaptiveColor {
    theme().muted_foreground
}

/// A quiet surface: tab strips, status bars, headers.
pub fn surface() -> AdaptiveColor {
    theme().card
}

/// Text and icons on [`surface`].
pub fn surface_foreground() -> AdaptiveColor {
    theme().card_foreground
}

/// Popovers, menus, dialogs.
pub fn popover() -> AdaptiveColor {
    theme().popover
}

/// What is hovered, pressed or focused.
pub fn accent() -> AdaptiveColor {
    theme().accent
}

/// Text on [`accent`].
pub fn accent_foreground() -> AdaptiveColor {
    theme().accent_foreground
}

/// Borders and dividers.
pub fn border() -> AdaptiveColor {
    theme().border
}

/// The main action and the focused pane.
pub fn primary() -> AdaptiveColor {
    theme().primary
}

/// Text and icons on [`primary`].
pub fn primary_foreground() -> AdaptiveColor {
    theme().primary_foreground
}

/// Errors and destructive commands.
pub fn destructive() -> AdaptiveColor {
    theme().destructive
}

/// A process that ended well.
pub fn success() -> AdaptiveColor {
    theme().color("success").unwrap_or_else(|| theme().primary)
}

/// A process that ended badly, and warnings.
pub fn warning() -> AdaptiveColor {
    theme()
        .color("warning")
        .unwrap_or_else(|| theme().destructive)
}

/// The corner radius of controls and surfaces.
pub fn radius() -> Points {
    theme().radius
}

/// A menu row: compact, one line, highlighted while hovered or pressed.
pub fn menu_row(label: impl Into<SharedString>, on_press: impl Fn() + 'static) -> impl IntoElement {
    view()
        .px_3()
        .py_1()
        .rounded(radius())
        .hover(|s| s.bg(accent()))
        .active(|s| s.bg(accent()))
        .on_press(on_press)
        .child(text(label.into()).text_sm().text_color(foreground()))
}

/// An icon-only button: a fixed square frame, with an accessible name.
pub fn icon_button(
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
        .hover(|s| s.bg(accent()))
        .active(|s| s.bg(accent()))
        .on_press(on_press)
        .child(text(label).text_base().text_color(muted_foreground()))
}

/// A quiet text button for a toolbar or an inline action.
pub fn text_button(
    label: impl Into<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .px_2()
        .py_1()
        .rounded(px(4.0))
        .hover(|s| s.bg(accent()))
        .active(|s| s.bg(accent()))
        .on_press(on_press)
        .child(
            text(label.into())
                .text_sm()
                .text_color(surface_foreground()),
        )
}

/// The one emphasized action in a decision area.
pub fn primary_button(
    label: impl Into<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .px_3()
        .py_2()
        .rounded(radius())
        .bg(primary())
        .hover(|s| s.bg(primary().opacity(0.9)))
        .active(|s| s.bg(primary().opacity(0.8)))
        .on_press(on_press)
        .child(
            text(label.into())
                .text_sm()
                .text_color(primary_foreground()),
        )
}

/// The shared popup surface: menus, palettes and dialogs all wear it.
pub fn popup_surface(s: StyleRefinement) -> StyleRefinement {
    s.p_1()
        .rounded(radius())
        .border_1()
        .border_color(border())
        .bg(popover())
        .shadow_lg()
}

/// A short state, count or classification.
pub fn badge(label: impl Into<SharedString>, tone: AdaptiveColor) -> impl IntoElement {
    view()
        .px_2()
        .py(px(2.0))
        .rounded(px(4.0))
        .bg(tone.opacity(0.16))
        .child(text(label.into()).text_xs().text_color(tone))
}

/// A settings row: a label, an optional value, and a selected mark.
pub fn setting_row(
    label: impl Into<SharedString>,
    selected: bool,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let label = label.into();
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .rounded(radius())
        .hover(|s| s.bg(accent()))
        .active(|s| s.bg(accent()))
        .on_press(on_press)
        .child(text(label).text_sm().text_color(foreground()))
        .child(view().flex_1())
        .child(
            text(if selected { "✓" } else { "" })
                .text_sm()
                .text_color(primary()),
        )
}

/// A section label inside a panel.
pub fn section_label(label: impl Into<SharedString>) -> impl IntoElement {
    text(label.into()).text_xs().text_color(muted_foreground())
}

/// The scrim behind a modal overlay: dims the content under it and closes it
/// when pressed.
pub fn backdrop(on_press: impl Fn() + 'static) -> impl IntoElement {
    view()
        .absolute()
        .left(px(0.0))
        .top(px(0.0))
        .w_full()
        .h_full()
        .bg(nsh_core::color::black().opacity(0.45))
        .on_press(on_press)
}

/// A keyboard shortcut, shown as a small chip beside the command it runs.
pub fn shortcut_chip(keys: impl IntoReactive<SharedString>) -> impl IntoElement {
    view()
        .px_1()
        .py(px(2.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(border())
        .child(text(keys).text_xs().text_color(muted_foreground()))
}

/// A title inside a panel, with an optional one-line description under it.
pub fn panel_title(title: impl Into<SharedString>) -> impl IntoElement {
    text(title.into())
        .text_lg()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(foreground())
}

/// A description under a [`panel_title`].
pub fn panel_subtitle(text_value: impl Into<SharedString>) -> impl IntoElement {
    text(text_value.into())
        .text_sm()
        .text_color(muted_foreground())
}

/// A short cut of the terminal's own colors, used where a scheme is chosen:
/// its background with three of its ANSI colors. These are data, not tokens.
pub fn scheme_chip(background: u32, colors: [u32; 3]) -> impl IntoElement {
    view()
        .flex_row()
        .size(px(28.0))
        .rounded(px(4.0))
        .overflow_hidden()
        .border_1()
        .border_color(border())
        .bg(rgb(background))
        .child(view().flex_1().h_full().bg(rgb(colors[0])))
        .child(view().flex_1().h_full().bg(rgb(colors[1])))
        .child(view().flex_1().h_full().bg(rgb(colors[2])))
}
