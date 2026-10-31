//! `gen-entries`: deterministic synthetic entry generator.
//!
//! Used by `scripts/bench-1m.sh` and by integration tests that need a large,
//! reproducible working set. The same arguments always produce byte-identical
//! output, and no real dictionary content is ever involved.

use std::io::{BufWriter, Write};
use std::path::PathBuf;

/// Parsed command line.
struct Args {
    count: usize,
    out: PathBuf,
    seed: u64,
    stride: usize,
    start: usize,
}

fn usage() -> ! {
    eprintln!("usage: gen-entries --count N --out PATH [--seed S] [--stride K] [--start I]");
    std::process::exit(2);
}

fn parse_args() -> Args {
    let mut count: Option<usize> = None;
    let mut out: Option<PathBuf> = None;
    let mut seed = 1u64;
    let mut stride = 1usize;
    let mut start = 0usize;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = || arguments.next().unwrap_or_else(|| usage());
        match argument.as_str() {
            "--count" => count = Some(value().parse().unwrap_or_else(|_| usage())),
            "--out" => out = Some(PathBuf::from(value())),
            "--seed" => seed = value().parse().unwrap_or_else(|_| usage()),
            "--stride" => stride = value().parse().unwrap_or_else(|_| usage()),
            "--start" => start = value().parse().unwrap_or_else(|_| usage()),
            _ => usage(),
        }
    }
    let (Some(count), Some(out)) = (count, out) else {
        usage();
    };
    if count == 0 || stride == 0 {
        usage();
    }
    Args {
        count,
        out,
        seed,
        stride,
        start,
    }
}

/// One synthetic entry, written as one JSONL line.
fn write_entry<W: Write>(out: &mut W, index: usize, seed: u64) -> std::io::Result<()> {
    let word = format!("w{index:07}");
    let display = format!("W{index:07}");
    let jitter = (u64::try_from(index)
        .unwrap_or(0)
        .wrapping_mul(2_654_435_761)
        % 97)
        + seed;
    writeln!(
        out,
        "{{\"word\":\"{word}\",\"display\":\"{display}\",\
         \"phonetic\":[\"{display} phonetic\"],\
         \"defs\":[{{\"pos\":\"noun\",\"sense\":\"sense {jitter} for {word}\",\
         \"examples\":[\"example {jitter} of {word}\"]}}]}}"
    )
}

fn main() -> std::io::Result<()> {
    let args = parse_args();
    let file = std::fs::File::create(&args.out)?;
    let mut out = BufWriter::with_capacity(256 * 1024, file);
    let mut index = args.start;
    for _ in 0..args.count {
        write_entry(&mut out, index, args.seed)?;
        index = index.saturating_add(args.stride);
    }
    out.flush()?;
    Ok(())
}
