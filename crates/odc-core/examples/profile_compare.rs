//! Compare the LDOCE6 adapter with an odc.profile/v0 pack on the same input.
//!
//! Three modes:
//!
//! * --raw <jsonl|dir> runs the Rust adapter and the profile over the same
//!   fragments and reports the first structural difference per record;
//! * --golden <dir> compares the profile output with the frozen acceptance
//!   panes the adapter produced (entries/<key>-l1.html);
//! * --source <jsonl> compares the profile output with the **source fragment
//!   it was built from**, using the D1 contract in
//!   `odc_core::profile::contract`: text bag, order inside a semantic block,
//!   P1 skeleton invariants and source residue. This is the only ruler a
//!   dictionary without an adapter can be measured with.
//!
//!   The contract prints one line: `coverage <adapted>/<total> (<pct>%) |
//!   mapped <pass>/<adapted> four-dim | unadapted <n>`. An unadapted record
//!   (raw fallback, spec section 6) is a copy of its source and would pass the
//!   four checks by construction, so the rates only count adapted records.
//!   `--strict` adds a gate: exit 6 unless every record was adapted and every
//!   adapted record passed all four checks. Without the flag the exit code is
//!   0 — the ruler measures, it does not judge.
//!
//! The comparison is semantic, not byte-wise: both sides are parsed into a
//! minimal element tree, whitespace-only text nodes are dropped, text runs are
//! whitespace-collapsed, class lists are sorted and attributes are compared as
//! sorted name/value pairs. Formatting differences therefore never mask a
//! structural one.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;

use odc_core::adapters::ldoce6::Ldoce6;
use odc_core::adapters::{Adapter, ParseContext};
use odc_core::profile::contract::ContractSummary;
use odc_core::profile::{Outcome, Profile};
use serde::Deserialize;
use serde_json::{json, Value};

/// One JSONL sample.
#[derive(Deserialize)]
struct Record {
    #[serde(default)]
    key: String,
    text: String,
}

/// Element tree used for comparisons.
#[derive(Clone, Debug)]
enum Node {
    /// An element with attributes and children.
    El(Elem),
    /// A collapsed text run.
    Text(String),
}

/// One element.
#[derive(Clone, Debug)]
struct Elem {
    /// Lowercase tag name.
    tag: String,
    /// Attributes in source order.
    attrs: Vec<(String, String)>,
    /// Children.
    kids: Vec<Node>,
}

/// First difference between two trees.
#[derive(Clone, Debug)]
struct Diff {
    /// One of structure, attribute or text.
    kind: &'static str,
    /// Slash-separated path of the node that differs.
    path: String,
    /// Human-readable detail.
    detail: String,
}

impl Diff {
    /// Render as JSON.
    fn to_json(&self) -> Value {
        json!({ "kind": self.kind, "path": self.path, "detail": self.detail })
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("profile_compare: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<u8, Box<dyn std::error::Error>> {
    let args = Args::parse()?;
    let profile = Profile::from_json(&fs::read_to_string(&args.profile)?)?;
    let mut contract = None;
    let report = if let Some(golden) = &args.golden {
        compare_golden(&profile, golden, args.dump.as_deref(), args.limit)?
    } else if let Some(raw) = &args.raw {
        compare_raw(&profile, raw, args.limit)?
    } else if let Some(source) = &args.source {
        let (report, summary) = compare_source(&profile, source, args.dump.as_deref(), args.limit)?;
        contract = Some(summary);
        report
    } else {
        return Err("either --raw, --golden or --source is required".into());
    };
    if let Some(path) = &args.report {
        fs::write(path, serde_json::to_string_pretty(&report)? + "\n")?;
    }
    // The source contract is a measurement: it reports coverage and the four
    // rates over the adapted slice, and only `--strict` turns that into a
    // gate. Without the flag a partial pass is information, not an error.
    if let Some(summary) = contract {
        println!("{}", summary.render_line());
        if args.strict {
            if let Some(violation) = summary.strict_violation() {
                eprintln!("strict: {violation}");
                return Ok(6);
            }
        }
        return Ok(0);
    }
    let summary = &report["summary"];
    println!(
        "{}: {}/{} equal ({} text-equal)",
        report["mode"].as_str().unwrap_or("?"),
        summary["equal"].as_u64().unwrap_or(0),
        summary["total"].as_u64().unwrap_or(0),
        summary["text_equal"].as_u64().unwrap_or(0),
    );
    Ok(u8::from(summary["equal"] != summary["total"]))
}

/// Parsed command line.
struct Args {
    profile: PathBuf,
    raw: Option<PathBuf>,
    golden: Option<PathBuf>,
    source: Option<PathBuf>,
    report: Option<PathBuf>,
    dump: Option<PathBuf>,
    limit: Option<usize>,
    strict: bool,
}

impl Args {
    fn parse() -> Result<Self, Box<dyn std::error::Error>> {
        let argv: Vec<String> = std::env::args().skip(1).collect();
        let mut profile = None;
        let mut raw = None;
        let mut golden = None;
        let mut source = None;
        let mut report = None;
        let mut dump = None;
        let mut limit = None;
        let mut strict = false;
        let mut index = 0usize;
        while index < argv.len() {
            let flag = argv[index].clone();
            index += 1;
            match flag.as_str() {
                "--profile" => profile = Some(PathBuf::from(value_of(&argv, &mut index, &flag)?)),
                "--raw" => raw = Some(PathBuf::from(value_of(&argv, &mut index, &flag)?)),
                "--golden" => golden = Some(PathBuf::from(value_of(&argv, &mut index, &flag)?)),
                "--source" => source = Some(PathBuf::from(value_of(&argv, &mut index, &flag)?)),
                "--report" => report = Some(PathBuf::from(value_of(&argv, &mut index, &flag)?)),
                "--dump" => dump = Some(PathBuf::from(value_of(&argv, &mut index, &flag)?)),
                "--ignore-ui-text" => {
                    let _ = UI_TEXT.set(true);
                }
                "--ignore-attr" => {
                    let name = value_of(&argv, &mut index, &flag)?;
                    let mut list = IGNORED.get().cloned().unwrap_or_default();
                    list.push(name);
                    let _ = IGNORED.set(list);
                }
                "--limit" => limit = Some(value_of(&argv, &mut index, &flag)?.parse()?),
                // Only the source contract has a gate; the flag is inert in
                // the other modes, which stay pass/fail comparisons.
                "--strict" => strict = true,
                other => return Err(format!("unknown argument {other}").into()),
            }
        }
        Ok(Self {
            profile: profile.ok_or_else(|| String::from("--profile is required"))?,
            raw,
            golden,
            source,
            report,
            dump,
            limit,
            strict,
        })
    }
}

/// Read the value that follows a flag.
fn value_of(
    argv: &[String],
    index: &mut usize,
    flag: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let value = argv
        .get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))?;
    *index += 1;
    Ok(value)
}

/// Compare the profile against the frozen golden panes.
fn compare_golden(
    profile: &Profile,
    dir: &Path,
    dump: Option<&Path>,
    limit: Option<usize>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let entries = dir.join("entries");
    let mut files = panes(&entries)?;
    if let Some(max) = limit {
        files.truncate(max);
    }
    let mut lines: Vec<Value> = Vec::new();
    let mut equal = 0usize;
    let mut text_equal = 0usize;
    let mut bag_equal = 0usize;
    let mut payload_equal = 0usize;
    let mut dump_index: BTreeMap<String, String> = BTreeMap::new();
    // Body records are compared; alias records have no body to compare.
    let mut body = 0usize;
    let alias = lines
        .iter()
        .filter(|line| line.get("alias").is_some())
        .count();
    for file in files {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let key = name.trim_end_matches("-raw.html");
        let source = pane_body(&fs::read_to_string(&file)?);
        let golden = entries.join(format!("{key}-l1.html"));
        let expected = pane_body(&fs::read_to_string(&golden)?);
        // The pane declares the record key it was rendered for; file names
        // cannot carry spaces ("Action Man" is stored as "Action_Man").
        let key = declared_key(&expected).unwrap_or_else(|| String::from(key));
        let ctx = ParseContext::new(key.clone(), "ldoce6encn", "en");
        let (produced, stats) = match profile.apply(&source, &ctx) {
            Outcome::Adapted { html, stats } | Outcome::Unadapted { html, stats } => (html, stats),
            Outcome::Alias { target } => {
                lines.push(json!({ "key": key, "equal": false, "alias": target }));
                continue;
            }
        };
        if let Some(dir) = dump {
            fs::create_dir_all(dir)?;
            let file = format!("{}.html", dump_file_name(&key));
            fs::write(dir.join(&file), &produced)?;
            dump_index.insert(key.clone(), file);
        }
        let left = parse_html(&expected);
        let right = parse_html(&produced);
        let same = compare_trees(&left, &right);
        let text_same = text_of(&left) == text_of(&right);
        let bag_same = bag_of(&left) == bag_of(&right);
        let payload = payload_diff(&left, &right);
        body += 1;
        if same.is_none() && text_same {
            equal += 1;
        }
        if text_same {
            text_equal += 1;
        }
        if bag_same {
            bag_equal += 1;
        }
        if payload.is_none() {
            payload_equal += 1;
        }
        lines.push(json!({
            "key": key,
            "kind": "body",
            "payload_equal": payload.is_none(),
            "payload_first_diff": payload.as_ref().map(Diff::to_json),
            "equal": same.is_none(),
            "text_equal": text_same,
            "bag_equal": bag_same,
            "first_diff": same.as_ref().map(Diff::to_json),
            "first_text_diff": if text_same { Value::Null } else { first_text_diff(&left, &right).unwrap_or(Value::Null) },
            "stats": stats_json(&stats),
        }));
    }
    if let Some(dir) = dump {
        write_dump_index(dir, &dump_index)?;
    }
    let total = lines.len();
    Ok(json!({
        "mode": "golden",
        "summary": {
            "total": total,
            "body": body,
            "alias": alias,
            "equal": equal,
            "text_equal": text_equal,
            "bag_equal": bag_equal,
            "payload_equal": payload_equal,
        },
        "entries": lines,
    }))
}

/// Compare the adapter and the profile over a JSONL sample set or a directory
/// of -raw.html panes.
fn compare_raw(
    profile: &Profile,
    path: &Path,
    limit: Option<usize>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let records = read_records(path, limit)?;
    let adapter = Ldoce6;
    let mut lines = Vec::new();
    let mut equal = 0usize;
    let mut text_equal = 0usize;
    let mut bag_equal = 0usize;
    let mut payload_equal = 0usize;
    // Alias records carry no body: they are counted, never compared.
    let mut alias = 0usize;
    for record in &records {
        if odc_core::profile::alias_target(&record.text).is_some() {
            alias += 1;
            continue;
        }
        let ctx = ParseContext::new(record.key.clone(), adapter.dict(), "en");
        let expected = adapter.parse(&record.text, &ctx)?;
        let (produced, stats) = match profile.apply(&record.text, &ctx) {
            Outcome::Adapted { html, stats } | Outcome::Unadapted { html, stats } => (html, stats),
            Outcome::Alias { target } => {
                lines.push(json!({ "key": record.key, "equal": false, "alias": target }));
                continue;
            }
        };
        let left = parse_html(&expected);
        let right = parse_html(&produced);
        let same = compare_trees(&left, &right);
        let text_same = text_of(&left) == text_of(&right);
        let bag_same = bag_of(&left) == bag_of(&right);
        let payload = payload_diff(&left, &right);
        if same.is_none() && text_same {
            equal += 1;
        }
        if text_same {
            text_equal += 1;
        }
        if bag_same {
            bag_equal += 1;
        }
        if payload.is_none() {
            payload_equal += 1;
        }
        lines.push(json!({
            "key": record.key,
            "payload_equal": payload.is_none(),
            "payload_first_diff": payload.as_ref().map(Diff::to_json),
            "equal": same.is_none(),
            "text_equal": text_same,
            "bag_equal": bag_same,
            "first_diff": same.as_ref().map(Diff::to_json),
            "first_text_diff": if text_same { Value::Null } else { first_text_diff(&left, &right).unwrap_or(Value::Null) },
            "stats": stats_json(&stats),
        }));
    }
    let total = lines.len();
    Ok(json!({
        "mode": "adapter-vs-profile",
        "summary": {
            "total": total,
            "body": total,
            "alias": alias,
            "equal": equal,
            "text_equal": text_equal,
            "bag_equal": bag_equal,
            "payload_equal": payload_equal,
        },
        "entries": lines,
    }))
}

/// Read the sample set: a JSONL file, or a directory of -raw.html panes.
fn read_records(
    path: &Path,
    limit: Option<usize>,
) -> Result<Vec<Record>, Box<dyn std::error::Error>> {
    let mut records = Vec::new();
    if path.is_dir() {
        let mut files = panes(path)?;
        if let Some(max) = limit {
            files.truncate(max);
        }
        for file in files {
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            let key = name.trim_end_matches("-raw.html");
            records.push(Record {
                key: String::from(key),
                text: pane_body(&fs::read_to_string(&file)?),
            });
        }
        return Ok(records);
    }
    let text = fs::read_to_string(path)?;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        records.push(serde_json::from_str::<Record>(line)?);
        if limit.is_some_and(|max| records.len() >= max) {
            break;
        }
    }
    Ok(records)
}

/// Dictionary slug the profile was written for: the first segment of its id.
///
/// The source fragment says nothing about the dictionary, so the L1 identity
/// comes from the profile and never from the caller.
fn profile_dict(profile: &Profile) -> String {
    profile
        .profile_id
        .split('.')
        .next()
        .unwrap_or("unknown")
        .to_string()
}

/// Compare every L1 fragment with the source fragment it was built from (D1).
///
/// A record passes only when the four contract checks pass; the reasons are
/// counted by code so the next profile change has a target instead of a vibe.
/// File name for one dumped fragment.
///
/// A headword is not a file name: `Action Man`, `a/b` and `1:2` are all legal
/// record keys and all illegal as a path segment. Percent-encode every byte
/// outside the safe set so the name stays reversible; `index.json` is the way
/// back to the original key.
fn dump_file_name(key: &str) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(key.len() + 8);
    for byte in key.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// Write the key → file name map that makes a dump directory auditable.
fn write_dump_index(
    dir: &Path,
    index: &BTreeMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let body = serde_json::to_string_pretty(index)? + "\n";
    fs::write(dir.join("index.json"), body)?;
    Ok(())
}

fn compare_source(
    profile: &Profile,
    path: &Path,
    dump: Option<&Path>,
    limit: Option<usize>,
) -> Result<(Value, ContractSummary), Box<dyn std::error::Error>> {
    let records = read_records(path, limit)?;
    let dict = profile_dict(profile);
    let mut failures: Vec<Value> = Vec::new();
    let mut codes: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut alias = 0usize;
    let mut summary = ContractSummary::default();
    let mut dump_index: BTreeMap<String, String> = BTreeMap::new();
    if let Some(dir) = dump {
        fs::create_dir_all(dir)?;
    }
    for record in &records {
        let ctx = ParseContext::new(record.key.clone(), dict.clone(), "en");
        let produced = match profile.apply(&record.text, &ctx) {
            Outcome::Adapted { html, .. } | Outcome::Unadapted { html, .. } => html,
            Outcome::Alias { .. } => {
                // An alias record has no body: it is a pointer, not a fragment.
                alias += 1;
                continue;
            }
        };
        if let Some(dir) = dump {
            let file = format!("{}.html", dump_file_name(&record.key));
            fs::write(dir.join(&file), &produced)?;
            dump_index.insert(record.key.clone(), file);
        }
        let verdict = odc_core::profile::contract::check(&record.text, &produced);
        summary.record(&verdict);
        if verdict.adapted_ok() || verdict.unadapted {
            // A pass, or a record the profile never adapted: an unadapted
            // record is a copy of its source and is reported through the
            // coverage line, not as a failure of the four checks.
            continue;
        }
        for reason in &verdict.reasons {
            *codes.entry(reason.code).or_insert(0) += 1;
        }
        failures.push(json!({
            "key": record.key,
            "bag_ok": verdict.bag_ok,
            "order_ok": verdict.order_ok,
            "structure_ok": verdict.structure_ok,
            "clean_ok": verdict.clean_ok,
            "clean_exempt_raw": verdict.clean_exempt_raw,
            "reasons": verdict
                .reasons
                .iter()
                .map(|reason| json!({ "code": reason.code, "detail": reason.detail }))
                .collect::<Vec<Value>>(),
            "first_diff": verdict.first_diff(),
        }));
    }
    if let Some(dir) = dump {
        write_dump_index(dir, &dump_index)?;
    }
    let mut reasons: Vec<Value> = codes
        .iter()
        .map(|(code, count)| json!({ "code": code, "count": count }))
        .collect();
    reasons.sort_by(|a, b| {
        b["count"]
            .as_u64()
            .cmp(&a["count"].as_u64())
            .then_with(|| a["code"].as_str().cmp(&b["code"].as_str()))
    });
    Ok((
        json!({
            "mode": "source-contract",
            "summary": {
                "total": records.len(),
                "body": summary.total,
                "alias": alias,
                "adapted": summary.adapted,
                "unadapted": summary.unadapted,
                "equal": summary.adapted_all_pass,
                "bag_ok": summary.bag_pass,
                "order_ok": summary.order_pass,
                "structure_ok": summary.structure_pass,
                "clean_ok": summary.clean_pass,
                "clean_exempt_raw": summary.unadapted,
            },
            "reason_counts": reasons,
            "failures": failures,
        }),
        summary,
    ))
}

/// Compact per-record rule statistics.
fn stats_json(stats: &odc_core::profile::RunStats) -> Value {
    json!({
        "hits": stats.hits,
        "failures": stats.failures,
        "p0_missing": stats.p0_missing,
        "warnings": stats.warnings,
        "errors": stats.errors,
    })
}

/// Sorted -raw.html panes in a directory.
fn panes(dir: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|item| item.ok())
        .map(|item| item.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("-raw.html"))
        })
        .collect();
    files.sort();
    Ok(files)
}

/// Inner HTML of the pane body in a golden page.
fn pane_body(html: &str) -> String {
    let start = html
        .find("<body")
        .and_then(|at| html[at..].find('>').map(|end| at + end + 1));
    let Some(start) = start else {
        return String::from(html);
    };
    let end = html.rfind("</body>").unwrap_or(html.len());
    String::from(&html[start..end.max(start)])
}

/// Parse an HTML fragment into a node list.
fn parse_html(src: &str) -> Vec<Node> {
    let mut stack: Vec<Elem> = Vec::new();
    let mut roots: Vec<Node> = Vec::new();
    let mut i = 0usize;
    while i < src.len() {
        if src.as_bytes()[i] != b'<' {
            let end = src[i..].find('<').map_or(src.len(), |offset| i + offset);
            let text = collapse(&src[i..end]);
            if !text.is_empty() {
                push(&mut stack, &mut roots, Node::Text(text));
            }
            i = end;
            continue;
        }
        let rest = &src[i..];
        if let Some(body) = rest.strip_prefix("<!--") {
            i += body.find("-->").map_or(rest.len(), |offset| offset + 7);
            continue;
        }
        if rest.starts_with("<!") {
            i += rest.find('>').map_or(rest.len(), |offset| offset + 1);
            continue;
        }
        if let Some(body) = rest.strip_prefix("</") {
            let Some(offset) = body.find('>') else { break };
            let name = body[..offset].trim().to_ascii_lowercase();
            i += offset + 3;
            if let Some(pos) = stack.iter().rposition(|elem| elem.tag == name) {
                while stack.len() > pos {
                    if let Some(elem) = stack.pop() {
                        push(&mut stack, &mut roots, Node::El(elem));
                    }
                }
            }
            continue;
        }
        let Some(offset) = rest.find('>') else { break };
        let inner = rest[1..offset].trim_end();
        i += offset + 1;
        let self_closing = inner.ends_with('/');
        let inner = inner.trim_end_matches('/').trim_end();
        let (tag, attrs) = parse_tag(inner);
        let elem = Elem {
            tag,
            attrs,
            kids: Vec::new(),
        };
        if self_closing || is_void(&elem.tag) {
            push(&mut stack, &mut roots, Node::El(elem));
        } else {
            stack.push(elem);
        }
    }
    while let Some(elem) = stack.pop() {
        push(&mut stack, &mut roots, Node::El(elem));
    }
    roots
}

fn push(stack: &mut [Elem], roots: &mut Vec<Node>, node: Node) {
    let slot = match stack.last_mut() {
        Some(parent) => &mut parent.kids,
        None => roots,
    };
    slot.push(node);
}

fn is_void(tag: &str) -> bool {
    matches!(
        tag,
        "img" | "br" | "hr" | "meta" | "link" | "input" | "source" | "wbr"
    )
}

fn parse_tag(inner: &str) -> (String, Vec<(String, String)>) {
    let name_end = inner.find(char::is_whitespace).unwrap_or(inner.len());
    let tag = inner[..name_end].to_ascii_lowercase();
    let mut rest = &inner[name_end..];
    let mut attrs = Vec::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let name_end = rest
            .find(|c: char| c == '=' || c.is_whitespace())
            .unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        let tail = rest[name_end..].trim_start();
        if let Some(after) = tail.strip_prefix('=') {
            let after = after.trim_start();
            let mut chars = after.chars();
            let (value, left) = match chars.next() {
                Some(quote @ ('"' | '\'')) => {
                    let body = &after[1..];
                    match body.find(quote) {
                        Some(end) => (&body[..end], &body[end + 1..]),
                        None => (body, ""),
                    }
                }
                _ => match after.find(char::is_whitespace) {
                    Some(end) => (&after[..end], &after[end..]),
                    None => (after, ""),
                },
            };
            attrs.push((name, decode(value)));
            rest = left;
        } else {
            attrs.push((name, String::new()));
            rest = tail;
        }
    }
    (tag, attrs)
}

fn decode(text: &str) -> String {
    if !text.contains('&') {
        return String::from(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let entity = match tail.find(';').filter(|end| *end <= 10) {
            Some(end) => (&tail[1..end], end),
            None => {
                out.push_str(tail);
                return out;
            }
        };
        let (name, end) = entity;
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => name
                .strip_prefix('#')
                .and_then(|digits| match digits.strip_prefix('x') {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => digits.parse::<u32>().ok(),
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(ch) => out.push(ch),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Canonical attribute view: sorted pairs, class list sorted and deduped.
fn canon_attrs(elem: &Elem) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (name, value) in &elem.attrs {
        if name == "data-odc-rule" {
            // Executor provenance, never part of the slot contract.
            continue;
        }
        if ignored(name) {
            continue;
        }
        if name == "class" {
            let mut classes: Vec<&str> = value.split_whitespace().collect();
            classes.sort_unstable();
            classes.dedup();
            out.insert(String::from("class"), classes.join(" "));
        } else {
            out.insert(name.clone(), value.clone());
        }
    }
    out
}

/// First text run that differs, with a little context.
fn first_text_diff(left: &[Node], right: &[Node]) -> Option<Value> {
    let mut expected = Vec::new();
    let mut actual = Vec::new();
    collect_text(left, &mut expected);
    collect_text(right, &mut actual);
    for index in 0..expected.len().max(actual.len()) {
        let a = expected.get(index).map_or("", String::as_str);
        let b = actual.get(index).map_or("", String::as_str);
        if a != b {
            return Some(json!({
                "index": index,
                "expected": a,
                "actual": b,
                "next_expected": expected.get(index + 1),
                "next_actual": actual.get(index + 1),
            }));
        }
    }
    None
}

/// Compare two trees; None means equal.
fn compare_trees(left: &[Node], right: &[Node]) -> Option<Diff> {
    let mut diff = None;
    diff_nodes(left, right, "", &mut diff);
    diff
}

fn diff_nodes(left: &[Node], right: &[Node], path: &str, out: &mut Option<Diff>) {
    if out.is_some() {
        return;
    }
    let left = strip(left);
    let right = strip(right);
    let shared = left.len().min(right.len());
    for index in 0..shared {
        match (&left[index], &right[index]) {
            (Node::Text(a), Node::Text(b)) => {
                if a != b {
                    *out = Some(Diff {
                        kind: "text",
                        path: String::from(path),
                        detail: format!("expected {a:?}, got {b:?}"),
                    });
                    return;
                }
            }
            (Node::El(a), Node::El(b)) => {
                if a.tag != b.tag {
                    *out = Some(Diff {
                        kind: "structure",
                        path: String::from(path),
                        detail: format!("expected <{}>, got <{}>", a.tag, b.tag),
                    });
                    return;
                }
                let here = format!("{path}/{}", a.tag);
                let attrs_a = canon_attrs(a);
                let attrs_b = canon_attrs(b);
                if attrs_a != attrs_b {
                    let names: Vec<String> = attrs_a
                        .keys()
                        .chain(attrs_b.keys())
                        .filter(|name| attrs_a.get(*name) != attrs_b.get(*name))
                        .cloned()
                        .collect();
                    *out = Some(Diff {
                        kind: "attribute",
                        path: here,
                        detail: format!("attributes {names:?} differ"),
                    });
                    return;
                }
                diff_nodes(&a.kids, &b.kids, &here, out);
                if out.is_some() {
                    return;
                }
            }
            (a, b) => {
                *out = Some(Diff {
                    kind: "structure",
                    path: String::from(path),
                    detail: format!("expected {}, got {}", describe(a), describe(b)),
                });
                return;
            }
        }
    }
    if left.len() != right.len() {
        let detail = if left.len() > right.len() {
            format!("expected {} more node(s)", left.len() - right.len())
        } else {
            format!("profile emitted {} extra node(s)", right.len() - left.len())
        };
        *out = Some(Diff {
            kind: "structure",
            path: String::from(path),
            detail,
        });
    }
}

fn describe(node: &Node) -> String {
    match node {
        Node::El(elem) => format!("<{}>", elem.tag),
        Node::Text(text) => format!("text {text:?}"),
    }
}

fn strip(nodes: &[Node]) -> Vec<Node> {
    nodes
        .iter()
        .filter(|node| match node {
            Node::Text(text) => !text.is_empty(),
            Node::El(_) => true,
        })
        .map(|node| match node {
            Node::El(elem) if ui_text_ignored() && is_ui(elem) => {
                let mut elem = elem.clone();
                elem.kids.retain(|kid| !matches!(kid, Node::Text(_)));
                elem.attrs.retain(|(name, _)| name != "data-odc-ui");
                Node::El(elem)
            }
            other => other.clone(),
        })
        .collect()
}

/// Whether `--ignore-ui-text` was given.
fn ui_text_ignored() -> bool {
    UI_TEXT.get().copied().unwrap_or(false)
}

/// Whether the element carries a `data-odc-ui` marker (P4).
///
/// A reference-bar caption is chrome whichever form it takes: the frozen
/// panes still spell the caption out, L1 keeps it in the marker.
fn is_ui(elem: &Elem) -> bool {
    elem.attrs.iter().any(|(name, _)| name == "data-odc-ui")
        || elem.attrs.iter().any(|(name, value)| {
            name == "class"
                && value
                    .split_whitespace()
                    .any(|part| part == "odc-refbar__label")
        })
}

/// Key declared by a rendered pane, if any.
fn declared_key(html: &str) -> Option<String> {
    let at = html.find("data-odc-key=\"")? + "data-odc-key=\"".len();
    let rest = &html[at..];
    let end = rest.find('"')?;
    Some(String::from(&rest[..end]))
}

/// Compare only the odc-entry payload, so a wrapper the profile cannot remove
/// does not hide the rest of the tree.
fn payload_diff(left: &[Node], right: &[Node]) -> Option<Diff> {
    let expected = find_class(left, "odc-entry")?;
    let actual = find_class(right, "odc-entry")?;
    let mut diff = None;
    diff_nodes(&expected.kids, &actual.kids, "section.odc-entry", &mut diff);
    diff
}

/// Whether UI captions (`data-odc-ui`) are excluded from text comparison.
static UI_TEXT: OnceLock<bool> = OnceLock::new();

/// Attributes excluded from the comparison, set from --ignore-attr.
static IGNORED: OnceLock<Vec<String>> = OnceLock::new();

fn ignored(name: &str) -> bool {
    IGNORED
        .get()
        .is_some_and(|list| list.iter().any(|item| item == name))
}

/// First element carrying a class, searched depth-first.
fn find_class<'a>(nodes: &'a [Node], class: &str) -> Option<&'a Elem> {
    for node in nodes {
        if let Node::El(elem) = node {
            let hit = elem.attrs.iter().any(|(name, value)| {
                name == "class" && value.split_whitespace().any(|part| part == class)
            });
            if hit {
                return Some(elem);
            }
            if let Some(found) = find_class(&elem.kids, class) {
                return Some(found);
            }
        }
    }
    None
}

/// Word multiset of a tree, for order-insensitive comparison.
fn bag_of(nodes: &[Node]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for word in text_of(nodes).split_whitespace() {
        *out.entry(String::from(word)).or_insert(0) += 1;
    }
    out
}

/// Whitespace-collapsed text of a tree, in document order.
fn text_of(nodes: &[Node]) -> String {
    let mut parts = Vec::new();
    collect_text(nodes, &mut parts);
    parts.join(" ")
}

fn collect_text(nodes: &[Node], parts: &mut Vec<String>) {
    for node in nodes {
        match node {
            Node::Text(text) => parts.push(text.clone()),
            Node::El(elem) => {
                if ui_text_ignored() && is_ui(elem) {
                    continue;
                }
                collect_text(&elem.kids, parts);
            }
        }
    }
}
