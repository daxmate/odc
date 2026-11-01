//! Stylesheets an entry links to.
//!
//! MDict dictionaries routinely keep their CSS *next to* the ~BT~.mdx~BT~ instead of
//! packing it into the ~BT~.mdd~BT~ (LDOCE6++ ships ~BT~ldoce6ec.css~BT~ as a sibling file).
//! The importer reads these keys so it can copy the files the volumes do not
//! carry, and the reader inlines them instead of waiting on a subresource load.

use crate::mdd_import::normalize_resource_key;

/// Keys of the stylesheets ~BT~html~BT~ links to, in document order.
///
/// Only relative references are returned: a sheet on the web cannot be read
/// out of the repository, so it is left to WebKit.
#[must_use]
pub fn link_targets(html: &str) -> Vec<String> {
    let lowered = html.to_ascii_lowercase();
    let mut keys = Vec::new();
    let mut cursor = 0usize;
    while let Some(offset) = lowered[cursor..].find("<link") {
        let start = cursor + offset;
        let Some(end) = lowered[start..].find('>').map(|end| start + end) else {
            break;
        };
        let tag = &html[start..=end];
        cursor = end + 1;
        if !is_stylesheet(tag) {
            continue;
        }
        let Some(href) = attribute(tag, "href") else {
            continue;
        };
        let key = normalize_resource_key(href);
        if key.is_empty() || keys.contains(&key) || is_absolute(&key) {
            continue;
        }
        keys.push(key);
    }
    keys
}

/// True when the tag's ~BT~rel~BT~ says it is a stylesheet.
fn is_stylesheet(tag: &str) -> bool {
    let Some(rel) = attribute(tag, "rel") else {
        return false;
    };
    rel.split_ascii_whitespace()
        .any(|token| token.eq_ignore_ascii_case("stylesheet"))
}

/// Value of one attribute in ~BT~tag~BT~, quotes stripped.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lowered = tag.to_ascii_lowercase();
    let mut cursor = 0usize;
    while let Some(offset) = lowered[cursor..].find(name) {
        let start = cursor + offset;
        let after = start + name.len();
        // The name must be an attribute of its own, not the tail of another.
        let boundary_before = start == 0
            || tag[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || c == '<');
        let rest = tag[after..].trim_start();
        if !boundary_before || !rest.starts_with('=') {
            cursor = after;
            continue;
        }
        let value = rest[1..].trim_start();
        let end = value
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(value.len());
        let raw = value[..end].trim_matches(|c| c == '"' || c == '\'');
        return Some(raw);
    }
    None
}

/// True when the key names something the repository cannot serve.
fn is_absolute(key: &str) -> bool {
    key.contains("://") || key.starts_with("data:") || key.contains("..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_relative_stylesheet_links_in_order() {
        let html =
            r#"<link rel="stylesheet" href="one.css"><LINK REL="StyleSheet" HREF="./two.css">"#;
        assert_eq!(link_targets(html), vec!["one.css", "two.css"]);
    }

    #[test]
    fn ignores_other_links_and_absolute_targets() {
        let html = r#"<link rel="icon" href="favicon.ico">
            <link rel="stylesheet" href="https://cdn.example/style.css">
            <link rel="stylesheet" href="../../escape.css">"#;
        assert!(link_targets(html).is_empty());
    }

    #[test]
    fn tolerates_attributes_without_quotes() {
        let html = "<link rel=stylesheet href=plain.css>";
        assert_eq!(link_targets(html), vec!["plain.css"]);
    }
}
