//! The CSS selector subset profiles are allowed to use.
//!
//! Supported syntax, and nothing else:
//!
//! * tag names and '*',
//! * '.class', '#id',
//! * '[attr]' and '[attr=value]' (quoted or bare value),
//! * the descendant (space), child ('>') and adjacent sibling ('+')
//!   combinators,
//! * comma-separated lists.
//!
//! XPath, pseudo-classes, attribute operators other than equality and any
//! scriptable extension are rejected at parse time. Matching runs directly on
//! the 'markup5ever_rcdom' tree the rest of the crate uses, so there is no
//! second HTML implementation.

use markup5ever_rcdom::{Handle, NodeData};

/// One compound selector, e.g. 'div.sense.note'.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Compound {
    tag: Option<String>,
    ids: Vec<String>,
    classes: Vec<String>,
    attrs: Vec<(String, Option<String>)>,
}

/// How two compound selectors are joined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Combinator {
    Descendant,
    Child,
    Adjacent,
}

/// A selector with combinators, e.g. 'div.sblocks > div.sense'.
#[derive(Clone, Debug)]
struct Complex {
    /// Combinator that precedes each compound; the first entry is unused.
    parts: Vec<(Combinator, Compound)>,
}

/// A parsed selector list.
#[derive(Clone, Debug)]
pub struct SelectorList {
    parts: Vec<Complex>,
}

/// Why a selector could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectorError {
    /// The offending selector text.
    pub selector: String,
    /// Human-readable reason.
    pub reason: &'static str,
}

impl std::fmt::Display for SelectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "selector '{}': {}", self.selector, self.reason)
    }
}

impl SelectorList {
    /// Parse a selector list.
    ///
    /// # Errors
    ///
    /// Returns ['SelectorError'] for empty selectors, unbalanced brackets and
    /// constructs outside the subset.
    pub fn parse(input: &str) -> Result<Self, SelectorError> {
        let mut parts = Vec::new();
        for chunk in input.split(',') {
            let chunk = chunk.trim();
            if chunk.is_empty() {
                return Err(err(input, "empty selector"));
            }
            parts.push(parse_complex(chunk).map_err(|reason| err(input, reason))?);
        }
        Ok(Self { parts })
    }

    /// Whether any selector in the list matches 'node'.
    ///
    /// 'ancestors' runs from the fragment root to the parent, 'prev' is the
    /// nearest preceding element sibling.
    #[must_use]
    pub fn matches(&self, node: &Handle, ancestors: &[Handle], prev: Option<&Handle>) -> bool {
        self.parts.iter().any(|c| c.matches(node, ancestors, prev))
    }
}

fn err(selector: &str, reason: &'static str) -> SelectorError {
    SelectorError {
        selector: String::from(selector),
        reason,
    }
}

impl Complex {
    fn matches(&self, node: &Handle, ancestors: &[Handle], prev: Option<&Handle>) -> bool {
        self.match_at(self.parts.len() - 1, node, ancestors, prev)
    }

    fn match_at(
        &self,
        idx: usize,
        node: &Handle,
        ancestors: &[Handle],
        prev: Option<&Handle>,
    ) -> bool {
        if !self.parts[idx].1.matches(node) {
            return false;
        }
        if idx == 0 {
            return true;
        }
        match self.parts[idx].0 {
            Combinator::Child => ancestors.last().is_some_and(|parent| {
                self.match_at(idx - 1, parent, &ancestors[..ancestors.len() - 1], None)
            }),
            Combinator::Adjacent => {
                prev.is_some_and(|sibling| self.match_at(idx - 1, sibling, ancestors, None))
            }
            Combinator::Descendant => (0..ancestors.len())
                .rev()
                .any(|i| self.match_at(idx - 1, &ancestors[i], &ancestors[..i], None)),
        }
    }
}

impl Compound {
    fn matches(&self, node: &Handle) -> bool {
        if !is_element(node) {
            return false;
        }
        if let Some(tag) = &self.tag {
            if tag != "*" {
                let Some(name) = element_name(node) else {
                    return false;
                };
                if !name.eq_ignore_ascii_case(tag) {
                    return false;
                }
            }
        }
        if !self.ids.is_empty() {
            let Some(id) = attr(node, "id") else {
                return false;
            };
            if !self.ids.iter().all(|want| want == &id) {
                return false;
            }
        }
        if !self.classes.is_empty() {
            let have = class_list(node);
            if !self
                .classes
                .iter()
                .all(|want| have.iter().any(|c| c == want))
            {
                return false;
            }
        }
        for (name, value) in &self.attrs {
            let Some(actual) = attr(node, name) else {
                return false;
            };
            if let Some(want) = value {
                if &actual != want {
                    return false;
                }
            }
        }
        true
    }
}

fn parse_complex(text: &str) -> Result<Complex, &'static str> {
    let bytes = text.as_bytes();
    let mut segments: Vec<(Combinator, &str)> = Vec::new();
    let mut comb = Combinator::Descendant;
    let mut start = 0; // byte offset into 'text', always on a char boundary
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => depth += 1,
            b']' => depth = depth.saturating_sub(1),
            b'>' | b'+' if depth == 0 => {
                push_segment(&mut segments, comb, &text[start..i])?;
                comb = if bytes[i] == b'>' {
                    Combinator::Child
                } else {
                    Combinator::Adjacent
                };
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    push_segment(&mut segments, comb, &text[start..])?;

    let parts: Vec<(Combinator, Compound)> = segments
        .iter()
        .map(|(comb, text)| parse_compound(text).map(|c| (*comb, c)))
        .collect::<Result<_, _>>()?;
    if parts.is_empty() {
        return Err("empty selector");
    }
    Ok(Complex { parts })
}

fn push_segment<'a>(
    out: &mut Vec<(Combinator, &'a str)>,
    comb: Combinator,
    text: &'a str,
) -> Result<(), &'static str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("dangling combinator");
    }
    out.push((comb, trimmed));
    Ok(())
}

fn parse_compound(text: &str) -> Result<Compound, &'static str> {
    let bytes = text.as_bytes();
    let mut compound = Compound::default();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'*' => {
                compound.tag = Some(String::from("*"));
                i += 1;
            }
            b'.' => {
                let (name, next) = ident(text, i + 1)?;
                compound.classes.push(name);
                i = next;
            }
            b'#' => {
                let (name, next) = ident(text, i + 1)?;
                compound.ids.push(name);
                i = next;
            }
            b'[' => {
                let close = text[i..].find(']').ok_or("unclosed attribute selector")? + i;
                let (name, value) = parse_attribute(&text[i + 1..close])?;
                compound.attrs.push((name, value));
                i = close + 1;
            }
            b if is_ident_byte(b) => {
                let (name, next) = ident(text, i)?;
                compound.tag = Some(name.to_ascii_lowercase());
                i = next;
            }
            _ => return Err("unsupported selector syntax"),
        }
    }
    if compound == Compound::default() {
        return Err("empty compound selector");
    }
    Ok(compound)
}

fn parse_attribute(inner: &str) -> Result<(String, Option<String>), &'static str> {
    let inner = inner.trim();
    if inner.is_empty() {
        return Err("empty attribute selector");
    }
    let Some((name, raw)) = inner.split_once('=') else {
        if !inner.bytes().all(is_ident_byte) {
            return Err("unsupported attribute name");
        }
        return Ok((inner.to_ascii_lowercase(), None));
    };
    let name = name.trim();
    if name.is_empty() || !name.bytes().all(is_ident_byte) {
        return Err("unsupported attribute name");
    }
    let value = raw.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value);
    if value.contains(['"', '\'']) {
        return Err("unsupported attribute value");
    }
    Ok((name.to_ascii_lowercase(), Some(String::from(value))))
}

fn ident(text: &str, start: usize) -> Result<(String, usize), &'static str> {
    let bytes = text.as_bytes();
    let mut end = start;
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    if end == start {
        return Err("expected a name after '.' or '#'");
    }
    Ok((String::from(&text[start..end]), end))
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':'
}

/// Whether 'node' is an element (not text, comment or the document).
#[must_use]
pub fn is_element(node: &Handle) -> bool {
    matches!(node.data, NodeData::Element { .. })
}

/// Lower-case tag name of an element node.
#[must_use]
pub fn element_name(node: &Handle) -> Option<String> {
    match &node.data {
        NodeData::Element { name, .. } => Some(name.local.to_string()),
        _ => None,
    }
}

/// Value of one attribute, if present.
#[must_use]
pub fn attr(node: &Handle, name: &str) -> Option<String> {
    let NodeData::Element { attrs, .. } = &node.data else {
        return None;
    };
    attrs
        .borrow()
        .iter()
        .find(|a| a.name.local.as_ref().eq_ignore_ascii_case(name))
        .map(|a| a.value.to_string())
}

/// Class list of an element node.
#[must_use]
pub fn class_list(node: &Handle) -> Vec<String> {
    attr(node, "class")
        .map(|value| value.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

/// Whitespace-collapsed text of a subtree, in document order.
#[must_use]
pub fn text_content(node: &Handle) -> String {
    let mut out = String::new();
    collect_text(node, &mut out);
    collapse_ws(&out)
}

fn collect_text(node: &Handle, out: &mut String) {
    if let NodeData::Text { contents } = &node.data {
        out.push_str(&contents.borrow());
    }
    for child in node.children.borrow().iter() {
        collect_text(child, out);
    }
}

/// Collapse every whitespace run to one space and trim the ends.
#[must_use]
pub fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
