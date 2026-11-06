//! Deterministic operator executor.
//!
//! The executor never guesses: every mutation comes from a rule in the
//! profile, stages run in a fixed order and the output is byte-stable for the
//! same input.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use html5ever::driver::parse_fragment_for_element;
use html5ever::serialize::{SerializeOpts, TraversalScope};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{ns, Attribute, LocalName, ParseOpts, QualName};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};

use crate::adapters::ParseContext;
use crate::doc::{element, make_attr, remove_at, remove_attr, replace_at, set_attr, take_children};
use crate::profile::schema::{
    Boundary, MatchSpec, MoveMode, Op, Profile, Rule, Scope, SplitBy, Stage,
};
use crate::profile::selector::{self, SelectorError, SelectorList};
use crate::profile::{Outcome, ProfileError, RunStats};

/// One selected element plus the context needed to mutate it.
struct Match {
    node: Handle,
    parent: Handle,
    index: usize,
    ancestors: Vec<Handle>,
}

impl Match {
    fn scope_chain(&self) -> Vec<Handle> {
        let mut chain = self.ancestors.clone();
        chain.push(self.node.clone());
        chain
    }
}

/// Parse a body fragment the same way ['crate::html'] does.
fn parse_fragment(input: &str) -> RcDom {
    let context = Node::new(NodeData::Element {
        name: QualName::new(None, ns!(html), LocalName::from("div")),
        attrs: RefCell::new(Vec::new()),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    });
    parse_fragment_for_element(RcDom::default(), ParseOpts::default(), context, true, None)
        .one(input.to_string())
}

/// Serialise the children of the document node, byte-stably.
fn serialize(root: &Handle) -> String {
    let handle = SerializableHandle::from(root.clone());
    let options = SerializeOpts {
        traversal_scope: TraversalScope::ChildrenOnly(None),
        ..SerializeOpts::default()
    };
    let mut buffer = Vec::new();
    if html5ever::serialize(&mut buffer, &handle, options).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

/// Run every rule of 'profile' over 'fragment'.
///
/// Returns an ['Outcome'] that always carries the full rule statistics, so a
/// caller can report partial coverage even when the fragment is unadapted.
pub fn apply(profile: &Profile, fragment: &str, ctx: &ParseContext) -> Outcome {
    let clean = crate::html::repair_sanitize(fragment).unwrap_or_else(|| String::from(fragment));
    let dom = parse_fragment(&clean);
    let root = dom.document.clone();
    normalize_document(&root);
    crate::presentation::capture(&root);
    let mut stats = RunStats::default();

    for stage in Stage::ORDER {
        for rule in profile.stage_rules(stage) {
            if let Err(error) = apply_rule(&root, rule, ctx, &mut stats) {
                stats.errors.push(format!("{}: {error}", rule.id));
            }
        }
    }

    for rule in &profile.rules {
        if !is_p0(rule) {
            continue;
        }
        let min = rule.matches.cardinality.as_ref().map_or(0, |c| c.min);
        if stats.hits.get(&rule.id).copied().unwrap_or(0) < min {
            stats.p0_missing.push(rule.id.clone());
        }
    }

    stats.presentation = crate::presentation::apply(&root, ctx);
    trim_document_edges(&root);
    canonical_whitespace(&root);
    let html = serialize(&root);
    // Re-running a profile on its own output must be a no-op: no rule matches
    // L1 markup, so an empty rule set plus an existing odc-entry root means
    // "already adapted", not "P0 rule failed".
    let already_adapted = contains_class(&root, "odc-entry");
    if stats.p0_failed() && !already_adapted {
        let fallback =
            crate::adapters::raw_fallback(&clean, ctx, crate::adapters::Reason::CoverageLow);
        return Outcome::Unadapted {
            html: fallback,
            stats,
        };
    }
    Outcome::Adapted { html, stats }
}

/// Hoist the body out of an implicit html/body wrapper.
///
/// A fragment whose source starts with a head element makes html5ever build a
/// document skeleton; L1 entries are plain fragments, so the skeleton is
/// removed before any rule runs.
fn contains_class(root: &Handle, class: &str) -> bool {
    if has_class(root, class) {
        return true;
    }
    root.children
        .borrow()
        .iter()
        .any(|child| contains_class(child, class))
}

/// Collapse line-breaking whitespace-only text nodes to a single newline.
///
/// The serialiser copies text nodes verbatim, so leaving the source's
/// indentation in place would make every parse/format round trip drift by a
/// few spaces. Whitespace that does not contain a newline stays untouched:
/// it can separate inline words.
fn canonical_whitespace(node: &Handle) {
    if matches!(node.data, NodeData::Text { .. }) {
        return;
    }
    merge_text_children(node);
    let children = node.children.borrow().clone();
    for child in children {
        canonical_whitespace(&child);
    }
}

/// Merge adjacent text children, then collapse line-breaking whitespace.
///
/// Parsing merges adjacent character tokens, so without the merge a parent
/// with two whitespace children would serialise as two newlines on the first
/// run and one on the second.
fn merge_text_children(node: &Handle) {
    let children = node.children.borrow().clone();
    let mut merged: Vec<Handle> = Vec::with_capacity(children.len());
    for child in children {
        let is_text = matches!(child.data, NodeData::Text { .. });
        if is_text {
            if let Some(previous) = merged.last() {
                if let NodeData::Text { contents: previous } = &previous.data {
                    if let NodeData::Text { contents } = &child.data {
                        let joined = format!("{}{}", previous.borrow(), contents.borrow());
                        previous.replace(StrTendril::from(joined));
                        continue;
                    }
                }
            }
        }
        merged.push(child);
    }
    *node.children.borrow_mut() = merged;

    let NodeData::Text { contents } = &node.data else {
        return;
    };
    let text = contents.borrow().to_string();
    if text.contains('\n') && text.trim().is_empty() {
        contents.replace(StrTendril::from("\n"));
    }
}

/// Drop whitespace-only text nodes at the document edges.
///
/// html5ever discards leading whitespace when it parses a fragment, so keeping
/// it would make the second run differ from the first byte for byte.
fn trim_document_edges(root: &Handle) {
    let mut children = root.children.borrow_mut();
    while children.first().is_some_and(is_blank_text) {
        children.remove(0);
    }
    while children.last().is_some_and(is_blank_text) {
        children.pop();
    }
}

fn is_blank_text(node: &Handle) -> bool {
    match &node.data {
        NodeData::Text { contents } => contents.borrow().trim().is_empty(),
        _ => false,
    }
}

fn normalize_document(root: &Handle) {
    for name in ["html", "body"] {
        loop {
            let position = root
                .children
                .borrow()
                .iter()
                .position(|child| selector::element_name(child).as_deref() == Some(name));
            let Some(position) = position else {
                break;
            };
            let node = root.children.borrow()[position].clone();
            let kids = take_children(&node);
            let mut children = root.children.borrow_mut();
            children.remove(position);
            for (offset, kid) in kids.into_iter().enumerate() {
                children.insert(position + offset, kid);
            }
        }
    }
    trim_document_edges(root);
}

/// Whether a rule emits a P0 slot class.
fn is_p0(rule: &Rule) -> bool {
    rule.emit
        .classes
        .iter()
        .any(|c| crate::adapters::P0_SLOTS.contains(&c.as_str()))
}

fn apply_rule(
    root: &Handle,
    rule: &Rule,
    ctx: &ParseContext,
    stats: &mut RunStats,
) -> Result<(), ProfileError> {
    let list = parse_selector(&rule.id, &rule.matches.selector)?;
    match rule.op {
        Op::Segment => apply_segment(root, rule, &list, ctx, stats),
        Op::Number => apply_number(root, rule, &list, stats),
        Op::Split => apply_split(root, rule, &list, ctx, stats),
        Op::Move => apply_move(root, rule, &list, stats),
        _ => {
            let mut matches = select(root, &list, &rule.matches.scope);
            matches.retain(|m| spec_matches(&rule.matches, m));
            check_cardinality(rule, matches.len(), stats);
            stats.hits.insert(rule.id.clone(), matches.len());
            match rule.op {
                Op::Map => apply_map(rule, &matches, ctx, stats),
                Op::Wrap => apply_wrap(rule, &matches, ctx),
                Op::Unwrap => apply_unwrap(&matches),
                Op::Rename => apply_rename(rule, &matches),
                Op::Drop => apply_drop(rule, &matches),
                Op::Attr => apply_attr(rule, &matches, ctx),
                Op::Redirect => apply_attr(rule, &matches, ctx),
                Op::Segment | Op::Number | Op::Split | Op::Move => {}
            }
            Ok(())
        }
    }
}

fn parse_selector(rule: &str, text: &str) -> Result<SelectorList, ProfileError> {
    SelectorList::parse(text).map_err(|error: SelectorError| ProfileError::Selector {
        rule: String::from(rule),
        error: error.to_string(),
    })
}

fn rule_error(rule: &Rule, message: &str) -> ProfileError {
    ProfileError::Rule {
        rule: rule.id.clone(),
        message: String::from(message),
    }
}

/// Whether the extra match predicates of a rule hold for one element.
fn spec_matches(spec: &MatchSpec, m: &Match) -> bool {
    if let Some(text) = &spec.text {
        if selector::text_content(&m.node) != selector::collapse_ws(text) {
            return false;
        }
    }
    for (name, value) in &spec.attr {
        if selector::attr(&m.node, name).as_deref() != Some(value.as_str()) {
            return false;
        }
    }
    true
}

fn check_cardinality(rule: &Rule, count: usize, stats: &mut RunStats) {
    let Some(card) = &rule.matches.cardinality else {
        return;
    };
    if count < card.min {
        stats.failures.push(format!(
            "{}: expected at least {} matches, found {}",
            rule.id, card.min, count
        ));
    }
    if let Some(max) = card.max {
        if count > max {
            stats.failures.push(format!(
                "{}: expected at most {} matches, found {}",
                rule.id, max, count
            ));
        }
    }
}

/// Select every element in document order, honouring the logical scope chain.
fn select(root: &Handle, list: &SelectorList, scope: &[Scope]) -> Vec<Match> {
    let mut out = Vec::new();
    let mut ancestors: Vec<Handle> = Vec::new();
    walk(root, &mut ancestors, list, scope, &mut out);
    out
}

fn walk(
    node: &Handle,
    ancestors: &mut Vec<Handle>,
    list: &SelectorList,
    scope: &[Scope],
    out: &mut Vec<Match>,
) {
    let children = node.children.borrow().clone();
    for (index, child) in children.iter().enumerate() {
        if !selector::is_element(child) {
            continue;
        }
        if crate::presentation::locked(child) {
            // Engine-owned nodes are invisible to rule packs (D3).
            continue;
        }
        let prev = children[..index]
            .iter()
            .rev()
            .find(|n| selector::is_element(n));
        if list.matches(child, ancestors, prev) && scope_ok(child, ancestors, scope) {
            out.push(Match {
                node: child.clone(),
                parent: node.clone(),
                index,
                ancestors: ancestors.clone(),
            });
        }
        ancestors.push(child.clone());
        walk(child, ancestors, list, scope, out);
        ancestors.pop();
    }
}

/// Whether the element sits inside every scope of the chain, outermost first.
fn has_class(node: &Handle, class: &str) -> bool {
    selector::class_list(node).iter().any(|c| c == class)
}

fn scope_ok(_node: &Handle, ancestors: &[Handle], scope: &[Scope]) -> bool {
    let mut need = scope.len();
    for ancestor in ancestors.iter().rev() {
        if need == 0 {
            break;
        }
        if has_class(ancestor, scope[need - 1].class()) {
            need -= 1;
        }
    }
    need == 0
}

/// Expand '@key', '@dict', '@lang' and '@text' placeholders.
fn substitute(value: &str, source: &Handle, ctx: &ParseContext) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let (token, advance) = split_token(rest);
        match token {
            "@key" => out.push_str(&ctx.key),
            "@dict" => out.push_str(&ctx.dict),
            "@lang" => out.push_str(&ctx.lang),
            "@text" => out.push_str(&selector::text_content(source)),
            _ => out.push_str(token),
        }
        rest = &rest[advance..];
    }
    out.push_str(rest);
    out
}

fn split_token(text: &str) -> (&str, usize) {
    let end = text
        .char_indices()
        .skip(1)
        .find(|(_, c)| !c.is_ascii_alphanumeric() && *c != '-')
        .map_or(text.len(), |(i, _)| i);
    (&text[..end], end)
}

/// Build the element a rule asks for.
fn build_element(
    rule: &Rule,
    source: &Handle,
    ctx: &ParseContext,
    extra: &[(String, String)],
) -> Handle {
    let name = rule
        .emit
        .element
        .clone()
        .or_else(|| selector::element_name(source))
        .unwrap_or_else(|| String::from("div"));
    let mut attrs: Vec<Attribute> = Vec::new();
    if !rule.emit.classes.is_empty() {
        attrs.push(make_attr("class", &rule.emit.classes.join(" ")));
    }
    for (key, value) in &rule.emit.data {
        let name = if key.starts_with("data-") {
            key.clone()
        } else {
            format!("data-{key}")
        };
        attrs.push(make_attr(&name, &substitute(value, source, ctx)));
    }
    for (key, value) in &rule.emit.attributes {
        attrs.push(make_attr(key, &substitute(value, source, ctx)));
    }
    for (key, value) in extra {
        attrs.push(make_attr(key, value));
    }
    // The engine stamps anchors before the rules run; a rename must keep them.
    if let NodeData::Element {
        attrs: source_attrs,
        ..
    } = &source.data
    {
        for attribute in source_attrs.borrow().iter() {
            if attribute.name.local.as_ref() == "data-odc-anchor" {
                attrs.push(attribute.clone());
            }
        }
    }
    attrs.push(make_attr("data-odc-rule", &rule.id));
    element(&name, attrs)
}

fn apply_map(rule: &Rule, matches: &[Match], ctx: &ParseContext, stats: &mut RunStats) {
    let mode = rule.emit.move_mode.unwrap_or(MoveMode::Children);
    for m in matches.iter().rev() {
        let new = build_element(rule, &m.node, ctx, &[]);
        match mode {
            MoveMode::Children => {
                *new.children.borrow_mut() = take_children(&m.node);
            }
            MoveMode::Element => {
                new.children.borrow_mut().push(m.node.clone());
            }
            MoveMode::None => {}
        }
        if mode != MoveMode::None {
            for child in &rule.children {
                let _ = apply_rule(&new, child, ctx, stats);
            }
        }
        replace_at(&m.parent, m.index, new);
    }
}

fn apply_wrap(rule: &Rule, matches: &[Match], ctx: &ParseContext) {
    for m in matches.iter().rev() {
        let new = build_element(rule, &m.node, ctx, &[]);
        new.children.borrow_mut().push(m.node.clone());
        replace_at(&m.parent, m.index, new);
    }
}

fn apply_unwrap(matches: &[Match]) {
    for m in matches.iter().rev() {
        let kids = take_children(&m.node);
        remove_at(&m.parent, m.index);
        let mut children = m.parent.children.borrow_mut();
        for (offset, kid) in kids.into_iter().enumerate() {
            children.insert(m.index + offset, kid);
        }
    }
}

fn apply_rename(rule: &Rule, matches: &[Match]) {
    for m in matches.iter().rev() {
        let Some(name) = rule.emit.element.clone() else {
            continue;
        };
        let NodeData::Element { attrs, .. } = &m.node.data else {
            continue;
        };
        let attrs = attrs.borrow().clone();
        let new = element(&name, attrs);
        if !rule.emit.classes.is_empty() {
            set_attr(&new, "class", &rule.emit.classes.join(" "));
        }
        set_attr(&new, "data-odc-rule", &rule.id);
        *new.children.borrow_mut() = take_children(&m.node);
        replace_at(&m.parent, m.index, new);
    }
}

fn apply_drop(rule: &Rule, matches: &[Match]) {
    if rule.attributes.is_empty() {
        for m in matches.iter().rev() {
            remove_at(&m.parent, m.index);
        }
        return;
    }
    for m in matches.iter().rev() {
        for pattern in &rule.attributes {
            if let Some(prefix) = pattern.strip_suffix('*') {
                let NodeData::Element { attrs, .. } = &m.node.data else {
                    continue;
                };
                let stale: Vec<String> = attrs
                    .borrow()
                    .iter()
                    .map(|a| a.name.local.to_string())
                    .filter(|name| name.starts_with(prefix))
                    .collect();
                for name in stale {
                    remove_attr(&m.node, &name);
                }
            } else {
                remove_attr(&m.node, pattern);
            }
        }
    }
}

fn apply_attr(rule: &Rule, matches: &[Match], ctx: &ParseContext) {
    for m in matches.iter().rev() {
        for (key, value) in &rule.emit.data {
            let name = if key.starts_with("data-") {
                key.clone()
            } else {
                format!("data-{key}")
            };
            set_attr(&m.node, &name, &substitute(value, &m.node, ctx));
        }
        for (key, value) in &rule.emit.attributes {
            if let Some(stripped) = key.strip_prefix('-') {
                remove_attr(&m.node, stripped);
            } else {
                set_attr(&m.node, key, &substitute(value, &m.node, ctx));
            }
        }
        if let Some(value) = &rule.emit.value {
            if let Some(key) = &rule.emit.attribute {
                set_attr(&m.node, key, &substitute(value, &m.node, ctx));
            }
        }
        if !rule.emit.classes.is_empty() {
            let mut classes = selector::class_list(&m.node);
            for class in &rule.emit.classes {
                if !classes.contains(class) {
                    classes.push(class.clone());
                }
            }
            set_attr(&m.node, "class", &classes.join(" "));
        }
        set_attr(&m.node, "data-odc-rule", &rule.id);
    }
}

fn apply_number(
    root: &Handle,
    rule: &Rule,
    list: &SelectorList,
    stats: &mut RunStats,
) -> Result<(), ProfileError> {
    let mut matches = select(root, list, &rule.matches.scope);
    matches.retain(|m| spec_matches(&rule.matches, m));
    check_cardinality(rule, matches.len(), stats);
    stats.hits.insert(rule.id.clone(), matches.len());

    let attribute = rule
        .emit
        .attribute
        .clone()
        .unwrap_or_else(|| String::from("data-odc-n"));
    let reset = if rule.reset_on.is_empty() {
        vec![Scope::PosBlock]
    } else {
        rule.reset_on.clone()
    };
    let start = rule
        .emit
        .numbering
        .as_ref()
        .and_then(|n| n.start)
        .unwrap_or(1);
    let mut counters: BTreeMap<usize, i64> = BTreeMap::new();
    for m in &matches {
        let key = scope_instance(m, &reset);
        let counter = counters.entry(key).or_insert(start);
        set_attr(&m.node, &attribute, &counter.to_string());
        set_attr(&m.node, "data-odc-rule", &rule.id);
        *counter += 1;
    }
    Ok(())
}

/// Pointer identity of the innermost reset scope that contains 'm'.
fn scope_instance(m: &Match, reset: &[Scope]) -> usize {
    let Some(scope) = reset.last() else {
        return 0;
    };
    if has_class(&m.node, scope.class()) {
        return Rc::as_ptr(&m.node) as usize;
    }
    for ancestor in m.ancestors.iter().rev() {
        if has_class(ancestor, scope.class()) {
            return Rc::as_ptr(ancestor) as usize;
        }
    }
    0
}

/// Copy every attribute of 'source' except class and the rule marker, so a
/// split copy keeps the entry identity (dictionary, key, language).
fn inherit_attrs(source: &Handle) -> Vec<(String, String)> {
    let NodeData::Element { attrs, .. } = &source.data else {
        return Vec::new();
    };
    attrs
        .borrow()
        .iter()
        .filter(|a| {
            let name = a.name.local.as_ref();
            !name.eq_ignore_ascii_case("class") && !name.eq_ignore_ascii_case("data-odc-rule")
        })
        .map(|a| (a.name.local.to_string(), a.value.to_string()))
        .collect()
}

fn apply_split(
    root: &Handle,
    rule: &Rule,
    list: &SelectorList,
    ctx: &ParseContext,
    stats: &mut RunStats,
) -> Result<(), ProfileError> {
    let split: &SplitBy = rule
        .split_by
        .as_ref()
        .ok_or_else(|| rule_error(rule, "split requires split_by"))?;
    let sub = parse_selector(&rule.id, &split.selector)?;
    let min = rule.matches.min_count.unwrap_or(2);
    let attribute = rule
        .emit
        .attribute
        .clone()
        .unwrap_or_else(|| String::from("data-odc-index"));
    let start = rule
        .emit
        .numbering
        .as_ref()
        .and_then(|n| n.start)
        .unwrap_or(1);

    let mut matches = select(root, list, &rule.matches.scope);
    matches.retain(|m| spec_matches(&rule.matches, m));
    stats.hits.insert(rule.id.clone(), matches.len());

    for m in matches.iter().rev() {
        let kids = m.node.children.borrow().clone();
        let mut chain = m.ancestors.clone();
        chain.push(m.node.clone());
        let targets: Vec<usize> = kids
            .iter()
            .enumerate()
            .filter(|(index, kid)| {
                let prev = kids[..*index]
                    .iter()
                    .rev()
                    .find(|n| selector::is_element(n));
                selector::is_element(kid) && sub.matches(kid, &chain, prev)
            })
            .map(|(index, _)| index)
            .collect();
        if targets.len() < min {
            continue;
        }
        let mut copies = Vec::new();
        for (offset, index) in targets.iter().enumerate() {
            let number = (start + offset as i64).to_string();
            let mut extra = inherit_attrs(&m.node);
            extra.push((attribute.clone(), number));
            if rule.emit.classes.is_empty() {
                let inherited = selector::class_list(&m.node).join(" ");
                if !inherited.is_empty() {
                    extra.push((String::from("class"), inherited));
                }
            }
            let new = build_element(rule, &kids[*index], ctx, &extra);
            if split.keep_selector {
                new.children.borrow_mut().push(kids[*index].clone());
            } else {
                *new.children.borrow_mut() = take_children(&kids[*index]);
            }
            copies.push(new);
        }
        // Empty the container before dropping it: rcdom's Drop walks the whole
        // subtree and takes the children vecs, which would clear the very nodes
        // the copies just adopted.
        let _ = take_children(&m.node);
        let mut children = m.parent.children.borrow_mut();
        if m.index < children.len() {
            children.remove(m.index);
        }
        for (offset, copy) in copies.into_iter().enumerate() {
            children.insert(m.index + offset, copy);
        }
    }
    Ok(())
}

fn apply_segment(
    root: &Handle,
    rule: &Rule,
    list: &SelectorList,
    ctx: &ParseContext,
    stats: &mut RunStats,
) -> Result<(), ProfileError> {
    let boundary: &Boundary = rule
        .boundary
        .as_ref()
        .ok_or_else(|| rule_error(rule, "segment requires boundary"))?;
    let start_sel = parse_selector(&rule.id, &boundary.start)?;
    let until: Vec<SelectorList> = boundary
        .until
        .iter()
        .map(|text| parse_selector(&rule.id, text))
        .collect::<Result<_, _>>()?;
    let max_depth = boundary.max_depth.unwrap_or(1);

    let mut matches = select(root, list, &rule.matches.scope);
    matches.retain(|m| spec_matches(&rule.matches, m));
    stats.hits.insert(rule.id.clone(), matches.len());

    for m in matches.iter().rev() {
        let mut starts = Vec::new();
        collect_starts(
            &m.node,
            &m.scope_chain(),
            1,
            max_depth,
            &start_sel,
            &mut starts,
        );
        for start in starts.into_iter().rev() {
            if !Rc::ptr_eq(&start.parent, &m.node) {
                stats.warnings.push(format!(
                    "{}: boundary start below max_depth, skipped",
                    rule.id
                ));
                continue;
            }
            let kids = m.node.children.borrow().clone();
            let mut end = start.index + 1;
            while end < kids.len() {
                let prev = Some(&kids[end - 1]);
                let stop = until
                    .iter()
                    .any(|sel| sel.matches(&kids[end], &m.scope_chain(), prev));
                if stop {
                    break;
                }
                end += 1;
            }
            let new = build_element(rule, &kids[start.index], ctx, &[]);
            let mut group = Vec::new();
            {
                let mut children = m.node.children.borrow_mut();
                for _ in start.index..end {
                    group.push(children.remove(start.index));
                }
                children.insert(start.index, new.clone());
            }
            *new.children.borrow_mut() = group;
        }
    }
    Ok(())
}

/// Collect boundary starts up to 'max_depth' levels below 'container'.
fn collect_starts(
    container: &Handle,
    chain: &[Handle],
    depth: usize,
    max_depth: usize,
    start_sel: &SelectorList,
    out: &mut Vec<Match>,
) {
    if depth > max_depth {
        return;
    }
    let kids = container.children.borrow().clone();
    for (index, kid) in kids.iter().enumerate() {
        if !selector::is_element(kid) {
            continue;
        }
        let prev = kids[..index].iter().rev().find(|n| selector::is_element(n));
        if start_sel.matches(kid, chain, prev) {
            out.push(Match {
                node: kid.clone(),
                parent: container.clone(),
                index,
                ancestors: chain.to_vec(),
            });
        }
        let mut inner = chain.to_vec();
        inner.push(kid.clone());
        collect_starts(kid, &inner, depth + 1, max_depth, start_sel, out);
    }
}

fn apply_move(
    root: &Handle,
    rule: &Rule,
    list: &SelectorList,
    stats: &mut RunStats,
) -> Result<(), ProfileError> {
    let target = rule
        .emit
        .target
        .as_ref()
        .ok_or_else(|| rule_error(rule, "move requires emit.target"))?;
    let target_sel = parse_selector(&rule.id, target)?;
    let mode = rule.emit.move_mode.unwrap_or(MoveMode::Element);

    let mut matches = select(root, list, &rule.matches.scope);
    matches.retain(|m| spec_matches(&rule.matches, m));
    stats.hits.insert(rule.id.clone(), matches.len());

    for m in matches.iter().rev() {
        let anchor = m.ancestors.last().cloned().unwrap_or_else(|| root.clone());
        let destination = select(&anchor, &target_sel, &[])
            .into_iter()
            .find(|candidate| !Rc::ptr_eq(&candidate.node, &m.node))
            .map(|candidate| candidate.node);
        let Some(destination) = destination else {
            stats
                .warnings
                .push(format!("{}: move target not found", rule.id));
            continue;
        };
        let moved = match mode {
            MoveMode::Children => take_children(&m.node),
            MoveMode::Element => vec![m.node.clone()],
            MoveMode::None => Vec::new(),
        };
        for node in moved {
            destination.children.borrow_mut().push(node);
        }
        let _ = take_children(&m.node);
        remove_at(&m.parent, m.index);
    }
    Ok(())
}
