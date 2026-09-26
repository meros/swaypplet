//! Tailscale, through its own CLI.
//!
//! `tailscaled` is not a NetworkManager connection: its tunnel shows up as
//! an unmanaged `tailscale0` device, which the adapter list filters out, so
//! until now the panel could not tell whether the tailnet was up or which
//! exit node carried the traffic. The CLI's `status --json` is the
//! supported read; `up`, `down` and `set --exit-node` the supported writes
//! (they need the user to be the daemon's operator, which the error says).

use std::process::Command;
use std::time::Duration;

/// What the section shows about the tailnet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// `Running`, `Stopped`, `NeedsLogin`, `Starting`, …
    pub state: String,
    pub tailnet: Option<String>,
    pub self_ip: Option<String>,
    pub self_name: Option<String>,
    /// The peer carrying all traffic, by name.
    pub exit_node: Option<String>,
    /// Peers that offer themselves as exit nodes.
    pub exit_options: Vec<Peer>,
    pub peers_online: usize,
    pub peers_total: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Peer {
    pub name: String,
    pub ip: String,
    pub online: bool,
}

impl Status {
    pub fn running(&self) -> bool {
        self.state == "Running"
    }
}

/// The first label of a MagicDNS name, or the host name.
fn short(dns: &str, host: &str) -> String {
    dns.split('.')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(host)
        .to_string()
}

/// `tailscale status --json`, as a [`Status`]. Pure.
pub fn parse(json: &str) -> Option<Status> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let str_of = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let me = v.get("Self");
    let mut s = Status {
        state: str_of(&v, "BackendState").unwrap_or_default(),
        tailnet: v
            .get("CurrentTailnet")
            .and_then(|t| str_of(t, "Name"))
            .filter(|n| !n.is_empty()),
        self_ip: me
            .and_then(|m| m.get("TailscaleIPs"))
            .and_then(|a| a.get(0))
            .and_then(|x| x.as_str())
            .map(str::to_string),
        self_name: me.map(|m| {
            short(
                &str_of(m, "DNSName").unwrap_or_default(),
                &str_of(m, "HostName").unwrap_or_default(),
            )
        }),
        ..Default::default()
    };
    if let Some(peers) = v.get("Peer").and_then(|p| p.as_object()) {
        for p in peers.values() {
            let online = p.get("Online").and_then(|x| x.as_bool()).unwrap_or(false);
            let name = short(
                &str_of(p, "DNSName").unwrap_or_default(),
                &str_of(p, "HostName").unwrap_or_default(),
            );
            s.peers_total += 1;
            if online {
                s.peers_online += 1;
            }
            if p.get("ExitNode").and_then(|x| x.as_bool()) == Some(true) {
                s.exit_node = Some(name.clone());
            }
            if p.get("ExitNodeOption").and_then(|x| x.as_bool()) == Some(true) {
                let ip = p
                    .get("TailscaleIPs")
                    .and_then(|a| a.get(0))
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string();
                s.exit_options.push(Peer { name, ip, online });
            }
        }
    }
    s.exit_options.sort_by(|a, b| b.online.cmp(&a.online).then(a.name.cmp(&b.name)));
    Some(s)
}

/// The CLI, if there is one, with a short leash: a wedged daemon must not
/// hold a worker thread.
fn run(args: &[&str]) -> Result<String, String> {
    let mut child = Command::new("tailscale")
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("tailscale: {e}"))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(40));
            }
            _ => {
                let _ = child.kill();
                return Err("tailscale did not answer".into());
            }
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().next().unwrap_or("tailscale failed").to_string())
    }
}

/// The tailnet's status; `None` without the CLI or the daemon.
pub fn status() -> Option<Status> {
    parse(&run(&["status", "--json"]).ok()?)
}

/// Route all traffic through `peer_ip`, or stop (`None`).
pub fn set_exit_node(peer_ip: Option<&str>) -> Result<(), String> {
    let arg = format!("--exit-node={}", peer_ip.unwrap_or(""));
    run(&["set", &arg]).map(|_| ())
}

pub fn set_up(up: bool) -> Result<(), String> {
    run(&[if up { "up" } else { "down" }]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "BackendState": "Running",
      "CurrentTailnet": {"Name": "example.github"},
      "Self": {"HostName": "laptop", "DNSName": "laptop.tail1.ts.net.",
               "TailscaleIPs": ["100.1.2.3", "fd7a::1"]},
      "Peer": {
        "a": {"HostName": "server", "DNSName": "server.tail1.ts.net.", "Online": true,
              "ExitNodeOption": true, "ExitNode": true, "TailscaleIPs": ["100.1.2.4"]},
        "b": {"HostName": "phone", "DNSName": "phone.tail1.ts.net.", "Online": false,
              "ExitNodeOption": false, "TailscaleIPs": ["100.1.2.5"]},
        "c": {"HostName": "box", "DNSName": "", "Online": false,
              "ExitNodeOption": true, "TailscaleIPs": ["100.1.2.6"]}
      }
    }"#;

    #[test]
    fn the_status_reads_back() {
        let s = parse(SAMPLE).unwrap();
        assert!(s.running());
        assert_eq!(s.tailnet.as_deref(), Some("example.github"));
        assert_eq!(s.self_ip.as_deref(), Some("100.1.2.3"));
        assert_eq!(s.self_name.as_deref(), Some("laptop"));
        assert_eq!(s.exit_node.as_deref(), Some("server"));
        assert_eq!((s.peers_online, s.peers_total), (1, 3));
        // Online exit nodes first; a peer with no MagicDNS name by host name.
        let names: Vec<&str> = s.exit_options.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["server", "box"]);
    }

    #[test]
    fn a_stopped_daemon_is_not_running() {
        let s = parse(r#"{"BackendState": "Stopped"}"#).unwrap();
        assert!(!s.running());
        assert!(s.exit_options.is_empty());
        assert!(parse("not json").is_none());
    }
}
