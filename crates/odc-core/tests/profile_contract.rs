//! Contract tests: a profile run, through the engine, judged by D1.
//!
//! The unit tests in `profile::contract` prove each rule in isolation;
//! these prove the wiring the command line uses: apply a profile, hand the
//! source and the L1 fragment to the ruler.

use odc_core::adapters::ParseContext;
use odc_core::profile::contract;
use odc_core::profile::{Outcome, Profile};

fn ctx() -> ParseContext {
    ParseContext::new("fixture", "fixture", "en")
}

/// One rule: a record wrapper becomes the entry skeleton.
fn profile() -> Profile {
    let text = r#"{"schema_version":"odc.profile/v0","profile_id":"fixture.en",
        "dict":{"name":"fixture","lang":"en"},"engine":{"name":"human"},
        "rules":[{"id":"root","stage":"root","op":"map","match":{"cardinality":{"min":0},"selector":"div.rec"},
                  "emit":{"element":"section","classes":["odc-entry"],"move":"children"}}]}"#;
    Profile::from_json(text).unwrap_or_else(|error| panic!("profile parses: {error}"))
}

fn l1(profile: &Profile, source: &str) -> String {
    match profile.apply(source, &ctx()) {
        Outcome::Adapted { html, .. } | Outcome::Unadapted { html, .. } => html,
        Outcome::Alias { target } => panic!("unexpected alias: {target}"),
    }
}

#[test]
fn a_profile_run_that_keeps_the_text_passes_the_contract() {
    let profile = profile();
    let source = r#"<div class="rec"><p>alpha beta</p><p>gamma</p></div>"#;
    let produced = l1(&profile, source);
    let verdict = contract::check(source, &produced);
    assert!(verdict.ok(), "{:?}\n{produced}", verdict.reasons);
}

#[test]
fn an_alias_record_is_an_alias_not_a_body() {
    // The command line counts these and never scores them: an alias record
    // carries no fragment to compare against.
    let profile = profile();
    match profile.apply("@@@LINK=other\n", &ctx()) {
        Outcome::Alias { target } => assert_eq!(target, "other"),
        _ => panic!("an @@@LINK record must stay an alias"),
    }
}

#[test]
fn a_source_class_that_survives_is_reported() {
    let profile = profile();
    let source = r#"<div class="rec"><p class="note">alpha</p></div>"#;
    let produced = l1(&profile, source);
    // P5 turns the source class into provenance, so the residue check passes.
    assert!(
        produced.contains("data-odc-orig-class=\"note\""),
        "{produced}"
    );
    let verdict = contract::check(source, &produced);
    assert!(verdict.clean_ok, "{:?}", verdict.reasons);
}
