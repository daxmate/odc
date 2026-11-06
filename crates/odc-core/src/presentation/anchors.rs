//! P6: anchor ids for position blocks, senses and subsenses.

use markup5ever_rcdom::Handle;

use crate::adapters::ParseContext;
use crate::doc;
use crate::presentation::{attr, elements, has_class};

/// Assign `data-odc-n`, `data-odc-label`, `data-odc-scope` and `id`.
pub fn assign(root: &Handle, ctx: &ParseContext) {
    let entry = entry_slug(&ctx.key);
    let mut scope = entry.clone();
    let mut seq = 0usize;
    for spot in elements(root) {
        let node = &spot.node;
        if has_class(node, "odc-pos-block") {
            let pos = position_slug(attr(node, "data-odc-pos").as_deref(), node);
            scope = format!("{entry}-{pos}");
            doc::set_attr(node, "id", &scope);
            seq = 0;
            continue;
        }
        if has_class(node, "odc-sense") {
            seq += 1;
            let label = sense_label(node);
            let n = numeric_prefix(&label).unwrap_or_else(|| seq.to_string());
            doc::set_attr(node, "data-odc-n", &n);
            doc::set_attr(node, "data-odc-label", &label);
            doc::set_attr(node, "data-odc-scope", &scope);
            doc::set_attr(node, "id", &format!("{scope}-s{n}"));
            continue;
        }
        if has_class(node, "odc-subsense") {
            let label = sense_label(node);
            let letter = label.trim_end_matches(')').to_string();
            let parent = attr(node, "data-odc-scope").unwrap_or_else(|| scope.clone());
            doc::set_attr(node, "data-odc-n", &letter);
            doc::set_attr(node, "data-odc-label", &label);
            doc::set_attr(node, "data-odc-scope", &parent);
            doc::set_attr(node, "id", &format!("{parent}-s{seq}{letter}"));
        }
    }
}

/// Text of the sense number slot, trimmed.
fn sense_label(node: &Handle) -> String {
    let mut label = String::new();
    for spot in elements(node) {
        if has_class(&spot.node, "odc-sense-number") {
            label = crate::profile::selector::text_content(&spot.node);
            break;
        }
    }
    label.trim().to_string()
}

/// Leading digits of a label, if any.
fn numeric_prefix(label: &str) -> Option<String> {
    let digits: String = label.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

/// URL-safe slug of the entry key (spec section 3.4).
pub fn entry_slug(key: &str) -> String {
    let lower = key.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut dash = false;
    for ch in lower.chars() {
        let keep = ch.is_ascii_alphanumeric();
        if keep {
            out.push(ch);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        let mut encoded = String::from("u");
        for (index, ch) in key.chars().enumerate() {
            if index > 0 {
                encoded.push('-');
            }
            encoded.push_str(&format!("{:X}", u32::from(ch)));
        }
        return encoded;
    }
    out
}

/// Slug of the part-of-speech label.
fn position_slug(text: Option<&str>, node: &Handle) -> String {
    let raw = text.map(str::to_string).unwrap_or_else(|| {
        for spot in elements(node) {
            if has_class(&spot.node, "odc-pos") {
                return crate::profile::selector::text_content(&spot.node);
            }
        }
        String::new()
    });
    let slug = entry_slug(raw.trim());
    if slug.is_empty() {
        String::from("pos")
    } else {
        slug
    }
}
