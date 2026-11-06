//! Adapter tests: one real LDOCE6 snippet per slot family.

use odc_core::adapters::ldoce6::{entry_slug, resource_key, same_words, Ldoce6, SLUG};
use odc_core::adapters::{count_slots, Adapter, ParseContext};

fn ctx(key: &str) -> ParseContext {
    ParseContext::new(key, SLUG, "en")
}

fn parse(fragment: &str, key: &str) -> String {
    Ldoce6.parse(fragment, &ctx(key)).expect("parse")
}

/// Entry head as stored by LDOCE6++ (the wrapper spans are unbalanced in the
/// real record; this is the repaired shape).
const HEAD: &str = concat!(
    "<div><div class=\"chwd\"><a href=\"entry://#make_1\"><i>verb</i></a></div>",
    "<a name=\"6504937708254a64972bf4d4ccdfcc9d_make_1\"></a>",
    "<span class=\"entry\"><span class=\"entryhead\">",
    "<span class=\"hwd\">make</span>",
    "<span class=\"hyphenation frequent\">make</span>",
    "<span class=\"homnum frequent\">1</span>",
    "<span class=\"proncodes\"><span class=\"neutral\">/</span>",
    "<span class=\"pron\">meɪk</span><span class=\"neutral\">/</span></span>",
    "<span class=\"proncodes\"><span class=\"neutral\">/</span>",
    "<span class=\"pron\">meɪk</span><span class=\"neutral\">/</span></span>",
    "<span class=\"level tooltip\">●●●</span>",
    "<span class=\"freq\">S1</span><span class=\"freq\">W1</span>",
    "<span class=\"pos\">verb</span>",
    "<span class=\"inflections\"><span class=\"neutral\">(</span>",
    "<span class=\"ptandpp\"><span class=\"infllab\">past tense and past participle</span>",
    " made</span><span class=\"neutral\">)</span></span>",
    "<a href=\"sound://hwd/bre/a/ld45make.mp3\"></a>",
    "<a href=\"sound://hwd/ame/a/ld45make.mp3\"></a>",
    "</span></span></div>",
);

/// One numbered sense with a signpost, definition, translation and example.
const SENSE: &str = concat!(
    "<div><span class=\"entry\"><span class=\"entryhead\"><span class=\"hwd\">make</span>",
    "<span class=\"pos\">verb</span>",
    "<span class=\"sense newline\"><span class=\"sensenum\">1</span>",
    "<span class=\"signpost\"><signen>produce</signen><sign>制作，制造</sign></span>",
    "<sigt></sigt><l6></l6> <span class=\"gram\">[transitive]</span> ",
    "<span class=\"def\"><en>to produce something</en><tran>制造</tran></span>",
    "<deft></deft><l6></l6><span class=\"neutral\">:</span> ",
    "<span class=\"example\"><exaen>I made a box.</exaen><example>我做了一个箱子。</example>",
    "</span><exat></exat></span></span></span></div>",
);

/// Two inline reference boxes in one sense (source nests them as siblings).
const INLINE_BOXES: &str = concat!(
    "<div><span class=\"entry\"><span class=\"entryhead\"><span class=\"hwd\">make</span>",
    "<span class=\"pos\">verb</span></span>",
    "<span class=\"sense\"><span class=\"sensenum\">1</span>",
    "<span class=\"def\">to create</span>",
    "<span class=\"thesbox\"><span class=\"heading\">THESAURUS</span>",
    "<span class=\"section\"><span class=\"secheading\">to make</span></span></span>",
    "<span class=\"collobox\"><span class=\"heading\">COLLOCATIONS</span>",
    "<span class=\"collo\"><span class=\"collocate\">make</span></span></span>",
    "</span></span></span></div>",
);
/// A phrasal verb sub-entry with its own head and sense.
const PHRASAL: &str = concat!(
    "<div><span class=\"entry\"><span class=\"entryhead\"><span class=\"hwd\">make</span>",
    "<span class=\"pos\">verb</span>",
    "<span class=\"phrvbentry\"><span class=\"entryhead\"><span class=\"phrvbhwd\">make away with",
    "<span class=\"object\">somebody/something</span></span>",
    "<span class=\"pos\">phrasal verb</span></span>",
    "<a name=\"make_1_s43\"></a>",
    "<span class=\"sense newline\"><span class=\"sensenum\">1</span>",
    "<span class=\"registerlab\">informal</span>",
    "<span class=\"def\"><en>to steal something</en><tran>偷走</tran></span></span>",
    "</span></span></span></div>",
);

/// A collocation popup, including its header and corpus source.
const REF: &str = concat!(
    "<div><span class=\"entry\"><span class=\"entryhead\"><span class=\"hwd\">make</span>",
    "<span class=\"pos\">verb</span>",
    "<span class=\"buttons\"><div class=\"at-link\"><span class=\"entry\">",
    "<span class=\"popheader popcollo\">COLLOCATIONS FROM THE CORPUS</span>",
    "<span class=\"colloc collo\"><span class=\"collocate\">make</span> ",
    "<span class=\"colloinexa\">a decision</span></span>",
    "</span></div></span>",
    "</span></span></div>",
);

const VERB_TABLE: &str = concat!(
    "<div><span class=\"entry\"><span class=\"entryhead\"><span class=\"hwd\">go</span>",
    "<span class=\"pos\">verb</span>",
    "<span class=\"buttons\"><div class=\"at-link\"><span class=\"entry\">",
    "<span class=\"popheader popetym\">WORD ORIGIN</span>",
    "<span class=\"etymorigin\">gan</span>",
    "<span class=\"popup-button\">Verb Table</span>",
    "<div class=\"verbtable\"><span class=\"popverbs\">VERB TABLE</span>",
    "<div class=\"lemma\">go</div>",
    "<table><tbody><tr><td class=\"header\">Simple Form</td></tr></tbody></table></div>",
    "</span></div></span></span></span></div>",
);

#[test]
fn head_maps_to_hw_phon_pos_block() {
    let out = parse(HEAD, "make");
    assert!(out.contains("<h1 class=\"odc-hw\">make</h1>"), "{out}");
    assert!(
        out.contains("class=\"odc-hw-alt\" data-odc-kind=\"hyphenation\""),
        "{out}"
    );
    assert!(
        out.contains("class=\"odc-phon\" data-odc-variant=\"uk\""),
        "{out}"
    );
    assert!(
        out.contains("class=\"odc-phon\" data-odc-variant=\"us\""),
        "{out}"
    );
    assert!(
        out.contains("<span class=\"odc-inflection\" data-odc-kind=\"past\">"),
        "{out}"
    );
    assert!(out.contains("<header class=\"odc-head\">"), "{out}");
    assert!(
        out.contains("data-odc-pos=\"verb\" id=\"make-verb\""),
        "{out}"
    );
    assert!(out.contains("<h2 class=\"odc-pos\">verb</h2>"), "{out}");
    assert!(out.contains("data-odc-dict=\"ldoce6encn\""), "{out}");
    assert!(out.contains("data-odc-key=\"make\""), "{out}");
    assert!(out.contains("class=\"odc-audio\""), "{out}");
    assert!(
        out.contains("data-odc-src=\"odc-resource://hwd/bre/a/ld45make.mp3\""),
        "{out}"
    );
}

#[test]
fn sense_maps_to_p0_slots() {
    let out = parse(SENSE, "make");
    assert!(
        out.contains("<div class=\"odc-sense\" data-odc-n=\"1\" data-odc-label=\"1\""),
        "{out}"
    );
    assert!(
        out.contains("data-odc-scope=\"make-verb\" id=\"make-verb-s1\""),
        "{out}"
    );
    assert!(
        out.contains("<span class=\"odc-sense-number\">1</span>"),
        "{out}"
    );
    assert!(out.contains("<span class=\"odc-signpost\">"), "{out}");
    assert!(
        out.contains("<span class=\"odc-grammar\">[transitive]</span>"),
        "{out}"
    );
    assert!(out.contains("<span class=\"odc-def\">"), "{out}");
    assert!(
        out.contains("<span class=\"odc-def-trans\" data-odc-lang=\"zh\">制造</span>"),
        "{out}"
    );
    assert!(out.contains("<ul class=\"odc-examples\">"), "{out}");
    assert!(out.contains("<li class=\"odc-example\">"), "{out}");
    assert!(out.contains("<span class=\"odc-ex-text\">"), "{out}");
    assert!(
        out.contains("<span class=\"odc-ex-trans\" data-odc-lang=\"zh\">我做了一个箱子。</span>"),
        "{out}"
    );
    let slots = count_slots(&out);
    assert_eq!(slots.get("odc-sense"), 1);
    assert_eq!(slots.get("odc-def"), 1);
    assert_eq!(slots.get("odc-example"), 1);
}

#[test]
fn phrasal_verb_is_a_sub_entry() {
    let out = parse(PHRASAL, "make");
    assert!(out.contains("<section class=\"odc-pvs\">"), "{out}");
    assert!(
        out.contains(
            "<article class=\"odc-pv\" data-odc-lemma=\"make away with somebody/something\""
        ),
        "{out}"
    );
    assert!(out.contains("id=\"make-verb-pv1\""), "{out}");
    assert!(out.contains("<h3 class=\"odc-pv-hw\">"), "{out}");
    assert!(out.contains("data-odc-scope=\"make-verb-pv1\""), "{out}");
    assert_eq!(count_slots(&out).get("odc-phrasal-verb"), 1);
}

#[test]
fn reference_block_keeps_kind_and_source() {
    let out = parse(REF, "make");
    assert!(out.contains("<nav class=\"odc-refbar\">"), "{out}");
    assert!(
        out.contains("<details class=\"odc-refbar__item\" data-odc-kind=\"collocation\">"),
        "{out}"
    );
    assert!(
        out.contains("<div class=\"odc-ref-block\" data-odc-source=\"corpus\">"),
        "{out}"
    );
    assert!(
        out.contains("<summary class=\"odc-refbar__label\">Collocations</summary>"),
        "{out}"
    );
    assert!(
        out.contains("<p class=\"odc-ref-panel__title\">COLLOCATIONS FROM THE CORPUS</p>"),
        "{out}"
    );
    assert!(
        !out.contains("<details class=\"odc-refbar__item\" open"),
        "{out}"
    );
    assert!(out.contains("class=\"odc-collocation\""), "{out}");
    assert!(!out.contains("class=\"odc-ref\""), "{out}");
}

#[test]
fn inline_reference_boxes_share_the_toolbar() {
    let out = parse(INLINE_BOXES, "make");
    let nav = out.find("<nav class=\"odc-refbar\">").expect("toolbar");
    let thes = out
        .find("data-odc-kind=\"thesaurus\"")
        .expect("thesaurus box");
    let collo = out
        .find("data-odc-kind=\"collocation\"")
        .expect("collocation box");
    assert!(
        nav < thes && nav < collo,
        "boxes outside the toolbar: {out}"
    );
    assert!(
        out.contains("<summary class=\"odc-refbar__label\">THESAURUS</summary>"),
        "{out}"
    );
    assert!(
        out.contains("<summary class=\"odc-refbar__label\">Collocations</summary>"),
        "{out}"
    );
    assert!(
        !out.contains("<details class=\"odc-refbar__item\" open"),
        "{out}"
    );
}
#[test]
fn verb_table_gets_its_own_toolbar_entry() {
    let out = parse(VERB_TABLE, "go");
    let vt = out
        .find("data-odc-kind=\"verb-table\"")
        .expect("verb table entry");
    let table = out
        .find("data-odc-orig-class=\"verbtable\"")
        .expect("table markup");
    let vt_next = out[vt + 1..]
        .find("<details class=\"odc-refbar__item\"")
        .map_or(out.len(), |at| vt + 1 + at);
    assert!(
        vt < table && table < vt_next,
        "table outside its panel: {out}"
    );
    let origin = out
        .find("data-odc-kind=\"word-origin\"")
        .expect("word origin entry");
    let origin_next = out[origin + 1..]
        .find("<details class=\"odc-refbar__item\"")
        .map_or(out.len(), |at| origin + 1 + at);
    assert!(
        !(origin < table && table < origin_next),
        "table leaked into word origin: {out}"
    );
    assert!(
        out.contains("<summary class=\"odc-refbar__label\">Verb Table</summary>"),
        "{out}"
    );
}

#[test]
fn text_is_preserved_verbatim() {
    for fragment in [HEAD, SENSE, PHRASAL, REF, INLINE_BOXES, MENU, VERB_TABLE] {
        let out = parse(fragment, "make");
        assert!(same_words(fragment, &out), "text drift for {fragment}");
    }
}

#[test]
fn parse_is_idempotent() {
    let once = parse(SENSE, "make");
    let twice = parse(&once, "make");
    assert_eq!(once, twice);
}

/// Entry with a popup entry menu, one sense and a thesaurus box: the menu and
/// reference blocks must follow the main senses in reading order.
const MENU: &str = concat!(
    "<div><span class=\"entry\"><span class=\"entryhead\"><span class=\"hwd\">make</span>",
    "<span class=\"pos\">verb</span>",
    "<span class=\"buttons\"><span class=\"popup-button\">Entry menu<atl></atl></span>",
    "<div class=\"at-link\"><span class=\"entry\">",
    "<span class=\"popheader pope_menu\">MENU FOR make</span>",
    "<span class=\"entrymenu\"><span class=\"hwd\">make</span>",
    "<span class=\"menuitem\"><span class=\"sensenum\">1</span> ",
    "<span class=\"signpost\"><signen>produce</signen><sign>制作</sign></span></span>",
    "</span></span></div></span>",
    "<span class=\"sense newline\"><span class=\"sensenum\">1</span>",
    "<span class=\"def\"><en>to produce something</en><tran>制造</tran></span>",
    "<span class=\"example\"><en>make a box</en><tran>做个箱子</tran></span>",
    "</span>",
    "<span class=\"thesbox\"><span class=\"popheader popthes\">THESAURUS</span>",
    "<span class=\"thes item\">create</span></span>",
    "</span></span></div>",
);

#[test]
fn toolbar_sits_between_head_and_senses() {
    let out = parse(MENU, "make");
    let head = out.find("</header>").expect("head");
    let nav = out.find("<nav class=\"odc-refbar\">").expect("toolbar");
    let sense = out.find("class=\"odc-sense\"").expect("sense block");
    assert!(
        head < nav && nav < sense,
        "head {head}, nav {nav}, sense {sense}: {out}"
    );
    assert!(
        out.contains("<summary class=\"odc-refbar__label\">Entry menu</summary>"),
        "{out}"
    );
    assert!(
        out.contains("<p class=\"odc-ref-panel__title\">MENU FOR make</p>"),
        "{out}"
    );
    assert!(
        !out.contains("<details class=\"odc-refbar__item\" open"),
        "{out}"
    );
    assert!(!out.contains("class=\"odc-toc\""), "{out}");
}
#[test]
fn probe_claims_ldoce_and_rejects_other_markup() {
    let probe = Ldoce6.probe(SENSE).expect("ldoce probe");
    assert_eq!(probe.dict, SLUG);
    assert!(probe.confidence > 0.9, "{}", probe.confidence);
    assert!(probe.signals.iter().any(|signal| signal.name == "sensenum"));
    assert!(Ldoce6.probe("<p>hello <b>world</b></p>").is_none());
}

#[test]
fn unknown_markup_falls_back_to_raw() {
    let out = parse("<div class=\"something-else\"><p>plain</p></div>", "word");
    assert!(out.contains("data-odc-unadapted=\"1\""), "{out}");
    assert!(out.contains("data-odc-reason=\"parse-failed\""), "{out}");
    assert!(out.contains("plain"), "{out}");
}

#[test]
fn resources_map_to_odc_keys() {
    assert_eq!(
        Ldoce6.resolve("sound://exa/bre/5/p008.mp3").as_deref(),
        Some("odc-resource://exa/bre/5/p008.mp3")
    );
    assert_eq!(
        Ldoce6.resolve("img/spkr_g.png").as_deref(),
        Some("odc-resource://img/spkr_g.png")
    );
    assert_eq!(Ldoce6.resolve("  "), None);
    assert_eq!(
        resource_key("sound://hwd/ame/a/x.mp3").as_deref(),
        Some("odc-resource://hwd/ame/a/x.mp3")
    );
}

#[test]
fn slugs_follow_the_spec_rules() {
    assert_eq!(entry_slug("make up"), "make-up");
    assert_eq!(entry_slug("Make"), "make");
    assert_eq!(entry_slug("back1"), "back1");
    assert_eq!(entry_slug("打"), "u6253");
}

#[test]
fn split_line_rate_is_zero() {
    for fragment in [HEAD, SENSE, PHRASAL, REF] {
        let out = parse(fragment, "make");
        assert_eq!(odc_core::adapters::split_line_violations(&out), 0);
    }
}
