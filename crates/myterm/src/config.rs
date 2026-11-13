//! Preferences that outlive the window.
//!
//! A flat `key = value` file rather than a serialization crate: the settings are
//! four scalars, and a dependency that writes a tree to store four scalars buys
//! nothing but a second thing that can fail to parse. Every unknown key is
//! ignored and every malformed line is dropped, so a hand-edited file can never
//! stop the app from starting.

use std::path::PathBuf;

/// The preferences the app actually reads.
#[derive(Clone, Default)]
pub struct Config {
    /// The theme to use, when it is not the one the desktop is using. `None`
    /// means "follow the system", which is the default.
    pub light: Option<bool>,
    /// The terminal's font size. `None` means the theme's own default.
    pub font_size: Option<f32>,
}

impl Config {
    /// Where the file lives.
    fn path() -> Option<PathBuf> {
        let local = std::env::var_os("LOCALAPPDATA")?;
        Some(PathBuf::from(local).join("myterm").join("config.txt"))
    }

    /// Where the file lives, spelled out for the settings sheet.
    pub fn location() -> Option<String> {
        Self::path().map(|path| path.display().to_string())
    }

    /// Reads the file. A missing file is an empty config, not an error.
    pub fn load() -> Self {
        let Ok(text) = Self::path()
            .map(std::fs::read_to_string)
            .unwrap_or_else(|| Ok(String::new()))
        else {
            return Self::default();
        };
        Self::parse(&text)
    }

    /// Writes the file, creating its directory.
    ///
    /// A failure here loses a preference, not the session: the setting still
    /// applies until the window closes.
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(dir) = path.parent()
            && std::fs::create_dir_all(dir).is_err()
        {
            return;
        }
        let _ = std::fs::write(path, self.encode());
    }

    fn parse(text: &str) -> Self {
        let mut config = Self::default();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();

            match key {
                "theme" => match value {
                    "light" => config.light = Some(true),
                    "dark" => config.light = Some(false),
                    "system" => config.light = None,
                    _ => {}
                },
                "font_size" => {
                    // Clamped here as well as at the control: a hand-edited file
                    // is as able to say 2 as the buttons are to say 9.
                    if let Ok(size) = value.parse::<f32>()
                        && (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&size)
                    {
                        config.font_size = Some(size);
                    }
                }

                _ => {}
            }
        }

        config
    }

    fn encode(&self) -> String {
        let mut text = String::from("# myterm 的设置，每一行是 key = value\n");

        let theme = match self.light {
            Some(true) => "light",
            Some(false) => "dark",
            None => "system",
        };
        text.push_str(&format!("theme = {theme}\n"));
        text.push_str(&format!(
            "font_size = {}\n",
            self.font_size.unwrap_or(DEFAULT_FONT_SIZE)
        ));

        text
    }
}

/// The smallest size the terminal will draw at.
pub const MIN_FONT_SIZE: f32 = 9.0;
/// The largest.
pub const MAX_FONT_SIZE: f32 = 24.0;
/// What the app starts at, and what "reset" returns to.
pub const DEFAULT_FONT_SIZE: f32 = 13.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_config_reads_back() {
        let config = Config {
            light: Some(false),
            font_size: Some(16.0),
        };
        let read = Config::parse(&config.encode());

        assert_eq!(read.light, Some(false));
        assert_eq!(read.font_size, Some(16.0));
    }

    #[test]
    fn an_unknown_key_and_a_broken_line_are_ignored() {
        let read = Config::parse("theme = light\nnonsense\nfont_size = 14\nextra = 1\n");
        assert_eq!(read.light, Some(true));
        assert_eq!(read.font_size, Some(14.0));
    }

    #[test]
    fn an_out_of_range_size_is_not_adopted() {
        let read = Config::parse("font_size = 200\n");
        assert_eq!(read.font_size, None);
    }

    #[test]
    fn system_means_no_override() {
        let read = Config::parse("theme = system\n");
        assert_eq!(read.light, None);
    }
}
