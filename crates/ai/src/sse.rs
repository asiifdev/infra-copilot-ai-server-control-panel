//! A Server-Sent Events decoder. Bytes in, events out, whatever the chunk boundaries are.
//!
//! Follows the WHATWG event-stream rules: `\n`, `\r\n` and `\r` end a line; a blank line ends an
//! event; `data:` lines of one event are joined with `\n`; `:` lines are comments; one leading
//! space after the colon is dropped; a leading byte order mark is ignored. Lines are cut only at
//! newline bytes, which never occur inside a multi-byte UTF-8 character, so a character split
//! across two chunks decodes correctly.

use std::fmt;

/// One decoded event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` name, if any.
    pub event: Option<String>,
    /// The `data:` lines joined with `\n`.
    pub data: String,
}

/// The stream broke a size limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SseError;

impl fmt::Display for SseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("event stream exceeds its size limit")
    }
}

impl std::error::Error for SseError {}

/// Incremental decoder.
#[derive(Debug)]
pub struct SseDecoder {
    buf: Vec<u8>,
    data: String,
    has_data: bool,
    event: Option<String>,
    max: usize,
    at_start: bool,
}

impl SseDecoder {
    /// A decoder that refuses any line or event larger than `max_bytes`.
    #[must_use]
    pub fn new(max_bytes: usize) -> Self {
        Self {
            buf: Vec::new(),
            data: String::new(),
            has_data: false,
            event: None,
            max: max_bytes,
            at_start: true,
        }
    }

    /// Feeds bytes and returns the events they complete.
    ///
    /// # Errors
    /// [`SseError`] when a line or an event grows past the limit.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, SseError> {
        self.buf.extend_from_slice(bytes);
        if self.at_start {
            if self.buf.len() < 3 && b"\xEF\xBB\xBF".starts_with(&self.buf) {
                return Ok(Vec::new());
            }
            if self.buf.starts_with(b"\xEF\xBB\xBF") {
                self.buf.drain(..3);
            }
            self.at_start = false;
        }
        let mut events = Vec::new();
        let mut start = 0;
        let mut i = 0;
        while i < self.buf.len() {
            let b = self.buf[i];
            if b != b'\n' && b != b'\r' {
                i += 1;
                continue;
            }
            // A trailing `\r` may be the first half of `\r\n`: wait for the next byte.
            if b == b'\r' && i + 1 == self.buf.len() {
                break;
            }
            let line = String::from_utf8_lossy(&self.buf[start..i]).into_owned();
            let skip = if b == b'\r' && self.buf.get(i + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            i += skip;
            start = i;
            if let Some(event) = self.line(&line)? {
                events.push(event);
            }
        }
        self.buf.drain(..start);
        if self.buf.len() > self.max {
            return Err(SseError);
        }
        Ok(events)
    }

    /// Ends the stream and returns an event that was started but not closed by a blank line.
    /// (Some servers omit the last blank line; the spec would drop it, we keep it.)
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buf.is_empty() {
            let line = String::from_utf8_lossy(&self.buf).into_owned();
            self.buf.clear();
            let _ = self.line(&line);
        }
        self.dispatch()
    }

    fn line(&mut self, line: &str) -> Result<Option<SseEvent>, SseError> {
        if line.is_empty() {
            return Ok(self.dispatch());
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => {
                if self.data.len() + value.len() + 1 > self.max {
                    return Err(SseError);
                }
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            "event" => self.event = Some(value.chars().take(100).collect()),
            _ => {} // id, retry and unknown fields do not matter to us
        }
        Ok(None)
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        if !self.has_data {
            return None;
        }
        self.has_data = false;
        Some(SseEvent {
            event,
            data: std::mem::take(&mut self.data),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut d = SseDecoder::new(1 << 20);
        let mut out = Vec::new();
        for c in chunks {
            out.extend(d.push(c).unwrap_or_default());
        }
        out.extend(d.finish());
        out
    }

    fn data(events: &[SseEvent]) -> Vec<&str> {
        events.iter().map(|e| e.data.as_str()).collect()
    }

    #[test]
    fn decodes_simple_events_with_every_line_ending() {
        for sep in ["\n", "\r\n", "\r"] {
            let text = format!("data: one{sep}{sep}data: two{sep}{sep}");
            assert_eq!(
                data(&decode_all(&[text.as_bytes()])),
                ["one", "two"],
                "{sep:?}"
            );
        }
    }

    #[test]
    fn the_result_is_the_same_for_every_way_of_cutting_the_bytes() {
        let text = "data: {\"a\":\"é😀\"}\r\n\r\n: comment\r\nevent: x\r\ndata: l1\r\ndata: l2\r\n\r\ndata: [DONE]\n\n";
        let whole = decode_all(&[text.as_bytes()]);
        assert_eq!(data(&whole), ["{\"a\":\"é😀\"}", "l1\nl2", "[DONE]"]);
        assert_eq!(whole[1].event.as_deref(), Some("x"));
        let bytes = text.as_bytes();
        for cut in 1..bytes.len() {
            assert_eq!(
                decode_all(&[&bytes[..cut], &bytes[cut..]]),
                whole,
                "cut at {cut}"
            );
        }
        let singles: Vec<&[u8]> = bytes.chunks(1).collect();
        assert_eq!(decode_all(&singles), whole);
    }

    #[test]
    fn comments_unknown_fields_and_empty_data_are_handled() {
        let events = decode_all(&[b": ping\n\nid: 7\nretry: 100\ndata:x\n\ndata\n\ndata: \n\n"]);
        // `data:x` keeps the value, a bare `data` line and `data: ` give empty-data events.
        assert_eq!(data(&events), ["x", "", ""]);
    }

    #[test]
    fn a_byte_order_mark_is_ignored_even_when_split() {
        assert_eq!(data(&decode_all(&[b"\xEF\xBB\xBFdata: a\n\n"])), ["a"]);
        assert_eq!(
            data(&decode_all(&[b"\xEF", b"\xBB", b"\xBFdata: a\n\n"])),
            ["a"]
        );
    }

    #[test]
    fn an_unterminated_last_event_is_kept_but_a_lone_comment_is_not() {
        assert_eq!(data(&decode_all(&[b"data: tail"])), ["tail"]);
        assert!(decode_all(&[b": only a comment"]).is_empty());
    }

    #[test]
    fn invalid_utf8_is_replaced_not_fatal() {
        let events = decode_all(&[b"data: a\xFFb\n\n"]);
        assert_eq!(data(&events), ["a\u{fffd}b"]);
    }

    #[test]
    fn oversized_lines_and_events_are_refused() {
        let mut d = SseDecoder::new(64);
        assert_eq!(d.push(&[b'x'; 100]), Err(SseError));
        let mut d = SseDecoder::new(64);
        let mut result = Ok(Vec::new());
        for _ in 0..20 {
            result = d.push(b"data: 0123456789\n");
            if result.is_err() {
                break;
            }
        }
        assert_eq!(result, Err(SseError));
    }
}
