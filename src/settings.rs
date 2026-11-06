//! The app's settings and its colour schemes.

use std::collections::HashMap;

/// What the app remembers between runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The grid's font family.
    pub font_family: String,
    /// The grid's font size in points.
    pub font_size: f32,
    /// The line height, as a multiple of the font size.
    pub line_height: f32,
    /// The grid's padding in points.
    pub padding: f32,
    /// The colour scheme's name.
    pub scheme: String,
    /// The profile a new tab starts when none is chosen.
    pub default_profile: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_family: "Maple Mono NF CN".to_owned(),
            font_size: 13.0,
            line_height: 1.25,
            padding: 8.0,
            scheme: "Graphite Dark".to_owned(),
            default_profile: String::new(),
        }
    }
}

/// A terminal colour scheme: the sixteen ANSI colours and the base colours.
#[derive(Clone, Debug, PartialEq)]
pub struct Scheme {
    pub name: String,
    pub background: u32,
    pub foreground: u32,
    pub cursor: u32,
    /// The ANSI palette, indices 0..16.
    pub palette: [u32; 16],
}

impl Scheme {
    /// The ANSI palette as the 256-entry table the terminal wants: the first
    /// sixteen come from the scheme, the rest from libghostty's generated
    /// ramp.
    pub fn palette_entries(&self) -> Vec<(usize, u32)> {
        self.palette
            .iter()
            .enumerate()
            .map(|(index, color)| (index, *color))
            .collect()
    }
}

pub fn schemes() -> Vec<Scheme> {
    vec![
        Scheme {
            name: "Graphite Dark".to_owned(),
            background: 0x101216,
            foreground: 0xe6e8ec,
            cursor: 0x5b9dff,
            palette: [
                0x2b3038, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xc8ccd4,
                0x5c6370, 0xff7b86, 0xb5e08c, 0xffd48a, 0x7cc4ff, 0xdba0f0, 0x7fd6e0, 0xf2f4f8,
            ],
        },
        Scheme {
            name: "Paper Light".to_owned(),
            background: 0xfbfbfd,
            foreground: 0x1f2328,
            cursor: 0x2563eb,
            palette: [
                0x24292f, 0xcf222e, 0x116329, 0x9a6700, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781,
                0x57606a, 0xa40e26, 0x1a7f37, 0xbf8700, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
            ],
        },
        Scheme {
            name: "High Contrast".to_owned(),
            background: 0x000000,
            foreground: 0xffffff,
            cursor: 0x00ff66,
            palette: [
                0x000000, 0xff5555, 0x55ff55, 0xffff55, 0x5599ff, 0xff55ff, 0x55ffff, 0xffffff,
                0x808080, 0xff8080, 0x80ff80, 0xffff80, 0x80bfff, 0xff80ff, 0x80ffff, 0xffffff,
            ],
        },
    ]
}

pub fn scheme(name: &str) -> Scheme {
    schemes()
        .into_iter()
        .find(|scheme| scheme.name == name)
        .unwrap_or_else(|| schemes().remove(0))
}

/// Where the settings are kept, as `Storage` keys.
const KEY: &str = "myterm.settings";

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut settings = Settings::default();
        let mut values: HashMap<&str, &str> = HashMap::new();
        for line in text.lines() {
            if let Some((key, value)) = line.split_once('=') {
                values.insert(key.trim(), value.trim());
            }
        }
        if let Some(value) = values.get("font_family") {
            if !value.is_empty() {
                settings.font_family = (*value).to_owned();
            }
        }
        if let Some(value) = values.get("font_size").and_then(|v| v.parse().ok()) {
            settings.font_size = (9.0..=32.0)
                .contains(&value)
                .then_some(value)
                .unwrap_or(13.0);
        }
        if let Some(value) = values.get("line_height").and_then(|v| v.parse().ok()) {
            settings.line_height = (1.0..=1.6)
                .contains(&value)
                .then_some(value)
                .unwrap_or(1.25);
        }
        if let Some(value) = values.get("padding").and_then(|v| v.parse().ok()) {
            settings.padding = (0.0..=24.0)
                .contains(&value)
                .then_some(value)
                .unwrap_or(8.0);
        }
        if let Some(value) = values.get("scheme") {
            if schemes().iter().any(|scheme| &scheme.name == value) {
                settings.scheme = (*value).to_owned();
            }
        }
        if let Some(value) = values.get("default_profile") {
            settings.default_profile = (*value).to_owned();
        }
        settings
    }

    pub fn serialize(&self) -> String {
        format!(
            "font_family={}\nfont_size={}\nline_height={}\npadding={}\nscheme={}\ndefault_profile={}\n",
            self.font_family,
            self.font_size,
            self.line_height,
            self.padding,
            self.scheme,
            self.default_profile
        )
    }

    /// The `Storage` key the settings live under.
    pub fn storage_key() -> &'static str {
        KEY
    }
}
