//! The HTTP server a TV fetches the stream from: the files of one HLS
//! directory ([`Site::Files`]), or the live fragmented MP4 and the page that
//! plays it ([`Site::Live`]), and nothing else.
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
//! One thread accepts; each request gets a thread of its own, so a slow
//! segment fetch does not hold up the playlist.
//!
//! # The live routes
//!
//! - `live.mp4`: one response that does not end while the stream runs. No
//!   `Content-Length`; `Connection: close`, so the body is everything until
//!   the connection closes. It starts with the init segment and goes on
//!   from the next keyframe (`live::Hub`), each fragment written as it
//!   comes. A write that cannot finish in [`WRITE_TIMEOUT`] drops the
//!   viewer; the page reconnects.
//! - `player.html`: the page (`player.html` beside this file), with the
//!   stream's codec string filled in.
//! - `report` (POST, JSON, at most [`MAX_REPORT`] bytes): what the page says.
//!   `{"event":"playing"}` marks the stream as on screen, which the backend
//!   waits for; the rest is logged at info level.
//! - `clock` and `t0`: this machine's time and the stream's start, in epoch
//!   ms, for the latency figure in the page's debug view.

use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::live::Hub;

/// The page that plays the live stream, with `{{CODEC}}` to fill in.
const PLAYER: &str = include_str!("player.html");
/// A viewer's write that takes longer than this ends its stream.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(3);
/// The largest report the page may send.
pub const MAX_REPORT: usize = 8 * 1024;

/// What a server serves.
#[derive(Clone)]
pub enum Site {
    /// The files of a directory: HLS.
    Files(PathBuf),
    /// The live stream and its page.
    Live(Arc<Hub>),
}

/// The port the stream is served on. The firewall opens it (the nixos repo).
pub const PORT: u16 = 8891;

/// A server that runs until [`Server::stop`] or drop.
pub struct Server {
    pub port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Serve `site` under `/<token>/` on `port` (0: any free one, for tests),
/// to `allow` only when it is set.
pub fn serve(
    site: Site,
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
    let site = Arc::new(site);
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
                        let (site, token) = (site.clone(), token.clone());
                        let _ = std::thread::Builder::new()
                            .name("swaypplet-cast-get".into())
                            .spawn(move || answer(stream, peer, &site, &token));
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
        Some("html") => "text/html; charset=utf-8",
        _ => "application/octet-stream",
    }
}

const CORS: &str = "Access-Control-Allow-Origin: *\r\n\
    Access-Control-Allow-Methods: GET, HEAD, POST, OPTIONS\r\n\
    Access-Control-Allow-Headers: *\r\n";

/// The page, with the codec string filled in.
pub fn player_page(codec: &str) -> String {
    PLAYER.replace("{{CODEC}}", codec)
}

fn answer(stream: TcpStream, peer: SocketAddr, site: &Site, token: &str) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    // The headers: read to the blank line, keep the length.
    let mut header = String::new();
    let mut length = 0usize;
    while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
        if let Some((k, v)) = header.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            length = v.trim().parse().unwrap_or(usize::MAX);
        }
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
    let Some(name) = requested(path, token) else {
        return reply(&mut out, "404 Not Found", "", b"");
    };
    log::info!("cast: {} asked for {method} {name}", peer.ip());
    let dir = match (site, method, name) {
        (Site::Live(hub), "POST", "report") => {
            if length > MAX_REPORT {
                return reply(&mut out, "413 Payload Too Large", "", b"");
            }
            let mut body = vec![0u8; length];
            if std::io::Read::read_exact(&mut reader, &mut body).is_err() {
                return;
            }
            report(hub, &String::from_utf8_lossy(&body));
            return reply(&mut out, "204 No Content", "", b"");
        }
        (_, "GET" | "HEAD", _) => match site {
            Site::Files(dir) => dir,
            Site::Live(hub) => return live(&stream, hub, method, name),
        },
        _ => return reply(&mut out, "405 Method Not Allowed", "", b""),
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

/// What the page reported.
fn report(hub: &Hub, body: &str) {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    if v["event"] == "playing" {
        hub.set_playing();
    } else {
        if v["event"] == "stats" {
            hub.count_stats(body);
        }
        log::info!(
            "cast: page: {}",
            body.chars().take(MAX_REPORT).collect::<String>()
        );
    }
}

/// A GET on the live site.
fn live(mut out: &TcpStream, hub: &Hub, method: &str, name: &str) {
    let small = |out: &mut &TcpStream, status: &str, kind: &str, body: &[u8]| {
        let head = format!(
            "HTTP/1.1 {status}\r\n{CORS}Content-Type: {kind}\r\nCache-Control: no-store\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let body = if method == "HEAD" { &[][..] } else { body };
        let _ = out
            .write_all(head.as_bytes())
            .and_then(|()| out.write_all(body));
    };
    let ms = |v: Option<f64>| v.map(|v| format!("{v:.1}")).unwrap_or_default();
    match name {
        "player.html" => {
            let page = player_page(&hub.codec().unwrap_or_default());
            small(&mut out, "200 OK", content_type(name), page.as_bytes());
        }
        "clock" => small(
            &mut out,
            "200 OK",
            "text/plain",
            ms(Some(now_ms())).as_bytes(),
        ),
        "t0" => small(&mut out, "200 OK", "text/plain", ms(hub.t0_ms()).as_bytes()),
        "live.mp4" => stream_live(out, hub),
        _ => small(&mut out, "404 Not Found", "text/plain", b""),
    }
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// The endless response: the init segment, then fragments as they come.
fn stream_live(mut out: &TcpStream, hub: &Hub) {
    let Some((init, fragments)) = hub.watch() else {
        let _ = out.write_all(
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    };
    let _ = out.set_nodelay(true);
    let _ = out.set_write_timeout(Some(WRITE_TIMEOUT));
    let head = format!(
        "HTTP/1.1 200 OK\r\n{CORS}Content-Type: video/mp4\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    );
    if out
        .write_all(head.as_bytes())
        .and_then(|()| out.write_all(&init))
        .is_err()
    {
        return;
    }
    log::info!("cast: a viewer joined the live stream");
    // Ends when the hub drops this viewer (behind, or the stream ended) or
    // a write fails.
    for fragment in fragments {
        if out.write_all(&fragment).is_err() {
            break;
        }
    }
    log::info!("cast: a viewer left the live stream");
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
        let mut server = serve(Site::Files(d.clone()), "tok".into(), 0, None).unwrap();
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
        let server = serve(Site::Files(d.clone()), "tok".into(), 0, Some(other)).unwrap();
        assert_eq!(
            get(server.port, "GET /tok/stream.m3u8 HTTP/1.1\r\n\r\n"),
            ""
        );
        drop(server);
        let _ = std::fs::remove_dir_all(d);
    }

    fn hub() -> Arc<Hub> {
        use crate::services::cast::fmp4::{InitInfo, Piece};
        let hub = Arc::new(Hub::default());
        hub.publish(Piece::Init {
            bytes: b"INIT".to_vec(),
            info: InitInfo {
                codec: "avc1.640029".into(),
                timescale: 15360,
                trex_flags: 0,
            },
        });
        hub
    }

    #[test]
    fn the_page_carries_the_codec_and_a_report_marks_it_playing() {
        let hub = hub();
        let server = serve(Site::Live(hub.clone()), "tok".into(), 0, None).unwrap();
        let page = get(server.port, "GET /tok/player.html HTTP/1.1\r\n\r\n");
        assert!(page.starts_with("HTTP/1.1 200 OK"), "{page}");
        assert!(page.contains("text/html"));
        assert!(
            page.contains("const CODEC = 'avc1.640029'"),
            "codec filled in"
        );
        assert!(!page.contains("{{CODEC}}"));
        let body = r#"{"event":"playing"}"#;
        let r = get(
            server.port,
            &format!(
                "POST /tok/report HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        );
        assert!(r.starts_with("HTTP/1.1 204"), "{r}");
        assert!(hub.playing());
        let stats = r#"{"event":"stats","rate":1}"#;
        get(
            server.port,
            &format!(
                "POST /tok/report HTTP/1.1\r\nContent-Length: {}\r\n\r\n{stats}",
                stats.len()
            ),
        );
        assert_eq!(hub.stats_reports(), 1);
        let big = get(
            server.port,
            "POST /tok/report HTTP/1.1\r\nContent-Length: 999999\r\n\r\n",
        );
        assert!(big.starts_with("HTTP/1.1 413"), "{big}");
        // The live routes are not files: no reading the directory through them.
        assert!(get(server.port, "GET /tok/encoder.log HTTP/1.1\r\n\r\n").contains(" 404 "));
    }

    #[test]
    fn the_live_stream_is_the_init_then_fragments_from_a_keyframe_unending() {
        use crate::services::cast::fmp4::Piece;
        let hub = hub();
        let server = serve(Site::Live(hub.clone()), "tok".into(), 0, None).unwrap();
        let mut s = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        s.write_all(b"GET /tok/live.mp4 HTTP/1.1\r\n\r\n").unwrap();
        // Fragments keep coming, as from the encoder, until the viewer has
        // seen its keyframe.
        let feeder = {
            let hub = hub.clone();
            std::thread::spawn(move || {
                for n in 0..100u8 {
                    hub.publish(Piece::Fragment {
                        bytes: vec![b'P', n],
                        key: false,
                        at: 0.0,
                    });
                    if hub.viewers() > 0 {
                        hub.publish(Piece::Fragment {
                            bytes: b"KEY".to_vec(),
                            key: true,
                            at: 0.0,
                        });
                        hub.publish(Piece::Fragment {
                            bytes: b"NEXT".to_vec(),
                            key: false,
                            at: 0.0,
                        });
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            })
        };
        feeder.join().unwrap();
        let mut got = Vec::new();
        let mut chunk = [0u8; 512];
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        while !got.ends_with(b"NEXT") {
            let n = s.read(&mut chunk).unwrap();
            assert!(n > 0, "the stream ended early");
            got.extend_from_slice(&chunk[..n]);
        }
        let text = String::from_utf8_lossy(&got);
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("HTTP/1.1 200 OK"));
        assert!(head.contains("Content-Type: video/mp4"));
        assert!(
            !head.contains("Content-Length"),
            "an unending body has no length"
        );
        assert!(head.contains("Connection: close"));
        assert_eq!(body, "INITKEYNEXT", "init, then from the keyframe on");
        // The encoder ends: the response ends.
        hub.close();
        assert_eq!(s.read(&mut chunk).unwrap(), 0);
    }
}
