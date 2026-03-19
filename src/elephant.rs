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

// ── Public types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub identifier: String,
    pub text: String,
    pub subtext: String,
    pub icon: String,
    pub provider: String,
    pub score: i32,
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

fn send_message(stream: &mut UnixStream, msg_type: u8, payload: &[u8]) -> io::Result<()> {
    let mut buf = vec![msg_type, FORMAT_PROTOBUF];
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(payload);
    stream.write_all(&buf)
}

fn read_response_header(stream: &mut UnixStream) -> io::Result<(u8, u32)> {
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

    loop {
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
                let mut payload = vec![0u8; length as usize];
                stream.read_exact(&mut payload)?;

                if let Ok(resp) = QueryResponse::parse_from_bytes(&payload) {
                    if resp.item.is_some() {
                        let item = resp.item.unwrap();
                        results.push(SearchResult {
                            identifier: item.identifier,
                            text: item.text,
                            subtext: item.subtext,
                            icon: item.icon,
                            provider: item.provider,
                            score: item.score,
                        });
                    }
                }
            }
            _ => {
                // Unknown status — skip the payload
                if length > 0 {
                    let mut skip = vec![0u8; length as usize];
                    let _ = stream.read_exact(&mut skip);
                }
            }
        }
    }

    // Sort by score descending
    results.sort_by(|a, b| b.score.cmp(&a.score));
    Ok(results)
}

/// Activate a search result item. This is a blocking call.
pub fn activate(provider: &str, identifier: &str, query_text: &str) -> io::Result<()> {
    let mut stream = connect()?;

    let mut req = ActivateRequest::new();
    req.provider = provider.to_string();
    req.identifier = identifier.to_string();
    req.action = "select".to_string();
    req.query = query_text.to_string();

    let payload = req
        .write_to_bytes()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    send_message(&mut stream, MSG_TYPE_ACTIVATE, &payload)?;

    Ok(())
}

/// Check if the elephant socket exists (daemon is likely running).
#[allow(dead_code)]
pub fn is_available() -> bool {
    socket_path().exists()
}
