//! Generate a synthetic MDX fixture: gen-mdx <1|2> <path>.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let version = args.next().unwrap_or_else(|| usage());
    let path = PathBuf::from(args.next().unwrap_or_else(|| usage()));
    let entries = [
        ("alpha", "<p>alpha body</p>"),
        ("Bravo", "<p>bravo body</p>"),
        ("bravo", "<p>second bravo body</p>"),
        ("charlie", "<p>charlie body</p>"),
    ];
    let result = match version.as_str() {
        "1" => odc_testkit::write_v1_mdx(&path, &entries),
        "2" => odc_testkit::write_v2_mdx(&path, &entries),
        other => {
            eprintln!("unknown version {other}");
            std::process::exit(2);
        }
    };
    if let Err(error) = result {
        eprintln!("fixture error: {error}");
        std::process::exit(1);
    }
    println!("wrote {}", path.display());
}

fn usage() -> ! {
    eprintln!("usage: gen-mdx <1|2> <path>");
    std::process::exit(2);
}
