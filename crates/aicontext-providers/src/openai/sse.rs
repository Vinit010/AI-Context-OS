//! A minimal Server-Sent Events parser, for the `stream: true` response.
//!
//! `docs/AI_PROVIDER_SPEC.md` §2 says `stream` returns `BoxStream<StreamEvent>`, and the wire the
//! stream arrives on is SSE: `data:` lines, an event per blank line, ending with `data: [DONE]`.
//! That grammar is four lines of text, so this is a few dozen lines of our own rather than a crate
//! whose dependency list would only ever be one importer deep.
//!
//! The parser is incremental because chunks arrive on arbitrary boundaries: a field may split
//! across two reads, so a chunk is appended and the next complete event is taken out.

/// An incremental `text/event-stream` parser.
#[derive(Debug, Default)]
pub(super) struct SseParser {
    buffer: String,
}

impl SseParser {
    /// An empty parser.
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Appends a chunk, normalising the `\r\n` SSE is allowed to use.
    pub(super) fn push(&mut self, chunk: &str) {
        if chunk.contains('\r') {
            self.buffer.push_str(&chunk.replace("\r\n", "\n"));
        } else {
            self.buffer.push_str(chunk);
        }
    }

    /// The next event's `data` field, joined when it spans several lines, or `None` while the buffer
    /// holds no complete event.
    ///
    /// A comment-only event (SSE keep-alives start with `:`) carries no data and is skipped.
    pub(super) fn next(&mut self) -> Option<String> {
        loop {
            let boundary = self.buffer.find("\n\n")?;
            let event: String = self.buffer.drain(..boundary + 2).collect();
            let mut data = String::new();
            for line in event.lines() {
                let Some(value) = line.strip_prefix("data:") else {
                    continue;
                };
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.strip_prefix(' ').unwrap_or(value));
            }
            if !data.is_empty() {
                return Some(data);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SseParser;

    #[test]
    fn one_event_yields_its_data() {
        let mut parser = SseParser::new();
        parser.push("data: {\"a\":1}\n\n");
        assert_eq!(parser.next(), Some("{\"a\":1}".to_owned()));
        assert_eq!(parser.next(), None);
    }

    #[test]
    fn several_events_are_read_one_at_a_time() {
        let mut parser = SseParser::new();
        parser.push("data: first\n\ndata: second\n\n");
        assert_eq!(parser.next(), Some("first".to_owned()));
        assert_eq!(parser.next(), Some("second".to_owned()));
        assert_eq!(parser.next(), None);
    }

    #[test]
    fn an_event_split_across_chunks_is_reassembled() {
        let mut parser = SseParser::new();
        parser.push("data: {\"par");
        assert_eq!(parser.next(), None);
        parser.push("tial\":true}\n\n");
        assert_eq!(parser.next(), Some("{\"partial\":true}".to_owned()));
    }

    #[test]
    fn carriaged_returns_are_normalised() {
        let mut parser = SseParser::new();
        parser.push("data: crlf\r\n\r\n");
        assert_eq!(parser.next(), Some("crlf".to_owned()));
    }

    #[test]
    fn the_data_lines_of_one_event_are_joined() {
        let mut parser = SseParser::new();
        parser.push("data: one\ndata: two\n\n");
        assert_eq!(parser.next(), Some("one\ntwo".to_owned()));
    }

    #[test]
    fn a_comment_only_event_is_skipped() {
        let mut parser = SseParser::new();
        parser.push(": keep-alive\n\ndata: real\n\n");
        assert_eq!(parser.next(), Some("real".to_owned()));
    }
}
