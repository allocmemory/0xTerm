//! Hot reload: when config.toml is saved, every open window picks it up.

use std::time::Duration;

use gpui_kit::*;
use notify::{RecursiveMode, Watcher};
use oxt_config::Config;
use oxt_window::Workspace;

/// Every open window's workspace, so a config change can reach them all.
#[derive(Default)]
pub struct Workspaces(pub Vec<(AnyWindowHandle, WeakEntity<Workspace>)>);

impl Global for Workspaces {}

struct ConfigWatcher(#[allow(dead_code)] notify::RecommendedWatcher);

impl Global for ConfigWatcher {}

pub fn register(window: AnyWindowHandle, workspace: &Entity<Workspace>, cx: &mut App) {
    let workspaces = cx.default_global::<Workspaces>();
    workspaces.0.retain(|(_, ws)| ws.upgrade().is_some());
    workspaces.0.push((window, workspace.downgrade()));
}

/// The focused window's workspace, if a 0xTerm window is focused.
pub fn active(cx: &App) -> Option<(AnyWindowHandle, Entity<Workspace>)> {
    let active = cx.active_window()?;
    let workspaces = cx.try_global::<Workspaces>()?;
    workspaces.0.iter().find(|(window, _)| *window == active).and_then(|(window, ws)| Some((*window, ws.upgrade()?)))
}

pub fn watch(cx: &mut App) {
    let Some(path) = Config::path() else { return };
    let Some(dir) = path.parent().map(|d| d.to_path_buf()) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let (tx, rx) = async_channel::unbounded::<()>();
    let file = path.clone();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        // Editors often save by writing a new file and renaming it, so watch the folder.
        if event.is_ok_and(|e| e.paths.iter().any(|p| p == &file)) {
            let _ = tx.try_send(());
        }
    });
    let Ok(mut watcher) = watcher else { return };
    if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
        return;
    }
    cx.set_global(ConfigWatcher(watcher));

    cx.spawn(async move |cx| {
        while rx.recv().await.is_ok() {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            while rx.try_recv().is_ok() {}
            cx.update(apply);
        }
    })
    .detach();
}

fn apply(cx: &mut App) {
    crate::settings_window::refresh();
    let (config, error) = Config::load();
    if let Some(error) = error {
        eprintln!("0xTerm: config error, keeping the current settings: {error}");
        return;
    }
    crate::icon::apply_if_changed(&config);
    let targets = cx.try_global::<Workspaces>().map(|w| w.0.clone()).unwrap_or_default();
    for (window, workspace) in targets {
        let Some(workspace) = workspace.upgrade() else { continue };
        let config = config.clone();
        let _ = window.update(cx, |_, window, cx| workspace.update(cx, |ws, cx| ws.set_config(config, window, cx)));
    }
}
