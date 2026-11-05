//! Block-level HTML formatter for MDX entry bodies.
//!
//! The importer repairs and sanitises every body with `html5ever` and
//! `lightningcss`, then lays the serialised markup out with this module.
//! Unlike a Prettier-style formatter, the layout policy never rewraps text:
//!
//! * line breaks and indentation are only introduced at block-level element
//!   boundaries (see `BLOCK_TAGS`);
//! * tags are never split across lines and text nodes are copied byte for
//!   byte: no wrapping, no whitespace collapsing;
//! * void elements, comments, doctypes, CDATA sections and raw-text elements
//!   (`pre`, `script`, `style`, `textarea`) are emitted verbatim;
//! * quotes inside attribute values are honoured, so `title="a > b"` does
//!   not terminate the tag early.
//!
//! The input is expected to be serialized HTML (escaped `&lt;` in text,
//! balanced tags). Malformed markup is reported as an error instead of being
//! guessed at; callers keep the repaired body unchanged in that case.

use std::fmt;

use crate::{Error, Result};

/// One indentation step per nesting level.
const INDENT: &str = "  ";

/// Elements that may start or end a line.
const BLOCK_TAGS: &[&str] = &[
    "blockquote",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "tbody",
    "td",
    "th",
    "thead",
    "tr",
    "ul",
];

/// Elements whose content is never parsed or re-indented.
const RAW_TEXT_TAGS: &[&str] = &["pre", "script", "style", "textarea"];

/// Elements without an end tag.
const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Format `html` so that only block-level boundaries produce line breaks.
///
/// # Errors
///
/// Returns [`Error::InvalidArgument`] when the markup cannot be tokenised
/// (an unterminated tag, comment, CDATA section or raw-text element). The
/// caller is expected to keep the unformatted body and surface the message as
/// a `format.failed` diagnostic.
pub fn format_blocks(html: &str) -> Result<String> {
    let tokens =
        tokenize(html).map_err(|malformed| Error::InvalidArgument(format!("html: {malformed}")))?;
    let mut writer = Writer::default();
    for (index, token) in tokens.iter().enumerate() {
        let previous_boundary = index
            .checked_sub(1)
            .and_then(|before| tokens.get(before))
            .is_some_and(|kind| kind.is_block_boundary());
        let next_boundary = tokens
            .get(index + 1)
            .is_some_and(|kind| kind.is_block_boundary());
        writer.emit(token, previous_boundary, next_boundary);
    }
    Ok(writer.finish())
}

/// A construct that could not be tokenised.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Malformed {
    /// Byte offset of the construct's opening character.
    offset: usize,
    /// What was left unterminated.
    construct: &'static str,
}

impl fmt::Display for Malformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unterminated {} at byte {}", self.construct, self.offset)
    }
}

/// One lexical unit of the input.
#[derive(Debug, Clone, Copy)]
enum Kind<'a> {
    Text(&'a str),
    StartTag {
        name: &'a str,
        raw: &'a str,
        closed: bool,
    },
    EndTag {
        name: &'a str,
        raw: &'a str,
    },
    RawElement {
        name: &'a str,
        raw: &'a str,
    },
    Opaque(&'a str),
}

impl Kind<'_> {
    /// Whether this token can carry a line break in front of it.
    fn is_block_boundary(self) -> bool {
        match self {
            Self::StartTag { name, closed, .. } => !closed && is_block(name),
            Self::EndTag { name, .. } => is_block(name),
            Self::RawElement { name, .. } => is_block(name),
            Self::Text(_) | Self::Opaque(_) => false,
        }
    }
}

fn is_block(name: &str) -> bool {
    BLOCK_TAGS.iter().any(|tag| tag.eq_ignore_ascii_case(name))
}

fn is_void(name: &str) -> bool {
    VOID_TAGS.iter().any(|tag| tag.eq_ignore_ascii_case(name))
}

fn is_raw(name: &str) -> bool {
    RAW_TEXT_TAGS
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case(name))
}

/// Split `html` into tags, text and opaque runs.
fn tokenize(html: &str) -> std::result::Result<Vec<Kind<'_>>, Malformed> {
    let bytes = html.as_bytes();
    let mut tokens = Vec::new();
    let mut position = 0;
    let mut text_start = 0;
    while position < bytes.len() {
        if bytes[position] != b'<' {
            position += 1;
            continue;
        }
        if text_start < position {
            tokens.push(Kind::Text(&html[text_start..position]));
        }
        let (token, end) = scan_token(html, position)?;
        tokens.push(token);
        position = end;
        text_start = end;
    }
    if text_start < html.len() {
        tokens.push(Kind::Text(&html[text_start..]));
    }
    Ok(tokens)
}

/// Scan the markup starting at `start` (which must be a `<`).
fn scan_token(html: &str, start: usize) -> std::result::Result<(Kind<'_>, usize), Malformed> {
    let rest = &html[start..];
    let remaining = html.len() - start;

    for (opener, closer) in [("<!--", "-->"), ("<![CDATA[", "]]>")] {
        if rest.starts_with(opener) {
            let end = rest
                .find(closer)
                .map_or(remaining, |offset| offset + closer.len());
            if !rest[..end].ends_with(closer) {
                return Err(Malformed {
                    offset: start,
                    construct: if opener == "<!--" {
                        "comment"
                    } else {
                        "CDATA section"
                    },
                });
            }
            return Ok((Kind::Opaque(&rest[..end]), start + end));
        }
    }
    if rest.starts_with("<!") || rest.starts_with("<?") {
        let end = rest.find('>').map_or(remaining, |offset| offset + 1);
        if !rest[..end].ends_with('>') {
            return Err(Malformed {
                offset: start,
                construct: "declaration",
            });
        }
        return Ok((Kind::Opaque(&rest[..end]), start + end));
    }
    if rest.starts_with("</") {
        let Some(end) = rest.find('>').map(|offset| offset + 1) else {
            return Err(Malformed {
                offset: start,
                construct: "end tag",
            });
        };
        let raw = &rest[..end];
        return Ok((
            Kind::EndTag {
                name: tag_name(raw),
                raw,
            },
            start + end,
        ));
    }

    let tag_end = find_tag_end(rest).ok_or(Malformed {
        offset: start,
        construct: "start tag",
    })?;
    let raw = &rest[..tag_end];
    let name = tag_name(raw);
    let closed = raw.ends_with("/>") || is_void(name);
    if is_raw(name) && !closed {
        let end = find_raw_end(html, start + tag_end, name).ok_or(Malformed {
            offset: start,
            construct: "raw-text element",
        })?;
        return Ok((
            Kind::RawElement {
                name,
                raw: &html[start..end],
            },
            end,
        ));
    }
    Ok((Kind::StartTag { name, raw, closed }, start + tag_end))
}

/// Index just past the `>` that closes a start tag, honouring quoted
/// attribute values so that `title="a > b"` is not cut short.
fn find_tag_end(rest: &str) -> Option<usize> {
    let mut quote = None;
    for (index, byte) in rest.bytes().enumerate().skip(1) {
        match quote {
            Some(open) => {
                if byte == open {
                    quote = None;
                }
            }
            None => match byte {
                b'"' | b'\'' => quote = Some(byte),
                b'>' => return Some(index + 1),
                _ => {}
            },
        }
    }
    None
}

/// Extract the element name from `<name ...>` or `</name>`.
fn tag_name(raw: &str) -> &str {
    let inner = raw
        .strip_prefix("</")
        .or_else(|| raw.strip_prefix('<'))
        .unwrap_or(raw);
    inner
        .split(|character: char| character.is_whitespace() || character == '>' || character == '/')
        .next()
        .unwrap_or("")
}

/// End offset of a raw-text element, closing tag included.
fn find_raw_end(html: &str, from: usize, name: &str) -> Option<usize> {
    let needle = format!("</{name}").to_ascii_lowercase();
    let haystack = html[from..].to_ascii_lowercase();
    let offset = haystack.find(&needle)?;
    let close_start = from + offset;
    html[close_start..]
        .find('>')
        .map(|end| close_start + end + 1)
}

/// Line-oriented output buffer.
#[derive(Default)]
struct Writer {
    out: String,
    depth: usize,
    line_has_content: bool,
    last_block_boundary: bool,
}

impl Writer {
    fn emit(&mut self, token: &Kind<'_>, previous_boundary: bool, next_boundary: bool) {
        match *token {
            Kind::Text(text) => {
                if text.trim().is_empty() && previous_boundary && next_boundary {
                    self.line_break();
                    self.last_block_boundary = true;
                    return;
                }
                self.write(text);
                self.last_block_boundary = false;
            }
            Kind::StartTag { name, raw, closed } => {
                if is_block(name) {
                    self.line_break();
                }
                self.write(raw);
                if !closed && is_block(name) {
                    self.depth += 1;
                }
                self.last_block_boundary = !closed && is_block(name);
            }
            Kind::EndTag { name, raw } => {
                if is_block(name) {
                    self.depth = self.depth.saturating_sub(1);
                    if self.last_block_boundary {
                        self.line_break();
                    }
                }
                self.write(raw);
                self.last_block_boundary = is_block(name);
            }
            Kind::RawElement { name, raw } => {
                if is_block(name) {
                    self.line_break();
                }
                self.write(raw);
                self.last_block_boundary = is_block(name);
            }
            Kind::Opaque(raw) => {
                self.write(raw);
                self.last_block_boundary = false;
            }
        }
    }

    fn line_break(&mut self) {
        if !self.line_has_content {
            self.trim_layout_whitespace();
        }
        if self.out.ends_with('\n') {
            self.line_has_content = false;
            return;
        }
        if self.line_has_content {
            self.out.push('\n');
        }
        self.line_has_content = false;
    }

    fn write(&mut self, raw: &str) {
        if !self.line_has_content && !raw.starts_with('\n') {
            self.trim_layout_whitespace();
            for _ in 0..self.depth {
                self.out.push_str(INDENT);
            }
        }
        self.out.push_str(raw);
        match raw.rsplit_once('\n') {
            Some((_, tail)) => self.line_has_content = !tail.trim().is_empty(),
            None => self.line_has_content |= !raw.trim().is_empty(),
        }
    }

    /// Drop the layout whitespace that already sits at the start of the
    /// current line.
    ///
    /// Indentation is derived from the writer's nesting depth alone.
    /// Whitespace in front of a token that starts a line can only come from
    /// the source or from an earlier formatting pass (a line with content
    /// keeps `line_has_content` set), so it must be replaced, never extended:
    /// keeping it is what made nested lists drift one level further on every
    /// pass. Text that shares a line with content is left untouched, so text
    /// nodes still survive layout byte for byte.
    fn trim_layout_whitespace(&mut self) {
        while self.out.ends_with(' ') || self.out.ends_with('\t') {
            self.out.pop();
        }
    }

    fn finish(self) -> String {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(html: &str) -> String {
        format_blocks(html).expect("format")
    }

    fn assert_no_split_tag(html: &str) {
        for line in html.lines() {
            let last_open = line.rfind('<');
            let last_close = line.rfind('>');
            assert!(
                last_open <= last_close,
                "tag split across lines in: {line:?}"
            );
        }
    }

    fn lines(html: &str) -> Vec<&str> {
        html.lines().collect()
    }

    fn text_nodes(html: &str) -> Vec<&str> {
        tokenize(html)
            .expect("tokenize")
            .into_iter()
            .filter_map(|kind| match kind {
                Kind::Text(text) => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn adjacent_inline_elements_stay_together() {
        let html =
            "<div class=\"entry\"><span class=\"a\">one</span><span class=\"b\">two</span></div>";
        let out = format(html);
        assert_eq!(out, html);
        assert_no_split_tag(&out);
    }

    #[test]
    fn br_keeps_its_position() {
        let html = "<p>first<br>second<br/>third</p>";
        assert_eq!(format(html), html);
    }

    #[test]
    fn pre_content_is_byte_for_byte() {
        let html = "<div><pre>  keep\n\n    me   </pre></div>";
        let out = format(html);
        assert!(out.contains("<pre>  keep\n\n    me   </pre>"), "{out}");
        assert_eq!(
            lines(&out),
            vec!["<div>", "  <pre>  keep", "", "    me   </pre>", "</div>"]
        );
    }

    #[test]
    fn inline_style_attribute_is_untouched() {
        let html = "<p><span style=\"color: red; font-weight: bold\">loud</span> tail</p>";
        assert_eq!(format(html), html);
    }

    #[test]
    fn attribute_value_may_contain_angle_brackets() {
        let html = "<div><span title=\"a > b < c\">x</span></div>";
        let out = format(html);
        assert_eq!(out, html);
        assert_no_split_tag(&out);
    }

    #[test]
    fn comments_survive_verbatim() {
        let html = "<div><!-- a < b & c --><p>x</p></div>";
        let out = format(html);
        assert_eq!(
            lines(&out),
            vec!["<div><!-- a < b & c -->", "  <p>x</p>", "</div>"]
        );
    }

    #[test]
    fn block_boundaries_are_indented() {
        let html = "<div><section><p>a</p><p>b</p></section></div>";
        assert_eq!(
            lines(&format(html)),
            vec![
                "<div>",
                "  <section>",
                "    <p>a</p>",
                "    <p>b</p>",
                "  </section>",
                "</div>",
            ]
        );
    }

    #[test]
    fn formatting_is_idempotent() {
        let cases = [
            "<div><section><p>a</p><p>b</p></section></div>",
            "<ul><li>one</li><li>two</li></ul>",
            "<div>text<span>x</span></div>",
            "<div>text<p>x</p></div>",
            "<table><tbody><tr><td>1</td><td>2</td></tr></tbody></table>",
            "<div>alpha <span>beta</span> gamma<div>delta </div></div>",
        ];
        for case in cases {
            let once = format(case);
            let twice = format(&once);
            assert_eq!(once, twice, "not idempotent for {case:?}");
            assert_no_split_tag(&once);
        }
    }

    #[test]
    fn text_nodes_survive_layout() {
        let html = "<div>alpha <span>beta</span> gamma</div><div>delta </div>";
        let out = format(html);
        let before: Vec<&str> = text_nodes(html)
            .into_iter()
            .filter(|text| !text.trim().is_empty())
            .collect();
        let after: Vec<&str> = text_nodes(&out)
            .into_iter()
            .filter(|text| !text.trim().is_empty())
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn unbalanced_input_is_formatted() {
        let out = format("</div><p>x<span>y");
        assert!(out.contains("y"));
    }

    #[test]
    fn unterminated_tag_is_an_error() {
        assert!(format_blocks("<div class=\"x>y</div>").is_err());
        assert!(format_blocks("<div").is_err());
        assert!(format_blocks("<!-- open").is_err());
        assert!(format_blocks("<pre>open").is_err());
    }
}
