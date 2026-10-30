//! The Settings window (⌘,): native AppKit controls over config.toml. Every
//! change writes one key with `oxt_config::edit` and the file watcher applies
//! it, so the window and a hand edit take the same path. Colors stay in the
//! file; "Edit config.toml…" opens it.

use std::cell::{OnceCell, RefCell};
use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSBezelStyle, NSButton, NSButtonType, NSCellImagePosition, NSColor, NSControlStateValueOff, NSControlStateValueOn,
    NSFontManager, NSGridView, NSImage, NSLayoutAttribute, NSOpenPanel, NSPopUpButton, NSStackView, NSTextField, NSToolbar, NSToolbarDelegate,
    NSToolbarItem, NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowStyleMask, NSWindowToolbarStyle,
};
use objc2_foundation::{NSArray, NSEdgeInsets, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSUserDefaults};
use oxt_config::edit::{Value, write_key};
use oxt_config::icon::{self, OFFICIAL};
use oxt_config::{Config, IconChoice, ThemeMode};

use crate::macos::{self, IconApplied};

const PANES: [(&str, &str); 4] = [("Icon", "app.badge"), ("Font", "textformat"), ("Terminal", "terminal"), ("Appearance", "circle.lefthalf.filled")];
const LAST_PANE: &str = "SettingsPane";
const TILE: f64 = 72.0;
const WIDTH: f64 = 520.0;
const INSET: f64 = 24.0;
const ICON_COLUMNS: usize = 4;

thread_local! {
    static CONTROLLER: OnceCell<Retained<Controller>> = const { OnceCell::new() };
    /// Asks the GPUI side to show config.toml in the viewer.
    static OPEN_CONFIG: OnceCell<async_channel::Sender<()>> = const { OnceCell::new() };
}

/// Lets "Edit config.toml…" reach the app, which owns the viewer.
pub fn set_open_config(tx: async_channel::Sender<()>) {
    OPEN_CONFIG.with(|cell| _ = cell.set(tx));
}

/// Opens the window, or brings it forward with the current values.
pub fn show() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let controller = CONTROLLER.with(|cell| cell.get_or_init(|| Controller::new(mtm)).clone());
    controller.refresh();
    controller.ivars().window.center();
    controller.ivars().window.makeKeyAndOrderFront(None);
    #[cfg(debug_assertions)]
    controller.scripted_checks();
}

/// After config.toml changes (from here or by hand): show the new values.
pub fn refresh() {
    CONTROLLER.with(|cell| {
        if let Some(controller) = cell.get()
            && controller.ivars().window.isVisible()
        {
            controller.refresh();
        }
    });
}

struct Widgets {
    icon_grid: Retained<NSView>,
    icon_buttons: Vec<(String, Retained<NSButton>)>,
    remove: Retained<NSButton>,
    family: Retained<NSPopUpButton>,
    size: Retained<NSTextField>,
    line_height: Retained<NSTextField>,
    shell: Retained<NSTextField>,
    scrollback: Retained<NSTextField>,
    option_as_meta: Retained<NSButton>,
    theme: Retained<NSPopUpButton>,
}

pub struct Ivars {
    window: Retained<NSWindow>,
    pane_holder: Retained<NSView>,
    panes: RefCell<Vec<Retained<NSView>>>,
    banner: Retained<NSTextField>,
    status: Retained<NSTextField>,
    widgets: RefCell<Option<Widgets>>,
    /// The icon value currently selected, to tell which imported tile Remove acts on.
    icon: RefCell<String>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and Controller doesn't implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OxtSettingsController"]
    #[ivars = Ivars]
    pub struct Controller;

    impl Controller {
        #[unsafe(method(selectPane:))]
        fn select_pane_action(&self, sender: &NSToolbarItem) {
            let id = sender.itemIdentifier().to_string();
            if let Some(index) = PANES.iter().position(|(name, _)| *name == id) {
                self.select_pane(index);
            }
        }

        #[unsafe(method(iconClicked:))]
        fn icon_clicked(&self, sender: &NSButton) {
            let value = self.with_widgets(|w| w.icon_buttons.get(sender.tag() as usize).map(|(value, _)| value.clone())).flatten();
            if let Some(value) = value {
                self.write(&["app", "icon"], (!value.is_empty()).then(|| Value::from(value.as_str())));
            }
        }

        #[unsafe(method(importIcon:))]
        fn import_icon(&self, _sender: Option<&AnyObject>) {
            self.import();
        }

        #[unsafe(method(removeIcon:))]
        fn remove_icon(&self, _sender: Option<&AnyObject>) {
            self.remove_selected_import();
        }

        #[unsafe(method(fontChanged:))]
        fn font_changed(&self, _sender: Option<&AnyObject>) {
            let family = self.with_widgets(|w| w.family.titleOfSelectedItem().map(|t| t.to_string())).flatten();
            if let Some(family) = family {
                self.write(&["font", "family"], Some(Value::from(family.as_str())));
            }
        }

        #[unsafe(method(numberChanged:))]
        fn number_changed(&self, sender: &NSTextField) {
            let text = sender.stringValue().to_string();
            let Some((keys, label)) = self.number_key(sender) else { return };
            let value = if keys[1] == "scrollback" {
                text.trim().parse::<i64>().ok().filter(|n| *n >= 0).map(Value::from)
            } else {
                text.trim().parse::<f64>().ok().filter(|n| *n > 0.0).map(Value::from)
            };
            match value {
                Some(value) => self.write(&keys, Some(value)),
                None => {
                    self.set_status(&format!("{label} must be a positive number."));
                    self.refresh_values();
                }
            }
        }

        #[unsafe(method(shellChanged:))]
        fn shell_changed(&self, sender: &NSTextField) {
            self.write(&["terminal", "shell"], Some(Value::from(sender.stringValue().to_string().trim())));
        }

        #[unsafe(method(metaChanged:))]
        fn meta_changed(&self, sender: &NSButton) {
            self.write(&["terminal", "option_as_meta"], Some(Value::from(sender.state() == NSControlStateValueOn)));
        }

        #[unsafe(method(themeChanged:))]
        fn theme_changed(&self, sender: &NSPopUpButton) {
            let mode = sender.titleOfSelectedItem().map(|t| t.to_string().to_lowercase()).unwrap_or_default();
            self.write(&["theme", "mode"], Some(Value::from(mode.as_str())));
        }

        #[unsafe(method(editConfig:))]
        fn edit_config(&self, _sender: Option<&AnyObject>) {
            OPEN_CONFIG.with(|cell| {
                if let Some(tx) = cell.get() {
                    _ = tx.try_send(());
                }
            });
        }
    }

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl NSToolbarDelegate for Controller {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn toolbar_item(&self, _toolbar: &NSToolbar, id: &NSString, _insert: bool) -> Option<Retained<NSToolbarItem>> {
            self.make_toolbar_item(id)
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_items(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            pane_ids()
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_items(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            pane_ids()
        }

        #[unsafe(method_id(toolbarSelectableItemIdentifiers:))]
        fn selectable_items(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            pane_ids()
        }
    }
);

fn pane_ids() -> Retained<NSArray<NSString>> {
    let ids: Vec<Retained<NSString>> = PANES.iter().map(|(name, _)| NSString::from_str(name)).collect();
    NSArray::from_retained_slice(&ids)
}

impl Controller {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, 300.0));
        // SAFETY: the designated initializer; the window is kept by the controller, so it isn't released on close.
        let window =
            unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, style, NSBackingStoreType::Buffered, false) };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str("0xTerm Settings"));

        let banner = label("", mtm);
        banner.setTextColor(Some(&NSColor::systemRedColor()));
        banner.setHidden(true);
        let status = label("", mtm);
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let pane_holder = NSView::new(mtm);
        let content = stack(&[&banner, &pane_holder, &status], mtm);
        content.setEdgeInsets(NSEdgeInsets { top: 20.0, left: INSET, bottom: 20.0, right: INSET });
        window.setContentView(Some(&content));

        let this = Self::alloc(mtm).set_ivars(Ivars {
            window,
            pane_holder,
            panes: RefCell::new(Vec::new()),
            banner,
            status,
            widgets: RefCell::new(None),
            icon: RefCell::new(String::new()),
        });
        // SAFETY: NSObject's init.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        this.build_panes(mtm);

        let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &NSString::from_str("OxtSettings"));
        toolbar.setDelegate(Some(ProtocolObject::from_ref(&*this)));
        toolbar.setAllowsUserCustomization(false);
        this.ivars().window.setToolbar(Some(&toolbar));
        this.ivars().window.setToolbarStyle(NSWindowToolbarStyle::Preference);

        let last = NSUserDefaults::standardUserDefaults().integerForKey(&NSString::from_str(LAST_PANE)).clamp(0, PANES.len() as isize - 1);
        this.select_pane(last as usize);
        this
    }

    fn build_panes(&self, mtm: MainThreadMarker) {
        let target: &AnyObject = self.as_ref();

        // Icon
        let icon_grid = NSView::new(mtm);
        let import = button("Import…", target, sel!(importIcon:), mtm);
        let remove = button("Remove", target, sel!(removeIcon:), mtm);
        let note = label("Pick an icon, or import an .svg or .icns (up to 10 MB). More at 0xterm.org/icons.", mtm);
        note.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let buttons = row(&[&import, &remove], mtm);
        let icon_pane = stack(&[&icon_grid, &buttons, &note], mtm);

        // Font
        let family = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
        family.addItemsWithTitles(&NSFontManager::sharedFontManager(mtm).availableFontFamilies());
        set_action(&family, target, sel!(fontChanged:));
        let size = number_field(target, mtm);
        let line_height = number_field(target, mtm);
        let font_pane = form(&[("Family:", &family), ("Size:", &size), ("Line height:", &line_height)], mtm);

        // Terminal
        let shell = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
        shell.setPlaceholderString(Some(&NSString::from_str("Login shell ($SHELL)")));
        set_action(&shell, target, sel!(shellChanged:));
        send_on_end_editing(&shell);
        let scrollback = number_field(target, mtm);
        // SAFETY: a standard checkbox whose target is the long-lived controller.
        let option_as_meta = unsafe {
            NSButton::checkboxWithTitle_target_action(
                &NSString::from_str("Option key sends Meta (ESC+key)"),
                Some(target),
                Some(sel!(metaChanged:)),
                mtm,
            )
        };
        let terminal_pane = form(&[("Shell:", &shell), ("Scrollback lines:", &scrollback), ("", &option_as_meta)], mtm);

        // Appearance
        let theme = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
        let modes: Vec<Retained<NSString>> = ["System", "Light", "Dark"].iter().map(|m| NSString::from_str(m)).collect();
        theme.addItemsWithTitles(&NSArray::from_retained_slice(&modes));
        set_action(&theme, target, sel!(themeChanged:));
        let edit = button("Edit config.toml…", target, sel!(editConfig:), mtm);
        let colors = label("Theme colors are set in config.toml.", mtm);
        colors.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let appearance_pane = form(&[("Theme:", &theme), ("", &colors), ("", &edit)], mtm);

        *self.ivars().panes.borrow_mut() = vec![Retained::into_super(icon_pane), font_pane, terminal_pane, appearance_pane];
        *self.ivars().widgets.borrow_mut() =
            Some(Widgets { icon_grid, icon_buttons: Vec::new(), remove, family, size, line_height, shell, scrollback, option_as_meta, theme });
    }

    fn make_toolbar_item(&self, id: &NSString) -> Option<Retained<NSToolbarItem>> {
        let (name, symbol) = PANES.iter().find(|(name, _)| id.to_string() == *name)?;
        let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(self.mtm()), id);
        item.setLabel(&NSString::from_str(name));
        item.setImage(NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), None).as_deref());
        // SAFETY: the controller lives for the rest of the app (CONTROLLER), so the target outlives the item.
        unsafe {
            item.setTarget(Some(self));
            item.setAction(Some(sel!(selectPane:)));
        }
        Some(item)
    }

    fn select_pane(&self, index: usize) {
        let ivars = self.ivars();
        let panes = ivars.panes.borrow();
        let Some(pane) = panes.get(index) else { return };
        for view in ivars.pane_holder.subviews().iter() {
            view.removeFromSuperview();
        }
        ivars.pane_holder.addSubview(pane);
        pin(pane, &ivars.pane_holder);
        let (name, _) = PANES[index];
        if let Some(toolbar) = ivars.window.toolbar() {
            toolbar.setSelectedItemIdentifier(Some(&NSString::from_str(name)));
        }
        ivars.window.setTitle(&NSString::from_str(name));
        NSUserDefaults::standardUserDefaults().setInteger_forKey(index as isize, &NSString::from_str(LAST_PANE));
        drop(panes);
        self.fit_window();
    }

    /// Sizes the window to the pane it shows, keeping its top edge in place,
    /// as macOS settings windows do.
    fn fit_window(&self) {
        let window = &self.ivars().window;
        let Some(content) = window.contentView() else { return };
        content.layoutSubtreeIfNeeded();
        let fit = content.fittingSize();
        let size = NSSize::new(WIDTH, fit.height);
        let mut frame = window.frame();
        let target = window.frameRectForContentRect(NSRect::new(NSPoint::new(0.0, 0.0), size));
        frame.origin.y += frame.size.height - target.size.height;
        frame.size = target.size;
        window.setFrame_display_animate(frame, true, window.isVisible());
    }

    fn with_widgets<T>(&self, f: impl FnOnce(&Widgets) -> T) -> Option<T> {
        self.ivars().widgets.borrow().as_ref().map(f)
    }

    /// Re-reads config.toml into the controls. A file that doesn't parse
    /// shows its error and disables the controls until it's fixed, so the
    /// window never writes over a half-edited file.
    fn refresh(&self) {
        let (_, error) = Config::load();
        let ivars = self.ivars();
        ivars.banner.setStringValue(&NSString::from_str(
            &error.clone().map(|e| format!("config.toml has an error, so Settings can't change it: {e}")).unwrap_or_default(),
        ));
        ivars.banner.setHidden(error.is_none());
        self.rebuild_icons();
        self.refresh_values();
        let enabled = error.is_none();
        self.with_widgets(|w| {
            for (_, b) in &w.icon_buttons {
                b.setEnabled(enabled);
            }
            w.family.setEnabled(enabled);
            w.size.setEnabled(enabled);
            w.line_height.setEnabled(enabled);
            w.shell.setEnabled(enabled);
            w.scrollback.setEnabled(enabled);
            w.option_as_meta.setEnabled(enabled);
            w.theme.setEnabled(enabled);
        });
        if enabled {
            match crate::icon::last_applied() {
                Some(IconApplied::RuntimeOnly) => self.set_status("0xTerm.app couldn't be changed, so this icon shows only while 0xTerm runs."),
                Some(IconApplied::Unloadable) => self.set_status("That icon couldn't be loaded, so the built-in one is showing."),
                _ => {}
            }
        }
        self.fit_window();
    }

    fn refresh_values(&self) {
        let (config, _) = Config::load();
        *self.ivars().icon.borrow_mut() = config.app.icon.clone();
        let selected = selected_value(&config.app.icon);
        self.with_widgets(|w| {
            for (value, b) in &w.icon_buttons {
                b.setState(if *value == selected { NSControlStateValueOn } else { NSControlStateValueOff });
            }
            w.remove.setEnabled(matches!(IconChoice::parse(&config.app.icon), IconChoice::Imported(_)));
            w.family.selectItemWithTitle(&NSString::from_str(&config.font.family));
            w.size.setStringValue(&NSString::from_str(&format!("{}", config.font.size)));
            w.line_height.setStringValue(&NSString::from_str(&format!("{}", config.font.line_height)));
            w.shell.setStringValue(&NSString::from_str(&config.terminal.shell));
            w.scrollback.setStringValue(&NSString::from_str(&config.terminal.scrollback.to_string()));
            w.option_as_meta.setState(if config.terminal.option_as_meta { NSControlStateValueOn } else { NSControlStateValueOff });
            let mode = match config.theme.mode {
                ThemeMode::System => "System",
                ThemeMode::Light => "Light",
                ThemeMode::Dark => "Dark",
            };
            w.theme.selectItemWithTitle(&NSString::from_str(mode));
        });
    }

    /// One tile per choice: the built-in icon, the official set, then imports.
    fn rebuild_icons(&self) {
        let mtm = self.mtm();
        let target: &AnyObject = self.as_ref();
        let mut choices: Vec<(String, String, Option<PathBuf>)> = vec![("".into(), "Default".into(), default_icon_path())];
        for name in OFFICIAL {
            let choice = IconChoice::Bundled(name.to_string());
            choices.push((name.to_string(), name.to_string(), crate::icon::path_for(&choice)));
        }
        for file in imported_files() {
            let path = Config::icons_dir().map(|dir| dir.join(&file));
            choices.push((file.clone(), file, path));
        }

        let buttons: Vec<(String, Retained<NSButton>)> = choices
            .into_iter()
            .enumerate()
            .map(|(i, (value, title, path))| {
                let b = button(&title, target, sel!(iconClicked:), mtm);
                b.setButtonType(NSButtonType::OnOff);
                b.setBezelStyle(NSBezelStyle::SmallSquare);
                b.setImagePosition(NSCellImagePosition::ImageAbove);
                b.setTag(i as isize);
                if let Some(image) = path.as_deref().and_then(load_image) {
                    image.setSize(NSSize::new(TILE - 16.0, TILE - 16.0));
                    b.setImage(Some(&image));
                }
                (value, b)
            })
            .collect();

        let rows: Vec<Retained<NSArray<NSView>>> = buttons
            .chunks(ICON_COLUMNS)
            .map(|chunk| {
                let views: Vec<Retained<NSView>> = chunk.iter().map(|(_, b)| Retained::into_super(Retained::into_super(b.clone()))).collect();
                NSArray::from_retained_slice(&views)
            })
            .collect();
        let grid = NSGridView::gridViewWithViews(&NSArray::from_retained_slice(&rows), mtm);
        grid.setRowSpacing(8.0);
        grid.setColumnSpacing(8.0);

        self.with_widgets(|w| {
            for view in w.icon_grid.subviews().iter() {
                view.removeFromSuperview();
            }
            w.icon_grid.addSubview(&grid);
            pin(&grid, &w.icon_grid);
        });
        if let Some(w) = self.ivars().widgets.borrow_mut().as_mut() {
            w.icon_buttons = buttons;
        }
    }

    fn import(&self) {
        let mtm = self.mtm();
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        panel.setMessage(Some(&NSString::from_str("Choose an .svg or .icns icon")));
        if panel.runModal() != 1 {
            return;
        }
        let Some(path) = panel.URL().and_then(|url| url.path()).map(|p| PathBuf::from(p.to_string())) else { return };
        let result = icon::validate_import(&path).and_then(|name| {
            if load_image(&path).is_none() {
                return Err(format!("{} couldn't be loaded as an image.", path.display()));
            }
            icon::import(&path, &name).map(|_| name)
        });
        match result {
            Ok(name) => {
                self.set_status("");
                self.write(&["app", "icon"], Some(Value::from(name.as_str())));
            }
            Err(err) => self.set_status(&err),
        }
    }

    fn remove_selected_import(&self) {
        let current = self.ivars().icon.borrow().clone();
        let IconChoice::Imported(file) = IconChoice::parse(&current) else { return };
        if let Some(path) = Config::icons_dir().map(|dir| dir.join(&file))
            && let Err(err) = std::fs::remove_file(&path)
        {
            self.set_status(&format!("Couldn't remove {}: {err}", path.display()));
            return;
        }
        self.write(&["app", "icon"], None);
    }

    /// Writes one key; the watcher applies it and calls [`refresh`].
    fn write(&self, keys: &[&str], value: Option<Value>) {
        match write_key(keys, value) {
            Ok(()) => self.set_status(""),
            Err(err) => {
                self.set_status(&err);
                self.refresh_values();
            }
        }
    }

    fn number_key(&self, sender: &NSTextField) -> Option<([&'static str; 2], &'static str)> {
        self.with_widgets(|w| {
            if std::ptr::eq(sender, &*w.size) {
                Some((["font", "size"], "Size"))
            } else if std::ptr::eq(sender, &*w.line_height) {
                Some((["font", "line_height"], "Line height"))
            } else if std::ptr::eq(sender, &*w.scrollback) {
                Some((["terminal", "scrollback"], "Scrollback"))
            } else {
                None
            }
        })
        .flatten()
    }

    fn set_status(&self, text: &str) {
        self.ivars().status.setStringValue(&NSString::from_str(text));
    }

    /// Debug builds, for scripted checks without screen capture:
    /// OXTERM_SETTINGS_SNAPSHOT=/dir renders each pane to /dir/pane-N.png, and
    /// OXTERM_SETTINGS_CLICK=graphite presses that icon tile.
    #[cfg(debug_assertions)]
    fn scripted_checks(&self) {
        if let Ok(dir) = std::env::var("OXTERM_SETTINGS_SNAPSHOT") {
            let window = &self.ivars().window;
            for index in 0..PANES.len() {
                self.select_pane(index);
                let Some(view) = window.contentView() else { continue };
                view.layoutSubtreeIfNeeded();
                let bounds = view.bounds();
                if let Some(rep) = view.bitmapImageRepForCachingDisplayInRect(bounds) {
                    view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
                    // SAFETY: NSBitmapImageRep's PNG encoder with no properties.
                    let data: Option<Retained<objc2_foundation::NSData>> = unsafe {
                        msg_send![&*rep, representationUsingType: 4usize, properties: &*objc2_foundation::NSDictionary::<NSString, AnyObject>::new()]
                    };
                    if let Some(data) = data {
                        _ = std::fs::write(Path::new(&dir).join(format!("pane-{index}.png")), data.to_vec());
                    }
                }
            }
            self.select_pane(0);
        }
        if let Ok(value) = std::env::var("OXTERM_SETTINGS_CLICK") {
            let target = self.with_widgets(|w| w.icon_buttons.iter().find(|(v, _)| *v == value).map(|(_, b)| b.clone())).flatten();
            match target {
                // SAFETY: a button of this window, clicked on the main thread like a real click.
                Some(button) => unsafe { button.performClick(None) },
                None => eprintln!("OXTERM_SETTINGS_CLICK: no tile {value:?}"),
            }
        }
    }
}

/// The tile value that matches the configured icon: a missing import or an
/// unknown name shows as Default, which is what the app falls back to.
fn selected_value(icon: &str) -> String {
    match IconChoice::parse(icon) {
        IconChoice::Default => String::new(),
        choice => crate::icon::path_for(&choice).map(|_| icon.trim().to_string()).unwrap_or_default(),
    }
}

fn imported_files() -> Vec<String> {
    let Some(dir) = Config::icons_dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| matches!(IconChoice::parse(name), IconChoice::Imported(_)))
        .filter(|name| icon::IMPORT_EXTENSIONS.iter().any(|ext| name.to_ascii_lowercase().ends_with(&format!(".{ext}"))))
        .collect();
    files.sort();
    files
}

fn default_icon_path() -> Option<PathBuf> {
    macos::bundled_icons_dir()?.parent().map(|resources| resources.join("AppIcon.icns")).filter(|p| p.is_file())
}

fn load_image(path: &Path) -> Option<Retained<NSImage>> {
    NSImage::initWithContentsOfFile(NSImage::alloc(), &NSString::from_str(&path.to_string_lossy()))
}

/// A wrapping label no wider than the window's content, so long text adds lines, not width.
fn label(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    label.setPreferredMaxLayoutWidth(WIDTH - 2.0 * INSET);
    label
}

fn button(title: &str, target: &AnyObject, action: Sel, mtm: MainThreadMarker) -> Retained<NSButton> {
    // SAFETY: the target is the controller, which lives for the rest of the app.
    unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(title), Some(target), Some(action), mtm) }
}

fn set_action(control: &objc2_app_kit::NSControl, target: &AnyObject, action: Sel) {
    // SAFETY: as in `button`.
    unsafe {
        control.setTarget(Some(target));
        control.setAction(Some(action));
    }
}

fn number_field(target: &AnyObject, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
    set_action(&field, target, sel!(numberChanged:));
    send_on_end_editing(&field);
    field
}

/// Text fields send their action on Return; this also sends it when focus
/// leaves the field, so a typed value isn't lost by clicking elsewhere.
fn send_on_end_editing(field: &NSTextField) {
    // SAFETY: NSTextField's cell is an NSActionCell, which has this property.
    unsafe {
        let cell: Option<Retained<AnyObject>> = msg_send![field, cell];
        if let Some(cell) = cell {
            let _: () = msg_send![&*cell, setSendsActionOnEndEditing: true];
        }
    }
}

fn stack(views: &[&NSView], mtm: MainThreadMarker) -> Retained<NSStackView> {
    let views: Vec<&NSView> = views.to_vec();
    let stack = NSStackView::stackViewWithViews(&NSArray::from_slice(&views), mtm);
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    stack.setAlignment(NSLayoutAttribute::Leading);
    stack.setSpacing(14.0);
    stack
}

fn row(views: &[&NSView], mtm: MainThreadMarker) -> Retained<NSStackView> {
    let views: Vec<&NSView> = views.to_vec();
    let stack = NSStackView::stackViewWithViews(&NSArray::from_slice(&views), mtm);
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    stack.setSpacing(8.0);
    stack
}

/// Two columns, right-aligned labels and their controls, as macOS settings lay out.
fn form(rows: &[(&str, &NSView)], mtm: MainThreadMarker) -> Retained<NSView> {
    let rows: Vec<Retained<NSArray<NSView>>> = rows
        .iter()
        .map(|(title, control)| {
            let label: Retained<NSView> = Retained::into_super(Retained::into_super(NSTextField::labelWithString(&NSString::from_str(title), mtm)));
            let control: Retained<NSView> = (*control).retain();
            NSArray::from_retained_slice(&[label, control])
        })
        .collect();
    let grid = NSGridView::gridViewWithViews(&NSArray::from_retained_slice(&rows), mtm);
    grid.setRowSpacing(10.0);
    grid.columnAtIndex(0).setXPlacement(objc2_app_kit::NSGridCellPlacement::Trailing);
    grid.columnAtIndex(1).setWidth(280.0);
    Retained::into_super(grid)
}

/// Pins `view` to `parent`'s edges with Auto Layout.
fn pin(view: &NSView, parent: &NSView) {
    view.setTranslatesAutoresizingMaskIntoConstraints(false);
    for (a, b) in [(view.leadingAnchor(), parent.leadingAnchor()), (view.trailingAnchor(), parent.trailingAnchor())] {
        a.constraintEqualToAnchor(&b).setActive(true);
    }
    view.topAnchor().constraintEqualToAnchor(&parent.topAnchor()).setActive(true);
    view.bottomAnchor().constraintEqualToAnchor(&parent.bottomAnchor()).setActive(true);
}
