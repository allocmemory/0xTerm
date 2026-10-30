//! 0xTerm: start the app, install the menu bar, open the first window.

mod config_watch;
mod icon;
mod macos;
mod menus;

use std::path::PathBuf;

use gpui_kit::*;
use oxt_config::Config;
use oxt_toolbelt::OpenRequest;
use oxt_window::Workspace;

actions!(oxterm, [Quit, NewWindow, OpenSettings]);

/// Written by Settings… when there's no config file yet.
const STARTER_CONFIG: &str = "\
# 0xTerm settings. Anything you set here overrides the defaults, and every
# open window picks it up when you save. The defaults, with notes:
# https://github.com/allocmemory/0xTerm/blob/main/crates/oxt-config/src/default.toml
";

/// Opens a window whose shell and file tree start in `cwd` (home when `None`).
fn open_workspace(cwd: Option<PathBuf>, cx: &mut App) -> Option<(AnyWindowHandle, Entity<Workspace>)> {
    let (config, error) = Config::load();
    if let Some(error) = error {
        eprintln!("0xTerm: config error, using defaults: {error}");
    }
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("0xTerm".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(12.))),
        }),
        window_bounds: Some(WindowBounds::centered(size(px(1240.), px(800.)), cx)),
        window_min_size: Some(size(px(480.), px(300.))),
        focus: !background_launch(),
        // A floating window keeps drawing without taking the keyboard from other apps.
        kind: if background_launch() { WindowKind::PopUp } else { WindowKind::Normal },
        ..Default::default()
    };
    // Debug builds: the OXTERM_* hooks drive only the first window, so an
    // action that opens a window doesn't set them off again.
    #[cfg(debug_assertions)]
    let scripted = cx.windows().is_empty();
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        let workspace = cx.new(|cx| Workspace::new(config, cwd, window, cx));
        config_watch::register(window.window_handle(), &workspace, cx);
        // Debug builds: OXTERM_TYPE is typed into the first pane, for scripted checks.
        #[cfg(debug_assertions)]
        if let Some(script) = std::env::var_os("OXTERM_TYPE").filter(|_| scripted) {
            let script = script.to_string_lossy().replace("\\n", "\n");
            workspace.read(cx).type_into_active(&script, cx);
        }
        // Debug builds: OXTERM_OPEN="path[:line[:find]]" opens a file in the viewer.
        #[cfg(debug_assertions)]
        if let Some(spec) = std::env::var("OXTERM_OPEN").ok().filter(|_| scripted) {
            let mut parts = spec.splitn(3, ':');
            let path = parts.next().unwrap_or_default().into();
            let line = parts.next().and_then(|l| l.parse().ok());
            let find = parts.next().map(str::to_owned);
            workspace.update(cx, |ws, cx| ws.open(oxt_toolbelt::OpenRequest { path, line, find }, window, cx));
        }
        // Debug builds: OXTERM_ACTIONS="workspace::SplitRight,workspace::NewTab" runs actions after startup.
        #[cfg(debug_assertions)]
        if let Some(names) = std::env::var("OXTERM_ACTIONS").ok().filter(|_| scripted) {
            window.defer(cx, move |window, cx| {
                for name in names.split(',') {
                    match cx.build_action(name.trim(), None) {
                        Ok(action) => window.dispatch_action(action, cx),
                        Err(err) => eprintln!("OXTERM_ACTIONS: {name}: {err}"),
                    }
                }
            });
        }
        // Debug builds: OXTERM_KEYS="tab ctrl-c" presses keys through GPUI's dispatch once the shell is up.
        #[cfg(debug_assertions)]
        if let Some(keys) = std::env::var("OXTERM_KEYS").ok().filter(|_| scripted) {
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor().timer(std::time::Duration::from_secs(3)).await;
                    cx.update(|window, cx| {
                        for key in keys.split_whitespace() {
                            match Keystroke::parse(key) {
                                Ok(keystroke) => _ = window.dispatch_keystroke(keystroke, cx),
                                Err(err) => eprintln!("OXTERM_KEYS: {key}: {err}"),
                            }
                        }
                    })
                    .ok();
                })
                .detach();
        }
        workspace
    });
    opened.inspect_err(|err| eprintln!("0xTerm: couldn't open a window: {err}")).ok()
}

fn open_in_viewer(window: AnyWindowHandle, workspace: &Entity<Workspace>, path: PathBuf, cx: &mut App) {
    let _ = window.update(cx, |_, window, cx| workspace.update(cx, |ws, cx| ws.open(OpenRequest::path(path), window, cx)));
}

/// Folders and files opened from Finder or dropped on the Dock icon: a folder
/// gets a window that starts there, a file opens in the viewer of a window
/// that starts in its folder.
fn open_urls(urls: Vec<String>, cx: &mut App) {
    for url in urls {
        let Some(path) = url::Url::parse(&url).ok().and_then(|url| url.to_file_path().ok()) else { continue };
        if path.is_dir() {
            open_workspace(Some(path), cx);
        } else if let Some((window, workspace)) = open_workspace(path.parent().map(PathBuf::from), cx) {
            open_in_viewer(window, &workspace, path, cx);
        }
    }
    cx.activate(true);
}

/// Shows config.toml in the viewer, creating a starter file the first time.
fn open_settings(cx: &mut App) {
    let Some(path) = Config::path() else { return };
    if !path.exists() {
        let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, STARTER_CONFIG));
        if let Err(err) = written {
            eprintln!("0xTerm: couldn't create {}: {err}", path.display());
            return;
        }
    }
    if let Some((window, workspace)) = config_watch::active(cx).or_else(|| open_workspace(None, cx)) {
        open_in_viewer(window, &workspace, path, cx);
    }
}

/// Debug builds: OXTERM_BACKGROUND=1 opens without taking focus, so scripted
/// checks don't steal the keyboard from whatever you're typing in.
fn background_launch() -> bool {
    cfg!(debug_assertions) && std::env::var_os("OXTERM_BACKGROUND").is_some()
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    // Clicking the Dock icon with no windows open opens one.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_workspace(None, cx);
        }
    });
    // macOS hands over opened folders and files before the app has finished
    // launching, so they wait here until the app can open windows.
    let (opened_tx, opened_rx) = async_channel::unbounded::<Vec<String>>();
    app.on_open_urls(move |urls| _ = opened_tx.try_send(urls));
    app.run(|cx| {
        gpui_kit::init(cx);
        oxt_terminal_view::bind_keys(cx);
        oxt_window::bind_keys(cx);
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None), KeyBinding::new("cmd-n", NewWindow, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &NewWindow, cx| _ = open_workspace(None, cx));
        // Deferred: it updates the focused window, which GPUI is updating while it runs actions.
        cx.on_action(|_: &OpenSettings, cx| cx.defer(open_settings));
        cx.bind_keys([KeyBinding::new("cmd-,", OpenSettings, None)]);
        macos::start_updater();
        menus::install(cx);
        icon::apply_at_launch(&Config::load().0);
        config_watch::watch(cx);

        // Launched by opening a folder or file: open that instead of a window at home.
        let launch_urls: Vec<String> = std::iter::from_fn(|| opened_rx.try_recv().ok()).flatten().collect();
        if launch_urls.is_empty() {
            open_workspace(None, cx);
        } else {
            open_urls(launch_urls, cx);
        }
        cx.spawn(async move |cx| {
            while let Ok(urls) = opened_rx.recv().await {
                cx.update(|cx| open_urls(urls, cx));
            }
        })
        .detach();
        if !background_launch() {
            cx.activate(true);
        }
    });
}
