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
//! the state they would draw changed; the position is read along with the
//! rest, so a reader that does not show it drops it before comparing
//! ([`MediaState::without_position`]).
//!
//! [`MprisService::control`] sends play/pause, next and previous to that
//! same player over the same connection, with no `playerctl` process.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

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
    /// The position in seconds when this state was read. MPRIS does not
    /// signal the position as it advances, only when it jumps (`Seeked`), so
    /// a reader that shows it counts on from here while the player plays.
    pub position_secs: Option<f64>,
}

impl MediaState {
    /// Local album-art path for the bar media popover; remote URLs are
    /// skipped (would need fetch + cache), same rule as the panel section.
    pub fn art_path(&self) -> Option<String> {
        self.art_url.as_deref().and_then(resolve_art_path)
    }

    /// The state without its position, for a reader that does not show
    /// one: a read that only moved the position then compares equal.
    pub fn without_position(self) -> MediaState {
        MediaState {
            position_secs: None,
            ..self
        }
    }

    /// The title to show, with a stand-in for a player that names none.
    pub fn display_title(&self) -> &str {
        if self.title.is_empty() {
            "Unknown track"
        } else {
            &self.title
        }
    }

    /// One line for the bar's pill: "Title · Artist", or whichever half
    /// exists.
    pub fn pill_text(&self) -> String {
        match (self.title.trim(), self.artist.trim()) {
            ("", "") => "♪".to_owned(),
            (title, "") => title.to_owned(),
            ("", artist) => artist.to_owned(),
            (title, artist) => format!("{} · {}", title, artist),
        }
    }

    /// The player's name as a badge shows it: `spotify` → `Spotify`.
    pub fn player_label(&self) -> Option<String> {
        self.player_name.as_deref().map(capitalize)
    }
}

/// `m:ss` for a position or a length in seconds.
pub fn format_time(secs: f64) -> String {
    let total = secs.round() as u64;
    let m = total / 60;
    let s = total % 60;
    format!("{}:{:02}", m, s)
}

/// How far through the track `pos` is, as a bar fraction and as
/// "0:42 / 3:15".
pub fn progress(pos: f64, len: f64) -> (f64, String) {
    (
        (pos / len).clamp(0.0, 1.0),
        format!("{} / {}", format_time(pos), format_time(len)),
    )
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
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

/// Where `state`'s player is `elapsed` after the state was read: counted on
/// while it plays, up to the track's end; where it was otherwise.
fn position_after(state: &MediaState, elapsed: Duration) -> Option<f64> {
    let pos = state.position_secs?;
    if state.status != PlaybackStatus::Playing {
        return Some(pos);
    }
    let pos = pos + elapsed.as_secs_f64();
    Some(state.length_secs.map_or(pos, |len| pos.min(len)))
}

pub struct MprisService {
    state: Observed<Option<MediaState>>,
    controls: async_channel::Sender<Control>,
    /// When `state` was last read, which is when its position was true.
    read_at: Cell<Instant>,
}

impl MprisService {
    pub fn start() -> Rc<MprisService> {
        let (controls, control_rx) = async_channel::unbounded::<Control>();
        let service = Rc::new(MprisService {
            state: Observed::new(None),
            controls,
            read_at: Cell::new(Instant::now()),
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
                // Before the observers hear of it, so they count from now.
                this.read_at.set(Instant::now());
                this.state.set_if_changed(state);
            }
        });
        service
    }

    pub fn snapshot(&self) -> Option<MediaState> {
        self.state.with(Clone::clone)
    }

    /// The player's position now, in seconds. Nothing is asked of the
    /// player: this counts on from the last read, which a seek, a pause or
    /// a new track refreshes.
    pub fn position(&self) -> Option<f64> {
        let elapsed = self.read_at.get().elapsed();
        self.state
            .with(|state| state.as_ref().and_then(|s| position_after(s, elapsed)))
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
    // The position is not a property that signals; a jump in it is.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(PLAYER)?
        .member("Seeked")?
        .path(PATH)?
        .build();
    let mut seeks = zbus::MessageStream::for_match_rule(rule, &conn, None).await?;

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
                Some(_) = seeks.next() => break,
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
    let position_secs = props
        .get("Position")
        .and_then(|v| i64::try_from(v).ok())
        .map(|us| us as f64 / 1_000_000.0)
        .filter(|s| *s >= 0.0);

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
    Some(MediaState {
        status,
        artist,
        title,
        art_url,
        player_name: player,
        length_secs,
        position_secs,
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

    fn track(title: &str, artist: &str) -> MediaState {
        MediaState {
            status: PlaybackStatus::Playing,
            artist: artist.into(),
            title: title.into(),
            art_url: None,
            player_name: None,
            length_secs: None,
            position_secs: Some(12.0),
        }
    }

    #[test]
    fn the_pill_says_whichever_half_there_is() {
        assert_eq!(track("Song", "Band").pill_text(), "Song · Band");
        assert_eq!(track("Song", " ").pill_text(), "Song");
        assert_eq!(track("", "Band").pill_text(), "Band");
        assert_eq!(track("", "").pill_text(), "♪");
    }

    #[test]
    fn a_track_without_a_title_gets_a_stand_in() {
        assert_eq!(track("", "Band").display_title(), "Unknown track");
        assert_eq!(track("Song", "").display_title(), "Song");
    }

    #[test]
    fn the_badge_capitalizes_the_player() {
        let mut ms = track("Song", "Band");
        assert_eq!(ms.player_label(), None);
        ms.player_name = Some("spotify".into());
        assert_eq!(ms.player_label().as_deref(), Some("Spotify"));
        ms.player_name = Some(String::new());
        assert_eq!(ms.player_label().as_deref(), Some(""));
    }

    #[test]
    fn times_read_as_minutes_and_seconds() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(59.6), "1:00");
        assert_eq!(format_time(195.0), "3:15");
        assert_eq!(format_time(3725.0), "62:05");
    }

    #[test]
    fn progress_is_a_clamped_fraction_and_both_times() {
        assert_eq!(progress(42.0, 195.0).1, "0:42 / 3:15");
        assert_eq!(progress(97.5, 195.0).0, 0.5);
        assert_eq!(progress(200.0, 195.0).0, 1.0);
    }

    #[test]
    fn the_position_counts_on_only_while_playing_and_stops_at_the_end() {
        let mut ms = track("Song", "Band");
        ms.length_secs = Some(13.0);
        assert_eq!(position_after(&ms, Duration::from_millis(500)), Some(12.5));
        assert_eq!(position_after(&ms, Duration::from_secs(5)), Some(13.0));
        ms.status = PlaybackStatus::Paused;
        assert_eq!(position_after(&ms, Duration::from_secs(5)), Some(12.0));
        ms.position_secs = None;
        assert_eq!(position_after(&ms, Duration::from_secs(5)), None);
    }

    #[test]
    fn a_state_without_its_position_equals_one_read_later() {
        let a = track("Song", "Band");
        let b = MediaState {
            position_secs: Some(40.0),
            ..a.clone()
        };
        assert_ne!(a, b);
        assert_eq!(a.without_position(), b.without_position());
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
