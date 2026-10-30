//! Writing one setting back to config.toml, for Settings. The file is the
//! user's: only the key that changed is touched, and comments, order and every
//! other key stay as they wrote them.

use std::path::Path;

pub use toml_edit::Value;

use crate::Config;

/// Sets `keys` (e.g. `["app", "icon"]`) to `value` in config.toml, or removes
/// it with `None` so the default applies again. Refuses when the file doesn't
/// parse, so a half-edited file is never overwritten, and when the result
/// wouldn't load.
pub fn write_key(keys: &[&str], value: Option<Value>) -> Result<(), String> {
    let path = Config::path().ok_or("No home folder for config.toml.")?;
    write_key_at(&path, keys, value)
}

pub fn write_key_at(path: &Path, keys: &[&str], value: Option<Value>) -> Result<(), String> {
    let (last, tables) = keys.split_last().ok_or("No key given.")?;
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(format!("{}: {err}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;

    let mut table = doc.as_table_mut();
    for key in tables {
        if !table.contains_key(key) {
            if value.is_none() {
                return Ok(());
            }
            table.insert(key, toml_edit::Item::Table(toml_edit::Table::new()));
        }
        table = table.get_mut(key).and_then(|item| item.as_table_mut()).ok_or_else(|| format!("{key} isn't a table in {}", path.display()))?;
    }
    match value {
        Some(value) => match table.get_mut(last).and_then(|item| item.as_value_mut()) {
            // Keeps the comment and spacing around an existing value.
            Some(old) => {
                let decor = old.decor().clone();
                *old = value;
                *old.decor_mut() = decor;
            }
            None => {
                table.insert(last, toml_edit::value(value));
            }
        },
        None => {
            table.remove(last);
        }
    }

    let text = doc.to_string();
    Config::parse(&text).map_err(|e| format!("That value wouldn't load: {e}"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // Write beside it and rename, as editors do, so the watcher never reads half a file.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}
