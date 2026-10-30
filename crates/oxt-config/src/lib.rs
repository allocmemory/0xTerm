//! `~/.config/0xterm/config.toml`. The defaults live in `default.toml`, which is
//! also the documentation; a user file only needs the keys it changes.

use std::path::PathBuf;

use serde::Deserialize;

pub mod edit;
pub mod icon;

pub use icon::IconChoice;

const DEFAULT: &str = include_str!("default.toml");

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Config {
    pub app: App,
    pub font: Font,
    pub terminal: Terminal,
    pub theme: Theme,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct App {
    /// See [`IconChoice::parse`].
    pub icon: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Font {
    pub family: String,
    pub size: f32,
    pub line_height: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Terminal {
    pub shell: String,
    pub scrollback: usize,
    pub option_as_meta: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Theme {
    pub mode: ThemeMode,
    pub dark: Colors,
    pub light: Colors,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Colors {
    pub background: Hex,
    pub foreground: Hex,
    pub cursor: Hex,
    pub selection: Hex,
    pub ansi: [Hex; 16],
}

/// A `#rrggbb` color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Hex(pub u32);

impl Hex {
    pub fn rgb(self) -> (u8, u8, u8) {
        ((self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8)
    }
}

impl TryFrom<String> for Hex {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let digits = s.strip_prefix('#').filter(|d| d.len() == 6).ok_or_else(|| format!("{s:?} is not #rrggbb"))?;
        u32::from_str_radix(digits, 16).map(Hex).map_err(|_| format!("{s:?} is not #rrggbb"))
    }
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str(DEFAULT).expect("default.toml is valid")
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        Some(base.join("0xterm").join("config.toml"))
    }

    /// Reads the user's file. A missing file is the defaults; a broken one is
    /// an error message the UI shows while it keeps the defaults.
    pub fn load() -> (Config, Option<String>) {
        let Some(path) = Self::path() else { return (Config::default(), None) };
        match std::fs::read_to_string(&path) {
            Ok(text) => match Self::parse(&text) {
                Ok(config) => (config, None),
                Err(err) => (Config::default(), Some(format!("{}: {err}", path.display()))),
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => (Config::default(), None),
            Err(err) => (Config::default(), Some(format!("{}: {err}", path.display()))),
        }
    }

    /// The user's TOML layered over the defaults, key by key.
    pub fn parse(user: &str) -> Result<Config, String> {
        let mut base: toml::Table = toml::from_str(DEFAULT).expect("default.toml is valid");
        let user: toml::Table = toml::from_str(user).map_err(|e| e.to_string())?;
        merge(&mut base, user);
        toml::Value::Table(base).try_into().map_err(|e: toml::de::Error| e.to_string())
    }
}

fn merge(base: &mut toml::Table, user: toml::Table) {
    for (key, value) in user {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(u)) => merge(b, u),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// Window chrome colors (title row, sidebar, borders), as `0xRRGGBBAA` for
/// GPUI's `rgba()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chrome {
    pub chrome: u32,
    pub chrome_2: u32,
    pub panel: u32,
    pub border: u32,
    pub text: u32,
    pub dim: u32,
    pub faint: u32,
    pub accent: u32,
    pub hover: u32,
    pub selected: u32,
    pub find: u32,
    pub find_current: u32,
    pub dim_pane: u32,
}

impl Chrome {
    pub const DARK: Chrome = Chrome {
        chrome: 0x1f1f21ff,
        chrome_2: 0x2a2a2dff,
        panel: 0x161618ff,
        border: 0x2f2f33ff,
        text: 0xe6e6e6ff,
        dim: 0x98989dff,
        faint: 0x5c5c61ff,
        accent: 0x4c9dffff,
        hover: 0xffffff0f,
        selected: 0x4c9dff33,
        find: 0xffd20059,
        find_current: 0xff9600bf,
        dim_pane: 0x00000059,
    };

    pub const LIGHT: Chrome = Chrome {
        chrome: 0xecececff,
        chrome_2: 0xe0e0e0ff,
        panel: 0xf6f6f6ff,
        border: 0xd4d4d4ff,
        text: 0x1d1d1fff,
        dim: 0x6e6e73ff,
        faint: 0xa1a1a6ff,
        accent: 0x0a66ffff,
        hover: 0x0000000f,
        selected: 0x0a66ff24,
        find: 0xffc80073,
        find_current: 0xff8c00cc,
        dim_pane: 0xffffff59,
    };

    pub fn for_appearance(dark: bool) -> Chrome {
        if dark { Chrome::DARK } else { Chrome::LIGHT }
    }
}

impl Config {
    /// Whether to use the dark theme, given what macOS currently shows.
    pub fn is_dark(&self, system_dark: bool) -> bool {
        match self.theme.mode {
            ThemeMode::System => system_dark,
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse() {
        let c = Config::default();
        assert_eq!(c.theme.dark.background, Hex(0x0b0b0b));
        assert_eq!(c.terminal.scrollback, 10_000);
    }

    #[test]
    fn user_file_overrides_only_its_keys() {
        let c = Config::parse("[font]\nsize = 15\n[theme.dark]\nbackground = \"#000000\"").unwrap();
        assert_eq!(c.font.size, 15.0);
        assert_eq!(c.font.family, "Menlo");
        assert_eq!(c.theme.dark.background, Hex(0));
        assert_eq!(c.theme.dark.foreground, Hex(0xe6e6e6));
    }

    #[test]
    fn bad_values_are_errors() {
        assert!(Config::parse("[theme.dark]\nbackground = \"red\"").unwrap_err().contains("#rrggbb"));
        assert!(Config::parse("[font\n").is_err());
    }
}
