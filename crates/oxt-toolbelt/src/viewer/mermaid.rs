//! Mermaid diagrams without a browser: merman lays out and draws SVG, resvg
//! turns it into pixels. Results are cached by source and theme.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

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

type Cache = Mutex<HashMap<(String, bool), Result<Diagram, String>>>;

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
pub fn cached(source: &str, dark: bool) -> Option<Result<Diagram, String>> {
    cache().lock().unwrap().get(&(source.to_owned(), dark)).cloned()
}

/// Renders a diagram (slow: call off the UI thread) and caches it.
pub fn render(source: &str, dark: bool) -> Result<Diagram, String> {
    if let Some(done) = cached(source, dark) {
        return done;
    }
    let result = render_uncached(source, dark);
    cache().lock().unwrap().insert((source.to_owned(), dark), result.clone());
    result
}

fn render_uncached(source: &str, dark: bool) -> Result<Diagram, String> {
    let themed = if dark && !source.contains("%%{init") { format!("%%{{init: {{'theme':'dark'}}}}%%\n{source}") } else { source.to_owned() };
    // resvg can't draw HTML labels; this pipeline turns them into SVG text.
    let request = SvgRequest { pipeline: Some(SvgPipeline::resvg_safe()), ..SvgRequest::default() };
    let svg = match Renderer::new().render(RenderRequest::svg(&themed, OperationControl::new(), request)).map_err(|e| e.to_string())? {
        // The diagram sits on the viewer's own background, in light and dark.
        RenderOutput::Svg(Some(svg)) => svg.svg().replacen("background-color:white", "background-color:transparent", 1),
        _ => return Err("not a Mermaid diagram".into()),
    };

    let options = resvg::usvg::Options { fontdb: fonts(), ..Default::default() };
    let tree = resvg::usvg::Tree::from_str(&svg, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (w, h) = ((size.width() * SCALE).ceil() as u32, (size.height() * SCALE).ceil() as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or("diagram too large")?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(SCALE, SCALE), &mut pixmap.as_mut());
    let png = pixmap.encode_png().map_err(|e| e.to_string())?;
    Ok(Diagram { png: Arc::new(png), width: size.width(), height: size.height() })
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders_a_flowchart_and_reports_errors() {
        let d = super::render("flowchart LR\n  a[zsh] --> b[task]", false).unwrap();
        assert!(d.width > 50. && d.height > 10.);
        assert_eq!(&d.png[1..4], b"PNG");
        assert!(super::render("this is not mermaid", false).is_err());
    }
}
