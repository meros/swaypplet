//! The HTTP server a TV fetches the stream from: the files of one HLS
//! directory, and nothing else.
//!
//! It listens on every address at [`PORT`], fixed so the firewall can name
//! it, and only while a cast runs. Three things keep it narrow:
//!
//! - only the TV's address is answered (`allow`); anyone else is closed on
//!   before a byte is read;
//! - every path starts with a token made for this cast, so a URL from an
//!   earlier one, or a guess, is a 404;
//! - a name is a plain file name in the directory (letters, digits, `.`,
//!   `-`, `_`; no `..`, no `/`).
//!
//! The receiver plays the stream from a page on gstatic.com, so its
//! requests are cross-origin: every answer carries
//! `Access-Control-Allow-Origin: *`, and a CORS preflight gets one too.
//!
//! One thread accepts; each request gets a short-lived thread, so a slow
//! segment fetch does not hold up the playlist.

use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The port the stream is served on. The firewall opens it (the nixos repo).
pub const PORT: u16 = 8891;

/// A server that runs until [`Server::stop`] or drop.
pub struct Server {
    pub port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Serve `dir` under `/<token>/` on `port` (0: any free one, for tests),
/// to `allow` only when it is set.
pub fn serve(
    dir: PathBuf,
    token: String,
    port: u16,
    allow: Option<IpAddr>,
) -> Result<Server, String> {
    let listener = TcpListener::bind(("0.0.0.0", port))
        .map_err(|e| format!("serving the stream on port {port}: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let dir = Arc::new(dir);
    let token = Arc::new(token);
    let thread = std::thread::Builder::new()
        .name("swaypplet-cast-http".into())
        .spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        if allow.is_some_and(|ip| ip != peer.ip()) {
                            log::info!("cast: refused a request from {peer}");
                            continue;
                        }
                        let (dir, token) = (dir.clone(), token.clone());
                        let _ = std::thread::Builder::new()
                            .name("swaypplet-cast-get".into())
                            .spawn(move || answer(stream, peer, &dir, &token));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    Err(e) => log::debug!("cast: accept: {e}"),
                }
            }
        })
        .map_err(|e| format!("http thread: {e}"))?;
    Ok(Server {
        port,
        stop,
        thread: Some(thread),
    })
}

impl Server {
    /// Whether it still accepts.
    pub fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Stop accepting and close the port. Requests in flight finish.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

/// What a request path asks for: a file name under the token, or nothing.
pub fn requested<'a>(path: &'a str, token: &str) -> Option<&'a str> {
    let rest = path
        .strip_prefix('/')?
        .strip_prefix(token)?
        .strip_prefix('/')?;
    let rest = rest.split(['?', '#']).next()?;
    let plain = !rest.is_empty()
        && !rest.starts_with('.')
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    plain.then_some(rest)
}

/// The media type of a file the encoder writes.
pub fn content_type(name: &str) -> &'static str {
    match Path::new(name).extension().and_then(|e| e.to_str()) {
        Some("m3u8") => "application/vnd.apple.mpegurl",
        Some("ts") => "video/mp2t",
        Some("m4s") => "video/iso.segment",
        Some("mp4") => "video/mp4",
        _ => "application/octet-stream",
    }
}

const CORS: &str = "Access-Control-Allow-Origin: *\r\n\
    Access-Control-Allow-Methods: GET, HEAD, OPTIONS\r\n\
    Access-Control-Allow-Headers: *\r\n";

fn answer(stream: TcpStream, peer: SocketAddr, dir: &Path, token: &str) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    // The headers: read to the blank line, keep none.
    let mut header = String::new();
    while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
        header.clear();
    }
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut out = &stream;
    let reply = |out: &mut &TcpStream, status: &str, extra: &str, body: &[u8]| {
        let head = format!(
            "HTTP/1.1 {status}\r\n{CORS}{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = out
            .write_all(head.as_bytes())
            .and_then(|()| out.write_all(body));
    };
    if method == "OPTIONS" {
        return reply(&mut out, "204 No Content", "", b"");
    }
    if method != "GET" && method != "HEAD" {
        return reply(&mut out, "405 Method Not Allowed", "", b"");
    }
    let Some(name) = requested(path, token) else {
        return reply(&mut out, "404 Not Found", "", b"");
    };
    match std::fs::read(dir.join(name)) {
        Ok(body) => {
            log::debug!("cast: {peer} GET {name} ({} bytes)", body.len());
            let extra = format!(
                "Content-Type: {}\r\nCache-Control: no-cache\r\n",
                content_type(name)
            );
            let body = if method == "HEAD" { &[][..] } else { &body[..] };
            reply(&mut out, "200 OK", &extra, body);
        }
        Err(_) => reply(&mut out, "404 Not Found", "", b""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn only_plain_names_under_the_token_are_served() {
        assert_eq!(requested("/tok/stream.m3u8", "tok"), Some("stream.m3u8"));
        assert_eq!(
            requested("/tok/seg00012.ts?x=1", "tok"),
            Some("seg00012.ts")
        );
        assert_eq!(requested("/other/stream.m3u8", "tok"), None);
        assert_eq!(requested("/tok/../etc/passwd", "tok"), None);
        assert_eq!(requested("/tok/..", "tok"), None);
        assert_eq!(requested("/tok/.hidden", "tok"), None);
        assert_eq!(requested("/tok/a/b.ts", "tok"), None);
        assert_eq!(requested("/tok/", "tok"), None);
        assert_eq!(requested("/tokstream.m3u8", "tok"), None);
    }

    #[test]
    fn segment_types_are_what_hls_players_expect() {
        assert_eq!(content_type("stream.m3u8"), "application/vnd.apple.mpegurl");
        assert_eq!(content_type("seg00001.ts"), "video/mp2t");
    }

    fn get(port: u16, request: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(request.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("swaypplet-http-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("stream.m3u8"), "#EXTM3U\n").unwrap();
        d
    }

    #[test]
    fn a_playlist_is_served_with_cors_and_strangers_are_not() {
        let d = dir();
        let mut server = serve(d.clone(), "tok".into(), 0, None).unwrap();
        let ok = get(
            server.port,
            "GET /tok/stream.m3u8 HTTP/1.1\r\nHost: x\r\n\r\n",
        );
        assert!(ok.starts_with("HTTP/1.1 200 OK"), "{ok}");
        assert!(ok.contains("Access-Control-Allow-Origin: *"));
        assert!(ok.contains("Content-Type: application/vnd.apple.mpegurl"));
        assert!(ok.ends_with("#EXTM3U\n"));
        let pre = get(server.port, "OPTIONS /tok/stream.m3u8 HTTP/1.1\r\n\r\n");
        assert!(
            pre.starts_with("HTTP/1.1 204") && pre.contains("Allow-Origin"),
            "{pre}"
        );
        assert!(get(server.port, "GET /bad/stream.m3u8 HTTP/1.1\r\n\r\n").contains(" 404 "));
        assert!(get(server.port, "GET /tok/missing.ts HTTP/1.1\r\n\r\n").contains(" 404 "));
        server.stop();
        assert!(!server.running());
        assert!(
            TcpStream::connect(("127.0.0.1", server.port)).is_err(),
            "port closed"
        );

        // Allowed only for another address: localhost is closed on.
        let other: IpAddr = "192.0.2.7".parse().unwrap();
        let server = serve(d.clone(), "tok".into(), 0, Some(other)).unwrap();
        assert_eq!(
            get(server.port, "GET /tok/stream.m3u8 HTTP/1.1\r\n\r\n"),
            ""
        );
        drop(server);
        let _ = std::fs::remove_dir_all(d);
    }
}
