//! Greeter-side client for the root `swaypplet fp-agent` daemon, kept next to
//! the server so the two ends of the protocol live in one file pair.
//!
//! Worker model mirrors `lock::fprint`: a std::thread runs a current-thread
//! tokio runtime and pushes protocol events over an async channel, which the
//! GTK side awaits. Commands (retarget/stop) come in over a tokio unbounded
//! channel, which is safe to send to from the GTK thread.
//!
//! The worker reconnects with backoff while a verify target is desired, and
//! re-announces the target after every (re)connect, so an fp-agent restart
//! or a not-yet-up socket degrades to "fingerprint unavailable" and heals
//! itself — password auth is never affected.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;

use super::{Cmd, Ev, sock_path};

/// Start the client. Send it [`Cmd`]s; await its [`Ev`]s.
pub fn start() -> (
    tokio::sync::mpsc::UnboundedSender<Cmd>,
    async_channel::Receiver<Ev>,
) {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
    let (ev_tx, ev_rx) = async_channel::unbounded();
    swaypplet_base::spawn::spawn_tokio_thread("fp-agent-client", run(cmd_rx, ev_tx));
    (cmd_tx, ev_rx)
}

async fn run(mut cmds: tokio::sync::mpsc::UnboundedReceiver<Cmd>, evs: async_channel::Sender<Ev>) {
    let path = sock_path();
    let mut desired: Option<String> = None;
    let mut reported_down = false;
    loop {
        // Nothing to verify — sit on the command channel.
        while desired.is_none() {
            match cmds.recv().await {
                Some(Cmd::Verify { user }) => desired = Some(user),
                Some(Cmd::Stop) => {}
                None => return,
            }
        }

        let stream = match UnixStream::connect(&path).await {
            Ok(s) => s,
            Err(e) => {
                if !reported_down {
                    reported_down = true;
                    log::warn!("fp-agent unreachable at {path}: {e}");
                    if evs
                        .send(Ev::Unavailable {
                            msg: format!("fp-agent: {e}"),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                    cmd = cmds.recv() => match cmd {
                        Some(Cmd::Verify { user }) => desired = Some(user),
                        Some(Cmd::Stop) => desired = None,
                        None => return,
                    }
                }
                continue;
            }
        };
        reported_down = false;
        let (r, mut w) = stream.into_split();
        let mut lines = BufReader::new(r).lines();

        let announce = Cmd::Verify {
            user: desired.clone().unwrap_or_default(),
        };
        if send_cmd(&mut w, &announce).await.is_err() {
            continue;
        }

        loop {
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(l)) if l.trim().is_empty() => {}
                    Ok(Some(l)) => match serde_json::from_str::<Ev>(l.trim()) {
                        Ok(ev) => {
                            if evs.send(ev).await.is_err() {
                                return; // GTK side gone — process is exiting
                            }
                        }
                        Err(e) => log::warn!("bad fp-agent event: {e}"),
                    },
                    Ok(None) | Err(_) => {
                        // Agent went away — report, back off, reconnect.
                        reported_down = true;
                        if evs.send(Ev::Unavailable { msg: "fp-agent connection lost".into() }).await.is_err() {
                            return;
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        break;
                    }
                },
                cmd = cmds.recv() => {
                    let Some(cmd) = cmd else { return };
                    match &cmd {
                        Cmd::Verify { user } => desired = Some(user.clone()),
                        Cmd::Stop => desired = None,
                    }
                    if send_cmd(&mut w, &cmd).await.is_err() {
                        break; // reconnect re-announces `desired`
                    }
                }
            }
        }
    }
}

async fn send_cmd(w: &mut OwnedWriteHalf, cmd: &Cmd) -> std::io::Result<()> {
    let mut line = serde_json::to_string(cmd)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    w.write_all(line.as_bytes()).await
}
