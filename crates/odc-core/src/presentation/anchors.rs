//! P6: anchor ids for position blocks, senses and subsenses.

use std::collections::HashMap;

use markup5ever_rcdom::Handle;

use crate::adapters::ParseContext;
use crate::doc;
use crate::presentation::derive::{ANCHOR, ANCHOR_SELF};
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

/// P6: point in-page links at the ids the entry actually has.
///
/// A source record references its own anchors by opaque uuid (`<a id="…">`
/// plus `entry://#…`), while L1 uses generated ids. Two channels are recorded
/// during capture, and both survive the rule pass:
///
/// * the anchor element itself, resolved against the block that contains it;
///   an anchor that opens an entry resolves to the first block that follows it
///   inside the same entry,
/// * the block that follows the anchor, resolved against its own id or the
///   first id inside it — this is how a mapped source block keeps its anchor
///   after the bare anchor element is gone.
///
/// Mapping runs in document order, so a record with duplicate source anchors
/// always resolves the same way. Links are rewritten, ids are never invented:
/// an anchor that resolves to nothing stays as it is and is reported, because
/// adding an id would change the P6 contract.
pub fn alias(root: &Handle) -> Vec<String> {
    let mut notes = Vec::new();
    let mut map: HashMap<String, String> = HashMap::new();
    let mut sites: Vec<AnchorSite> = Vec::new();
    let mut ids: Vec<(usize, String)> = Vec::new();
    let mut next_entry = 0usize;
    scan(root, None, 0, &mut next_entry, &mut sites, &mut ids);

    let mut sources: Vec<String> = Vec::new();
    for site in &sites {
        sources.push(site.source.clone());
    }
    for site in sites {
        if let Some(target) = site.resolve(&ids) {
            record(&mut map, &mut notes, site.source, target);
        }
    }
    for spot in elements(root) {
        let Some(source) = attr(&spot.node, ANCHOR) else {
            continue;
        };
        if !sources.contains(&source) {
            sources.push(source.clone());
        }
        if let Some(target) = target_id(&spot.node) {
            record(&mut map, &mut notes, source, target);
        }
    }
    for source in sources {
        if !map.contains_key(&source) {
            notes.push(format!("source anchor {source:?} has no generated id"));
        }
    }
    rewrite_links(root, &map);
    strip_resolved_anchors(root, &map);
    notes
}

/// Drop the bare source anchors that were resolved.
///
/// Once a link points at a generated id, an empty `<a id>` is dead weight: L1
/// carries generated ids only. Anchors that did not resolve stay, because the
/// link still needs them.
fn strip_resolved_anchors(node: &Handle, map: &HashMap<String, String>) {
    let mut index = 0;
    loop {
        let Some(child) = node.children.borrow().get(index).cloned() else {
            break;
        };
        if !crate::profile::selector::is_element(&child) {
            index += 1;
            continue;
        }
        if is_resolved_anchor(&child, map) {
            node.children.borrow_mut().remove(index);
            continue;
        }
        strip_resolved_anchors(&child, map);
        index += 1;
    }
}

/// Whether `node` is an empty source anchor that no longer carries a link.
fn is_resolved_anchor(node: &Handle, map: &HashMap<String, String>) -> bool {
    if crate::presentation::tag(node) != "a" || attr(node, "href").is_some() {
        return false;
    }
    if !crate::profile::selector::text_content(node)
        .trim()
        .is_empty()
    {
        return false;
    }
    attr(node, "id").is_some_and(|id| map.contains_key(&id))
}

/// One bare source anchor, with what is needed to resolve it later.
struct AnchorSite {
    /// The source `id`/`name` value.
    source: String,
    /// Id of the block that contains the anchor, when it has one.
    scope: Option<String>,
    /// Entry the anchor belongs to; `0` means outside every entry.
    entry: usize,
    /// How many ids were known when the anchor was reached.
    after: usize,
}

impl AnchorSite {
    /// Generated id this anchor points at, if any.
    ///
    /// An anchor inside an entry resolves inside that entry only, so a link
    /// never jumps to a sibling homograph. A record-level anchor (`entry == 0`)
    /// resolves to the first generated id of the record, which is what a
    /// "back to top" link means.
    fn resolve(&self, ids: &[(usize, String)]) -> Option<String> {
        self.scope.clone().or_else(|| {
            ids.iter()
                .skip(self.after)
                .find(|(entry, _)| self.entry == 0 || *entry == self.entry)
                .map(|(_, id)| id.clone())
        })
    }
}

/// Collect every bare source anchor with the id of the block around it.
///
/// The scope is tracked while descending instead of read from the parent link:
/// the rule executor moves nodes between parents without rewriting the `Weak`
/// parent pointers, so only a top-down walk knows the real structure.
fn scan(
    node: &Handle,
    scope: Option<String>,
    entry: usize,
    next_entry: &mut usize,
    sites: &mut Vec<AnchorSite>,
    ids: &mut Vec<(usize, String)>,
) {
    for child in node.children.borrow().clone() {
        if !crate::profile::selector::is_element(&child) {
            continue;
        }
        let mut owner = entry;
        if has_class(&child, "odc-entry") {
            *next_entry += 1;
            owner = *next_entry;
        }
        let own_id = attr(&child, "id");
        if let Some(id) = &own_id {
            ids.push((owner, id.clone()));
        }
        if let Some(source) = attr(&child, ANCHOR_SELF) {
            doc::remove_attr(&child, ANCHOR_SELF);
            sites.push(AnchorSite {
                source,
                scope: scope.clone(),
                entry: owner,
                after: ids.len(),
            });
        }
        let inner = own_id.or_else(|| scope.clone());
        scan(&child, inner, owner, next_entry, sites, ids);
    }
}

/// Point every in-page link at its generated id.
fn rewrite_links(root: &Handle, map: &HashMap<String, String>) {
    if map.is_empty() {
        return;
    }
    for spot in elements(root) {
        let Some(href) = attr(&spot.node, "href") else {
            continue;
        };
        let Some(name) = href.strip_prefix('#') else {
            continue;
        };
        let Some(target) = map.get(name) else {
            continue;
        };
        doc::set_attr(&spot.node, "href", &format!("#{target}"));
        doc::set_attr(&spot.node, "data-odc-target", target);
    }
}

/// Generated id of `node`, or of the first descendant that has one.
fn target_id(node: &Handle) -> Option<String> {
    if let Some(id) = attr(node, "id") {
        return Some(id);
    }
    elements(node)
        .into_iter()
        .find_map(|spot| attr(&spot.node, "id"))
}

/// Add one mapping; the first resolved target for a source wins.
fn record(
    map: &mut HashMap<String, String>,
    notes: &mut Vec<String>,
    source: String,
    target: String,
) {
    match map.get(&source) {
        Some(existing) if existing == &target => {}
        Some(existing) => notes.push(format!(
            "source anchor {source:?} is ambiguous; kept {existing:?}, ignored {target:?}"
        )),
        None => {
            map.insert(source, target);
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
