//! Server-sent-event decoder (B01-59, AC21).
//!
//! Works on bytes, not on network packets: an event may arrive in any number of pieces, several
//! events may share a piece, and a multi-byte UTF-8 character may be split between pieces. Text is
//! decoded only once a whole line is available.

use dgos_types::{DgosError, DgosResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// Value of the `event:` field, when the event had one.
    pub event: Option<String>,
    /// All `data:` lines joined with `\n`.
    pub data: String,
}

/// Result of feeding the decoder: the events completed so far, and the error that stopped it.
#[derive(Debug)]
pub struct Decoded {
    pub events: Vec<SseEvent>,
    pub error: Option<DgosError>,
}

#[cfg(test)]
impl Decoded {
    /// All-or-nothing view, for callers (and tests) that do not need the partial events.
    pub fn into_result(self) -> DgosResult<Vec<SseEvent>> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(self.events),
        }
    }
}

pub struct SseDecoder {
    buf: Vec<u8>,
    max_event_bytes: usize,
    event: Option<String>,
    data: String,
    has_data: bool,
    event_bytes: usize,
    started: bool,
}

impl SseDecoder {
    pub fn new(max_event_bytes: usize) -> Self {
        Self {
            buf: Vec::new(),
            max_event_bytes,
            event: None,
            data: String::new(),
            has_data: false,
            event_bytes: 0,
            started: false,
        }
    }

    /// Feeds raw bytes. `events` holds everything those bytes completed, even when `error` is also
    /// set, so output that arrived before a bad event is not lost. After an error the decoder must
    /// not be used again.
    pub fn push(&mut self, chunk: &[u8]) -> Decoded {
        self.buf.extend_from_slice(chunk);
        let buf = std::mem::take(&mut self.buf);
        let mut events = Vec::new();
        let (consumed, mut error) = self.drain(&buf, &mut events);
        self.buf = buf[consumed..].to_vec();
        // An unterminated line counts towards the event it belongs to.
        if error.is_none() && self.event_bytes + self.buf.len() > self.max_event_bytes {
            error = Some(DgosError::StreamInterrupted);
        }
        Decoded { events, error }
    }

    /// Call at end of input. Returns events completed by a trailing bare `\r`; `error` is set when
    /// the input stopped inside an event.
    pub fn finish(&mut self) -> Decoded {
        let mut events = Vec::new();
        let mut error = None;
        if self.buf.last() == Some(&b'\r') {
            // A lone `\r` at the very end is a complete line terminator.
            self.buf.push(b'\n');
            let buf = std::mem::take(&mut self.buf);
            let (consumed, drained) = self.drain(&buf, &mut events);
            self.buf = buf[consumed..].to_vec();
            error = drained;
        }
        let partial_line = !self.buf.is_empty() && self.buf[0] != b':';
        if error.is_none() && (partial_line || self.has_data) {
            error = Some(DgosError::StreamInterrupted);
        }
        Decoded { events, error }
    }

    fn drain(&mut self, buf: &[u8], events: &mut Vec<SseEvent>) -> (usize, Option<DgosError>) {
        let mut pos = 0;
        while let Some(offset) = buf[pos..]
            .iter()
            .position(|byte| matches!(byte, b'\n' | b'\r'))
        {
            let end = pos + offset;
            let next = if buf[end] == b'\n' {
                end + 1
            } else if end + 1 < buf.len() {
                // `\r\n` is one terminator, a bare `\r` is another.
                if buf[end + 1] == b'\n' {
                    end + 2
                } else {
                    end + 1
                }
            } else {
                // Trailing `\r`: the next byte decides whether it is `\r` or `\r\n`.
                break;
            };
            match self.line(&buf[pos..end]) {
                Ok(Some(event)) => events.push(event),
                Ok(None) => {}
                Err(error) => return (pos, Some(error)),
            }
            pos = next;
        }
        (pos, None)
    }

    fn line(&mut self, raw: &[u8]) -> DgosResult<Option<SseEvent>> {
        let mut raw = raw;
        if !self.started {
            self.started = true;
            raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(raw);
        }
        // Comments (keep-alives) never grow an event.
        if raw.first() == Some(&b':') {
            return Ok(None);
        }
        self.event_bytes += raw.len() + 1;
        if self.event_bytes > self.max_event_bytes {
            return Err(DgosError::StreamInterrupted);
        }
        if raw.is_empty() {
            return Ok(self.dispatch());
        }
        let text = std::str::from_utf8(raw).map_err(|_| DgosError::StreamInterrupted)?;
        let (field, value) = match text.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (text, ""),
        };
        match field {
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            "event" => self.event = Some(value.to_owned()),
            // `id`, `retry` and unknown fields carry nothing the host uses.
            _ => {}
        }
        Ok(None)
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        self.event_bytes = 0;
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

    const LIMIT: usize = 1024 * 1024;

    fn decode_in_pieces(input: &[u8], sizes: &[usize]) -> DgosResult<Vec<SseEvent>> {
        let mut decoder = SseDecoder::new(LIMIT);
        let mut events = Vec::new();
        let mut rest = input;
        for size in sizes.iter().copied().cycle() {
            if rest.is_empty() {
                break;
            }
            let (head, tail) = rest.split_at(size.min(rest.len()));
            events.extend(decoder.push(head).into_result()?);
            rest = tail;
        }
        events.extend(decoder.finish().into_result()?);
        Ok(events)
    }

    fn data(text: &str) -> SseEvent {
        SseEvent {
            event: None,
            data: text.into(),
        }
    }

    #[test]
    fn sse_decodes_several_events_from_one_packet() {
        let events = decode_in_pieces(b"data: a\n\ndata: b\n\ndata: [DONE]\n\n", &[1000]).unwrap();
        assert_eq!(events, vec![data("a"), data("b"), data("[DONE]")]);
    }

    #[test]
    fn sse_yields_the_same_events_for_every_byte_split() {
        let input =
            "event: message\r\ndata: {\"a\":1}\r\n\r\n: keep-alive\r\n\r\ndata: x\rdata: y\r\r\
                     data: é世\n\ndata:z\n\n"
                .as_bytes();
        let whole = decode_in_pieces(input, &[input.len()]).unwrap();
        assert_eq!(
            whole,
            vec![
                SseEvent {
                    event: Some("message".into()),
                    data: "{\"a\":1}".into()
                },
                data("x\ny"),
                data("é世"),
                data("z"),
            ]
        );
        // Every possible single split point, and byte-at-a-time delivery.
        for cut in 1..input.len() {
            assert_eq!(
                decode_in_pieces(input, &[cut, input.len()]).unwrap(),
                whole,
                "split at {cut}"
            );
        }
        assert_eq!(decode_in_pieces(input, &[1]).unwrap(), whole);
    }

    #[test]
    fn sse_joins_multi_line_data_with_newlines() {
        let events =
            decode_in_pieces(b"data: one\ndata: two\ndata:\ndata: four\n\n", &[7]).unwrap();
        assert_eq!(events, vec![data("one\ntwo\n\nfour")]);
    }

    #[test]
    fn sse_skips_comments_and_events_without_data() {
        let events = decode_in_pieces(
            b": ping\n\n: another\nevent: noop\n\nretry: 10\nid: 7\n\ndata: kept\n\n",
            &[5],
        )
        .unwrap();
        assert_eq!(events, vec![data("kept")]);
    }

    #[test]
    fn sse_keeps_a_multibyte_character_split_across_packets_intact() {
        let input = "data: 你好\n\n".as_bytes();
        // Splits inside the three-byte characters.
        let events = decode_in_pieces(input, &[7, 1, 100]).unwrap();
        assert_eq!(events, vec![data("你好")]);
    }

    #[test]
    fn sse_strips_a_leading_byte_order_mark_once() {
        let events = decode_in_pieces(b"\xEF\xBB\xBFdata: a\n\n", &[2]).unwrap();
        assert_eq!(events, vec![data("a")]);
    }

    #[test]
    fn sse_treats_a_trailing_bare_cr_as_the_final_terminator() {
        let mut decoder = SseDecoder::new(LIMIT);
        let mut events = decoder.push(b"data: a\r\r").into_result().unwrap();
        events.extend(decoder.finish().into_result().unwrap());
        assert_eq!(events, vec![data("a")]);
    }

    #[test]
    fn sse_reports_input_that_ends_inside_an_event() {
        for input in [&b"data: a"[..], b"data: a\n", b"data: a\nda"] {
            let mut decoder = SseDecoder::new(LIMIT);
            decoder.push(input).into_result().unwrap();
            assert!(
                matches!(
                    decoder.finish().into_result(),
                    Err(DgosError::StreamInterrupted)
                ),
                "{:?}",
                String::from_utf8_lossy(input)
            );
        }
        let mut decoder = SseDecoder::new(LIMIT);
        decoder.push(b"data: a\n\n").into_result().unwrap();
        assert!(decoder.finish().into_result().unwrap().is_empty());
    }

    #[test]
    fn sse_rejects_invalid_utf8_and_oversized_events() {
        let mut decoder = SseDecoder::new(LIMIT);
        assert!(matches!(
            decoder.push(b"data: \xFF\xFE\n\n").into_result(),
            Err(DgosError::StreamInterrupted)
        ));

        let mut decoder = SseDecoder::new(64);
        assert!(decoder.push(&[b'x'; 40]).into_result().is_ok());
        // A line that never ends is bounded too.
        assert!(matches!(
            decoder.push(&[b'x'; 40]).into_result(),
            Err(DgosError::StreamInterrupted)
        ));

        let mut decoder = SseDecoder::new(64);
        assert!(decoder
            .push(b"data: 0123456789012345678901234567890\n")
            .into_result()
            .is_ok());
        assert!(matches!(
            decoder
                .push(b"data: 0123456789012345678901234567890\n")
                .into_result(),
            Err(DgosError::StreamInterrupted)
        ));
    }

    #[test]
    fn sse_does_not_count_comment_keep_alives_against_the_event_limit() {
        let mut decoder = SseDecoder::new(64);
        for _ in 0..100 {
            assert!(decoder
                .push(b": keep-alive\n")
                .into_result()
                .unwrap()
                .is_empty());
        }
        assert_eq!(
            decoder.push(b"data: ok\n\n").into_result().unwrap(),
            vec![data("ok")]
        );
    }

    #[test]
    fn sse_returns_events_completed_before_a_bad_event_together_with_the_error() {
        // An oversized event right after a good one, delivered in the same read.
        let mut decoder = SseDecoder::new(64);
        let mut input = b"data: good\n\n".to_vec();
        input.extend_from_slice(format!("data: {}\n\n", "x".repeat(200)).as_bytes());
        let decoded = decoder.push(&input);
        assert_eq!(decoded.events, vec![data("good")]);
        assert!(matches!(decoded.error, Some(DgosError::StreamInterrupted)));

        let mut decoder = SseDecoder::new(LIMIT);
        let decoded = decoder.push(b"data: good\n\ndata: \xFF\n\ndata: late\n\n");
        assert_eq!(decoded.events, vec![data("good")]);
        assert!(matches!(decoded.error, Some(DgosError::StreamInterrupted)));
    }
}
