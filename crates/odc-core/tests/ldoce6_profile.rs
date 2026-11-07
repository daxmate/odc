//! LDOCE6 profile tests.
//!
//! Every fixture below is synthetic: no dictionary text is copied here. The
//! profile under test is the real one shipped in profiles/ldoce6.profile.json.

use odc_core::adapters::ParseContext;
use odc_core::profile::{Outcome, Profile};

fn profile() -> Profile {
    let text = include_str!("../../../profiles/ldoce6.profile.json");
    Profile::from_json(text).unwrap_or_else(|error| panic!("profile parses: {error}"))
}

fn ctx(key: &str) -> ParseContext {
    ParseContext::new(key, "ldoce6encn", "en")
}

fn apply(html: &str) -> String {
    let profile = profile();
    match profile.apply(html, &ctx("fixture")) {
        Outcome::Adapted { html, .. } => html,
        Outcome::Unadapted { html, .. } => panic!("unexpected fallback: {html}"),
        Outcome::Alias { target } => panic!("unexpected alias: {target}"),
    }
}

/// Whitespace-collapsed text of a fragment.
fn text_of(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let mut collapsed = String::new();
    for word in out.split_whitespace() {
        if !collapsed.is_empty() {
            collapsed.push(' ');
        }
        collapsed.push_str(word);
    }
    collapsed
}

#[test]
fn root_entry_carries_context_attributes() {
    let html = r#"<div><a name="x"></a><span class="entry"><span class="entryhead">
        <span class="hwd">alpha</span><span class="hyphenation">al-pha</span></span></span></div>"#;
    let out = apply(html);
    assert!(out.contains(r#"class="odc-entry""#), "{out}");
    assert!(out.contains(r#"data-odc-dict="ldoce6encn""#), "{out}");
    assert!(out.contains(r#"data-odc-key="fixture""#), "{out}");
    assert!(out.contains(r#"data-odc-lang="en""#), "{out}");
    assert!(out.contains(r#"<h1 class="odc-hw""#), "{out}");
    assert!(out.contains(r#"data-odc-kind="hyphenation""#), "{out}");
}

#[test]
fn popup_trigger_label_is_kept_as_source_caption() {
    let html = r#"<div><span class="entry"><span class="hwd">alpha</span>
        <span class="buttons"><span class="popup-button">Word Origin<atl></atl></span></span>
        <script>var x = 1;</script></span>"#;
    let out = apply(html);
    // B: the trigger caption is source text; only its source classes and the
    // runtime script are dropped.
    assert!(out.contains("Word Origin"), "{out}");
    assert!(!out.contains("popup-button\""), "{out}");
    assert!(!out.contains("<script"), "{out}");
    assert!(!out.contains("var x"), "{out}");
}

#[test]
fn part_of_speech_opens_a_block_with_a_heading() {
    let html = r#"<div><span class="entry"><span class="hwd">alpha</span>
        <span class="pos">noun</span><span class="sense">body</span>
        <span class="pos">verb</span><span class="sense">body two</span></span>"#;
    let out = apply(html);
    assert_eq!(out.matches(r#"class="odc-pos-block""#).count(), 2, "{out}");
    assert_eq!(out.matches(r#"class="odc-pos""#).count(), 2, "{out}");
    assert!(out.contains(">noun<"), "{out}");
    assert!(out.contains(">verb<"), "{out}");
}

#[test]
fn senses_definitions_and_numbers_are_mapped() {
    let html = r#"<div><span class="entry"><span class="hwd">alpha</span>
        <span class="pos">noun</span>
        <span class="sense"><span class="sensenum">2</span><span class="def">a thing</span></span></span>"#;
    let out = apply(html);
    assert!(out.contains(r#"<div class="odc-sense""#), "{out}");
    assert!(out.contains(r#"<span class="odc-sense-number""#), "{out}");
    assert!(out.contains(r#"<span class="odc-def""#), "{out}");
    assert!(out.contains(">a thing<"), "{out}");
}

#[test]
fn examples_become_list_items() {
    let html = r#"<div><span class="entry"><span class="hwd">alpha</span><span class="pos">verb</span>
        <span class="sense"><span class="def">to do</span>
        <span class="example">one sample <tran>样例</tran></span></span></span>"#;
    let out = apply(html);
    assert!(out.contains(r#"<li class="odc-example""#), "{out}");
    assert!(out.contains("one sample"), "{out}");
    assert!(out.contains("样例"), "{out}");
}

#[test]
fn homograph_bar_stays_inside_the_entry() {
    let html = r#"<div><div class="chwd"><i>noun</i> | <i>verb</i></div>
        <span class="entry"><span class="hwd">alpha</span></span></div>"#;
    let out = apply(html);
    // The bar is entry content, not chrome: it stays a lead block of the entry.
    assert!(out.contains(r#"data-odc-orig-class="chwd""#), "{out}");
    assert!(!out.contains(r#"<nav class="odc-refbar""#), "{out}");
    assert!(out.contains("noun"), "{out}");
}

#[test]
fn text_order_survives_the_translation() {
    let html = r#"<div><span class="entry"><span class="entryhead"><span class="hwd">alpha</span>
        <span class="hyphenation">al-pha</span></span>
        <span class="pos">noun</span>
        <span class="sense"><span class="sensenum">1</span><span class="def">first</span>
        <span class="example">an example</span></span></span>"#;
    let out = apply(html);
    assert_eq!(text_of(html), text_of(&out), "{out}");
}

#[test]
fn the_profile_is_deterministic() {
    let html = r#"<div><span class="entry"><span class="hwd">alpha</span>
        <span class="pos">noun</span><span class="sense"><span class="def">body</span></span></span>"#;
    let first = apply(html);
    let second = apply(html);
    assert_eq!(first, second);
}

#[test]
fn every_rule_has_a_rationale() {
    let profile = profile();
    let missing: Vec<&str> = profile
        .rules
        .iter()
        .filter(|rule| rule.rationale.is_none())
        .map(|rule| rule.id.as_str())
        .collect();
    assert!(missing.is_empty(), "rules without rationale: {missing:?}");
}
