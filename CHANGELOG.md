# Changelog

All notable changes to 0xTerm are listed here. Versions follow
[Semantic Versioning](https://semver.org).

## [Unreleased]

### Fixed

- Settings… (⌘,), Minimize (⌘M), Zoom and Toggle Full Screen work; in 0.2.0
  they did nothing while a window was focused.

## [0.2.0] - 2026-10-25

### Added

- Releases are signed with a Developer ID and notarized by Apple, so macOS
  opens 0xTerm without a warning.
- Automatic updates through Sparkle, and **0xTerm → Check for Updates…**.
- The standard Mac menus: About 0xTerm, Settings… (⌘,, opens the config
  file), Services, Hide (⌘H), Hide Others (⌥⌘H), Show All, Minimize (⌘M),
  Zoom, Toggle Full Screen (⌃⌘F) and a Help menu.
- Drop a folder on the Dock icon, or use Open With in Finder, to open a
  window there; a text file opens in the viewer.
- Programs run in 0xTerm can ask for the camera, the microphone, Apple
  events and protected folders, with a prompt that names 0xTerm.

## [0.1.1] - 2026-10-15

A bug-fix release.

### Fixed

- Tab and Shift+Tab reach the shell, so file, folder and command completion
  work.
- Shift+Enter is sent as its own key through the kitty keyboard protocol.

### Added

- `packaging/install.sh` builds 0xTerm and installs it to /Applications.

## [0.1.0] - 2026-10-09

The first release.

### Added

- GPU-drawn terminal on Metal: 16, 256 and truecolor; bold, italic,
  underline and wide characters; block, bar and underline cursors.
- Your login shell on its own pty per pane, with `TERM=xterm-256color`,
  truecolor and your `PATH` even when launched from the Dock.
- Scrollback, mouse selection (word, line and block), copy and paste,
  bracketed paste, mouse reporting, focus events, synchronized updates and
  IME input.
- Tabs named after the running job, split panes with draggable dividers, and
  dimmed inactive panes.
- A file tree that follows the active pane's working folder through OSC 7 or
  the process table, hides `.git` and `.gitignore`d files, and updates live.
- Folder search off the main thread, grouped by file.
- A file viewer with syntax highlighting, line numbers, ⌘F find and live
  reload; Markdown preview with native Mermaid diagrams.
- ⌘-click on `path:line`, URLs and OSC 8 hyperlinks; drag a file onto a
  pane to type its path.
- `~/.config/0xterm/config.toml` with fonts, shell, scrollback and light and
  dark palettes, reloaded on save; the theme follows the system appearance.
- Native menus, a Dock icon, and a DMG installer.

### Known issues

- Expanding Desktop, Documents or Downloads in the file tree shows the macOS
  privacy prompt, and the window waits until you answer it.
- Emoji are drawn slightly wider than two cells.
- Wide Mermaid diagrams shrink to the viewer's width.
- Releases are ad-hoc signed, not notarized. See the README for the
  first-launch steps.

[0.1.1]: https://github.com/allocmemory/0xTerm/releases/tag/v0.1.1
[0.1.0]: https://github.com/allocmemory/0xTerm/releases/tag/v0.1.0
