//! 0xTerm: start the app, install the menu bar, open the first window.

mod config_watch;
mod menus;

use gpui_kit::*;
use oxt_config::Config;
use oxt_window::Workspace;

actions!(oxterm, [Quit, NewWindow]);

fn open_workspace(cx: &mut App) {
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
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        let workspace = cx.new(|cx| Workspace::new(config, None, window, cx));
        config_watch::register(window.window_handle(), &workspace, cx);
        // Debug builds: OXTERM_TYPE is typed into the first pane, for scripted checks.
        #[cfg(debug_assertions)]
        if let Some(script) = std::env::var_os("OXTERM_TYPE") {
            let script = script.to_string_lossy().replace("\\n", "\n");
            workspace.read(cx).type_into_active(&script, cx);
        }
        // Debug builds: OXTERM_OPEN="path[:line[:find]]" opens a file in the viewer.
        #[cfg(debug_assertions)]
        if let Ok(spec) = std::env::var("OXTERM_OPEN") {
            let mut parts = spec.splitn(3, ':');
            let path = parts.next().unwrap_or_default().into();
            let line = parts.next().and_then(|l| l.parse().ok());
            let find = parts.next().map(str::to_owned);
            workspace.update(cx, |ws, cx| ws.open(oxt_toolbelt::OpenRequest { path, line, find }, window, cx));
        }
        // Debug builds: OXTERM_ACTIONS="workspace::SplitRight,workspace::NewTab" runs actions after startup.
        #[cfg(debug_assertions)]
        if let Ok(names) = std::env::var("OXTERM_ACTIONS") {
            window.defer(cx, move |window, cx| {
                for name in names.split(',') {
                    match cx.build_action(name.trim(), None) {
                        Ok(action) => window.dispatch_action(action, cx),
                        Err(err) => eprintln!("OXTERM_ACTIONS: {name}: {err}"),
                    }
                }
            });
        }
        workspace
    });
    if let Err(err) = opened {
        eprintln!("0xTerm: couldn't open a window: {err}");
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
            open_workspace(cx);
        }
    });
    app.run(|cx| {
        gpui_kit::init(cx);
        oxt_terminal_view::bind_keys(cx);
        oxt_window::bind_keys(cx);
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None), KeyBinding::new("cmd-n", NewWindow, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &NewWindow, cx| open_workspace(cx));
        menus::install(cx);
        config_watch::watch(cx);

        open_workspace(cx);
        if !background_launch() {
            cx.activate(true);
        }
    });
}
