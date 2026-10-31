//! Lookup latency benchmark: samples committed keys and reports p50/p95.
//!
//! Usage: cargo run --release --example lookup_bench -- <repo-dir> [samples]

use odc_core::repo::Repo;
use std::time::{Duration, Instant};

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args
        .next()
        .expect("usage: lookup_bench <repo-dir> [samples]");
    let samples: usize = args
        .next()
        .map_or(1000, |value| value.parse().expect("sample count"));

    let repo = Repo::open(&dir).expect("open repository");
    let mut keys: Vec<String> = Vec::new();
    repo.for_each_entry(|entry| {
        keys.push(entry.word);
        Ok(())
    })
    .expect("scan entries");
    assert!(!keys.is_empty(), "repository has no committed entries");

    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut times: Vec<Duration> = Vec::with_capacity(samples);
    for _ in 0..samples {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let key = &keys[(state % keys.len() as u64) as usize];
        let start = Instant::now();
        let hit = repo.show(key).expect("lookup");
        times.push(start.elapsed());
        assert!(hit.is_some(), "sampled key {key} disappeared");
    }
    times.sort();
    let p50 = times[times.len() / 2];
    let p95 = times[times.len() * 95 / 100];
    let max = *times.last().expect("samples");
    println!(
        "lookups={} keys={} p50={:?} p95={:?} max={:?}",
        samples,
        keys.len(),
        p50,
        p95,
        max
    );
}
