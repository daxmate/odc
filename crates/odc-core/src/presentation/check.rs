//! P8: the five self-checks a rendered entry must pass.

use markup5ever_rcdom::Handle;

use crate::adapters::ParseContext;
use crate::presentation::{attr, elements, has_class, tag};

/// Result of one check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckItem {
    /// Stable check name.
    pub name: &'static str,
    /// Whether the check passed.
    pub ok: bool,
    /// Human readable detail (empty when `ok`).
    pub detail: String,
}

impl CheckItem {
    fn pass(name: &'static str) -> Self {
        Self {
            name,
            ok: true,
            detail: String::new(),
        }
    }

    fn fail(name: &'static str, detail: String) -> Self {
        Self {
            name,
            ok: false,
            detail,
        }
    }
}

/// Report of the presentation self-check.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckReport {
    /// Individual checks, always five entries.
    pub items: Vec<CheckItem>,
    /// Diagnostics that are not pass/fail checks (kept-verbatim references,
    /// unresolved source anchors). Empty on a fully resolved record.
    pub notes: Vec<String>,
}

impl CheckReport {
    /// Whether every check passed.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.items.iter().all(|item| item.ok)
    }

    /// Names of the failed checks.
    #[must_use]
    pub fn failures(&self) -> Vec<&'static str> {
        self.items
            .iter()
            .filter(|item| !item.ok)
            .map(|item| item.name)
            .collect()
    }
}

/// Run the checks over a finished document.
pub fn run(root: &Handle, ctx: &ParseContext, notes: Vec<String>) -> CheckReport {
    let items = vec![
        entry_roots(root),
        unique_ids(root),
        no_source_classes(root),
        head_present(root),
        key_matches(root, ctx),
    ];
    CheckReport { items, notes }
}

/// 1. Every entry is a `section.odc-entry` at the top level.
fn entry_roots(root: &Handle) -> CheckItem {
    let mut count = 0;
    for child in root.children.borrow().iter() {
        if !crate::profile::selector::is_element(child) {
            continue;
        }
        if tag(child) == "section" && has_class(child, "odc-entry") {
            count += 1;
        } else {
            return CheckItem::fail("entry-root", format!("stray <{}> at the top", tag(child)));
        }
    }
    if count == 0 {
        return CheckItem::fail("entry-root", String::from("no section.odc-entry"));
    }
    CheckItem::pass("entry-root")
}

/// 2. Anchor ids are unique.
fn unique_ids(root: &Handle) -> CheckItem {
    let mut seen = std::collections::BTreeSet::new();
    for spot in elements(root) {
        let Some(id) = attr(&spot.node, "id") else {
            continue;
        };
        if !seen.insert(id.clone()) {
            return CheckItem::fail("unique-id", format!("duplicate id {id}"));
        }
    }
    CheckItem::pass("unique-id")
}

/// 3. No element keeps a bare source `class`.
fn no_source_classes(root: &Handle) -> CheckItem {
    for spot in elements(root) {
        if attr(&spot.node, "class").is_none() {
            continue;
        }
        let classes = crate::presentation::classes(&spot.node);
        if classes.iter().any(|class| class.starts_with("odc-")) {
            continue;
        }
        return CheckItem::fail(
            "slot-class",
            format!("<{}> keeps class {:?}", tag(&spot.node), classes.join(" ")),
        );
    }
    CheckItem::pass("slot-class")
}

/// 4. Every entry has a header.
fn head_present(root: &Handle) -> CheckItem {
    for child in root.children.borrow().iter() {
        if !crate::profile::selector::is_element(child) || !has_class(child, "odc-entry") {
            continue;
        }
        let has_header = child
            .children
            .borrow()
            .iter()
            .any(|kid| tag(kid) == "header" && has_class(kid, "odc-head"));
        if !has_header {
            return CheckItem::fail("head", String::from("entry without header.odc-head"));
        }
    }
    CheckItem::pass("head")
}

/// 5. The entry key on the root matches the parse context.
fn key_matches(root: &Handle, ctx: &ParseContext) -> CheckItem {
    for child in root.children.borrow().iter() {
        if !crate::profile::selector::is_element(child) || !has_class(child, "odc-entry") {
            continue;
        }
        if let Some(key) = attr(child, "data-odc-key") {
            if key != ctx.key {
                return CheckItem::fail("key", format!("key {key} != {}", ctx.key));
            }
        }
    }
    CheckItem::pass("key")
}
