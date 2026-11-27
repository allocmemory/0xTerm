//! `[app] icon`: which icon the app shows, and bringing a user's icon file in.
//! Applying it is the app's job (AppKit); this is the part that needs no window.

use std::path::{Path, PathBuf};

use crate::Config;

/// The official set shipped in the app, in the order Settings and 0xterm.org show it.
/// Each name is a family of two files, `<name>-light` and `<name>-dark`, and the
/// app shows the one that matches the macOS appearance.
pub const OFFICIAL: [&str; 14] =
    ["chrome", "graphite", "midnight", "ivory", "amber", "mint", "ocean", "crimson", "violet", "phosphor", "mono", "glass", "prompt", "classic"];

/// The family the built-in icon belongs to: it is chrome's dark art, so in
/// light mode the default shows chrome's light one.
pub const DEFAULT_FAMILY: &str = "chrome";

/// The macOS appearance an icon is drawn for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    fn suffix(self) -> &'static str {
        match self {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }
}

/// Imports larger than this are refused: an icon is a few hundred KB at most.
pub const MAX_IMPORT_BYTES: u64 = 10 * 1024 * 1024;

/// The file types Settings imports, the ones AppKit loads at runtime. An Icon
/// Composer `.icon` isn't one: only Xcode's actool compiles it.
pub const IMPORT_EXTENSIONS: [&str; 2] = ["svg", "icns"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconChoice {
    /// The icon built into the app.
    Default,
    /// One of the official set shipped in the app, by name ("graphite").
    Bundled(String),
    /// A file in [`Config::icons_dir`], by file name ("mine.svg").
    Imported(String),
}

impl IconChoice {
    /// Reads the `icon` value: empty is the default, a bare name is bundled, a
    /// name with an extension is an imported file. Anything that could reach
    /// outside the icons folder falls back to the default.
    pub fn parse(value: &str) -> IconChoice {
        let value = value.trim();
        if value.is_empty() || value.contains(['/', '\\']) || value.starts_with('.') {
            return IconChoice::Default;
        }
        if Path::new(value).extension().is_some() {
            IconChoice::Imported(value.to_string())
        } else if value.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            IconChoice::Bundled(value.to_string())
        } else {
            IconChoice::Default
        }
    }

    /// The bundled file this choice shows in `appearance`, without its
    /// extension ("amber-dark"). `None` means no bundled file: the built-in
    /// icon in dark mode is the one compiled into the app, and an import is
    /// the user's own single file.
    /// A name that already ends in `-light` or `-dark` ("amber-light", as the
    /// downloads are named) pins that file whatever the appearance.
    pub fn bundled_stem(&self, appearance: Appearance) -> Option<String> {
        match self {
            IconChoice::Default => (appearance == Appearance::Light).then(|| format!("{DEFAULT_FAMILY}-light")),
            IconChoice::Bundled(name) if name.ends_with("-light") || name.ends_with("-dark") => Some(name.clone()),
            IconChoice::Bundled(name) => Some(format!("{name}-{}", appearance.suffix())),
            IconChoice::Imported(_) => None,
        }
    }
}

impl Config {
    /// Where imported icons are kept: `icons/` next to config.toml.
    pub fn icons_dir() -> Option<PathBuf> {
        Some(Self::path()?.parent()?.join("icons"))
    }
}

/// Checks a file the user picked to import and returns the name it should be
/// stored under. Doesn't decode it: the app test-loads it with AppKit first.
pub fn validate_import(path: &Path) -> Result<String, String> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !IMPORT_EXTENSIONS.contains(&ext.as_str()) {
        return Err("Only .svg and .icns icons can be imported.".to_string());
    }
    let meta = std::fs::metadata(path).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} isn't a file.", path.display()));
    }
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(format!("{} is larger than 10 MB.", path.display()));
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    let clean: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let clean = clean.trim_matches('-');
    let clean = if clean.is_empty() { "icon" } else { clean };
    Ok(format!("{clean}.{ext}"))
}

/// Copies a validated file into the icons folder under `name` and returns
/// the path it now has.
pub fn import(path: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = Config::icons_dir().ok_or("No home folder to keep icons in.")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    let dest = dir.join(name);
    std::fs::copy(path, &dest).map_err(|e| format!("Couldn't copy into {}: {e}", dir.display()))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_stem_follows_the_appearance() {
        let amber = IconChoice::parse("amber");
        assert_eq!(amber.bundled_stem(Appearance::Light).as_deref(), Some("amber-light"));
        assert_eq!(amber.bundled_stem(Appearance::Dark).as_deref(), Some("amber-dark"));
    }

    #[test]
    fn a_variant_name_pins_that_file() {
        let pinned = IconChoice::parse("amber-light");
        assert_eq!(pinned.bundled_stem(Appearance::Dark).as_deref(), Some("amber-light"));
        assert_eq!(IconChoice::parse("prompt-dark").bundled_stem(Appearance::Light).as_deref(), Some("prompt-dark"));
    }

    #[test]
    fn default_is_the_compiled_icon_in_dark_and_chrome_light_in_light() {
        assert_eq!(IconChoice::Default.bundled_stem(Appearance::Dark), None);
        assert_eq!(IconChoice::Default.bundled_stem(Appearance::Light).as_deref(), Some("chrome-light"));
    }

    #[test]
    fn imports_have_no_bundled_file() {
        let mine = IconChoice::parse("mine.svg");
        assert_eq!(mine, IconChoice::Imported("mine.svg".into()));
        assert_eq!(mine.bundled_stem(Appearance::Light), None);
        assert_eq!(mine.bundled_stem(Appearance::Dark), None);
    }

    /// Every official family ships both variants, as art and as an Icon Composer source.
    #[test]
    fn every_family_has_both_variants() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
        for name in OFFICIAL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let stem = IconChoice::Bundled(name.into()).bundled_stem(appearance).unwrap();
                assert!(dir.join(format!("{stem}.svg")).is_file(), "missing {stem}.svg");
                assert!(dir.join(format!("{stem}.icon/icon.json")).is_file(), "missing {stem}.icon");
            }
        }
    }
}
