//! C5: strip the source page's presentation state.
//!
//! A dump keeps the state of the *browser session that produced it*: a box
//! that happened to be open (`is-active`), a block the page hides
//! (`hide="y"`), inline styling and event handlers. None of it describes the
//! dictionary, and keeping it makes L1 depend on the machine the dump came
//! from. The executor never needs these attributes either: a rule that wants
//! to hide something can ask for it through the slot vocabulary.
//!
//! Retention whitelist (never touched by this pass):
//!
//! | kept                          | why                                        |
//! |-------------------------------|--------------------------------------------|
//! | every `data-odc-*` attribute   | slot marks, provenance, anchors, resources |
//! | every `odc-*` class            | the L1 vocabulary itself                   |
//! | `id`, `href`, `src`, `alt`,    | content and navigation carried by L1       |
//! | `title`, `lang`, `dir`,        |                                            |
//! | `colspan`, `rowspan`, `headers`|                                            |
//! | every other attribute         | unknown source data stays, we only strip   |
//! |                               | what we can name as runtime state          |
//!
//! State that this pass removes:
//!
//! - the `is-active` class token (the rest of the class list stays);
//! - `hide` and `style` attributes;
//! - event handlers (`on` + letters, e.g. `onclick`, `onmouseover`).

use markup5ever_rcdom::{Handle, NodeData};

use crate::doc;
use crate::presentation::{attr, elements};

/// Class token that records "this box is open right now".
const STATE_CLASS: &str = "is-active";

/// Attributes that describe the source page, never the entry.
const STATE_ATTRS: [&str; 2] = ["hide", "style"];

/// Prefix that makes an attribute an engine marker, never a source state.
const SLOT_PREFIX: &str = "data-odc-";

/// Remove source runtime state from every element under `root`.
pub fn strip(root: &Handle) {
    for spot in elements(root) {
        let node = &spot.node;
        drop_state_attrs(node);
        drop_state_classes(node);
    }
}

fn drop_state_attrs(node: &Handle) {
    let NodeData::Element { attrs, .. } = &node.data else {
        return;
    };
    let doomed: Vec<String> = attrs
        .borrow()
        .iter()
        .filter(|item| !item.name.local.as_ref().starts_with(SLOT_PREFIX))
        .map(|item| item.name.local.to_string())
        .filter(|name| is_state(name))
        .collect();
    for name in doomed {
        doc::remove_attr(node, &name);
    }
}

fn drop_state_classes(node: &Handle) {
    let Some(class) = attr(node, "class") else {
        return;
    };
    let kept: Vec<&str> = class
        .split_whitespace()
        .filter(|token| *token != STATE_CLASS)
        .collect();
    if kept.len() == class.split_whitespace().count() {
        return;
    }
    if kept.is_empty() {
        doc::remove_attr(node, "class");
    } else {
        doc::set_attr(node, "class", &kept.join(" "));
    }
}

/// Whether an attribute name describes source runtime state.
fn is_state(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if STATE_ATTRS.contains(&lower.as_str()) {
        return true;
    }
    matches!(lower.strip_prefix("on"), Some(rest) if !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_alphabetic()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::exec::{parse_fragment, serialize};

    fn stripped(html: &str) -> String {
        let dom = parse_fragment(html);
        strip(&dom.document);
        serialize(&dom.document)
    }

    #[test]
    fn runtime_state_is_removed() {
        let html = r#"<div class="box is-active" hide="y" style="display:none" onclick="open()">
            <span onmouseover="x()">body</span>
        </div>"#;
        let out = stripped(html);
        assert!(!out.contains("is-active"), "{out}");
        assert!(!out.contains("hide="), "{out}");
        assert!(!out.contains("style="), "{out}");
        assert!(!out.contains("onclick"), "{out}");
        assert!(!out.contains("onmouseover"), "{out}");
        // The rest of the class list survives.
        assert!(out.contains(r#"class="box""#), "{out}");
    }

    #[test]
    fn the_whitelist_survives() {
        let html = r##"<section class="odc-pos-block is-active" data-odc-pos="verb" id="go-verb" style="x">
            <a class="odc-crossref" href="#go-noun" title="go" data-odc-target="go-noun" lang="en">go</a>
            <button class="odc-audio" data-odc-src="odc-resource://go.mp3" src="go.png" alt="play"></button>
        </section>"##;
        let out = stripped(html);
        for kept in [
            r#"class="odc-pos-block""#,
            r#"data-odc-pos="verb""#,
            r#"id="go-verb""#,
            r#"class="odc-crossref""#,
            r##"href="#go-noun""##,
            r#"data-odc-target="go-noun""#,
            r#"class="odc-audio""#,
            r#"data-odc-src="odc-resource://go.mp3""#,
            r#"src="go.png""#,
            r#"alt="play""#,
            r#"title="go""#,
            r#"lang="en""#,
        ] {
            assert!(out.contains(kept), "missing {kept}: {out}");
        }
        assert!(!out.contains("is-active"), "{out}");
        assert!(!out.contains("style="), "{out}");
    }

    #[test]
    fn a_state_only_class_disappears_instead_of_going_empty() {
        let out = stripped(r#"<div class="is-active">x</div>"#);
        assert!(!out.contains("class"), "{out}");
    }

    #[test]
    fn ordinary_attributes_are_not_state() {
        let html = r#"<div class="box" data-page="7" role="note">x</div>"#;
        assert_eq!(stripped(html), stripped(html));
        let out = stripped(html);
        assert!(out.contains(r#"data-page="7""#), "{out}");
        assert!(out.contains(r#"role="note""#), "{out}");
    }
}
