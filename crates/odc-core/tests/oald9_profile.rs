//! OALD9 profile tests.
//!
//! Every fixture below is synthetic: it reproduces the *markup shape* of the
//! OALD9 dump (unclosed ol.h-g, flat li.sn-g siblings, span.collapse boxes,
//! pv-gs groups) without copying dictionary text. The profile under test is the
//! real one shipped in profiles/oald9.profile.json.

use odc_core::adapters::ParseContext;
use odc_core::profile::{Outcome, Profile};

fn profile() -> Profile {
    let text = include_str!("../../../profiles/oald9.profile.json");
    Profile::from_json(text).unwrap_or_else(|error| panic!("profile parses: {error}"))
}

fn ctx(key: &str) -> ParseContext {
    ParseContext::new(key, "oald9", "en")
}

fn apply(html: &str) -> String {
    let profile = profile();
    match profile.apply(html, &ctx("fixture")) {
        Outcome::Adapted { html, .. } => html,
        Outcome::Unadapted { html, .. } => panic!("unexpected fallback: {html}"),
        Outcome::Alias { target } => panic!("unexpected alias: {target}"),
    }
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// A record whose ol.h-g / top-container / top-g are never closed, exactly as
/// the OALD9 dump writes them.
fn record(entries: &str) -> String {
    format!(
        r#"<link rel="stylesheet" href="x.css"><a id="jump"></a>
<div class="dict-top"><a href="entry://#one" class="to-pos"><span class="pos">noun</span></a></div>
<div class="dict-merge">{entries}</div>"#
    )
}

fn entry(headword: &str, pos: &str, sense: &str) -> String {
    format!(
        r#"<div class="entry"><div class="h-g"><div class="top-container"><div class="top-g">
<div class="webtop-g"><span class="z">&nbsp;</span><h2 class="h">{headword}</h2>
<a href="entry://#one" class="to-top"><span class="pos">{pos}</span></a></div>
<span class="pos-g"><span class="pos">{pos}</span></span>
<div class="pron-gs ei-g"><span class="pron-g" geo="br"><span class="prefix">BrE</span>
<span class="phon">/x/</span></span></div>
<div class="clear">&nbsp;</div>
<span class="sn-gs"><li class="sn-g"><span class="num">1</span>
<span class="def">{sense}</span></li></span>
</div></div></div>"#
    )
}

#[test]
fn every_entry_keeps_its_own_head() {
    let html = record(&format!(
        "{}{}",
        entry("alpha", "noun", "first thing"),
        entry("alpha", "verb", "to do")
    ));
    let out = apply(&html);
    assert_eq!(count(&out, r#"class="odc-entry"#), 2, "{out}");
    assert_eq!(count(&out, r#"class="odc-head"#), 2, "{out}");
    assert!(out.contains(">alpha<"), "{out}");
}

#[test]
fn flat_senses_become_one_pos_block() {
    let out = apply(&record(&entry("alpha", "noun", "first thing")));
    assert!(
        out.contains(r#"class="odc-pos-block" data-odc-pos="noun""#),
        "{out}"
    );
    assert!(count(&out, r#"class="odc-sense"#) >= 1, "{out}");
    assert!(out.contains(r#"class="odc-def""#), "{out}");
}

#[test]
fn study_boxes_become_reference_blocks() {
    let boxed = format!(
        r#"{}<span class="collapse" otitle="Word Origin"><span class="unbox">
<span class="heading">Word Origin</span><span class="body">from latin</span></span></span>
<span class="collapse" otitle="Topic Tennis"><span class="unbox">
<span class="heading">Topic Tennis</span><span class="body">racket</span></span></span>"#,
        entry("alpha", "noun", "first thing")
    );
    let out = apply(&record(&boxed));
    assert!(out.contains(r#"class="odc-ref-block""#), "{out}");
    assert_eq!(count(&out, r#"data-odc-kind="reference""#), 1, "{out}");
    assert_eq!(count(&out, r#"data-odc-kind="topic""#), 1, "{out}");
    assert!(count(&out, r#"class="odc-ref-block"#) == 2, "{out}");
}

#[test]
fn phrasal_verbs_keep_their_group() {
    let pv = format!(
        r#"{}<span class="pv-gs"><span class="pv-g"><h4 class="pv">alpha out</h4>
<span class="sn-gs"><span class="sn-g"><span class="def">to leave</span></span></span>
</span></span>"#,
        entry("alpha", "noun", "first thing")
    );
    let out = apply(&record(&pv));
    assert!(out.contains(r#"class="odc-phrasal-verbs""#), "{out}");
    assert!(out.contains(r#"class="odc-phrasal-verb""#), "{out}");
    assert!(out.contains(r#"class="odc-phrasal-verb-hw""#), "{out}");
}

#[test]
fn no_source_class_survives() {
    let boxed = format!(
        r#"{}<span class="collapse" otitle="Word Origin"><span class="unbox">
<span class="heading">Word Origin</span><span class="body">from latin</span></span></span>"#,
        entry("alpha", "noun", "first thing")
    );
    let out = apply(&record(&boxed));
    for (index, _) in out.match_indices("class=\"") {
        // \`data-odc-orig-class\` ends with the same text but is a provenance
        // attribute, not a class attribute; only a bare class= counts.
        if index > 0 && !matches!(out.as_bytes()[index - 1], b' ' | b'\n' | b'\t' | b'<') {
            continue;
        }
        let value = &out[index + 7..];
        let list = value.split('"').next().unwrap_or_default();
        for token in list.split_whitespace() {
            assert!(token.starts_with("odc-"), "source class {token:?} in {out}");
        }
    }
}
