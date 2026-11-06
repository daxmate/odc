//! LDOCE6 adapter acceptance harness (spec `docs/odc-html-spec.md` section 8).
//!
//! Samples 200 MDX records with a fixed seed, adds the polysemy keys, maps each
//! one to L1, and writes:
//!
//! * `/tmp/odc-ldoce6-adapter/samples/<key>.html` — normalised output,
//! * `/tmp/odc-ldoce6-adapter/viewer.html` — side-by-side comparison,
//! * `metrics.json` and `report.md` — the acceptance numbers.
//!
//! Run with `cargo run -p odc-core --release --example ldoce6_adapter`.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use odc_core::adapters::ldoce6::{redirect_target, same_words, Ldoce6, SLUG};
use odc_core::adapters::{count_slots, Adapter, ParseContext, SlotCounts};
use odc_core::html::repair_sanitize;
use odc_core::mdx::{MdxRecord, MdxSource, MAX_RECORD_BYTES};
use serde_json::{json, Map, Value};

/// Number of randomly sampled records.
const SAMPLE_SIZE: usize = 200;
/// Fixed seed for the reservoir sampler (deterministic across runs).
const SEED: u64 = 0x5EED_0DC6_2026_1105;
/// Extra polysemy keys appended to the sample.
const POLYSEMY_LIMIT: usize = 12;
/// Default output directory.
const OUT_DIR: &str = "/tmp/odc-ldoce6-adapter";
/// Default dictionary path.
const DEFAULT_MDX: &str =
    "/Users/dax/Documents/Personal/Dictionary/ldoce6encn/LDOCE6++ En-Cn V2-19.mdx";
/// Polysemy key list produced by the earlier survey.
const POLYSEMY_LIST: &str = "/tmp/odc-polysemy-survey/keys_en.txt";

/// xorshift64* — small, seeded, reproducible.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// One sampled record plus everything the metrics need.
struct Sample {
    key: String,
    file: String,
    clean: String,
    l1: String,
    unadapted: Option<String>,
    slots: SlotCounts,
    expected: Vec<&'static str>,
    fidelity_ok: bool,
    split_lines: usize,
    idempotent: bool,
    elapsed_us: u128,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mdx_path = args.next().unwrap_or_else(|| String::from(DEFAULT_MDX));
    let out_dir = PathBuf::from(args.next().unwrap_or_else(|| String::from(OUT_DIR)));
    fs::create_dir_all(&out_dir)?;

    let source = MdxSource::open(&mdx_path)?;
    println!("dictionary: {mdx_path}");
    println!("records: {}", source.len());

    let wanted = read_polysemy_keys();
    let (sampled, stats) = sample_records(&source, &wanted)?;
    println!(
        "sampled: {} (eligible {}, skipped empty {}, skipped >1 MiB {}, skipped redirect {}, read errors {})",
        sampled.len(),
        stats.eligible,
        stats.skipped_empty,
        stats.skipped_large,
        stats.skipped_redirect,
        stats.read_errors
    );

    let adapter = Ldoce6;
    let mut samples = Vec::with_capacity(sampled.len());
    for record in &sampled {
        samples.push(run_one(&adapter, record));
    }
    // Two keys can fold onto one file name (case or punctuation); keep both.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for sample in &mut samples {
        let base = sample.file.clone();
        let count = seen.entry(base.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            sample.file = format!("{base}-{count}");
        }
    }

    write_samples(&out_dir, &samples)?;
    write_viewer(&out_dir, &samples, &mdx_path)?;
    let metrics = write_metrics(&out_dir, &samples, &stats, &mdx_path)?;
    write_report(&out_dir, &metrics)?;
    println!("wrote {}", out_dir.display());
    Ok(())
}

/// Sampling statistics.
#[derive(Default)]
struct SampleStats {
    eligible: usize,
    skipped_empty: usize,
    skipped_large: usize,
    skipped_redirect: usize,
    read_errors: usize,
    polysemy_found: Vec<String>,
}

fn read_polysemy_keys() -> Vec<String> {
    fs::read_to_string(POLYSEMY_LIST)
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Reservoir-sample `SAMPLE_SIZE` eligible records and capture the polysemy keys.
fn sample_records(
    source: &MdxSource,
    wanted: &[String],
) -> Result<(Vec<MdxRecord>, SampleStats), Box<dyn std::error::Error>> {
    let mut stats = SampleStats::default();
    let mut rng = Rng::new(SEED);
    let mut reservoir: Vec<MdxRecord> = Vec::new();
    let mut extra: Vec<MdxRecord> = Vec::new();
    let mut seen_polysemy: Vec<String> = Vec::new();
    for record in source.records() {
        let record = match record {
            Ok(record) => record,
            Err(_) => {
                stats.read_errors += 1;
                continue;
            }
        };
        if wanted.iter().any(|key| key == &record.key)
            && !seen_polysemy.iter().any(|key| key == &record.key)
            && extra.len() < POLYSEMY_LIMIT
        {
            seen_polysemy.push(record.key.clone());
            extra.push(MdxRecord {
                key: record.key.clone(),
                html: record.html.clone(),
            });
        }
        if record.html.trim().is_empty() {
            stats.skipped_empty += 1;
            continue;
        }
        if redirect_target(&record.html).is_some() {
            stats.skipped_redirect += 1;
            continue;
        }
        if record.html.len() > MAX_RECORD_BYTES {
            stats.skipped_large += 1;
            continue;
        }
        stats.eligible += 1;
        if reservoir.len() < SAMPLE_SIZE {
            reservoir.push(MdxRecord {
                key: record.key.clone(),
                html: record.html.clone(),
            });
        } else {
            #[allow(clippy::cast_possible_truncation)]
            let slot = (rng.next() % stats.eligible as u64) as usize;
            if slot < SAMPLE_SIZE {
                reservoir[slot] = MdxRecord {
                    key: record.key.clone(),
                    html: record.html.clone(),
                };
            }
        }
    }
    stats.polysemy_found.clone_from(&seen_polysemy);
    reservoir.sort_by(|a, b| a.key.cmp(&b.key));
    extra.sort_by(|a, b| a.key.cmp(&b.key));
    reservoir.extend(extra);
    Ok((reservoir, stats))
}

/// Map one record and collect its metrics.
fn run_one(adapter: &Ldoce6, record: &MdxRecord) -> Sample {
    let ctx = ParseContext::new(record.key.clone(), SLUG, "en");
    let clean = repair_sanitize(&record.html).unwrap_or_else(|| record.html.clone());
    let expected = odc_core::adapters::source_slots(&clean);

    let start = Instant::now();
    let l1 = adapter
        .parse(&record.html, &ctx)
        .unwrap_or_else(|_| String::new());
    let elapsed_us = start.elapsed().as_micros();

    let unadapted = unadapted_reason(&l1);
    // Popups move into the toolbar, so fidelity is word-level, not order-level.
    let fidelity_ok = same_words(&clean, &l1);
    let split_lines = odc_core::adapters::split_line_violations(&l1);
    let slots = count_slots(&l1);
    let idempotent = adapter
        .parse(&l1, &ctx)
        .map(|again| again == l1)
        .unwrap_or(false);

    Sample {
        key: record.key.clone(),
        file: file_name(&record.key),
        clean,
        l1,
        unadapted,
        slots,
        expected,
        fidelity_ok,
        split_lines,
        idempotent,
        elapsed_us,
    }
}

fn unadapted_reason(l1: &str) -> Option<String> {
    if !l1.contains("data-odc-unadapted=\"1\"") {
        return None;
    }
    let marker = "data-odc-reason=\"";
    let start = l1.find(marker)? + marker.len();
    let rest = &l1[start..];
    let end = rest.find('"')?;
    Some(String::from(&rest[..end]))
}

/// Write one file per sample plus the raw / L1 panes used by the viewer.
fn write_samples(out_dir: &Path, samples: &[Sample]) -> std::io::Result<()> {
    let samples_dir = out_dir.join("samples");
    fs::create_dir_all(&samples_dir)?;
    let raw_dir = out_dir.join("panes").join("raw");
    let l1_dir = out_dir.join("panes").join("l1");
    fs::create_dir_all(&raw_dir)?;
    fs::create_dir_all(&l1_dir)?;
    for dir in [&samples_dir, &raw_dir, &l1_dir] {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "html") {
                fs::remove_file(path)?;
            }
        }
    }
    for sample in samples {
        let name = &sample.file;
        fs::write(samples_dir.join(format!("{name}.html")), &sample.l1)?;
        fs::write(
            raw_dir.join(format!("{name}.html")),
            page(&sample.key, "raw", &sample.clean),
        )?;
        fs::write(
            l1_dir.join(format!("{name}.html")),
            page(&sample.key, "l1", &sample.l1),
        )?;
    }
    Ok(())
}

/// Substitute characters that are unsafe in file names.
fn file_name(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for ch in key.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        String::from("_")
    } else {
        out
    }
}

/// Minimal ODC stylesheet stub used by the right-hand pane.
const ODC_STUB: &str = r"
body { font-family: -apple-system, 'Helvetica Neue', sans-serif; margin: 12px; color: #1c1c1e; }
.odc-entry { display: block; }
.odc-head { border-bottom: 1px solid #d0d0d5; padding-bottom: 6px; margin-bottom: 10px; }
.odc-hw { display: inline; font-size: 22px; margin: 0 6px 0 0; }
.odc-hw-alt { color: #6b6b70; font-size: 13px; margin-right: 6px; }
.odc-phon { color: #48484a; margin-right: 8px; }
.odc-inflection { color: #48484a; margin-right: 8px; }
.odc-pos-block { margin: 14px 0; }
.odc-pos { display: inline-block; font-size: 14px; font-style: italic; color: #8e8e93; margin: 0 0 6px; }
.odc-sense { margin: 6px 0 6px 14px; position: relative; }
.odc-sense-number { font-weight: 600; margin-right: 4px; color: #0a84ff; }
.odc-subsense { margin-left: 18px; }
.odc-signpost { font-variant: small-caps; font-weight: 600; color: #b25000; margin-right: 6px; }
.odc-grammar { color: #6b6b70; font-style: italic; margin-right: 6px; }
.odc-def { display: inline; }
.odc-def-trans { color: #c0392b; margin-left: 6px; }
.odc-examples { margin: 4px 0 6px 0; padding-left: 18px; }
.odc-example { margin: 2px 0; }
.odc-ex-text { color: #2c2c2e; }
.odc-ex-trans { color: #c0392b; display: block; }
.odc-label { color: #6b6b70; font-style: italic; margin-right: 4px; }
.odc-collocation { color: #2c2c2e; font-weight: 500; }
.odc-pvs { margin-top: 12px; border-top: 1px dashed #d0d0d5; }
.odc-pv { margin: 10px 0 10px 4px; }
.odc-pv-hw { font-size: 16px; margin: 0 0 4px; color: #1c1c1e; }
.odc-refs { margin-top: 16px; }
.odc-ref { border-top: 1px solid #e5e5ea; margin-top: 10px; padding-top: 6px; }
.odc-ref-title { font-size: 12px; letter-spacing: .08em; text-transform: uppercase; color: #6b6b70; }
.odc-toc { background: #f2f2f7; padding: 6px 10px; margin: 10px 0; font-size: 13px; }
.odc-crossref { color: #0a84ff; text-decoration: none; }
.odc-audio { width: 14px; height: 14px; border: 0; background: #0a84ff; border-radius: 50%; margin: 0 3px; }
.odc-img { max-width: 100%; }
.odc-phrase-index { display: inline-block; margin-right: 10px; }
";

/// One pane: either the original fragment with the dictionary CSS, or L1.
fn page(key: &str, kind: &str, body: &str) -> String {
    let head = if kind == "raw" {
        String::from(
            "<link rel=\"stylesheet\" type=\"text/css\" href=\"../../ldoce6ec.css\">\
             <style>body{background:#fff;margin:10px;font-size:13px}</style>",
        )
    } else {
        format!("<style>{ODC_STUB}</style>")
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title>{head}</head>\
         <body data-pane=\"{kind}\">{body}</body></html>",
        html_escape(key)
    )
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Pick the entries shown in the viewer: polysemy keys first, then spread.
fn viewer_selection(samples: &[Sample], limit: usize) -> Vec<usize> {
    let mut picked: Vec<usize> = Vec::new();
    for (index, sample) in samples.iter().enumerate() {
        if sample
            .key
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
            || [
                "make", "take", "get", "run", "set", "work", "time", "way", "play", "point",
                "line", "turn", "hold", "stand", "break", "call", "go", "come", "look", "put",
            ]
            .contains(&sample.key.as_str())
        {
            picked.push(index);
            if picked.len() >= limit {
                return picked;
            }
        }
    }
    let stride = samples.len().max(1) / limit.max(1);
    let mut cursor = 0;
    while picked.len() < limit && cursor < samples.len() {
        if !picked.contains(&cursor) {
            picked.push(cursor);
        }
        cursor += stride.max(1);
    }
    picked
}

fn write_viewer(out_dir: &Path, samples: &[Sample], mdx_path: &str) -> std::io::Result<()> {
    let picked = viewer_selection(samples, 20);
    let mut body = String::new();
    let mut nav = String::new();
    for index in &picked {
        let sample = &samples[*index];
        let name = &sample.file;
        let _ = write!(nav, "<a href=\"#{name}\">{}</a>", html_escape(&sample.key));
        let _ = write!(
            body,
            "<section id=\"{name}\"><h2>{}</h2><div class=\"pair\">\
             <figure><figcaption>原样 (ldoce6ec.css)</figcaption>\
             <iframe loading=\"lazy\" src=\"panes/raw/{name}.html\"></iframe></figure>\
             <figure><figcaption>规范化 (ODC L1)</figcaption>\
             <iframe loading=\"lazy\" src=\"panes/l1/{name}.html\"></iframe></figure>\
             </div></section>",
            html_escape(&sample.key)
        );
    }
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>LDOCE6 adapter viewer</title>\
         <style>body{{font:14px -apple-system,sans-serif;margin:0;padding:16px;background:#f2f2f7}}\
         h1{{font-size:18px}}nav a{{margin-right:12px;color:#0a84ff;text-decoration:none}}\
         section{{margin:22px 0}}h2{{font-size:15px;color:#3a3a3c}}\
         .pair{{display:flex;gap:12px}}.pair figure{{flex:1;margin:0;background:#fff;border:1px solid #d0d0d5;border-radius:8px;overflow:hidden}}\
         figcaption{{font-size:12px;padding:6px 8px;background:#e5e5ea;color:#3a3a3c}}\
         iframe{{width:100%;height:520px;border:0;background:#fff}}</style></head>\
         <body><h1>LDOCE6 → ODC L1 对照（{} 条）</h1><p>词典：{}</p><nav>{nav}</nav>{body}</body></html>",
        picked.len(),
        html_escape(mdx_path)
    );
    fs::write(out_dir.join("viewer.html"), html)
}

/// Compute every acceptance number and write `metrics.json`.
fn write_metrics(
    out_dir: &Path,
    samples: &[Sample],
    stats: &SampleStats,
    mdx_path: &str,
) -> std::io::Result<Value> {
    let total = samples.len();
    let fidelity_pass = samples.iter().filter(|s| s.fidelity_ok).count();
    let idempotent = samples.iter().filter(|s| s.idempotent).count();
    let split_total: usize = samples.iter().map(|s| s.split_lines).sum();

    let mut expected_total = 0usize;
    let mut expected_hit = 0usize;
    let mut slot_hits: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut expected_by_slot: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    let mut slot_misses: Vec<Value> = Vec::new();
    let mut presence_total = 0usize;
    for sample in samples {
        for slot in &sample.expected {
            expected_total += 1;
            let entry = expected_by_slot.entry(slot).or_default();
            entry.0 += 1;
            if sample.slots.get(slot) > 0 {
                expected_hit += 1;
                entry.1 += 1;
            } else {
                slot_misses.push(json!({ "key": sample.key, "slot": slot }));
            }
        }
        for (slot, count) in &sample.slots.p0 {
            if *count > 0 {
                *slot_hits.entry(slot).or_default() += 1;
                presence_total += 1;
            }
        }
    }

    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for sample in samples {
        if let Some(reason) = &sample.unadapted {
            *reasons.entry(reason.clone()).or_default() += 1;
        }
    }

    let mut elapsed: Vec<(String, u128)> = samples
        .iter()
        .map(|s| (s.key.clone(), s.elapsed_us))
        .collect();
    elapsed.sort_by_key(|(_, us)| std::cmp::Reverse(*us));
    let total_us: u128 = elapsed.iter().map(|(_, us)| *us).sum();
    let max = elapsed.first().cloned().unwrap_or_default();
    let mean_us = if total == 0 {
        0.0
    } else {
        total_us as f64 / total as f64
    };

    let failures: Vec<Value> = samples
        .iter()
        .filter(|s| !s.fidelity_ok || s.split_lines > 0 || !s.idempotent || s.unadapted.is_some())
        .map(|s| {
            json!({
                "key": s.key,
                "fidelity": s.fidelity_ok,
                "split_lines": s.split_lines,
                "idempotent": s.idempotent,
                "unadapted": s.unadapted,
            })
        })
        .collect();

    let ratio = |hit: usize, of: usize| if of == 0 { 1.0 } else { hit as f64 / of as f64 };
    let metrics = json!({
        "dictionary": mdx_path,
        "generated_by": "crates/odc-core/examples/ldoce6_adapter.rs",
        "sample": {
            "size": total,
            "random": SAMPLE_SIZE.min(total),
            "polysemy": stats.polysemy_found,
            "eligible": stats.eligible,
            "skipped_empty": stats.skipped_empty,
            "skipped_large": stats.skipped_large,
            "skipped_redirect": stats.skipped_redirect,
            "read_errors": stats.read_errors,
        },
        "fidelity": {
            "passed": fidelity_pass,
            "total": total,
            "rate": ratio(fidelity_pass, total),
        },
        "p0": {
            "expected_slots": expected_total,
            "expected_hit": expected_hit,
            "coverage": ratio(expected_hit, expected_total),
            "presence_per_entry": ratio(presence_total, total * 9),
            "entries_with_slot": slot_hits,
            "expected_by_slot": expected_by_slot,
            "misses": slot_misses,
        },
        "split_lines": {
            "violations": split_total,
            "rate": ratio(split_total, total),
        },
        "idempotent": {
            "passed": idempotent,
            "total": total,
        },
        "performance": {
            "total_ms": total_us as f64 / 1000.0,
            "mean_ms": mean_us / 1000.0,
            "max_ms": max.1 as f64 / 1000.0,
            "max_key": max.0,
        },
        "unadapted": {
            "count": total - samples.iter().filter(|s| s.unadapted.is_none()).count(),
            "reasons": reasons,
        },
        "failures": failures,
    });
    fs::write(
        out_dir.join("metrics.json"),
        serde_json::to_string_pretty(&metrics)?,
    )?;
    Ok(metrics)
}

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap_or(0.0)
}

fn write_report(out_dir: &Path, metrics: &Value) -> std::io::Result<()> {
    let total = metrics["fidelity"]["total"].as_u64().unwrap_or(0);
    let fidelity_pass = metrics["fidelity"]["passed"].as_u64().unwrap_or(0);
    let fidelity = number(&metrics["fidelity"]["rate"]) * 100.0;
    let coverage = number(&metrics["p0"]["coverage"]) * 100.0;
    let presence = number(&metrics["p0"]["presence_per_entry"]) * 100.0;
    let split = metrics["split_lines"]["violations"].as_u64().unwrap_or(0);
    let idem = metrics["idempotent"]["passed"].as_u64().unwrap_or(0);
    let mut slot_rows = String::new();
    if let Some(map) = metrics["p0"]["entries_with_slot"].as_object() {
        for (slot, count) in map {
            let _ = writeln!(
                slot_rows,
                "| {slot} | {} / {total} |",
                count.as_u64().unwrap_or(0)
            );
        }
    }
    let reasons = metrics["unadapted"]["reasons"].clone();
    let reason_text = if reasons.as_object().is_some_and(Map::is_empty) {
        String::from("无")
    } else {
        serde_json::to_string(&reasons).unwrap_or_default()
    };
    let perf = &metrics["performance"];
    let report = format!(
        "# LDOCE6 → ODC L1 适配器原型：验收报告\n\n\
         > 自动生成，数字来自 `metrics.json`。规范：`docs/odc-html-spec.md` v0。\n\n\
         ## 结论\n\n\
         | 判据 | 目标 | 实测 |\n|---|---|---|\n\
         | 文本保真 | 100% | {fidelity:.2}% ({}/{total}) |\n\
         | P0 覆盖率（按源证据） | ≥ 95% | {coverage:.2}% |\n\
         | P0 槽位每词条出现率 | （参考） | {presence:.2}% |\n\
         | 拆行违规 | 0 | {split} |\n\
         | 幂等 | 100% | {idem}/{total} |\n\n\
         ## 性能\n\n\
         总耗时 {:.1} ms，单条均值 {:.2} ms，最慢 {:.1} ms（{}）。\n\n\
         ## 未适配\n\n\
         条目数 {}, 原因: {reason_text}\n\n\
         ## 每槽位命中条目数\n\n\
         | 槽位 | 条目数 |\n|---|---|\n{slot_rows}\n",
        fidelity_pass,
        number(&perf["total_ms"]),
        number(&perf["mean_ms"]),
        number(&perf["max_ms"]),
        perf["max_key"].as_str().unwrap_or(""),
        metrics["unadapted"]["count"].as_u64().unwrap_or(0),
    );
    fs::write(out_dir.join("report.md"), report)
}
