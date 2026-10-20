//! The untrusted-data envelope, shared by every adapter.
//!
//! `docs/AI_PROVIDER_SPEC.md` §5 is the rule this module exists for: untrusted `DataBlock`s may be
//! sent as data, never as instruction, so each block is wrapped in an envelope that names its
//! source, path, and tier and marks itself untrusted. The envelope is the same whichever adapter
//! sends it — the wire format only decides where the rendered text lands (a `user` message for
//! OpenAI-compatible and Anthropic, a user part for Gemini), never what it says. Keeping the
//! rendering here means one escaping rule and one shape, so a later prompt-injection review has one
//! place to read.

use std::fmt::Write as _;

use crate::types::DataBlock;

/// Renders the untrusted `DataBlock`s into one envelope, never into a forwardable instruction.
pub(crate) fn render_data(blocks: &[DataBlock]) -> String {
    let mut rendered = String::new();
    for (position, block) in blocks.iter().enumerate() {
        if position > 0 {
            rendered.push_str("\n\n");
        }
        let _ = write!(
            rendered,
            "<untrusted-data source=\"{}\" path=\"{}\" tier=\"{}\" trust=\"untrusted\">\n{}\n\
             </untrusted-data>",
            escape(&block.source),
            escape(&block.path),
            block.tier,
            block.content,
        );
    }
    rendered
}

/// Escapes the four characters that can end the envelope's attribute or open one.
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::render_data;
    use crate::types::{DataBlock, Trust};

    fn block(source: &str, path: &str, content: &str) -> DataBlock {
        DataBlock {
            source: source.to_owned(),
            path: path.to_owned(),
            tier: 2,
            trust: Trust::Untrusted,
            content: content.to_owned(),
        }
    }

    #[test]
    fn one_block_names_its_origin_and_marks_itself_untrusted() {
        let rendered = render_data(&[block("RULES-001", "RULES.md", "use the fake server")]);
        assert!(rendered.contains("source=\"RULES-001\""), "{rendered}");
        assert!(rendered.contains("path=\"RULES.md\""), "{rendered}");
        assert!(rendered.contains("trust=\"untrusted\""), "{rendered}");
        assert!(rendered.contains("use the fake server"), "{rendered}");
    }

    #[test]
    fn attribute_breakers_are_escaped() {
        let rendered = render_data(&[block("a\"<b>&c", "p>q", "body")]);
        assert!(
            rendered.contains("source=\"a&quot;&lt;b&gt;&amp;c\""),
            "{rendered}"
        );
        assert!(rendered.contains("path=\"p&gt;q\""), "{rendered}");
    }

    #[test]
    fn several_blocks_are_separated_and_stay_in_order() {
        let rendered = render_data(&[block("first", "a", "1"), block("second", "b", "2")]);
        let first = rendered.find("first").expect("the first source");
        let second = rendered.find("second").expect("the second source");
        assert!(first < second, "blocks must keep their order: {rendered}");
        assert!(rendered.contains("\n\n"), "{rendered}");
    }
}
