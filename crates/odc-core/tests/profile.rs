//! Profile executor tests.
//!
//! Every fixture below is synthetic: no dictionary text is copied here.

use odc_core::adapters::ParseContext;
use odc_core::profile::verify;
use odc_core::profile::{alias_target, Outcome, Profile};

fn ctx() -> ParseContext {
    ParseContext::new("fixture", "fixture", "en")
}

fn profile(rules: &str) -> Profile {
    let text = format!(
        r#"{{"schema_version":"odc.profile/v0","profile_id":"test.case",
            "dict":{{"name":"fixture","lang":"en"}},"engine":{{"name":"human"}},
            "rules":[{rules}]}}"#
    );
    Profile::from_json(&text).unwrap_or_else(|e| panic!("profile parses: {e}\n{text}"))
}

fn apply(p: &Profile, html: &str) -> String {
    match p.apply(html, &ctx()) {
        Outcome::Adapted { html, .. } => html,
        Outcome::Unadapted { html, .. } => panic!("unexpected fallback: {html}"),
        Outcome::Alias { target } => panic!("unexpected alias: {target}"),
    }
}

#[test]
fn selector_subset_matches_expected_elements() {
    let p = profile(
        r#"{"id":"slot.a","stage":"slot","op":"map",
             "match":{"selector":"div.a > span.b[data-k=v], #only"},
             "emit":{"element":"span","classes":["odc-x"],"move":"children"}}"#,
    );
    let html = r#"<div class="a"><span class="b" data-k="v">one</span><span class="b">two</span></div><i id="only">three</i>"#;
    let out = apply(&p, html);
    assert_eq!(out.matches("odc-x").count(), 2, "{out}");
    assert!(out.contains(">one<"), "{out}");
    assert!(out.contains(">three<"), "{out}");
}

#[test]
fn selector_adjacent_sibling() {
    let p = profile(
        r#"{"id":"slot.a","stage":"slot","op":"map",
             "match":{"selector":"h4 + p.note"},
             "emit":{"element":"p","classes":["odc-note"],"move":"children"}}"#,
    );
    let out = apply(
        &p,
        r#"<h4>h</h4><p class="note">n</p><p class="note">m</p>"#,
    );
    assert_eq!(out.matches("odc-note").count(), 1, "{out}");
}

#[test]
fn scope_limits_matches_to_their_block() {
    let p = profile(
        r#"{"id":"root","stage":"root","op":"map","match":{"selector":"div.rec"},
             "emit":{"element":"section","classes":["odc-entry"],"move":"children"}},
           {"id":"pos.one","stage":"slot","priority":90,"op":"map",
             "match":{"selector":"div.p"},
             "emit":{"element":"section","classes":["odc-pos-block"],"move":"children"}},
           {"id":"def.two","stage":"slot","priority":50,"op":"map",
             "match":{"selector":"span.d","scope":["pos-block"]},
             "emit":{"element":"div","classes":["odc-def"],"move":"children"}}"#,
    );
    let html = r#"<div class="rec"><div class="p"><span class="d">a</span></div><span class="d">b</span></div>"#;
    let out = apply(&p, html);
    assert_eq!(out.matches("odc-def").count(), 1, "{out}");
}

#[test]
fn wrap_and_rename_keep_text() {
    let p = profile(
        r#"{"id":"w","stage":"structure","priority":90,"op":"wrap","match":{"selector":"p.note"},
             "emit":{"element":"aside","classes":["odc-note"]}},
           {"id":"r","stage":"structure","priority":80,"op":"rename","match":{"selector":"b.strong"},
             "emit":{"element":"strong"}}"#,
    );
    let out = apply(&p, r#"<p class="note">x</p><b class="strong">y</b>"#);
    assert!(
        out.contains(
            "<aside class=\"odc-note\" data-odc-rule=\"w\"><p class=\"note\">x</p></aside>"
        ),
        "{out}"
    );
    assert!(
        out.contains("<strong class=\"strong\" data-odc-rule=\"r\">y</strong>"),
        "{out}"
    );
}

#[test]
fn unwrap_replaces_element_with_children() {
    let p =
        profile(r#"{"id":"u","stage":"structure","op":"unwrap","match":{"selector":"div.wrap"}}"#);
    let out = apply(&p, r#"<div class="wrap"><b>a</b><i>b</i></div>"#);
    assert!(!out.contains("wrap"), "{out}");
    assert!(
        out.contains("<b>a</b>") && out.contains("<i>b</i>"),
        "{out}"
    );
}

#[test]
fn drop_removes_elements_and_attributes() {
    let p = profile(
        r#"{"id":"c1","stage":"cleanup","priority":100,"op":"drop","match":{"selector":"script"}},
           {"id":"c2","stage":"cleanup","priority":90,"op":"drop","match":{"selector":"*"},
            "attributes":["style","on*"]}"#,
    );
    let out = apply(
        &p,
        r#"<script>x</script><p style="color:red" onclick="f()">keep</p>"#,
    );
    assert!(
        !out.contains("script") && !out.contains("style=") && !out.contains("onclick"),
        "{out}"
    );
    assert!(out.contains("keep"), "{out}");
}

#[test]
fn attr_sets_and_removes_attributes() {
    let p = profile(
        r#"{"id":"a1","stage":"numbering","op":"attr","match":{"selector":"div.s"},
             "emit":{"data":{"odc-n":"@key"},"attributes":{"-data-tmp":"x"}}}"#,
    );
    let out = apply(&p, r#"<div class="s" data-tmp="1">x</div>"#);
    assert!(out.contains("data-odc-n=\"fixture\""), "{out}");
    assert!(!out.contains("data-tmp"), "{out}");
}

#[test]
fn move_transfers_children_to_a_target() {
    let p = profile(
        r#"{"id":"m","stage":"structure","op":"move","match":{"selector":"span.note"},
             "emit":{"target":"footer","move":"children"}}"#,
    );
    let out = apply(
        &p,
        r#"<section><span class="note">moved</span><footer>f</footer></section>"#,
    );
    assert!(out.contains("<footer>fmoved</footer>"), "{out}");
    assert!(!out.contains("note"), "{out}");
}

#[test]
fn number_restarts_per_scope() {
    let p = profile(
        r#"{"id":"s","stage":"slot","op":"map","match":{"selector":"div.blk"},
             "emit":{"element":"section","classes":["odc-pos-block"],"move":"children"}},
           {"id":"n","stage":"numbering","op":"number","match":{"selector":"div.i","scope":["pos-block"]},
             "emit":{"attribute":"data-odc-n"},"reset_on":["pos-block"]}"#,
    );
    let html = r#"<div class="blk"><div class="i">a</div><div class="i">b</div></div><div class="blk"><div class="i">c</div></div>"#;
    let out = apply(&p, html);
    let numbers: Vec<&str> = out
        .split("data-odc-n=\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap_or(""))
        .collect();
    assert_eq!(numbers, vec!["1", "2", "1"], "{out}");
}

#[test]
fn segment_cuts_sibling_runs() {
    let p = profile(
        r#"{"id":"g","stage":"segment","op":"segment","match":{"selector":"div.body"},
             "boundary":{"start":"h3.pos","until":["h3.pos"],"max_depth":1},
             "emit":{"element":"section","classes":["odc-pos-block"]}}"#,
    );
    let html = r#"<div class="body"><h3 class="pos">noun</h3><p>a</p><h3 class="pos">verb</h3><p>b</p></div>"#;
    let out = apply(&p, html);
    assert_eq!(out.matches("odc-pos-block").count(), 2, "{out}");
    assert!(out.contains("noun") && out.contains("verb"), "{out}");
}

#[test]
fn split_separates_homographs() {
    let p = profile(
        r#"{"id":"root","stage":"root","op":"map","match":{"selector":"div.rec"},
             "emit":{"element":"section","classes":["odc-entry"],"move":"children"}},
           {"id":"sp","stage":"split","op":"split","match":{"selector":"section.odc-entry","min_count":2},
             "split_by":{"selector":"div.hom","keep_selector":true},
             "emit":{"element":"section","attribute":"data-odc-homograph","numbering":{"start":1}}},
           {"id":"un","stage":"split","priority":40,"op":"unwrap","match":{"selector":"div.hom"}}"#,
    );
    let html = r#"<div class="rec"><div class="hom"><b>one</b></div><div class="hom"><b>two</b></div></div>"#;
    let out = apply(&p, html);
    assert_eq!(out.matches("odc-entry").count(), 2, "{out}");
    assert!(out.contains("data-odc-homograph=\"2\""), "{out}");
    assert!(out.contains("one") && out.contains("two"), "{out}");
}

#[test]
fn redirect_marks_the_target() {
    let p = profile(
        r#"{"id":"rd","stage":"root","op":"redirect","match":{"selector":"p.alias"},
             "emit":{"classes":["odc-entry--redirect"],"data":{"odc-target":"@key"}}}"#,
    );
    let out = apply(&p, r#"<p class="alias">see</p>"#);
    assert!(out.contains("odc-entry--redirect"), "{out}");
    assert!(out.contains("data-odc-target=\"fixture\""), "{out}");
}

#[test]
fn p0_miss_falls_back_to_raw_html() {
    let p = profile(
        r#"{"id":"head","stage":"slot","op":"map","match":{"selector":"span.def","cardinality":{"min":1}},
             "emit":{"element":"div","classes":["odc-def"],"move":"children"}}"#,
    );
    let outcome = p.apply("<p>no head here</p>", &ctx());
    let html = outcome.html().expect("html");
    assert!(html.contains("data-odc-unadapted=\"1\""), "{html}");
    assert!(html.contains("data-odc-reason="), "{html}");
}

#[test]
fn alias_records_do_not_enter_the_pipeline() {
    assert_eq!(alias_target("@@@LINK=target\n").as_deref(), Some("target"));
    assert_eq!(alias_target("<div>x</div>"), None);
    let p = profile(
        r#"{"id":"head","stage":"slot","op":"map","match":{"selector":"span.hw"},
             "emit":{"element":"h1","classes":["odc-hw"],"move":"children"}}"#,
    );
    assert!(matches!(
        p.apply("@@@LINK=other", &ctx()),
        Outcome::Alias { .. }
    ));
}

#[test]
fn fidelity_helpers_compare_whitespace_collapsed_text() {
    let left = "<p> one  <b>two</b>\n three </p>";
    let right = "<div>one <i>two</i> three</div>";
    assert!(verify::text_fidelity(left, right));
    assert!(verify::word_fidelity(left, right));
    assert!(!verify::text_fidelity(left, "<p>two one three</p>"));
    assert!(verify::word_fidelity(left, "<p>two one three</p>"));
}

#[test]
fn fingerprint_is_stable_and_histogram_based() {
    let mut first = verify::FingerprintBuilder::default();
    first.add(r#"<div class="a"><span class="b">x</span></div>"#);
    let mut second = verify::FingerprintBuilder::default();
    second.add(r#"<div class="a"><span class="b">different text</span></div>"#);
    assert_eq!(first.digest(), second.digest());
    let mut third = verify::FingerprintBuilder::default();
    third.add(r#"<div class="c"><span class="b">x</span></div>"#);
    assert_ne!(first.digest(), third.digest());
}

#[test]
fn policy_profile_is_a_fixed_point_on_its_own_output() {
    let p = profile(
        r#"{"id":"root","stage":"root","op":"map","match":{"selector":"div.rec"},
             "emit":{"element":"section","classes":["odc-entry"],"move":"children"}},
           {"id":"head","stage":"slot","op":"map","match":{"selector":"span.hw","scope":["entry"]},
             "emit":{"element":"h1","classes":["odc-hw"],"move":"children"}}"#,
    );
    let once = apply(&p, r#"<div class="rec"><span class="hw">word</span></div>"#);
    let twice = apply(&p, &once);
    assert_eq!(once, twice);
    assert!(verify::idempotent(&p, &once, &ctx()));
}

#[test]
fn unknown_schema_and_empty_rules_are_rejected() {
    let bad_schema = Profile::from_json(
        r#"{"schema_version":"odc.profile/v9","profile_id":"x","dict":{"name":"d","lang":"en"},"engine":{"name":"human"},"rules":[{"id":"a.b","stage":"slot","op":"map","match":{"selector":"p"},"emit":{}}]}"#,
    );
    assert!(bad_schema.is_err());
    let empty = Profile::from_json(
        r#"{"schema_version":"odc.profile/v0","profile_id":"x","dict":{"name":"d","lang":"en"},"engine":{"name":"human"},"rules":[]}"#,
    );
    assert!(empty.is_err());
}

#[test]
fn unsupported_selector_is_rejected() {
    let bad = profile(
        r#"{"id":"a","stage":"slot","op":"map","match":{"selector":"div > > span"},"emit":{}}"#,
    );
    let outcome = bad.apply("<div><span>x</span></div>", &ctx());
    let stats = outcome.stats().expect("stats");
    assert!(!stats.errors.is_empty(), "expected selector error");
}
