//! Client for the elephant search daemon (used by walker).
//!
//! Elephant communicates over a Unix socket with a binary framing protocol:
//! Request:  [1B type] [1B format] [4B big-endian length] [protobuf payload]
//! Response: [1B status] [4B big-endian length] [protobuf payload]

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use protobuf::Message;

// Generated protobuf types
include!(concat!(env!("OUT_DIR"), "/proto/mod.rs"));
use elephant::{ActivateRequest, QueryRequest, QueryResponse};

// ── Wire protocol constants ─────────────────────────────────────────────────

const MSG_TYPE_QUERY: u8 = 0;
const MSG_TYPE_ACTIVATE: u8 = 1;
const FORMAT_PROTOBUF: u8 = 0;

const RESP_QUERY_ITEM: u8 = 0;
const RESP_QUERY_ASYNC_ITEM: u8 = 1;
const RESP_QUERY_NO_RESULTS: u8 = 254;
const RESP_QUERY_DONE: u8 = 255;

/// Cap on a single response frame's declared length, to guard against an
/// allocation bomb from a desynced or misbehaving daemon.
const MAX_FRAME_LEN: u32 = 4 * 1024 * 1024;
/// Overall wall-clock budget for a query, since the per-read socket timeout
/// resets on every frame and won't catch a daemon that streams without DONE.
const QUERY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

// ── Public types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub identifier: String,
    pub text: String,
    pub subtext: String,
    pub icon: String,
    pub provider: String,
    pub score: i32,
    pub actions: Vec<String>,
}

// ── Socket path ─────────────────────────────────────────────────────────────

fn socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("elephant/elephant.sock")
    } else {
        PathBuf::from("/tmp/elephant/elephant.sock")
    }
}

// ── Connection ──────────────────────────────────────────────────────────────

fn connect() -> io::Result<UnixStream> {
    let path = socket_path();
    let stream = UnixStream::connect(&path)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(1)))?;
    Ok(stream)
}

/// One request frame: type, format, big-endian length, payload.
fn send_message(stream: &mut impl Write, msg_type: u8, payload: &[u8]) -> io::Result<()> {
    let mut buf = vec![msg_type, FORMAT_PROTOBUF];
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(payload);
    stream.write_all(&buf)
}

/// One response frame's header: the status and the payload's length.
fn read_response_header(stream: &mut impl Read) -> io::Result<(u8, u32)> {
    let mut header = [0u8; 5];
    stream.read_exact(&mut header)?;
    let status = header[0];
    let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
    Ok((status, length))
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Query elephant for search results. Returns up to `max_results` items.
/// This is a blocking call — run on a background thread.
pub fn query(
    search_text: &str,
    providers: &[&str],
    max_results: i32,
) -> io::Result<Vec<SearchResult>> {
    let mut stream = connect()?;

    let mut req = QueryRequest::new();
    req.providers = providers.iter().map(|s| s.to_string()).collect();
    req.query = search_text.to_string();
    req.maxresults = max_results;

    let payload = req
        .write_to_bytes()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    send_message(&mut stream, MSG_TYPE_QUERY, &payload)?;

    let mut results = Vec::new();
    let deadline = std::time::Instant::now() + QUERY_DEADLINE;

    loop {
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "elephant query exceeded overall deadline",
            ));
        }

        let (status, length) = match read_response_header(&mut stream) {
            Ok(h) => h,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == io::ErrorKind::TimedOut => break,
            Err(e) => return Err(e),
        };

        match status {
            RESP_QUERY_DONE | RESP_QUERY_NO_RESULTS => break,
            RESP_QUERY_ITEM | RESP_QUERY_ASYNC_ITEM => {
                if length == 0 {
                    continue;
                }
                if length > MAX_FRAME_LEN {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("elephant response frame too large ({length} bytes)"),
                    ));
                }
                let mut payload = vec![0u8; length as usize];
                stream.read_exact(&mut payload)?;

                if let Ok(resp) = QueryResponse::parse_from_bytes(&payload)
                    && resp.item.is_some()
                {
                    let item = resp.item.unwrap();
                    results.push(SearchResult {
                        identifier: item.identifier,
                        text: item.text,
                        subtext: item.subtext,
                        icon: item.icon,
                        provider: item.provider,
                        score: item.score,
                        actions: item.actions,
                    });
                }
            }
            _ => {
                // Unknown status — skip the payload
                if length > 0 {
                    if length > MAX_FRAME_LEN {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("elephant response frame too large ({length} bytes)"),
                        ));
                    }
                    let mut skip = vec![0u8; length as usize];
                    let _ = stream.read_exact(&mut skip);
                }
            }
        }
    }

    // Sort by score descending
    results.sort_by_key(|r| std::cmp::Reverse(r.score));
    Ok(results)
}

/// Activate a search result item. This is a blocking call.
pub fn activate(
    provider: &str,
    identifier: &str,
    action: &str,
    query_text: &str,
) -> io::Result<()> {
    let mut stream = connect()?;

    let mut req = ActivateRequest::new();
    req.provider = provider.to_string();
    req.identifier = identifier.to_string();
    req.action = action.to_string();
    req.query = query_text.to_string();

    let payload = req
        .write_to_bytes()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    send_message(&mut stream, MSG_TYPE_ACTIVATE, &payload)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn a_request_is_type_format_length_payload() {
        let mut out = Cursor::new(Vec::new());
        send_message(&mut out, MSG_TYPE_ACTIVATE, b"abc").unwrap();
        assert_eq!(
            out.into_inner(),
            [
                MSG_TYPE_ACTIVATE,
                FORMAT_PROTOBUF,
                0,
                0,
                0,
                3,
                b'a',
                b'b',
                b'c'
            ]
        );
        let mut out = Cursor::new(Vec::new());
        send_message(&mut out, MSG_TYPE_QUERY, &[]).unwrap();
        assert_eq!(
            out.into_inner(),
            [MSG_TYPE_QUERY, FORMAT_PROTOBUF, 0, 0, 0, 0]
        );
    }

    #[test]
    fn a_response_header_is_status_and_big_endian_length() {
        let mut input = Cursor::new(vec![RESP_QUERY_ITEM, 0, 0, 1, 2, 0xff]);
        assert_eq!(
            read_response_header(&mut input).unwrap(),
            (RESP_QUERY_ITEM, 258)
        );
        // The payload is left for the caller.
        assert_eq!(input.position(), 5);
        // A header cut short is an error, not a guess.
        let mut short = Cursor::new(vec![RESP_QUERY_DONE, 0, 0]);
        assert_eq!(
            read_response_header(&mut short).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    /// A query's payload survives the framing: what is sent reads back.
    #[test]
    fn a_query_round_trips_through_a_frame() {
        let mut req = QueryRequest::new();
        req.query = "fire".into();
        req.maxresults = 5;
        let payload = req.write_to_bytes().unwrap();
        let mut wire = Cursor::new(Vec::new());
        send_message(&mut wire, MSG_TYPE_QUERY, &payload).unwrap();
        let mut wire = Cursor::new(wire.into_inner());
        let mut head = [0u8; 2];
        wire.read_exact(&mut head).unwrap();
        assert_eq!(head, [MSG_TYPE_QUERY, FORMAT_PROTOBUF]);
        let mut len = [0u8; 4];
        wire.read_exact(&mut len).unwrap();
        let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
        wire.read_exact(&mut body).unwrap();
        let back = QueryRequest::parse_from_bytes(&body).unwrap();
        assert_eq!(back.query, "fire");
        assert_eq!(back.maxresults, 5);
    }
}
