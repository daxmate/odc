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
