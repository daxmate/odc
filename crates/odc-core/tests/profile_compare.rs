//! End-to-end coverage for `examples/profile_compare --source`.
//!
//! The library tests pin the ruler rules; these pin the command line: one
//! summary line, the strict exit code, and dump file naming. `cargo test --all`
//! builds examples to make sure they compile, so the binary is next to the
//! test binaries; a run that selects only this test target does not build the
//! example, and the tests then report a skip instead of failing on a missing
//! file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// One root rule: `div.rec` becomes the entry skeleton.
///
/// `min: 1` makes the rule a P0 requirement, so a fragment without a `div.rec`
/// comes back as a raw fallback (`Outcome::Unadapted`) instead of a mangled
/// adaptation.
const PROFILE: &str = concat!(
    r#"{"schema_version":"odc.profile/v0","profile_id":"fixture.en","#,
    r#""dict":{"name":"fixture","lang":"en"},"engine":{"name":"human"},"#,
    r#""rules":[{"id":"root","stage":"root","op":"map","#,
    r#""match":{"selector":"div.rec","cardinality":{"min":1}},"#,
    r#""emit":{"element":"section","classes":["odc-entry"],"move":"children"}}]}"#
);

fn temp_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "odc-profile-compare-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// The example binary, when the caller built examples.
fn example_binary() -> Option<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent()?.parent()?;
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => root.join("target"),
    };
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let path = target
        .join(profile)
        .join("examples")
        .join("profile_compare");
    path.is_file().then_some(path)
}

fn write_profile(dir: &Path) -> PathBuf {
    let path = dir.join("fixture.profile.json");
    fs::write(&path, PROFILE).expect("profile");
    path
}

fn write_jsonl(dir: &Path, name: &str, lines: &[&str]) -> PathBuf {
    let path = dir.join(name);
    let mut body = lines.join("\n");
    body.push('\n');
    fs::write(&path, body).expect("jsonl");
    path
}

fn run(dir: &Path, source: &Path, extra: &[&str]) -> Output {
    let binary = example_binary().expect("example binary");
    let profile = write_profile(dir);
    let mut command = Command::new(binary);
    command
        .arg("--profile")
        .arg(profile)
        .arg("--source")
        .arg(source);
    command.args(extra);
    command.output().expect("run profile_compare")
}

fn skip_without_example() -> bool {
    if example_binary().is_some() {
        return false;
    }
    eprintln!("skipped: examples/profile_compare was not built for this run");
    true
}

const MAPPED: &str = r#"{"key":"able","text":"<div class=\"rec\"><p>alpha</p></div>"}"#;
const RAW: &str = r#"{"key":"baker","text":"<p>beta</p>"}"#;

#[test]
fn the_summary_line_reports_coverage_and_keeps_unadapted_out_of_the_rates() {
    if skip_without_example() {
        return;
    }
    let dir = temp_dir();
    let source = write_jsonl(&dir, "mixed.jsonl", &[MAPPED, RAW]);
    let output = run(&dir, &source, &[]);
    assert!(
        output.status.success(),
        "default mode measures, it does not gate: {} {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim_end(),
        "coverage 1/2 (50.000%) | mapped 1/1 four-dim | unadapted 1"
    );
}

#[test]
fn strict_fails_on_an_unadapted_record_and_passes_on_a_clean_run() {
    if skip_without_example() {
        return;
    }
    let dir = temp_dir();
    let mixed = write_jsonl(&dir, "mixed.jsonl", &[MAPPED, RAW]);
    let strict = run(&dir, &mixed, &["--strict"]);
    assert_eq!(
        strict.status.code(),
        Some(6),
        "unadapted records must fail the gate: {}",
        String::from_utf8_lossy(&strict.stderr)
    );

    let clean = write_jsonl(&dir, "clean.jsonl", &[MAPPED]);
    let passed = run(&dir, &clean, &["--strict"]);
    assert!(
        passed.status.success(),
        "a fully adapted run must pass: {} {}",
        passed.status,
        String::from_utf8_lossy(&passed.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&passed.stdout).trim_end(),
        "coverage 1/1 (100.000%) | mapped 1/1 four-dim | unadapted 0"
    );
}

#[test]
fn dump_escapes_headwords_and_keeps_an_index() {
    if skip_without_example() {
        return;
    }
    let dir = temp_dir();
    let awkward = r#"{"key":"a/b c","text":"<div class=\"rec\"><p>alpha</p></div>"}"#;
    let source = write_jsonl(&dir, "awkward.jsonl", &[awkward]);
    let dump = dir.join("dump");
    let output = run(
        &dir,
        &source,
        &["--dump", dump.to_str().expect("utf-8 path")],
    );
    assert!(
        output.status.success(),
        "--dump must not fail on a headword with a slash: {} {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let index: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dump.join("index.json")).expect("index.json"))
            .expect("index is JSON");
    let file = index["a/b c"].as_str().expect("key is mapped");
    assert_eq!(file, "a%2Fb%20c.html");
    assert!(dump.join(file).is_file(), "dumped file");
}
