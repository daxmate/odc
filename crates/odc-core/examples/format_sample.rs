//! Measure the block-level HTML formatter on a real MDX dictionary.
//!
//! Usage:
//!
//! ```text
//! cargo run --release --example format_sample -- <dictionary.mdx> <out-dir> [sample-size]
//! ```
//!
//! The sample is drawn with the same deterministic reservoir sampler used by
//! the formatter-selection spike (seed `0x5eed_1234_abcd_0001`), so the
//! numbers stay comparable with the earlier measurement. Per-record rows and
//! a `samples.json` (headword → formatted body) are written to `<out-dir>`;
//! the final line is a JSON summary.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use html5ever::driver::{parse_fragment_for_element, ParseOpts};
use html5ever::tendril::TendrilSink;
use html5ever::{local_name, ns, QualName};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom};
use odc_core::html::{process_html, repair_sanitize, HtmlMode, HtmlOptions};
use odc_core::mdx::{MdxRecord, MdxSource, DEFAULT_MAX_RECORD_BYTES};
use serde_json::{json, Map, Value};

const DEFAULT_SAMPLE: usize = 200;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mdx_path = args
        .next()
        .ok_or("usage: format_sample <mdx> <out-dir> [sample-size]")?;
    let out_dir = PathBuf::from(
        args.next()
            .ok_or("usage: format_sample <mdx> <out-dir> [sample-size]")?,
    );
    let sample_size = match args.next() {
        Some(raw) => raw.parse::<usize>()?,
        None => DEFAULT_SAMPLE,
    };
    fs::create_dir_all(&out_dir)?;

    let source = MdxSource::open(&mdx_path)?;
    let (sample, scanned, eligible) = collect_records(&source, sample_size);
    println!(
        "scanned: {scanned}, eligible: {eligible}, sampled: {}",
        sample.len()
    );

    let mut totals = Totals::default();
    let mut samples = Map::new();
    for (index, record) in sample.iter().enumerate() {
        let started = Instant::now();
        let formatted = process_html(&record.html, &format_options()).html;
        let elapsed = started.elapsed();
        let again = process_html(&formatted, &format_options()).html;

        let repaired = repair_sanitize(&record.html).unwrap_or_else(|| record.html.clone());
        let original_text: Vec<String> = text_sequence(&repaired)
            .iter()
            .filter(|node| !node.trim().is_empty())
            .map(|node| node.trim().to_string())
            .collect();
        let formatted_text: Vec<String> = text_sequence(&formatted)
            .iter()
            .filter(|node| !node.trim().is_empty())
            .map(|node| node.trim().to_string())
            .collect();

        let split = split_lines(&formatted);
        let lines = formatted.lines().count();
        totals.records += 1;
        totals.lines += lines;
        totals.split_lines += split;
        totals.duration += elapsed;
        let raw_text: Vec<String> = text_sequence(&record.html)
            .iter()
            .filter(|node| !node.trim().is_empty())
            .map(|node| node.trim().to_string())
            .collect();
        totals.text_equal += usize::from(original_text == formatted_text);
        totals.text_equal_raw += usize::from(raw_text == formatted_text);
        totals.idempotent += usize::from(again == formatted);
        totals.changed += usize::from(formatted != record.html);

        if original_text != formatted_text {
            let mismatch = original_text
                .iter()
                .zip(&formatted_text)
                .position(|(before, after)| before != after)
                .unwrap_or(original_text.len().min(formatted_text.len()));
            println!(
                "TEXT MISMATCH {} at node {mismatch}: {:?} != {:?}",
                record.key.trim(),
                original_text.get(mismatch),
                formatted_text.get(mismatch),
            );
        }
        println!(
            "{}",
            json!({
                "index": index,
                "key": record.key.trim(),
                "bytes": record.html.len(),
                "lines": lines,
                "split_lines": split,
                "text_equal": original_text == formatted_text,
                "idempotent": again == formatted,
                "micros": elapsed.as_micros(),
            })
        );

        let slug: String = record
            .key
            .trim()
            .chars()
            .map(|character| {
                if character.is_alphanumeric() {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        fs::write(out_dir.join(format!("{index:03}-{slug}.html")), &formatted)?;
        fs::write(
            out_dir.join(format!("{index:03}-{slug}.original.html")),
            &record.html,
        )?;
        samples.insert(
            record.key.trim().to_string(),
            Value::String(formatted.clone()),
        );
    }
    fs::write(
        out_dir.join("samples.json"),
        serde_json::to_vec_pretty(&samples)?,
    )?;

    let count = totals.records.max(1) as f64;
    let summary = json!({
        "sample_size": totals.records,
        "scanned": scanned,
        "eligible": eligible,
        "lines": totals.lines,
        "split_lines": totals.split_lines,
        "split_rate": if totals.lines == 0 { 0.0 } else { totals.split_lines as f64 / totals.lines as f64 },
        "text_equal": totals.text_equal,
        "text_equal_rate": totals.text_equal as f64 / count,
        "text_equal_raw": totals.text_equal_raw,
        "text_equal_raw_rate": totals.text_equal_raw as f64 / count,
        "idempotent": totals.idempotent,
        "idempotent_rate": totals.idempotent as f64 / count,
        "changed": totals.changed,
        "millis_total": totals.duration.as_secs_f64() * 1000.0,
        "millis_per_record": totals.duration.as_secs_f64() * 1000.0 / count,
    });
    fs::write(
        out_dir.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    println!("{summary}");
    Ok(())
}

#[derive(Default)]
struct Totals {
    records: usize,
    lines: usize,
    split_lines: usize,
    text_equal: usize,
    text_equal_raw: usize,
    idempotent: usize,
    changed: usize,
    duration: Duration,
}

fn format_options() -> HtmlOptions {
    HtmlOptions {
        mode: HtmlMode::Format,
        ..HtmlOptions::default()
    }
}

fn split_lines(html: &str) -> usize {
    html.lines()
        .filter(|line| match (line.rfind('<'), line.rfind('>')) {
            (Some(open), Some(close)) => open > close,
            (Some(_), None) => true,
            _ => false,
        })
        .count()
}

fn text_sequence(html: &str) -> Vec<String> {
    let dom: RcDom = parse_fragment_for_element(
        RcDom::default(),
        ParseOpts::default(),
        fragment_context(),
        true,
        None,
    )
    .one(html.to_string());
    let mut nodes = Vec::new();
    collect_text(&dom.document, &mut nodes);
    nodes
}

fn collect_text(handle: &Handle, out: &mut Vec<String>) {
    if let NodeData::Text { ref contents } = handle.data {
        out.push(contents.borrow().to_string());
    }
    for child in handle.children.borrow().iter() {
        collect_text(child, out);
    }
}

fn fragment_context() -> Handle {
    Node::new(NodeData::Element {
        name: QualName::new(None, ns!(html), local_name!("div")),
        attrs: std::cell::RefCell::new(Vec::new()),
        template_contents: std::cell::RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    })
}

fn collect_records(source: &MdxSource, sample_size: usize) -> (Vec<MdxRecord>, usize, usize) {
    let mut sample: Vec<MdxRecord> = Vec::new();
    let mut scanned = 0usize;
    let mut eligible = 0u64;
    let mut rng = Lcg(0x5eed_1234_abcd_0001);
    for record in source.records() {
        let Ok(record) = record else {
            continue;
        };
        scanned += 1;
        if !eligible_record(&record) {
            continue;
        }
        eligible += 1;
        if sample.len() < sample_size {
            sample.push(record);
        } else {
            let pick = rng.below(eligible);
            if pick < sample_size as u64 {
                sample[pick as usize] = record;
            }
        }
    }
    (sample, scanned, eligible as usize)
}

fn eligible_record(record: &MdxRecord) -> bool {
    let body = record.html.as_str();
    !body.trim().is_empty() && body.contains('<') && body.len() <= DEFAULT_MAX_RECORD_BYTES
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound.max(1)
    }
}
