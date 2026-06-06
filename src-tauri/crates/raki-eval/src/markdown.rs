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
}
