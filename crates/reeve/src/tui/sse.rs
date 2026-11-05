//! The server's `text/event-stream` (`GET /api/v1/events`). Bytes are buffered until a line ends, so a character split
//! across chunks decodes whole. Lines end in LF or CRLF; a blank line ends a frame; `:` lines are comments (the
//! server's keep-alives); fields other than `event`, `id` and `data` (`retry` included) are ignored.

/// A frame (its buffered bytes plus its lines) larger than this drops the connection.
pub const MAX_FRAME: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frame {
    pub event: Option<String>,
    pub id: Option<String>,
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLarge;

#[derive(Debug, Default)]
pub struct Parser {
    buf: Vec<u8>,
    cur: Frame,
    has_data: bool,
    size: usize,
}

impl Parser {
    /// Feeds bytes and returns the frames they complete. After `Err(TooLarge)` the parser is in an undefined state:
    /// discard it (the connection is dropped and resumed via `Last-Event-ID`).
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, TooLarge> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut start = 0;
        while let Some(rel) = self.buf[start..].iter().position(|&b| b == b'\n') {
            let mut raw = &self.buf[start..start + rel];
            start += rel + 1;
            if raw.last() == Some(&b'\r') {
                raw = &raw[..raw.len() - 1];
            }
            if raw.is_empty() {
                let cur = std::mem::take(&mut self.cur);
                // No dispatch with an empty data buffer (an event- or id-only block yields nothing).
                if self.has_data {
                    out.push(cur);
                }
                self.has_data = false;
                self.size = 0;
                continue;
            }
            self.size += raw.len();
            if self.size > MAX_FRAME {
                return Err(TooLarge);
            }
            if raw[0] == b':' {
                continue;
            }
            let line = String::from_utf8_lossy(raw);
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (&*line, ""),
            };
            match field {
                "event" => self.cur.event = Some(value.to_string()),
                "id" => self.cur.id = Some(value.to_string()),
                "data" => {
                    if self.has_data {
                        self.cur.data.push('\n');
                    }
                    self.cur.data.push_str(value);
                    self.has_data = true;
                }
                _ => {}
            }
        }
        self.buf.drain(..start);
        if self.buf.len() + self.size > MAX_FRAME {
            return Err(TooLarge);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(chunks: &[&[u8]]) -> Vec<Frame> {
        let mut p = Parser::default();
        chunks.iter().flat_map(|c| p.push(c).unwrap()).collect()
    }
    fn f(event: &str, id: &str, data: &str) -> Frame {
        Frame {
            event: Some(event.into()),
            id: Some(id.into()),
            data: data.into(),
        }
    }

    #[test]
    fn cursor_and_activity_frames() {
        let got = frames(&[
            b"event: cursor\nid: 5\ndata: 5\n\nevent: activity\nid: 6\ndata: {\"id\":6}\n\n",
        ]);
        assert_eq!(
            got,
            vec![f("cursor", "5", "5"), f("activity", "6", "{\"id\":6}")]
        );
    }

    #[test]
    fn a_frame_split_across_chunks_including_inside_a_multibyte_character() {
        let whole = "event: activity\nid: 7\ndata: {\"t\":\"Åsa\"}\n\n".as_bytes();
        let cut = whole.iter().position(|&b| b == 0xC3).unwrap() + 1; // between the two bytes of Å
        let got = frames(&[&whole[..cut], &whole[cut..]]);
        assert_eq!(got, vec![f("activity", "7", "{\"t\":\"Åsa\"}")]);
    }

    #[test]
    fn crlf_comments_unknown_fields_retry_and_id_before_event() {
        let got = frames(&[b": keep-alive\r\n\r\nid: 8\r\nevent: activity\r\nretry: 10\r\nfoo: bar\r\ndata: x\r\n\r\n"]);
        assert_eq!(got, vec![f("activity", "8", "x")]);
    }

    #[test]
    fn multi_line_data_joins_with_newlines() {
        let got = frames(&[b"event: a\nid: 1\ndata: one\ndata: two\n\n"]);
        assert_eq!(got[0].data, "one\ntwo");
    }

    #[test]
    fn a_keep_alive_alone_yields_nothing() {
        assert!(frames(&[b": keep-alive\n\n"]).is_empty());
    }

    #[test]
    fn an_unterminated_frame_waits() {
        let mut p = Parser::default();
        assert!(p.push(b"event: a\nid: 1\ndata: x\n").unwrap().is_empty());
        assert_eq!(p.push(b"\n").unwrap(), vec![f("a", "1", "x")]);
    }

    #[test]
    fn an_oversized_frame_is_refused() {
        let mut p = Parser::default();
        let big = vec![b'x'; MAX_FRAME + 1];
        assert_eq!(p.push(b"data: ").and_then(|_| p.push(&big)), Err(TooLarge));
    }

    #[test]
    fn a_block_without_data_yields_nothing() {
        assert!(frames(&[b"event: a\nid: 1\n\n"]).is_empty());
    }

    #[test]
    fn many_short_lines_are_parsed_in_linear_time() {
        for chunk in [vec![b'\n'; 512 * 1024], b": k\n".repeat(300_000)] {
            let t = std::time::Instant::now();
            let mut p = Parser::default();
            assert!(p.push(&chunk).unwrap().is_empty());
            assert!(
                t.elapsed() < std::time::Duration::from_secs(1),
                "{:?}",
                t.elapsed()
            );
        }
    }

    #[test]
    fn many_short_data_lines_without_a_blank_line_are_refused() {
        let mut p = Parser::default();
        assert_eq!(p.push(&b"data: x\n".repeat(MAX_FRAME / 4)), Err(TooLarge));
    }

    #[test]
    fn a_blank_line_resets_the_size_count() {
        let mut p = Parser::default();
        let line = format!("data: {}\n", "x".repeat(MAX_FRAME - 100));
        assert!(p.push(line.as_bytes()).unwrap().is_empty());
        assert_eq!(p.push(b"\n").unwrap().len(), 1);
        assert!(p.push(line.as_bytes()).unwrap().is_empty());
        assert_eq!(p.push(b"\n").unwrap().len(), 1);
    }
}
