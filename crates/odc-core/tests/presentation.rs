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
