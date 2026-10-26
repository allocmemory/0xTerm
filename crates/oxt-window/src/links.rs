//! Where a clicked URL goes, for the terminal and the viewer alike. macOS
//! would run a `file:` app or script, or hand an odd scheme to whatever app
//! claims it, so only web and mail links leave the app; files open in the
//! viewer. A link whose target isn't on screen asks first.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// A file, for the viewer.
    File(PathBuf),
    /// A web or mail link, for the browser or mail app, as the parser read it.
    External(String),
}

/// Where a link came from, which decides whether its target is in view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The URL itself, printed in the output.
    Text,
    /// An OSC 8 hyperlink over this text on screen.
    Hyperlink(String),
    /// A Markdown link in the viewer.
    Markdown,
}

/// The route for a URL, or `None` for one that goes nowhere.
pub fn route(url: &str) -> Option<Route> {
    let parsed = url::Url::parse(url).ok()?;
    match parsed.scheme() {
        "file" => {
            // The host is ignored: `ls --hyperlink` puts the Mac's name there.
            let path = url::Url::parse(&format!("file://{}", parsed.path())).ok()?.to_file_path().ok()?;
            path.is_file().then_some(Route::File(path))
        }
        // The parsed form, so what the prompt shows and macOS opens is what
        // was checked, not a raw spelling another parser might read differently.
        "http" | "https" | "mailto" => Some(Route::External(parsed.into())),
        _ => None,
    }
}

/// What to show before opening `url` outside the app, or `None` to open it
/// right away because the URL itself is what was clicked.
pub fn confirm_detail(url: &str, source: &Source) -> Option<String> {
    match source {
        Source::Text => None,
        // Compared as parsed, the way `url` was: `https://Example.com` on screen
        // is the same link as `https://example.com/`.
        Source::Hyperlink(shown) if !same_url(shown.trim(), url) => Some(format!("{url}\n\nThe text on screen says: “{}”", shown.trim())),
        Source::Hyperlink(_) | Source::Markdown => Some(url.to_owned()),
    }
}

fn same_url(shown: &str, url: &str) -> bool {
    shown == url || url::Url::parse(shown).is_ok_and(|parsed| parsed.as_str() == url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_and_mail_links_leave_the_app() {
        assert_eq!(route("https://example.com/a"), Some(Route::External("https://example.com/a".into())));
        assert_eq!(route("mailto:a@b.c"), Some(Route::External("mailto:a@b.c".into())));
    }

    #[test]
    fn the_url_shown_and_opened_is_the_one_checked() {
        // Odd spellings the parser cleans up: the prompt and macOS get the clean form.
        assert_eq!(route(r"https:\\good.com\@evil.com"), Some(Route::External("https://good.com/@evil.com".into())));
        assert_eq!(route("https:evil.com"), Some(Route::External("https://evil.com/".into())));
        assert_eq!(route("HTTPS://Example.COM/a"), Some(Route::External("https://example.com/a".into())));
    }

    #[test]
    fn files_and_odd_schemes_never_reach_macos() {
        let dir = std::env::temp_dir().join(format!("oxt-route-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let file = dir.join("run.command");
        std::fs::write(&file, "").unwrap();
        let path = file.to_str().unwrap();

        for url in [format!("file://{path}"), format!("FILE://{path}"), format!("File://{path}"), format!("file:{path}")] {
            assert_eq!(route(&url), Some(Route::File(file.clone())), "{url}");
        }
        for url in ["smb://evil/share", "ssh://evil", "vscode://x", "ftp://example.com/a", "x-man-page://ls"] {
            assert_eq!(route(url), None, "{url}");
        }

        // Still opens: a host (`ls --hyperlink`, ripgrep), and an escaped name.
        assert_eq!(route(&format!("file://my-mac.local{path}")), Some(Route::File(file.clone())));
        let spaced = dir.join("a b.txt");
        std::fs::write(&spaced, "").unwrap();
        assert_eq!(route(&format!("file://{}/a%20b.txt", dir.display())), Some(Route::File(spaced)));
        assert_eq!(route(&format!("file://{}", dir.display())), None, "a folder isn't a file to view");

        // From the bytes a program prints to where a ⌘-click goes.
        let (tx, _rx) = async_channel::unbounded();
        let mut engine = oxt_session::Engine::new(oxt_session::GridSize { rows: 2, cols: 20 }, 0, oxt_session::Listener::new(|_| {}, tx));
        engine.advance(format!("\x1b]8;;FILE://{path}\x1b\\github.com\x1b]8;;\x1b\\").as_bytes());
        use oxt_session::alacritty_terminal::index::{Column, Line, Point};
        let cell = &engine.term.grid()[Point::new(Line(0), Column(0))];
        assert_eq!(cell.c, 'g');
        assert_eq!(route(cell.hyperlink().unwrap().uri()), Some(Route::File(file.clone())));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_hidden_targets_ask_first() {
        let url = "https://evil.example/x";
        assert_eq!(confirm_detail(url, &Source::Text), None);
        assert_eq!(confirm_detail(url, &Source::Markdown), Some(url.to_owned()));
        assert_eq!(confirm_detail(url, &Source::Hyperlink("github.com".into())), Some(format!("{url}\n\nThe text on screen says: “github.com”")));
        // A hyperlink that shows its own URL still asks: it's a link, not text you read.
        assert_eq!(confirm_detail(url, &Source::Hyperlink(url.into())), Some(url.to_owned()));
        // The same URL, spelled the way people write it, isn't a mismatch.
        for shown in ["https://example.com", "HTTPS://Example.COM/", " https://example.com:443/ "] {
            assert_eq!(confirm_detail("https://example.com/", &Source::Hyperlink(shown.into())), Some("https://example.com/".into()), "{shown:?}");
        }
    }
}
