//! Renders PDF pages to PPM files, for checking the renderer by eye or
//! against another one: `render <file.pdf> <width> <out-dir> [pages]`.

use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: render <file.pdf> <width> <out-dir> [first-page [last-page]]");
        std::process::exit(2);
    }
    let data = std::fs::read(&args[1]).expect("read the file");
    let width: usize = args[2].parse().expect("a width");
    let t = Instant::now();
    let doc = match lain_pdf::Document::open(data) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("open: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("opened in {:?}: {} pages, repaired {}", t.elapsed(), doc.page_count(), doc.repaired);
    let first: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
    let last: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(doc.page_count()).min(doc.page_count());
    for p in first..=last {
        let t = Instant::now();
        let opts = lain_pdf::render::RenderOptions { width, ..Default::default() };
        match lain_pdf::render::render_page(&doc, p - 1, &opts) {
            Ok(r) => {
                let mut out = format!("P6\n{} {}\n255\n", r.width, r.height).into_bytes();
                out.extend_from_slice(&r.rgb);
                std::fs::write(format!("{}/p{p:04}.ppm", args[3]), out).expect("write");
                eprintln!(
                    "page {p}: {:?} ops {} images {} missing_fonts {} unsupported {:?}{}",
                    t.elapsed(),
                    r.stats.ops,
                    r.stats.images,
                    r.stats.missing_fonts,
                    r.stats.unsupported,
                    if r.stats.truncated { " TRUNCATED" } else { "" }
                );
            }
            Err(e) => eprintln!("page {p}: {e}"),
        }
    }
}
