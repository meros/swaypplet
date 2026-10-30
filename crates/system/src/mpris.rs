//! What is playing, from the players themselves, over D-Bus.
//!
//! The bar's media mark used to ask `playerctl` every three seconds and on
//! every sway event, one process per field: seven or eight a poll, per bar,
//! to learn that the same song was still playing. That was a steady stream
//! of process starts and D-Bus connections on an idle desktop, and each poll
//! reloaded the album art into the bar, which made the bar lay itself out
//! and redraw, and the compositor draw it again.
//!
//! This is one connection for the process. It reads the players when it
//! starts and then only when one of them says something changed: a
//! `PropertiesChanged` on `/org/mpris/MediaPlayer2`, or a player arriving or
//! leaving the bus. The chosen player is the one playing, else the first,
//! which is what `playerctl` picks by default. Observers are told only when
//! the state they would draw changed.
//!
//! [`MprisService::control`] sends play/pause, next and previous to that
//! same player over the same connection, with no `playerctl` process.

use std::collections::HashMap;
use std::process::Command;
use std::rc::Rc;

use zbus::zvariant::OwnedValue;

use swaypplet_base::service::Observed;

#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaState {
    pub status: PlaybackStatus,
    pub artist: String,
    pub title: String,
    art_url: Option<String>,
    pub player_name: Option<String>,
    /// Track length in seconds (from mpris:length, which is in microseconds).
    pub length_secs: Option<f64>,
    /// Current position in seconds.
    pub position_secs: Option<f64>,
}

impl MediaState {
    /// Local album-art path for the bar media popover; remote URLs are
    /// skipped (would need fetch + cache), same rule as the panel section.
    pub fn art_path(&self) -> Option<String> {
        self.art_url.as_deref().and_then(resolve_art_path)
    }
}

pub fn playerctl(args: &[&str]) -> Option<String> {
    let out = Command::new("playerctl").args(args).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        None
    }
}

pub fn read_state() -> Option<MediaState> {
    let status_str = playerctl(&["status"])?;

    let status = match status_str.as_str() {
        "Playing" => PlaybackStatus::Playing,
        "Paused" => PlaybackStatus::Paused,
        _ => {
            let artist = playerctl(&["metadata", "artist"]).unwrap_or_default();
            let title = playerctl(&["metadata", "title"]).unwrap_or_default();
            if artist.is_empty() && title.is_empty() {
                return None;
            }
            PlaybackStatus::Paused
        }
    };

    let artist = playerctl(&["metadata", "artist"]).unwrap_or_default();
    let title = playerctl(&["metadata", "title"]).unwrap_or_default();

    // Album art URL — may be file:///path or https://...
    let art_url = playerctl(&["metadata", "mpris:artUrl"]).filter(|s| !s.is_empty());

    // Player identity (e.g. "Spotify", "firefox")
    let player_name =
        playerctl(&["metadata", "--format", "{{playerName}}"]).filter(|s| !s.is_empty());

    // Track length (mpris:length is in microseconds)
    let length_secs = playerctl(&["metadata", "mpris:length"])
        .and_then(|s| s.parse::<f64>().ok())
        .map(|us| us / 1_000_000.0)
        .filter(|&s| s > 0.0);

    // Current position in seconds
    let position_secs = playerctl(&["position"])
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|&s| s >= 0.0);

    Some(MediaState {
        status,
        artist,
        title,
        art_url,
        player_name,
        length_secs,
        position_secs,
    })
}

/// Resolve an art URL to a local file path for GTK.
/// - `file:///path` → `/path`
/// - Other URLs are ignored (would need HTTP fetch + cache)
fn resolve_art_path(url: &str) -> Option<String> {
    url.strip_prefix("file://").map(|path| path.to_string())
}

/// What a media button asks of the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    PlayPause,
    Next,
    Previous,
}

impl Control {
    /// The `org.mpris.MediaPlayer2.Player` method.
    fn method(self) -> &'static str {
        match self {
            Control::PlayPause => "PlayPause",
            Control::Next => "Next",
            Control::Previous => "Previous",
        }
    }
}

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

pub struct MprisService {
    state: Observed<Option<MediaState>>,
    controls: async_channel::Sender<Control>,
}

impl MprisService {
    pub fn start() -> Rc<MprisService> {
        let (controls, control_rx) = async_channel::unbounded::<Control>();
        let service = Rc::new(MprisService {
            state: Observed::new(None),
            controls,
        });
        let (tx, rx) = async_channel::unbounded::<Option<MediaState>>();
        swaypplet_base::spawn::spawn_tokio_thread("mpris", async move {
            if let Err(e) = follow(&tx, &control_rx).await {
                log::warn!("mpris: {e}");
            }
        });
        let this = service.clone();
        gtk4::glib::spawn_future_local(async move {
            while let Ok(state) = rx.recv().await {
                this.state.set_if_changed(state);
            }
        });
        service
    }

    pub fn snapshot(&self) -> Option<MediaState> {
        self.state.with(Clone::clone)
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.state.connect_change(cb);
    }

    /// Send `control` to the player [`snapshot`](Self::snapshot) shows.
    /// Returns at once; the player's own change signal brings the new
    /// state. Nothing happens when no player is on the bus.
    pub fn control(&self, control: Control) {
        if self.controls.try_send(control).is_err() {
            log::warn!("mpris: {control:?} dropped, the connection is gone");
        }
    }
}

async fn follow(
    tx: &async_channel::Sender<Option<MediaState>>,
    controls: &async_channel::Receiver<Control>,
) -> zbus::Result<()> {
    use zbus::export::futures_util::StreamExt;

    let conn = zbus::Connection::session().await?;
    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;

    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path(PATH)?
        .build();
    let mut changes = zbus::MessageStream::for_match_rule(rule, &conn, None).await?;
    // Filtered on the bus, not here: other clients connect and leave many
    // times a second, and each one would wake this thread for nothing.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg0ns("org.mpris.MediaPlayer2")?
        .build();
    let mut owners = zbus::MessageStream::for_match_rule(rule, &conn, None).await?;

    loop {
        let found = read(&conn, &dbus).await;
        let state = found.as_ref().map(|(_, s)| s.clone());
        // The bus name of the player this state came from: where a
        // control goes.
        let current = found.map(|(name, _)| name);
        if tx.send(state).await.is_err() {
            return Ok(());
        }
        loop {
            tokio::select! {
                Some(_) = changes.next() => break,
                Some(_) = owners.next() => break,
                Ok(control) = controls.recv() => {
                    if let Some(name) = &current {
                        send(&conn, name, control).await;
                    }
                }
                else => return Ok(()),
            }
        }
    }
}

/// Call `control`'s method on the player `name`. A failure is logged: the
/// player may have left between the read and the press.
async fn send(conn: &zbus::Connection, name: &str, control: Control) {
    let sent = conn
        .call_method(Some(name), PATH, Some(PLAYER), control.method(), &())
        .await;
    if let Err(e) = sent {
        log::warn!("mpris: {} on {name}: {e}", control.method());
    }
}

/// The state of the player that counts, with its bus name: the first one
/// playing, else the first one there is. `None` when no player is on the
/// bus.
async fn read(
    conn: &zbus::Connection,
    dbus: &zbus::fdo::DBusProxy<'_>,
) -> Option<(String, MediaState)> {
    let names = dbus.list_names().await.ok()?;
    let mut players: Vec<String> = names
        .iter()
        .map(|n| n.to_string())
        .filter(|n| n.starts_with(PREFIX))
        .collect();
    players.sort();

    let mut first = None;
    for name in players {
        let Some(props) = player_props(conn, &name).await else {
            continue;
        };
        let Some(state) = parse(&name, &props) else {
            continue;
        };
        if state.status == PlaybackStatus::Playing {
            return Some((name, state));
        }
        if first.is_none() {
            first = Some((name, state));
        }
    }
    first
}

async fn player_props(conn: &zbus::Connection, name: &str) -> Option<HashMap<String, OwnedValue>> {
    let props = zbus::fdo::PropertiesProxy::builder(conn)
        .destination(name.to_string())
        .ok()?
        .path(PATH)
        .ok()?
        .build()
        .await
        .ok()?;
    let iface = zbus::names::InterfaceName::from_static_str_unchecked(PLAYER);
    props.get_all(Some(iface).into()).await.ok()
}

/// A player's properties as the bar draws them. `None` for a player that is
/// stopped and has nothing loaded, as `playerctl` treats it.
fn parse(name: &str, props: &HashMap<String, OwnedValue>) -> Option<MediaState> {
    let status = props
        .get("PlaybackStatus")
        .and_then(|v| <&str>::try_from(v).ok())
        .unwrap_or("Stopped");
    let meta: HashMap<String, OwnedValue> = props
        .get("Metadata")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| HashMap::<String, OwnedValue>::try_from(v).ok())
        .unwrap_or_default();
    let text = |key: &str| {
        meta.get(key)
            .and_then(|v| <&str>::try_from(v).ok())
            .map(str::to_string)
            .unwrap_or_default()
    };
    let title = text("xesam:title");
    let artist = meta
        .get("xesam:artist")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .map(|a| a.join(", "))
        .unwrap_or_default();
    let art_url = Some(text("mpris:artUrl")).filter(|s| !s.is_empty());
    let length_secs = meta
        .get("mpris:length")
        .and_then(|v| {
            i64::try_from(v)
                .ok()
                .or_else(|| u64::try_from(v).ok().map(|u| u as i64))
        })
        .map(|us| us as f64 / 1_000_000.0)
        .filter(|s| *s > 0.0);

    let status = match status {
        "Playing" => PlaybackStatus::Playing,
        "Paused" => PlaybackStatus::Paused,
        _ if title.is_empty() && artist.is_empty() => return None,
        _ => PlaybackStatus::Paused,
    };
    let player = name.strip_prefix(PREFIX).map(|p| {
        // `spotify`, or `firefox.instance_1_23` → `firefox`, as playerctl
        // names it.
        p.split('.').next().unwrap_or(p).to_string()
    });
    // No position: MPRIS does not signal it, and the bar does not show it.
    Some(MediaState {
        status,
        artist,
        title,
        art_url,
        player_name: player,
        length_secs,
        position_secs: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_control_is_its_mpris_method() {
        assert_eq!(Control::PlayPause.method(), "PlayPause");
        assert_eq!(Control::Next.method(), "Next");
        assert_eq!(Control::Previous.method(), "Previous");
    }
}

#[cfg(test)]
mod live {
    //! Against the real session bus. Ignored: needs a player running.

    #[test]
    #[ignore]
    fn reads_the_player_that_counts() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let conn = zbus::Connection::session().await.unwrap();
            let dbus = zbus::fdo::DBusProxy::new(&conn).await.unwrap();
            println!("{:?}", super::read(&conn, &dbus).await);
        });
    }
}
