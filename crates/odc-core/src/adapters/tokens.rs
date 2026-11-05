//! Minimal HTML tokeniser shared by the adapters and the slot counter.
//!
//! The input is always markup that already went through
//! [`crate::html::repair_sanitize`], so the tokeniser can stay small: no
//! entity decoding, and no raw-text element handling beyond `script` /
//! `style` (which the sanitiser removes anyway).

/// One lexical unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Tok {
    /// A start tag.
    Start {
        /// Lower-cased tag name.
        name: String,
        /// Attributes in source order; values keep their source escaping.
        attrs: Vec<(String, String)>,
        /// Whether the tag closed itself (`<img/>`, `<br>`).
        void: bool,
    },
    /// An end tag.
    End {
        /// Lower-cased tag name.
        name: String,
    },
    /// A run of character data, copied verbatim.
    Text(String),
}

/// Tags that never take an end tag.
const VOID: [&str; 9] = [
    "area", "br", "col", "hr", "img", "input", "link", "meta", "source",
];

/// Tokenise `input`; unterminated constructs are dropped.
pub(crate) fn tokenize(input: &str) -> Vec<Tok> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if i > text_start {
            out.push(Tok::Text(String::from(&input[text_start..i])));
        }
        if !input[i..].starts_with("</") {
            if input[i..].starts_with("<!--") {
                i = input[i..].find("-->").map_or(bytes.len(), |p| i + p + 3);
                text_start = i;
                continue;
            }
            if input[i..].starts_with("<!") {
                i = find_tag_end(bytes, i).map_or(bytes.len(), |p| p + 1);
                text_start = i;
                continue;
            }
        }
        let Some(end) = find_tag_end(bytes, i) else {
            break;
        };
        let inner = &input[i + 1..end];
        i = end + 1;
        text_start = i;
        if let Some(rest) = inner.strip_prefix('/') {
            out.push(Tok::End {
                name: rest.trim().to_ascii_lowercase(),
            });
            continue;
        }
        let (name, attrs) = parse_tag(inner);
        if name.is_empty() {
            continue;
        }
        let void = VOID.contains(&name.as_str()) || inner.trim_end().ends_with('/');
        out.push(Tok::Start { name, attrs, void });
    }
    if text_start < bytes.len() {
        out.push(Tok::Text(String::from(&input[text_start..])));
    }
    out
}

/// Byte offset of the `>` that closes the tag starting at `start`.
fn find_tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut quote = 0u8;
    let mut i = start + 1;
    while i < bytes.len() {
        let b = bytes[i];
        if quote != 0 {
            if b == quote {
                quote = 0;
            }
        } else if b == b'"' || b == b'\'' {
            quote = b;
        } else if b == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Split `name attr="value" ...` into a name and its attributes.
fn parse_tag(inner: &str) -> (String, Vec<(String, String)>) {
    let mut name = String::new();
    for ch in inner.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == ':' || ch == '_' {
            name.push(ch.to_ascii_lowercase());
        } else {
            break;
        }
    }
    let mut attrs = Vec::new();
    let rest = &inner[name.len()..];
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len()
            && (bytes[i] == b' '
                || bytes[i] == b'\t'
                || bytes[i] == b'\n'
                || bytes[i] == b'\r'
                || bytes[i] == b'/')
        {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key_start = i;
        while i < bytes.len()
            && bytes[i] != b'='
            && bytes[i] != b' '
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let key = rest[key_start..i].trim().to_ascii_lowercase();
        let mut value = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                value = String::from(&rest[start..i.min(rest.len())]);
                i += usize::from(i < bytes.len());
            } else {
                let start = i;
                while i < bytes.len() && bytes[i] != b' ' && bytes[i] != b'>' {
                    i += 1;
                }
                value = String::from(&rest[start..i]);
            }
        }
        if !key.is_empty() {
            attrs.push((key, value));
        }
    }
    (name, attrs)
}

/// Value of the first attribute called `name`.
pub(crate) fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Whether the `class` attribute contains `class` as a whole token.
pub(crate) fn has_class(attrs: &[(String, String)], class: &str) -> bool {
    attr(attrs, "class").is_some_and(|value| value.split_whitespace().any(|token| token == class))
}
