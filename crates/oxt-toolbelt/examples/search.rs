//! `cargo run --release --example search -- <folder> <text>`: the sidebar search, in a shell.

use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(root), Some(query)) = (args.next(), args.next()) else {
        eprintln!("usage: search <folder> <text>");
        std::process::exit(2);
    };
    let start = Instant::now();
    let results = oxt_toolbelt::search::search(root.as_ref(), &query, &AtomicBool::new(false));
    for file in &results.files {
        println!("{}", file.path.display());
        for hit in &file.hits {
            println!("  {:>5}  {}", hit.line, hit.text);
        }
    }
    let more = if results.truncated { "+" } else { "" };
    eprintln!("{}{more} hits in {} files, {:?}", results.total, results.files.len(), start.elapsed());
}
