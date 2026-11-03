//! Import-level tests for HTML repair, sanitising and formatting.

use odc_core::html::{HtmlMode, HtmlOptions};
use odc_core::{import_mdx, MdxImportOptions, MdxImportReport, Repo};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
    mdx: PathBuf,
    repo: PathBuf,
}

impl Fixture {
    fn new(label: &str, entries: &[(&str, &str)]) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("odc-html-import-{label}-{nanos}"));
        std::fs::create_dir_all(&root).expect("temp dir");
        let mdx = root.join("fixture.mdx");
        let repo = root.join("repo");
        odc_testkit::write_v2_mdx(&mdx, entries).expect("write fixture");
        Self { root, mdx, repo }
    }

    fn import(&self, mode: HtmlMode) -> (MdxImportReport, Vec<(String, Option<String>)>) {
        let mut repo = Repo::init(&self.repo).expect("init repo");
        let report = import_mdx(
            &self.mdx,
            &mut repo,
            MdxImportOptions {
                import_mdd: false,
                html: HtmlOptions {
                    mode,
                    ..HtmlOptions::default()
                },
                ..MdxImportOptions::default()
            },
        )
        .expect("import");
        repo.commit("html import fixture", odc_core::DEFAULT_AUTHOR, 0)
            .expect("commit import");
        let mut bodies = Vec::new();
        repo.for_each_entry(|entry| {
            bodies.push((entry.word.clone(), entry.raw_html.clone()));
            Ok(())
        })
        .expect("walk entries");
        (report, bodies)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn default_import_formats_bodies() {
    let fixture = Fixture::new(
        "format",
        &[("apple", "<p>a<b>b</p>"), ("banana", "<p>c</p>")],
    );
    let (report, bodies) = fixture.import(HtmlMode::Format);
    assert_eq!(report.html.entries_checked, 2);
    assert_eq!(report.html.entries_changed, 1);
    assert_eq!(report.html.entries_failed, 0);
    assert!(report
        .html
        .diagnostics
        .get("html.parse_error")
        .is_some_and(|count| *count > 0));
    assert_eq!(
        report
            .html
            .samples
            .first()
            .map(|sample| sample.headword.as_str()),
        Some("apple")
    );
    let apple = bodies
        .iter()
        .find(|(word, _)| word == "apple")
        .and_then(|(_, html)| html.clone());
    assert_eq!(apple.as_deref(), Some("<p>a<b>b</b></p>"));
    let banana = bodies
        .iter()
        .find(|(word, _)| word == "banana")
        .and_then(|(_, html)| html.clone());
    assert_eq!(banana.as_deref(), Some("<p>c</p>"));
}

#[test]
fn check_mode_keeps_bodies_byte_identical() {
    let source = "<p onclick=\"x()\">t<script>alert(1)</script>  <b>u</p>";
    let fixture = Fixture::new("check", &[("apple", source)]);
    let (report, bodies) = fixture.import(HtmlMode::Check);
    assert_eq!(report.html.entries_checked, 1);
    assert_eq!(report.html.entries_changed, 0);
    assert_eq!(report.html.entries_failed, 0);
    assert!(report.html.diagnostics.contains_key("html.removed_element"));
    assert_eq!(bodies[0].1.as_deref(), Some(source));
}

#[test]
fn off_mode_skips_the_pipeline() {
    let source = "<p>a<b>b</p>";
    let fixture = Fixture::new("off", &[("apple", source)]);
    let (report, bodies) = fixture.import(HtmlMode::Off);
    assert_eq!(report.html.entries_checked, 0);
    assert!(report.html.diagnostics.is_empty());
    assert_eq!(bodies[0].1.as_deref(), Some(source));
}

#[test]
fn import_sanitises_dangerous_markup() {
    let fixture = Fixture::new(
        "sanitise",
        &[(
            "apple",
            "<p onclick=\"x()\">t<script>alert(1)</script><a href=\"javascript:void(0)\">l</a></p>",
        )],
    );
    let (report, bodies) = fixture.import(HtmlMode::Format);
    let html = bodies[0].1.clone().expect("raw html");
    assert!(!html.contains("script"), "{html}");
    assert!(!html.contains("onclick"), "{html}");
    assert!(!html.contains("javascript"), "{html}");
    assert_eq!(report.html.entries_changed, 1);
    assert!(report.html.diagnostics.contains_key("html.removed_element"));
    assert!(report
        .html
        .diagnostics
        .contains_key("html.removed_attribute"));
    assert!(report.html.diagnostics.contains_key("html.unsafe_url"));
}
