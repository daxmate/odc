//! Presentation layer tests.
//!
//! The fixtures are synthetic: they reproduce the source dialect of the demo
//! dictionary without quoting any dictionary text. Every assertion is about the
//! engine output, not about the rule pack.

use odc_core::adapters::ParseContext;
use odc_core::profile::{Outcome, Profile};

fn profile() -> Profile {
    let text = include_str!("../../../profiles/ldoce6.profile.json");
    Profile::from_json(text).unwrap_or_else(|error| panic!("profile parses: {error}"))
}

fn ctx(key: &str) -> ParseContext {
    ParseContext::new(key, "ldoce6encn", "en")
}

/// Render a fragment and return the L1 markup plus the self-check.
fn render(fragment: &str, key: &str) -> (String, odc_core::presentation::CheckReport) {
    match odc_core::profile::exec::apply(&profile(), fragment, &ctx(key)) {
        Outcome::Adapted { html, stats } => (html, stats.presentation),
        other => panic!("expected an adapted fragment, got {other:?}"),
    }
}

const HEAD: &str = r#"<span class="entryhead"><span class="hwd">make</span><span class="proncodes"><span class="neutral">/</span><span class="pron">meɪk</span><span class="neutral">/</span></span> <span class="pos">verb</span> <a href="sound://hwd/bre/f/make.mp3"><img src="img/spkr_r.png"></a></span>"#;

#[test]
fn p1_skeleton_and_p6_ids() {
    let fragment = format!(
        r#"<div><span class="entry">{HEAD}<a name="anchor_s1" id="anchor_s1"></a><span class="sense"><span class="sensenum">1</span><span class="def">produce</span></span><span class="sense"><span class="sensenum">2</span><span class="def">earn</span></span></span></div>"#
    );
    let (html, report) = render(&fragment, "make");
    assert!(html.contains(r#"<section class="odc-entry""#), "{html}");
    assert!(
        !html.contains("<div><section"),
        "the wrapper must be gone: {html}"
    );
    assert!(html.contains(r#"<header class="odc-head""#), "{html}");
    assert!(html.contains(r#"id="make-verb""#), "{html}");
    assert!(html.contains(r#"id="make-verb-s1""#), "{html}");
    assert!(html.contains(r#"id="make-verb-s2""#), "{html}");
    assert!(html.contains(r#"data-odc-n="2""#), "{html}");
    assert!(report.ok(), "{report:?}");
}

#[test]
fn p5_source_classes_become_orig_classes() {
    let fragment = format!(
        r#"<div><span class="entry">{HEAD}<span class="sense"><span class="sensenum">1</span><span class="def">a <span class="nondv">toy</span></span></span></span></div>"#
    );
    let (html, _) = render(&fragment, "make");
    assert!(html.contains(r#"class="odc-def""#), "{html}");
    assert!(html.contains(r#"data-odc-orig-class="nondv""#), "{html}");
    // Slots never carry the source class.
    assert!(
        !html.contains(r#"class="odc-def" data-odc-orig-class"#),
        "{html}"
    );
}

#[test]
fn p7_sound_links_become_audio_buttons() {
    let fragment = format!(
        r#"<div><span class="entry">{HEAD}<span class="sense"><span class="sensenum">1</span><span class="def">produce</span></span></span></div>"#
    );
    let (html, _) = render(&fragment, "make");
    assert!(html.contains(r#"<button "#), "{html}");
    assert!(html.contains(r#"class="odc-audio""#), "{html}");
    assert!(
        html.contains(r#"data-odc-src="odc-resource://hwd/bre/f/make.mp3""#),
        "{html}"
    );
    assert!(
        html.contains(r#"data-odc-orig-src="sound://hwd/bre/f/make.mp3""#),
        "{html}"
    );
    assert!(html.contains(r#"data-odc-variant="uk""#), "{html}");
}

#[test]
fn p2_p3_p4_reference_bar_chrome() {
    let fragment = format!(
        r#"<div><span class="entry">{HEAD}<span class="buttons"><span class="popup-button">Entry menu</span><div class="at-link"><span class="entry"><span class="popheader pope_menu">MENU FOR make</span></span>menu body</div></span><div class="at-link"><span class="entry"><span class="popheader popetym">WORD ORIGIN</span></span>origin body</div><span class="sense"><span class="sensenum">1</span><span class="def">produce</span></span></span></div>"#
    );
    let (html, report) = render(&fragment, "make");
    assert!(html.contains(r#"<nav class="odc-refbar">"#), "{html}");
    assert!(
        html.contains(r#"<details class="odc-refbar__item" data-odc-kind="toc">"#),
        "{html}"
    );
    // P4: the entry menu caption is chrome, carried by the marker, not by text.
    assert!(html.contains(r#"data-odc-ui="entry-menu""#), "{html}");
    assert!(
        html.contains(r#"<details class="odc-refbar__item" data-odc-kind="word-origin">"#),
        "{html}"
    );
    assert!(html.contains("Word Origin"), "{html}");
    assert!(html.contains(r#"<div class="odc-ref-panel">"#), "{html}");
    assert!(
        html.contains(r#"<div class="odc-ref-block" data-odc-source="entry">"#),
        "{html}"
    );
    assert!(report.ok(), "{report:?}");
}

#[test]
fn p8_self_check_reports_a_missing_key() {
    let fragment = format!(
        r#"<div><span class="entry">{HEAD}<span class="sense"><span class="sensenum">1</span><span class="def">produce</span></span></span></div>"#
    );
    let (_, report) = render(&fragment, "make");
    assert!(report.ok(), "{report:?}");
    let names: Vec<&str> = report.items.iter().map(|item| item.name).collect();
    assert_eq!(
        names,
        vec!["entry-root", "unique-id", "slot-class", "head", "key"]
    );
}

// ---------------------------------------------------------------------------
// OALD9 dialect: skeleton rebuild on real samples (C1)
// ---------------------------------------------------------------------------

/// Minimal OALD9 mapping used as a *fixture*: it only exercises the executor
/// and the presentation layer, it is not a shipping profile.
const OALD9_FIXTURE: &str = r#"
{"schema_version":"odc.profile/v0","profile_id":"test.oald9.shape",
 "dict":{"name":"fixture","lang":"en"},"engine":{"name":"human"},
 "rules":[
  {"id":"cleanup.link","stage":"cleanup","priority":100,"op":"drop","match":{"selector":"link"}},
  {"id":"cleanup.script","stage":"cleanup","priority":99,"op":"drop","match":{"selector":"script"}},
  {"id":"root.entry","stage":"root","priority":100,"op":"map",
   "match":{"selector":"div.entry","cardinality":{"min":1}},
   "emit":{"element":"section","classes":["odc-entry"],"move":"children",
           "data":{"odc-key":"@key","odc-dict":"@dict","odc-lang":"@lang"}}},
  {"id":"head.wrap","stage":"slot","priority":96,"op":"wrap",
   "match":{"selector":"h2.h"},
   "emit":{"element":"header","classes":["odc-head"]}},
  {"id":"head.hw","stage":"slot","priority":95,"op":"map",
   "match":{"selector":"h2.h"},
   "emit":{"element":"h1","classes":["odc-hw"],"move":"children"}},
  {"id":"pos","stage":"slot","priority":90,"op":"map",
   "match":{"selector":"span.pos"},
   "emit":{"element":"h2","classes":["odc-pos"],"move":"children"}},
  {"id":"sense","stage":"slot","priority":80,"op":"map",
   "match":{"selector":"li.sn-g"},
   "emit":{"element":"div","classes":["odc-sense"],"move":"children"}},
  {"id":"def","stage":"slot","priority":70,"op":"map",
   "match":{"selector":"span.def"},
   "emit":{"element":"span","classes":["odc-def"],"move":"children"}}
 ]}
"#;

fn oald9_profile() -> Profile {
    Profile::from_json(OALD9_FIXTURE).unwrap_or_else(|error| panic!("fixture parses: {error}"))
}

/// Read a sample from the survey output; `None` when the survey is absent
/// (CI has no sample tree, so the test reports a skip instead of failing).
fn sample(name: &str) -> Option<String> {
    let path = format!("/tmp/odc-oald9-samples/{name}");
    match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(_) => {
            eprintln!("skip: {path} is not available");
            None
        }
    }
}

/// The survey samples start with the record's own navigation (dict-top) and
/// keep an unclosed `ol`; both are part of what the rebuild has to survive, so
/// the test keeps the text as-is instead of trimming it.
fn render_sample(name: &str) -> Option<String> {
    let text = sample(name)?;
    let start = text
        .find("div class=\"entry\"")
        .map(|at| at.saturating_sub(5))?;
    let body = &text[start..];
    match odc_core::profile::exec::apply(&oald9_profile(), body, &ctx(name)) {
        Outcome::Adapted { html, .. } | Outcome::Unadapted { html, .. } => Some(html),
        Outcome::Alias { .. } => None,
    }
}

#[test]
fn oald9_senses_are_grouped_into_position_blocks() {
    let mut checked = 0;
    for name in ["028382_go.html", "066502_take.html", "007803_break.html"] {
        let Some(out) = render_sample(name) else {
            continue;
        };
        checked += 1;
        assert!(out.contains("odc-entry"), "{name}: no entry\n{out}");
        let first_block = out.find("odc-pos-block").unwrap_or_else(|| {
            panic!("{name}: no position block\n{out}");
        });
        let first_sense = out.find("class=\"odc-sense\"").unwrap_or_else(|| {
            panic!("{name}: no sense\n{out}");
        });
        assert!(
            first_block < first_sense,
            "{name}: a sense escaped the position blocks\n{out}"
        );
        let verb = out.find("data-odc-pos=\"verb\"").unwrap_or_else(|| {
            panic!("{name}: verb block missing\n{out}");
        });
        let noun = out.find("data-odc-pos=\"noun\"").unwrap_or_else(|| {
            panic!("{name}: noun block missing\n{out}");
        });
        assert!(verb < noun, "{name}: positions lost their source order");
        // The long, unclosed list wrapper is gone: it was a generic container,
        // not a slot.
        assert!(
            !out.contains("<ol") || out.matches("<ol").count() < 2,
            "{name}: list wrapper survived around the blocks"
        );
    }
    if checked == 0 {
        eprintln!("skip: no OALD9 sample available");
    }
}
