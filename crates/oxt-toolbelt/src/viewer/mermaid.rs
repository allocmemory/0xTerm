//! Mermaid diagrams without a browser: merman lays out and draws SVG, resvg
//! turns it into pixels. Results are cached by source and theme.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use merman::svg::SvgPipeline;
use merman::{OperationControl, RenderOutput, RenderRequest, Renderer, SvgRequest};

/// A rendered diagram: PNG bytes at 2× and its size in points.
#[derive(Clone)]
pub struct Diagram {
    pub png: Arc<Vec<u8>>,
    pub width: f32,
    pub height: f32,
}

const SCALE: f32 = 2.;

// Limits, so a hostile README can't tie up a core and gigabytes of memory.
// Drawing the pixels is the slow part: a 60 KB label of one word draws
// 174 M pixels in 12 s.
const MAX_SOURCE: usize = 64 * 1024;
const MAX_PIXELS: u64 = 4096 * 4096;
const DEADLINE: Duration = Duration::from_secs(5);

/// Why a diagram wasn't drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum RenderError {
    /// Over a size or time limit. Shown without its source, which may be huge.
    TooLarge(String),
    /// Not a diagram merman can draw. Shown with its source, to find the mistake.
    Invalid(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::TooLarge(why) | RenderError::Invalid(why) => f.write_str(why),
        }
    }
}

impl From<merman::RenderError> for RenderError {
    fn from(error: merman::RenderError) -> Self {
        match error {
            merman::RenderError::Cancelled(_) => RenderError::TooLarge(format!("diagram too large (took over {} s)", DEADLINE.as_secs())),
            merman::RenderError::ResourceLimitExceeded(_) => RenderError::TooLarge("diagram too large (too complex)".into()),
            error => RenderError::Invalid(error.to_string()),
        }
    }
}

type Cache = Mutex<HashMap<(String, bool), Result<Diagram, RenderError>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// System fonts load once; it takes a moment.
fn fonts() -> Arc<resvg::usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// The cached diagram, if it was rendered already.
pub fn cached(source: &str, dark: bool) -> Option<Result<Diagram, RenderError>> {
    cache().lock().unwrap().get(&(source.to_owned(), dark)).cloned()
}

/// Renders a diagram (slow: call off the UI thread) and caches it.
pub fn render(source: &str, dark: bool) -> Result<Diagram, RenderError> {
    if let Some(done) = cached(source, dark) {
        return done;
    }
    let result = render_uncached(source, dark);
    cache().lock().unwrap().insert((source.to_owned(), dark), result.clone());
    result
}

fn render_uncached(source: &str, dark: bool) -> Result<Diagram, RenderError> {
    let too_large = |why: String| RenderError::TooLarge(format!("diagram too large ({why})"));
    if source.len() > MAX_SOURCE {
        return Err(too_large(format!("{} KB, limit {} KB", source.len() / 1024, MAX_SOURCE / 1024)));
    }
    let themed = if dark && !source.contains("%%{init") { format!("%%{{init: {{'theme':'dark'}}}}%%\n{source}") } else { source.to_owned() };
    // resvg can't draw HTML labels; this pipeline turns them into SVG text.
    let request = SvgRequest { pipeline: Some(SvgPipeline::resvg_safe()), ..SvgRequest::default() };
    let svg = match Renderer::new().render(RenderRequest::svg(&themed, OperationControl::new().with_deadline(DEADLINE), request))? {
        // The diagram sits on the viewer's own background, in light and dark.
        RenderOutput::Svg(Some(svg)) => svg.svg().replacen("background-color:white", "background-color:transparent", 1),
        _ => return Err(RenderError::Invalid("not a Mermaid diagram".into())),
    };

    let options = resvg::usvg::Options { fontdb: fonts(), ..Default::default() };
    let tree = resvg::usvg::Tree::from_str(&svg, &options).map_err(|e| RenderError::Invalid(e.to_string()))?;
    let size = tree.size();
    let (w, h) = ((size.width() * SCALE).ceil() as u32, (size.height() * SCALE).ceil() as u32);
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(too_large(format!("{} × {} pt", size.width().round(), size.height().round())));
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or_else(|| too_large(format!("{w} × {h} px")))?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(SCALE, SCALE), &mut pixmap.as_mut());
    let png = pixmap.encode_png().map_err(|e| RenderError::Invalid(e.to_string()))?;
    Ok(Diagram { png: Arc::new(png), width: size.width(), height: size.height() })
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders_a_flowchart_and_reports_errors() {
        let d = super::render("flowchart LR\n  a[zsh] --> b[task]", false).unwrap();
        assert!(d.width > 50. && d.height > 10.);
        assert_eq!(&d.png[1..4], b"PNG");
        assert!(matches!(super::render("this is not mermaid", false), Err(super::RenderError::Invalid(_))));
    }

    /// The README's own diagram, the biggest real one in this repo.
    #[test]
    fn readme_diagram_still_renders() {
        let readme = include_str!("../../../../README.md");
        let source: String = readme.split("```mermaid\n").nth(1).unwrap().split("```").next().unwrap().into();
        assert!(super::render(&source, true).is_ok());
    }

    /// A long chain of nodes: what a hostile README could hold.
    fn chain(nodes: usize) -> String {
        let mut s = String::from("flowchart TD\n");
        for i in 0..nodes {
            s.push_str(&format!("  n{i}[node {i}] --> n{}[node {}]\n", i + 1, i + 1));
        }
        s
    }

    /// Hostile input gives up quickly instead of tying up a core.
    #[test]
    fn hostile_diagrams_give_up_quickly() {
        let wide = |n: usize| format!("flowchart LR\n  a[{}] --> b", "W".repeat(n));
        let mut slow = Vec::new();
        for (name, source) in
            [("chain", chain(3000)), ("wide label", wide(200_000)), ("small chain", chain(1500)), ("small wide label", wide(60_000))]
        {
            let start = std::time::Instant::now();
            let result = super::render_uncached(&source, false);
            let took = start.elapsed();
            eprintln!(
                "{name}: {} bytes of source, {took:?}, {}",
                source.len(),
                match &result {
                    Ok(d) => format!("ok {}x{} pt, {} KB png", d.width, d.height, d.png.len() / 1024),
                    Err(e) => format!("err: {e}"),
                }
            );
            if took > std::time::Duration::from_secs(6) || !matches!(result, Err(super::RenderError::TooLarge(_))) {
                slow.push(name);
            }
        }
        assert!(slow.is_empty(), "rendered or took too long: {slow:?}");
    }
}
