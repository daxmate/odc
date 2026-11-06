//! Run an ODC profile over a JSONL sample set.
//!
//! Usage:
//!
//!   profile_run --profile profiles/mwaled.profile.json \
//!     --input /tmp/odc-dialect-survey/raw/mwaled.jsonl \
//!     --report /tmp/report.json
//!
//! Every record prints one JSON line; the summary is written to '--report'
//! when given. The runner performs no inference: it applies the profile and
//! measures the result.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use odc_core::adapters::{ParseContext, P0_SLOTS};
use odc_core::profile::verify::{self, FingerprintBuilder};
use odc_core::profile::{Outcome, Profile};
use serde::Deserialize;
use serde_json::{json, Map, Value};

/// One JSONL sample.
#[derive(Deserialize)]
struct Record {
    #[serde(default)]
    kind: String,
    key: String,
    text: String,
}

/// Parsed command line.
struct Args {
    profile: PathBuf,
    input: PathBuf,
    out: Option<PathBuf>,
    report: Option<PathBuf>,
    limit: Option<usize>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("profile_run: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let profile = Profile::from_json(&fs::read_to_string(&args.profile)?)?;
    let records = read_records(&args.input, args.limit)?;

    let mut fingerprint = FingerprintBuilder::default();
    for record in &records {
        if odc_core::profile::alias_target(&record.text).is_none() {
            fingerprint.add(&record.text);
        }
    }
    let digest = fingerprint.digest();
    let fingerprint_match = profile.fingerprint_matches(&digest);
    if fingerprint_match == Some(false) {
        eprintln!(
            "profile_run: fingerprint mismatch (profile {}, corpus {digest})",
            profile
                .fingerprint
                .as_ref()
                .map_or("", |f| f.sha256.as_str())
        );
    }

    let mut samples = Vec::new();
    let mut hits: BTreeMap<String, u64> = BTreeMap::new();
    let mut totals = Totals::default();
    let ctx_dict = "mwaled";

    for record in &records {
        let ctx = ParseContext::new(record.key.clone(), ctx_dict, "en");
        let outcome = profile.apply(&record.text, &ctx);
        let clean =
            odc_core::html::repair_sanitize(&record.text).unwrap_or_else(|| record.text.clone());

        let mut line = Map::new();
        line.insert(String::from("key"), json!(record.key));
        line.insert(String::from("source_kind"), json!(record.kind));
        match &outcome {
            Outcome::Alias { target } => {
                totals.aliases += 1;
                line.insert(String::from("kind"), json!("alias"));
                line.insert(String::from("target"), json!(target));
            }
            Outcome::Adapted { html, stats } | Outcome::Unadapted { html, stats } => {
                let adapted = matches!(outcome, Outcome::Adapted { .. });
                if adapted {
                    totals.adapted += 1;
                } else {
                    totals.unadapted += 1;
                    line.insert(String::from("reason"), json!(stats.p0_missing));
                }
                let fidelity = verify::text_fidelity(&clean, html);
                let words = verify::word_fidelity(&clean, html);
                let idempotent = verify::idempotent(&profile, html, &ctx);
                let counts = verify::slot_counts(html);
                // Expected coverage comes from the profile itself: a P0 rule
                // that matched says the source carries that structure. The
                // adapters' source_slots() heuristic is LDOCE-specific and
                // cannot see mwaled classes.
                let mut expected: Vec<&str> = Vec::new();
                for rule in &profile.rules {
                    let fired = stats.hits.get(&rule.id).copied().unwrap_or(0) > 0;
                    if !fired {
                        continue;
                    }
                    for class in &rule.emit.classes {
                        if P0_SLOTS.contains(&class.as_str()) && !expected.contains(&class.as_str())
                        {
                            expected.push(class.as_str());
                        }
                    }
                }
                let covered: Vec<&str> = expected
                    .iter()
                    .copied()
                    .filter(|slot| counts.get(slot) > 0)
                    .collect();
                if fidelity {
                    totals.fidelity += 1;
                } else {
                    totals.fidelity_failed.push(record.key.clone());
                }
                if words {
                    totals.words += 1;
                }
                if idempotent {
                    totals.idempotent += 1;
                }
                for slot in &expected {
                    let entry = totals.expected.entry(String::from(*slot)).or_insert(0);
                    *entry += 1;
                    if counts.get(slot) > 0 {
                        *totals.covered.entry(String::from(*slot)).or_insert(0) += 1;
                    }
                }
                for (rule, count) in &stats.hits {
                    *hits.entry(rule.clone()).or_insert(0) += *count as u64;
                }
                if let Some(out) = &args.out {
                    fs::create_dir_all(out)?;
                    fs::write(out.join(format!("{}.html", slug(&record.key))), html)?;
                }
                line.insert(
                    String::from("kind"),
                    json!(if adapted { "adapted" } else { "unadapted" }),
                );
                line.insert(String::from("fidelity"), json!(fidelity));
                line.insert(String::from("word_fidelity"), json!(words));
                line.insert(String::from("idempotent"), json!(idempotent));
                line.insert(String::from("p0_present"), json!(counts.p0_present()));
                line.insert(String::from("unadapted"), json!(!adapted));
                line.insert(String::from("covered"), json!(covered));
                line.insert(String::from("rules"), json!(stats.hits.len()));
            }
        }
        samples.push(Value::Object(line));
    }

    let denominator = totals.adapted + totals.unadapted;
    let summary = json!({
        "profile": profile.profile_id,
        "dict": profile.dict.name,
        "records": records.len(),
        "aliases": totals.aliases,
        "adapted": totals.adapted,
        "unadapted": totals.unadapted,
        "fidelity_pass": totals.fidelity,
        "fidelity_rate": rate(totals.fidelity, denominator),
        "fidelity_failed": totals.fidelity_failed,
        "word_fidelity_pass": totals.words,
        "idempotent_pass": totals.idempotent,
        "idempotent_rate": rate(totals.idempotent, denominator),
        "fingerprint": {
            "algorithm": verify::FINGERPRINT_ALGORITHM,
            "corpus": digest,
            "profile": profile.fingerprint.as_ref().map(|f| f.sha256.clone()),
            "match": fingerprint_match,
            "samples": fingerprint.samples(),
        },
        "rules": profile.rules.len(),
        "rule_hits": hits,
        "p0_coverage": p0_coverage(&totals),
        "samples": samples,
    });

    if let Some(path) = &args.report {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(&summary)?)?;
    }
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

/// Aggregate counters.
#[derive(Default)]
struct Totals {
    aliases: u64,
    adapted: u64,
    unadapted: u64,
    fidelity: u64,
    words: u64,
    idempotent: u64,
    fidelity_failed: Vec<String>,
    expected: BTreeMap<String, u64>,
    covered: BTreeMap<String, u64>,
}

/// P0 slot coverage, expected versus produced.
fn p0_coverage(totals: &Totals) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for slot in P0_SLOTS {
        let expected = totals.expected.get(slot).copied().unwrap_or(0);
        let covered = totals.covered.get(slot).copied().unwrap_or(0);
        out.insert(
            String::from(slot),
            json!({ "expected": expected, "covered": covered }),
        );
    }
    out
}

fn rate(part: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    part as f64 / total as f64
}

fn slug(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut profile = None;
    let mut input = None;
    let mut out = None;
    let mut report = None;
    let mut limit = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || -> Result<String, Box<dyn std::error::Error>> {
            args.next()
                .ok_or_else(|| format!("missing value for {flag}").into())
        };
        match flag.as_str() {
            "--profile" => profile = Some(PathBuf::from(value()?)),
            "--input" => input = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--report" => report = Some(PathBuf::from(value()?)),
            "--limit" => limit = Some(value()?.parse()?),
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    Ok(Args {
        profile: profile.ok_or("--profile is required")?,
        input: input.ok_or("--input is required")?,
        out,
        report,
        limit,
    })
}

fn read_records(
    path: &Path,
    limit: Option<usize>,
) -> Result<Vec<Record>, Box<dyn std::error::Error>> {
    let text = fs::read_to_string(path)?;
    let mut records = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        records.push(serde_json::from_str::<Record>(line)?);
        if limit.is_some_and(|max| records.len() >= max) {
            break;
        }
    }
    Ok(records)
}
