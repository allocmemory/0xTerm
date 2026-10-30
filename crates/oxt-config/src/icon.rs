//! `[app] icon`: which icon the app shows, and bringing a user's icon file in.
//! Applying it is the app's job (AppKit); this is the part that needs no window.

use std::path::{Path, PathBuf};

use crate::Config;

/// The official set shipped in the app, in the order Settings and 0xterm.org show it.
pub const OFFICIAL: [&str; 12] =
    ["chrome", "graphite", "midnight", "ivory", "amber", "mint", "ocean", "crimson", "violet", "phosphor", "mono", "glass"];

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
