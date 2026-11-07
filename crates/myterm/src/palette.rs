//! The app's colour scheme.
//!
//! One set of tokens drives everything the app draws: the title bar, the panes,
//! the terminal grid and the ANSI palette. The scheme is a warm, slightly olive
//! neutral with a single lime accent, and it keeps the same structure in both
//! modes — four surface steps, three text steps, one hairline — so a screen built
//! from these reads the same whichever mode it is in.
//!
//! The values are installed into the component theme rather than threaded
//! through every widget, so the few library widgets the app uses land on the same
//! palette for free, and the many existing `theme.<token>` reads keep working.

use gpui_kit::*;

use vt::Rgb;

/// Raw `0xRRGGBB` values for one mode.
struct Tokens {
    /// The terminal grid: the deepest surface in the app.
    bg: u32,
    /// Chrome: the title bar and the space between panes.
    bg_soft: u32,
    /// Raised surfaces: the selected tab, popovers.
    surface: u32,
    /// Every hairline.
    stroke: u32,
    text: u32,
    text_2: u32,
    text_3: u32,
    brand: u32,
    /// The accent at the opacity used for tints and washes.
    brand_soft: u32,
    pill: u32,
    red: u32,
    amber: u32,
    sky: u32,
}

const DARK: Tokens = Tokens {
    bg: 0x111310,
    bg_soft: 0x161915,
    surface: 0x191c17,
    stroke: 0x2a2f27,
    text: 0xf4f6f0,
    text_2: 0xa4ab9b,
    text_3: 0x6f7768,
    brand: 0xc3ec74,
    brand_soft: 0x262b23,
    pill: 0x262b23,
    red: 0xff6b6b,
    amber: 0xf5b84a,
    sky: 0x7cc7ff,
};

const LIGHT: Tokens = Tokens {
    bg: 0xffffff,
    bg_soft: 0xf4f5f1,
    surface: 0xf7f8f4,
    stroke: 0xe4e6df,
    text: 0x13160f,
    text_2: 0x5d6356,
    text_3: 0x8b9184,
    brand: 0x4f7d14,
    brand_soft: 0xeef0ea,
    pill: 0xeef0ea,
    red: 0xc02b2b,
    amber: 0x9a6b00,
    sky: 0x1f5fb4,
};

fn hex(value: u32) -> Hsla {
    gpui_kit::rgb(value).into()
}

/// The tokens resolved for one mode.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Hsla,
    pub bg_soft: Hsla,
    pub surface: Hsla,
    pub stroke: Hsla,
    pub text: Hsla,
    pub text_2: Hsla,
    pub text_3: Hsla,
    pub brand: Hsla,
    pub brand_soft: Hsla,
    pub pill: Hsla,
    pub red: Hsla,
    pub amber: Hsla,
    pub sky: Hsla,
    /// Several opacity choices depend on which way the scheme runs.
    pub dark: bool,
}

impl Palette {
    pub fn for_mode(mode: gpui_kit::component::ThemeMode) -> Self {
        let (t, dark) = match mode {
            gpui_kit::component::ThemeMode::Light => (&LIGHT, false),
            _ => (&DARK, true),
        };

        Self {
            bg: hex(t.bg),
            bg_soft: hex(t.bg_soft),
            surface: hex(t.surface),
            stroke: hex(t.stroke),
            text: hex(t.text),
            text_2: hex(t.text_2),
            text_3: hex(t.text_3),
            brand: hex(t.brand),
            brand_soft: hex(t.brand_soft),
            pill: hex(t.pill),
            red: hex(t.red),
            amber: hex(t.amber),
            sky: hex(t.sky),
            dark,
        }
    }

    /// The palette for the theme that is currently installed.
    pub fn current(cx: &App) -> Self {
        Self::for_mode(gpui_kit::component::Theme::global(cx).mode)
    }

    /// How strongly a selection tints a cell.
    ///
    /// The tint has to leave the text legible, and the two modes need opposite
    /// amounts of it: lime laid over near-black reads as a wash at a fifth, while
    /// the same lime over white needs more before it is noticeable at all.
    pub fn selection_alpha(self) -> f32 {
        if self.dark { 0.20 } else { 0.30 }
    }

    /// The 16 ANSI colours, tuned to sit inside this scheme rather than beside it.
    pub fn ansi(self) -> [Rgb; 16] {
        if self.dark {
            [
                Rgb::new(0x1b, 0x1f, 0x19),
                Rgb::new(0xff, 0x6b, 0x6b),
                Rgb::new(0xa8, 0xd9, 0x6a),
                Rgb::new(0xf5, 0xb8, 0x4a),
                Rgb::new(0x7c, 0xc7, 0xff),
                Rgb::new(0xc9, 0xa6, 0xff),
                Rgb::new(0x6f, 0xd7, 0xc4),
                Rgb::new(0xe6, 0xe9, 0xe0),
                Rgb::new(0x6f, 0x77, 0x68),
                Rgb::new(0xff, 0x8f, 0x8f),
                Rgb::new(0xc3, 0xec, 0x74),
                Rgb::new(0xff, 0xd2, 0x7a),
                Rgb::new(0xa3, 0xdb, 0xff),
                Rgb::new(0xdc, 0xbf, 0xff),
                Rgb::new(0x8f, 0xe8, 0xd8),
                Rgb::new(0xf4, 0xf6, 0xf0),
            ]
        } else {
            [
                Rgb::new(0x13, 0x16, 0x0f),
                Rgb::new(0xc0, 0x2b, 0x2b),
                Rgb::new(0x3f, 0x7d, 0x1f),
                Rgb::new(0x8a, 0x5a, 0x00),
                Rgb::new(0x1f, 0x5f, 0xb4),
                Rgb::new(0x7b, 0x3f, 0xb0),
                Rgb::new(0x0f, 0x75, 0x70),
                Rgb::new(0x5d, 0x63, 0x56),
                Rgb::new(0x8b, 0x91, 0x84),
                Rgb::new(0xe0, 0x4a, 0x4a),
                Rgb::new(0x4f, 0x7d, 0x14),
                Rgb::new(0xb0, 0x7c, 0x00),
                Rgb::new(0x2f, 0x7f, 0xd8),
                Rgb::new(0x9a, 0x55, 0xcf),
                Rgb::new(0x14, 0x90, 0x8a),
                Rgb::new(0x13, 0x16, 0x0f),
            ]
        }
    }
}

/// Rewrites a theme's colours to this palette.
///
/// Every field is named explicitly rather than derived, because the mapping is a
/// design decision: `muted` becomes the hover wash, `accent` the same pill, and
/// `ring` the one accent that anything focused is allowed to use.
fn recolour(colors: &mut gpui_kit::component::ThemeColor, p: Palette) {
    let tint = if p.dark { 0.16 } else { 0.22 };

    colors.background = p.bg;
    colors.foreground = p.text;
    colors.border = p.stroke;

    colors.muted = p.pill;
    colors.muted_foreground = p.text_2;
    colors.accent = p.pill;
    colors.accent_foreground = p.text;

    colors.secondary = p.pill;
    colors.secondary_hover = p.surface;
    colors.secondary_active = p.surface;
    colors.secondary_foreground = p.text;

    colors.title_bar = p.bg_soft;
    colors.title_bar_border = p.stroke;
    colors.status_bar = p.bg_soft;
    colors.status_bar_border = p.stroke;

    colors.tab_bar = p.bg_soft;
    colors.tab = p.pill;
    colors.tab_active = p.surface;
    colors.tab_foreground = p.text_2;
    colors.tab_active_foreground = p.text;

    colors.popover = p.surface;
    colors.popover_foreground = p.text;
    colors.overlay = p.bg;
    colors.group_box = p.bg_soft;
    colors.group_box_foreground = p.text;
    colors.description_list_label = p.surface;
    colors.description_list_label_foreground = p.text;

    colors.list = p.bg;
    colors.list_active = p.brand_soft;
    colors.list_active_border = p.brand;
    colors.list_hover = p.pill;
    colors.list_head = p.surface;
    colors.list_even = p.bg_soft;

    colors.selection = p.brand.opacity(tint);
    colors.caret = p.text;
    colors.ring = p.brand;
    colors.drag_border = p.brand;
    colors.drop_target = p.brand_soft;

    colors.danger = p.red;
    colors.danger_foreground = p.bg;
    colors.warning = p.amber;
    colors.warning_foreground = p.bg;
    colors.info = p.sky;
    colors.info_foreground = p.bg;
    colors.success = p.brand;
    colors.success_foreground = p.bg;

    colors.primary = p.brand;
    colors.primary_foreground = p.bg;
    colors.primary_hover = p.brand.opacity(0.85);
    colors.primary_active = p.brand.opacity(0.7);

    colors.button = p.pill;
    colors.button_hover = p.surface;
    colors.button_active = p.surface;
    colors.button_foreground = p.text;
    colors.button_secondary = p.pill;
    colors.button_secondary_hover = p.surface;
    colors.button_secondary_active = p.surface;
    colors.button_secondary_foreground = p.text;
    colors.button_primary = p.brand;
    colors.button_primary_foreground = p.bg;
    colors.button_primary_hover = p.brand.opacity(0.85);
    colors.button_primary_active = p.brand.opacity(0.7);

    colors.input = p.bg;
    colors.switch = p.brand;
    colors.switch_thumb = p.bg;
    colors.slider_bar = p.stroke;
    colors.slider_thumb = p.brand;
    colors.progress_bar = p.brand;

    colors.scrollbar = p.bg_soft;
    colors.scrollbar_thumb = p.text_3.opacity(0.5);
    colors.scrollbar_thumb_hover = p.text_3.opacity(0.8);

    colors.sidebar = p.bg_soft;
    colors.sidebar_foreground = p.text;
    colors.sidebar_border = p.stroke;
    colors.sidebar_accent = p.pill;
    colors.sidebar_accent_foreground = p.text;

    colors.window_border = p.stroke;
    colors.skeleton = p.pill;
}

/// Selects `mode` and paints the theme with this palette.
///
/// `Theme::change` re-applies the registered theme file, which still holds the
/// library's own colours, so the palette is written immediately afterwards. Every
/// mode change goes through here, which is what keeps the two from disagreeing.
pub fn install(cx: &mut App, mode: gpui_kit::component::ThemeMode) {
    use gpui_kit::component::Theme;

    Theme::change(mode, None, cx);

    let palette = Palette::for_mode(mode);
    Theme::update(cx, |theme| recolour(&mut theme.colors, palette));
}
