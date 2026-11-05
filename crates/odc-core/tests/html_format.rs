//! Contract tests for the block-level HTML formatter used by the importer.

use odc_core::block_fmt::format_blocks;

/// Dictionary-style bodies with dense inline markup, the shape that made
/// width-based formatters wrap inside entries.
const BODIES: &[&str] = &[
    "<div class=\"entry\"><span class=\"hw\">word</span><span class=\"ph\">/wɜːd/</span><ol><li><span class=\"pos\">n.</span> <span class=\"def\">a single unit of language</span></li><li><span class=\"pos\">v.</span> <span class=\"def\">express in words</span></li></ol></div>",
    "<div class=\"entry\"><h2>run</h2><p><b>1.</b> move quickly on foot, faster than walking, with both feet leaving the ground.</p><p><b>2.</b> manage or operate something such as a business or machine.</p></div>",
    "<ul><li><i>alpha</i> — first</li><li><i>beta</i> — second with a longer explanation that would wrap at eighty columns</li></ul>",
    "<table><tbody><tr><td><b>headword</b></td><td><span style=\"color:red\">translation</span></td></tr></tbody></table>",
    "<p>plain <a href=\"/x\" title=\"a > b\">link</a> tail<br>next line</p>",
];

fn split_line_count(html: &str) -> (usize, usize) {
    let lines: Vec<&str> = html.lines().collect();
    let split = lines
        .iter()
        .filter(|line| match (line.rfind('<'), line.rfind('>')) {
            (Some(open), Some(close)) => open > close,
            (Some(_), None) => true,
            _ => false,
        })
        .count();
    (split, lines.len())
}

/// Text segments between tags; comments and attribute values are skipped.
fn text_nodes(html: &str) -> Vec<String> {
    let mut nodes = Vec::new();
    let mut buffer = String::new();
    let mut chars = html.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '<' {
            buffer.push(character);
            continue;
        }
        if !buffer.is_empty() {
            nodes.push(std::mem::take(&mut buffer));
        }
        let mut quote = None;
        for inside in chars.by_ref() {
            match quote {
                Some(open) if inside == open => quote = None,
                Some(_) => {}
                None => match inside {
                    '"' | '\'' => quote = Some(inside),
                    '>' => break,
                    _ => {}
                },
            }
        }
    }
    if !buffer.is_empty() {
        nodes.push(buffer);
    }
    nodes
}

fn content_nodes(html: &str) -> Vec<String> {
    text_nodes(html)
        .into_iter()
        .filter(|node| !node.trim().is_empty())
        .map(|node| node.trim().to_string())
        .collect()
}

#[test]
fn no_tag_is_split_across_lines() {
    let mut split = 0;
    let mut lines = 0;
    for body in BODIES {
        let formatted = format_blocks(body).expect("format");
        let (body_split, body_lines) = split_line_count(&formatted);
        split += body_split;
        lines += body_lines;
        assert!(body_split == 0, "split tag in {body:?}:\n{formatted}");
    }
    assert!(lines > 0);
    assert_eq!(split, 0, "split rate {}/{lines}", split);
}

#[test]
fn formatting_is_idempotent() {
    for body in BODIES {
        let once = format_blocks(body).expect("first pass");
        let twice = format_blocks(&once).expect("second pass");
        assert_eq!(once, twice, "not idempotent for {body:?}");
    }
}

#[test]
fn text_nodes_are_equivalent() {
    for body in BODIES {
        let formatted = format_blocks(body).expect("format");
        assert_eq!(
            content_nodes(body),
            content_nodes(&formatted),
            "text changed for {body:?}:\n{formatted}"
        );
    }
}

#[test]
fn pre_content_is_byte_for_byte() {
    let html = "<div><pre>  keep\n\n    me   </pre></div>";
    let formatted = format_blocks(html).expect("format");
    assert!(
        formatted.contains("<pre>  keep\n\n    me   </pre>"),
        "{formatted}"
    );
}

#[test]
fn adjacent_inline_elements_keep_their_text() {
    let html = "<div><span>one</span><span>two</span></div>";
    assert_eq!(format_blocks(html).expect("format"), html);
}

#[test]
fn br_comments_and_inline_style_survive() {
    let html = "<p>a<br>b</p><!-- keep < me --><p><span style=\"color: red\">c</span></p>";
    let formatted = format_blocks(html).expect("format");
    assert!(formatted.contains("<br>"));
    assert!(formatted.contains("<!-- keep < me -->"));
    assert!(formatted.contains("style=\"color: red\""));
}

#[test]
fn malformed_markup_is_reported() {
    assert!(format_blocks("<div class=\"x>y</div>").is_err());
    assert!(format_blocks("<div").is_err());
}

/// Layout has to be a fixed point of `format_blocks`: a second pass over the
/// formatter's own output must be byte-identical, otherwise every import
/// rewrites the body and the stored hash changes for reasons unrelated to the
/// content.
fn assert_layout_is_stable(html: &str) {
    let first = format_blocks(html).expect("format");
    let second = format_blocks(&first).expect("format");
    assert_eq!(
        first, second,
        "second pass changed the layout\nfirst:\n{first}\nsecond:\n{second}"
    );
    assert_eq!(
        second,
        format_blocks(&second).expect("format"),
        "layout keeps drifting after the second pass"
    );
}

#[test]
fn nested_lists_are_idempotent() {
    // Single line source: the layout is created by the first pass.
    assert_layout_is_stable("<ul><li>one<ul><li>two<ul><li>three</li></ul></li></ul></li></ul>");
    // Already laid out source: every list item ends with a newline, so a
    // formatter that extends the existing indentation drifts one level per
    // pass. That was the reported `-ware` defect.
    assert_layout_is_stable(
        "<ul>\n  <li>one\n    <ul>\n      <li>two\n      </li>\n    </ul>\n  </li>\n</ul>",
    );
}

#[test]
fn indentation_is_derived_from_depth() {
    let canonical =
        "<ul>\n  <li>one\n    <ul>\n      <li>two\n      </li>\n    </ul>\n  </li>\n</ul>";
    let padded = "<ul>\n        <li>one\n                <ul>\n                        <li>two\n                        </li>\n                </ul>\n        </li>\n</ul>";
    assert_eq!(format_blocks(padded).expect("format"), canonical);
    assert_layout_is_stable(padded);
}

/// Real `-ware` subtree (LDOCE6++ En-Cn V2-19), truncated to the list that used
/// to drift: the closing `</li>` follows a text node that ends with a newline.
#[test]
fn ldoce6_ware_list_is_idempotent() {
    let body = include_str!("fixtures/ldoce6_ware_list.html");
    assert_layout_is_stable(body);
    let formatted = format_blocks(body).expect("format");
    assert!(
        formatted.contains("\n    </li>\n  </ul>\n</div>\n"),
        "closing tags must sit at the depth-derived indentation:\n{formatted}"
    );
}
