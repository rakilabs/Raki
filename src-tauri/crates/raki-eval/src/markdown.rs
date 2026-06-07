//! Markdown → plain text for the real-data eval. Strips YAML frontmatter, drops HTML, and
//! emits code-block *contents* without fences or language tags. A deliberate approximation of
//! the eventual ProseMirror block-aware pipeline (see the real-data spec, Limitations).

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

/// Strip a leading `---\n … \n---` YAML frontmatter block, if present.
pub fn strip_frontmatter(src: &str) -> &str {
    let Some(rest) = src.strip_prefix("---\n") else {
        return src;
    };
    // End delimiter: a line that is exactly `---`.
    if let Some(end) = rest.find("\n---\n") {
        &rest[end + 5..]
    } else if let Some(end) = rest.find("\n---") {
        &rest[end + 4..]
    } else {
        src
    }
}

/// Extract readable text: Text + inline Code + code-block text; HTML events are dropped;
/// block boundaries become single spaces. Collapses runs of whitespace.
pub fn to_plain_text(md: &str) -> String {
    let body = strip_frontmatter(md);
    let mut out = String::with_capacity(body.len());
    for event in Parser::new(body) {
        match event {
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            Event::End(TagEnd::Paragraph)
            | Event::End(TagEnd::Heading(_))
            | Event::End(TagEnd::Item)
            | Event::End(TagEnd::CodeBlock) => out.push(' '),
            Event::Start(Tag::CodeBlock(_)) => out.push(' '),
            // Event::Html / InlineHtml deliberately dropped (no tag leakage).
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One content block of a note, with the nearest preceding heading as section context.
/// Headings are NOT standalone blocks; they annotate the blocks beneath them.
#[derive(Debug, Clone)]
pub struct Block {
    pub heading: Option<String>,
    pub text: String,
}

/// Split markdown into content blocks: each paragraph, each WHOLE list (items joined — not one
/// block per item), and each code block. A heading updates the running section context applied to
/// the blocks that follow it. Frontmatter is stripped; HTML is dropped; code contents are kept.
pub fn to_blocks(md: &str) -> Vec<Block> {
    let body = strip_frontmatter(md);
    let mut blocks = Vec::new();
    let mut heading: Option<String> = None;

    // Accumulators for the block currently being assembled.
    let mut buf = String::new();
    let mut in_heading = false;
    let mut heading_buf = String::new();
    let mut list_depth: usize = 0; // >0 while inside a list: keep items in ONE block
    let mut in_code = false;

    let flush = |buf: &mut String, blocks: &mut Vec<Block>, heading: &Option<String>| {
        let t: String = buf.split_whitespace().collect::<Vec<_>>().join(" ");
        if !t.is_empty() {
            blocks.push(Block {
                heading: heading.clone(),
                text: t,
            });
        }
        buf.clear();
    };

    for event in Parser::new(body) {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                in_heading = true;
                heading_buf.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = false;
                heading = Some(heading_buf.trim().to_string()).filter(|s| !s.is_empty());
            }
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
                if list_depth == 0 {
                    flush(&mut buf, &mut blocks, &heading); // whole list = one block
                }
            }
            Event::Start(Tag::CodeBlock(_)) => in_code = true,
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                flush(&mut buf, &mut blocks, &heading);
            }
            Event::End(TagEnd::Paragraph) => {
                if list_depth == 0 {
                    flush(&mut buf, &mut blocks, &heading);
                } else {
                    buf.push(' '); // paragraph inside a list item: keep accumulating
                }
            }
            Event::End(TagEnd::Item) => buf.push(' '),
            Event::Text(t) | Event::Code(t) => {
                if in_heading {
                    heading_buf.push_str(&t);
                } else {
                    buf.push_str(&t);
                    if in_code {
                        buf.push(' ');
                    }
                }
            }
            Event::SoftBreak | Event::HardBreak => buf.push(' '),
            _ => {}
        }
    }
    flush(&mut buf, &mut blocks, &heading); // trailing block, if any
    blocks
}

/// The first level-1 heading's text, or `None` if the doc has no H1.
pub fn first_h1(md: &str) -> Option<String> {
    let body = strip_frontmatter(md);
    let mut in_h1 = false;
    let mut title = String::new();
    for event in Parser::new(body) {
        match event {
            Event::Start(Tag::Heading {
                level: pulldown_cmark::HeadingLevel::H1,
                ..
            }) => {
                in_h1 = true;
            }
            Event::Text(t) if in_h1 => title.push_str(&t),
            Event::End(TagEnd::Heading(pulldown_cmark::HeadingLevel::H1)) if in_h1 => {
                return Some(title.trim().to_string());
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRICKY: &str = "---\ntitle: Frontmatter Note\ntags: [a, b]\n---\n# Real Title\n\nText with a [[WikiLink]] and <b>html</b> inline.\n\n```rust\nfn main() { println!(\"hi\"); }\n```\n";

    #[test]
    fn extracts_text_without_frontmatter_html_or_fence_noise() {
        let text = to_plain_text(TRICKY);
        // Code block contents survive, fences + language tag do not.
        assert!(text.contains("fn main() { println!(\"hi\"); }"));
        assert!(!text.contains("```"));
        assert!(
            !text.contains("rust fn main"),
            "language id must not prefix code"
        );
        // HTML tags dropped (no leakage); inner text may remain.
        assert!(!text.contains("<b>"));
        // Frontmatter keys are gone.
        assert!(!text.contains("tags:"));
        assert!(!text.contains("title: Frontmatter"));
        // Wikilink target text is preserved as content.
        assert!(text.contains("WikiLink"));
    }

    #[test]
    fn first_h1_is_the_title() {
        assert_eq!(first_h1(TRICKY).as_deref(), Some("Real Title"));
        assert_eq!(first_h1("no heading here").as_deref(), None);
    }

    #[test]
    fn to_blocks_splits_content_and_tracks_heading_with_list_as_one_block() {
        let md = "---\ntags: [x]\n---\n# Title\n\n## Logistics\nCheck-in is 3pm. Payment is cash only.\n\n- milk\n- eggs\n- bread\n\n```rust\nfn main() { println!(\"hi\"); }\n```\n";
        let blocks = to_blocks(md);
        // frontmatter gone; the H1 is folded as context, not a content block.
        // content blocks: the Logistics paragraph, the whole list (ONE block), the code block.
        assert_eq!(
            blocks.len(),
            3,
            "para + whole-list + code = 3 content blocks"
        );
        // the paragraph carries its nearest heading (the H2), not a standalone heading chunk.
        let para = &blocks[0];
        assert_eq!(para.heading.as_deref(), Some("Logistics"));
        assert!(para.text.contains("Payment is cash only"));
        // the list is a single block joining its items.
        let list = &blocks[1];
        assert!(
            list.text.contains("milk") && list.text.contains("eggs") && list.text.contains("bread")
        );
        // code contents survive intact; fences/lang do not leak.
        let code = &blocks[2];
        assert!(code.text.contains("fn main() { println!(\"hi\"); }"));
        assert!(!code.text.contains("```"));
        assert!(blocks.iter().all(|b| !b.text.contains("tags:")));
    }
}
