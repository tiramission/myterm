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

use crate::config::Accent;
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
    /// The same line, one step brighter. A card that holds the keyboard wears
    /// this instead of the accent: focus is a hierarchy, not a highlight.
    stroke_strong: u32,
    text: u32,
    text_2: u32,
    text_3: u32,
    /// The scrim behind an overlay. Always darker than the surfaces under it,
    /// in both modes: a scrim the colour of the background covers nothing.
    scrim: u32,
    pill: u32,
    red: u32,
    amber: u32,
    sky: u32,
}

const DARK: Tokens = Tokens {
    bg: 0x101210,
    bg_soft: 0x1a1d18,
    surface: 0x22261e,
    stroke: 0x33382d,
    stroke_strong: 0x4d5648,
    text: 0xf2f5ec,
    text_2: 0xa6ada0,
    text_3: 0x8a9281,
    scrim: 0x000000,
    pill: 0x262b23,
    red: 0xff6b6b,
    amber: 0xf5b84a,
    sky: 0x7cc7ff,
};

/// Light keeps a white grid, so the chrome has to be visibly deeper than it:
/// a raised row is still lighter than the surface it sits on, the same
/// direction as dark. A white card on a near-white ground was 1.10:1 and
/// left the card with no edge at all.
const LIGHT: Tokens = Tokens {
    bg: 0xffffff,
    bg_soft: 0xe7eadf,
    surface: 0xffffff,
    stroke: 0xd2d6c6,
    stroke_strong: 0x9aa08e,
    text: 0x14170f,
    text_2: 0x555b4d,
    text_3: 0x656b5c,
    scrim: 0x1b1f18,
    pill: 0xdfe3d7,
    red: 0xb12b26,
    amber: 0x8a5e00,
    sky: 0x1f5fb4,
};

/// The hue each accent sits on, in degrees.
///
/// The neutrals are pulled toward this so the window reads as one family. The
/// amount is small on purpose: a chrome that is visibly coloured would compete
/// with the accent, which is the one colour allowed to mean something.
fn accent_hue(accent: Accent) -> f32 {
    match accent {
        // Lime sits in the yellow-green band; the neutrals are already there.
        Accent::Lime => 78.0,
        Accent::Amber => 42.0,
        Accent::Sky => 200.0,
        Accent::Violet => 268.0,
    }
}

/// Pulls a neutral toward `hue`, keeping its lightness.
///
/// The neutrals carry a little saturation already (they are warm, not grey), so
/// this replaces that trace with the accent's rather than adding to it. Dark
/// chrome takes a little more: a few points of saturation is invisible on a
/// near-black surface until it is not.
fn tint(value: u32, hue: f32, dark: bool) -> u32 {
    let r = ((value >> 16) & 0xff) as f32 / 255.0;
    let g = ((value >> 8) & 0xff) as f32 / 255.0;
    let b = (value & 0xff) as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let light = (max + min) / 2.0;

    // The trace of colour the surface carries. Enough to read as a family with
    // the accent, not enough to read as a colour.
    let saturation = if dark { 0.14 } else { 0.10 };

    let (r, g, b) = hsl_to_rgb(hue, saturation, light);
    let to = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (to(r) << 16) | (to(g) << 8) | to(b)
}

/// HSL to RGB, all in 0..=1 (hue in degrees).
fn hsl_to_rgb(hue: f32, saturation: f32, light: f32) -> (f32, f32, f32) {
    if saturation == 0.0 {
        return (light, light, light);
    }
    let q = if light < 0.5 {
        light * (1.0 + saturation)
    } else {
        light + saturation - light * saturation
    };
    let p = 2.0 * light - q;
    let h = hue / 360.0;
    (
        hue_to_rgb(p, q, h + 1.0 / 3.0),
        hue_to_rgb(p, q, h),
        hue_to_rgb(p, q, h - 1.0 / 3.0),
    )
}

fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

/// The ink and wash for an accent, for the settings swatches.
pub fn accent_ink(accent: Accent, mode: gpui_kit::component::ThemeMode) -> (Hsla, Hsla) {
    let dark = !matches!(mode, gpui_kit::component::ThemeMode::Light);
    let (ink, wash) = accents(accent, dark);
    (hex(ink), hex(wash))
}

/// The four accents, as `(ink, wash)` per mode.
///
/// The wash is the accent laid on its mode's chrome at low strength, resolved
/// here rather than multiplied at every use site: a tint that has to be computed
/// per frame is a tint that drifts.
fn accents(accent: Accent, dark: bool) -> (u32, u32) {
    match (accent, dark) {
        (Accent::Lime, true) => (0xc6ee7a, 0x262b23),
        (Accent::Lime, false) => (0x3f6b12, 0xedf4dc),
        (Accent::Amber, true) => (0xe3c06c, 0x2b271c),
        (Accent::Amber, false) => (0x8a6a1f, 0xf5efdc),
        (Accent::Sky, true) => (0x7cc4e8, 0x1d262b),
        (Accent::Sky, false) => (0x1f6a8a, 0xdcecf5),
        (Accent::Violet, true) => (0xc9a7f5, 0x262230),
        (Accent::Violet, false) => (0x6b3fa0, 0xefe6f9),
    }
}

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
    pub stroke_strong: Hsla,
    pub text: Hsla,
    pub text_2: Hsla,
    #[allow(dead_code)]
    pub text_3: Hsla,
    pub brand: Hsla,
    pub brand_soft: Hsla,
    /// Black in dark mode, the deepest ink tone in light mode; the two need
    /// different scrims to look equally dim over their own surfaces.
    pub scrim: Hsla,
    pub pill: Hsla,
    pub red: Hsla,
    pub amber: Hsla,
    pub sky: Hsla,
    /// Several opacity choices depend on which way the scheme runs.
    pub dark: bool,
}

impl Palette {
    pub fn for_mode(mode: gpui_kit::component::ThemeMode, accent: Accent) -> Self {
        let (t, dark) = match mode {
            gpui_kit::component::ThemeMode::Light => (&LIGHT, false),
            _ => (&DARK, true),
        };
        let (brand, brand_soft) = accents(accent, dark);
        // The neutrals are tinted toward the accent's hue. Without this the
        // chrome stayed warm olive whatever accent was chosen, so "sky" or
        // "violet" recoloured one outline in a greenish window.
        let hue = accent_hue(accent);
        let t = Tokens {
            bg: tint(t.bg, hue, dark),
            bg_soft: tint(t.bg_soft, hue, dark),
            surface: tint(t.surface, hue, dark),
            stroke: tint(t.stroke, hue, dark),
            stroke_strong: tint(t.stroke_strong, hue, dark),
            pill: tint(t.pill, hue, dark),
            ..*t
        };

        Self {
            bg: hex(t.bg),
            bg_soft: hex(t.bg_soft),
            surface: hex(t.surface),
            stroke: hex(t.stroke),
            stroke_strong: hex(t.stroke_strong),
            text: hex(t.text),
            text_2: hex(t.text_2),
            text_3: hex(t.text_3),
            brand: hex(brand),
            brand_soft: hex(brand_soft),
            scrim: hex(t.scrim),
            pill: hex(t.pill),
            red: hex(t.red),
            amber: hex(t.amber),
            sky: hex(t.sky),
            dark,
        }
    }

    /// The palette for the theme that is currently installed.
    pub fn current(cx: &App) -> Self {
        Self::for_mode(
            gpui_kit::component::Theme::global(cx).mode,
            crate::config::Config::load().accent,
        )
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
    // The accent the library paints with is the accent's *wash*, not its ink:
    // this token ends up behind inline code and behind text selections, where
    // the full colour would swallow the text.
    colors.accent = p.brand_soft;
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
    // The scrim has to darken what is under it in both modes, so it is a near
    // black rather than the background tone. Using `bg` here made it invisible
    // over the very surface it was meant to cover.
    colors.overlay = p.scrim.opacity(if p.dark { 0.58 } else { 0.34 });
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
    // The caret is the accent, the way the prototype draws it: it is the one
    // thing in the grid that says where you are, and it belongs to the same
    // family as the outline on the focused card.
    colors.caret = p.brand;
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
    // The thumb is the accent at rest: a terminal's scrollbar is the one piece
    // of chrome that is always about position, which is what the accent means.
    colors.scrollbar_thumb = p.brand.opacity(if p.dark { 0.45 } else { 0.35 });
    colors.scrollbar_thumb_hover = p.brand.opacity(0.7);

    colors.sidebar = p.bg_soft;
    colors.sidebar_foreground = p.text;
    colors.sidebar_border = p.stroke;
    colors.sidebar_accent = p.brand_soft;
    colors.sidebar_accent_foreground = p.brand;

    colors.window_border = p.stroke;
    colors.skeleton = p.pill;
}

/// Selects `mode` and paints the theme with this palette.
///
/// `Theme::change` re-applies the registered theme file, which still holds the
/// library's own colours, so the palette is written immediately afterwards. Every
/// mode change goes through here, which is what keeps the two from disagreeing.
pub fn install(cx: &mut App, mode: gpui_kit::component::ThemeMode, accent: Accent) {
    use gpui_kit::component::Theme;

    Theme::change(mode, None, cx);

    let palette = Palette::for_mode(mode, accent);
    Theme::update(cx, |theme| recolour(&mut theme.colors, palette));
}
