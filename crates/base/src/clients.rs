//! The messaging clients the bar's Messages cluster knows, and the model
//! that says what each one is doing.
//!
//! A client is "running" when a window of it is in the sway tree or its
//! StatusNotifier item is on the bus. Slack and Discord hide to the tray when
//! their window closes, so the window alone says nothing; Teams is a Chrome
//! PWA with no tray item of its own, so the tray alone says nothing either.
//!
//! Where the unread count comes from, best evidence first:
//!
//! 1. The tray item's tool tip ("You have 3 notifications"). Slack sets it,
//!    and it is the only source that still works with no window open.
//! 2. The window title: a leading `(3)`, or Slack's `3 new items`.
//! 3. Notifications that arrived since the window was last focused. A toast
//!    that expires leaves the history, so this keeps the notification ids it
//!    has seen instead of reading the history back.
//! 4. A marker in the title (`*`, `!`) with no number: a dot.
//!
//! No GTK here, so all of it is unit tests. Nothing in this module reads the
//! desktop; the bar feeds it plain data.

use std::collections::{HashMap, HashSet};

/// One messaging application the cluster can show.
#[derive(Debug, PartialEq, Eq)]
pub struct Client {
    pub id: &'static str,
    pub name: &'static str,
    /// Sway `app_id`, XWayland class or instance. Lowercase. An entry that
    /// ends in `-` is a prefix (Chrome names a PWA's `app_id` after its
    /// extension id and profile), any other entry is the whole name.
    pub window_ids: &'static [&'static str],
    /// StatusNotifier `Id` prefixes, lowercase.
    pub sni_prefixes: &'static [&'static str],
    /// Substrings of a notification's `app_name` or `desktop-entry`, lowercase.
    pub notify_names: &'static [&'static str],
    /// The `.desktop` file that starts the client, without the suffix.
    pub desktop_id: &'static str,
}

/// The table. A client that is not installed is simply never drawn.
pub const CLIENTS: &[Client] = &[
    Client {
        id: "slack",
        name: "Slack",
        window_ids: &["slack"],
        sni_prefixes: &["slack"],
        notify_names: &["slack"],
        desktop_id: "slack",
    },
    Client {
        id: "teams",
        name: "Teams",
        // The Chrome PWA on this desktop (`cifhbcnohmdccbgoicgdjpfamggdegmo`),
        // and the Electron wrapper for anyone who runs that instead.
        window_ids: &[
            "chrome-cifhbcnohmdccbgoicgdjpfamggdegmo-",
            "teams-for-linux",
        ],
        sni_prefixes: &["teams"],
        notify_names: &["teams", "cifhbcnohmdccbgoicgdjpfamggdegmo"],
        desktop_id: "chrome-cifhbcnohmdccbgoicgdjpfamggdegmo-Profile_1",
    },
    Client {
        id: "discord",
        name: "Discord",
        window_ids: &["discord", "vesktop", "legcord"],
        sni_prefixes: &["discord", "vesktop", "legcord"],
        notify_names: &["discord", "vesktop", "legcord"],
        desktop_id: "discord",
    },
    Client {
        id: "telegram",
        name: "Telegram",
        window_ids: &["org.telegram.desktop", "telegramdesktop"],
        sni_prefixes: &["telegram"],
        notify_names: &["telegram"],
        desktop_id: "org.telegram.desktop",
    },
    Client {
        id: "messenger",
        name: "Messenger",
        // A Chrome app window on messenger.com. Chrome names its `app_id`
        // after the address, `chrome-www.messenger.com__-Profile_1`; the
        // desktop file is in the nixos repo (chrome-profiles.nix).
        window_ids: &["chrome-www.messenger.com__-"],
        sni_prefixes: &[],
        notify_names: &["messenger"],
        desktop_id: "messenger-pwa",
    },
];

fn lower(s: &str) -> String {
    s.to_lowercase()
}

fn window_id_matches(pattern: &str, id: &str) -> bool {
    if pattern.ends_with('-') {
        id.starts_with(pattern)
    } else {
        id == pattern
    }
}

/// The client a sway window belongs to. Wayland windows have an `app_id`;
/// XWayland ones have a class and an instance instead.
pub fn client_for_window(
    app_id: Option<&str>,
    class: Option<&str>,
    instance: Option<&str>,
) -> Option<&'static Client> {
    let ids: Vec<String> = [app_id, class, instance]
        .into_iter()
        .flatten()
        .map(lower)
        .collect();
    CLIENTS.iter().find(|c| {
        c.window_ids
            .iter()
            .any(|p| ids.iter().any(|id| window_id_matches(p, id)))
    })
}

/// The client a StatusNotifier item belongs to, by its `Id`.
pub fn client_for_sni(id: &str) -> Option<&'static Client> {
    let id = lower(id);
    CLIENTS
        .iter()
        .find(|c| c.sni_prefixes.iter().any(|p| id.starts_with(p)))
}

/// The client a notification came from, by what it says about itself.
pub fn client_for_notification(
    app_name: &str,
    desktop_entry: Option<&str>,
) -> Option<&'static Client> {
    let name = lower(app_name);
    let entry = desktop_entry.map(lower).unwrap_or_default();
    CLIENTS.iter().find(|c| {
        c.notify_names
            .iter()
            .any(|n| name.contains(n) || entry.contains(n))
    })
}

pub fn client_by_id(id: &str) -> Option<&'static Client> {
    CLIENTS.iter().find(|c| c.id == id)
}

// ── Reading the numbers ─────────────────────────────────────────────────

/// The first run of digits in `s`.
fn first_number(s: &str) -> Option<u32> {
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let digits: String = s[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// "You have 3 notifications" → 3. "0 notifications" is `Some(0)`: the app
/// says nothing is waiting, which outranks a stale title or a missed count.
/// Only a tool tip that talks about
/// notifications, messages or unread items counts, so a version number in
/// some other tip is not read as a count.
pub fn tooltip_count(tip: &str) -> Option<u32> {
    let lowered = lower(tip);
    let about_messages = ["notification", "unread", "message", "mention"]
        .iter()
        .any(|w| lowered.contains(w));
    if !about_messages {
        return None;
    }
    first_number(&lowered)
}

/// A count in a window title: a leading `(3)` as Discord and web apps write
/// it, or Slack's "3 new items". `None` for anything else.
pub fn title_count(title: &str) -> Option<u32> {
    let t = title.trim_start_matches(['*', '!', '•', '●', ' ']);
    if let Some(rest) = t.strip_prefix('(')
        && let Some(end) = rest.find(')')
        && let Ok(n) = rest[..end].trim().parse::<u32>()
    {
        return Some(n);
    }
    let lowered = lower(title);
    let at = lowered.find(" new item")?;
    // The number just before " new item". Trimming from the end never cuts
    // inside a character, which a title from a channel name (café, Ålesund)
    // would otherwise do.
    let before = lowered[..at].trim_end();
    let digits = before.len() - before.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    before[before.len() - digits..].parse().ok()
}

/// A title that begins with an unread marker, with no number to read.
pub fn title_marker(title: &str) -> bool {
    title.starts_with(['*', '!', '•', '●'])
}

// ── What the bar feeds in ───────────────────────────────────────────────

/// A tray item's standing. Ordered: a stale `Passive` item loses to an
/// `Active` one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SniStatus {
    Passive,
    Active,
    NeedsAttention,
}

/// One StatusNotifier item that belongs to a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SniPresence {
    pub client: &'static str,
    /// The bus address the tray service activates it by.
    pub address: String,
    pub status: SniStatus,
    pub tooltip: String,
}

/// One sway window that belongs to a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientWindow {
    pub client: &'static str,
    pub con_id: i64,
    pub title: String,
    /// The workspace to switch to, when it is one a person can switch to.
    pub workspace: Option<String>,
    pub focused: bool,
    /// Hidden in sway's scratchpad: shown by con_id, with no workspace switch.
    pub scratchpad: bool,
}

/// The number of the bus name in `:1.98324`, for ordering. A name that is
/// not of that form sorts first.
fn bus_serial(address: &str) -> u64 {
    address
        .strip_prefix(':')
        .and_then(|rest| rest.split('/').next())
        .and_then(|name| name.rsplit('.').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

// ── Notifications seen since the window was last in front ───────────────

/// Notification ids seen per client. An id set rather than a counter, so a
/// notification that replaces another (same id) is counted once. Dismissing
/// a card never lowers it: only looking at the client does.
#[derive(Debug, Default, Clone)]
pub struct Missed(HashMap<&'static str, HashSet<u32>>);

impl Missed {
    pub fn record(&mut self, client: &'static str, notification_id: u32) {
        self.0.entry(client).or_default().insert(notification_id);
    }

    pub fn clear(&mut self, client: &str) {
        self.0.remove(client);
    }

    pub fn count(&self, client: &str) -> u32 {
        self.0
            .get(client)
            .map_or(0, |ids| u32::try_from(ids.len()).unwrap_or(u32::MAX))
    }
}

// ── The merge ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presence {
    /// A window exists. `con_id` is the one to bring forward.
    Open {
        con_id: i64,
        workspace: Option<String>,
        focused: bool,
        scratchpad: bool,
    },
    /// In the tray with no window. `address` is where to send Activate.
    Background { address: String },
    /// Nothing runs; the chip exists only because messages are waiting.
    Gone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unread {
    None,
    /// Something is waiting and nothing says how many.
    Dot,
    Count(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub client: &'static Client,
    pub presence: Presence,
    pub unread: Unread,
}

/// One chip per client that runs or has something waiting, in table order.
///
/// A focused window means the person is looking, so it shows no unread: the
/// caller clears [`Missed`] for it in the same step.
pub fn merge(sni: &[SniPresence], windows: &[ClientWindow], missed: &Missed) -> Vec<Chip> {
    CLIENTS
        .iter()
        .filter_map(|client| {
            let mine_win: Vec<&ClientWindow> =
                windows.iter().filter(|w| w.client == client.id).collect();
            // Newest process first among the same status: two Slack
            // processes can be on the bus at once, and the older one may be
            // a leftover that ignores Activate.
            let item = sni
                .iter()
                .filter(|s| s.client == client.id)
                .max_by_key(|s| (s.status, bus_serial(&s.address)));

            let focus = mine_win.iter().any(|w| w.focused);
            let pick = mine_win
                .iter()
                .find(|w| w.focused)
                .or_else(|| mine_win.iter().find(|w| !w.scratchpad))
                .or_else(|| mine_win.first());

            let unread = if focus {
                Unread::None
            } else {
                let numbered = item
                    .and_then(|s| tooltip_count(&s.tooltip))
                    .or_else(|| mine_win.iter().filter_map(|w| title_count(&w.title)).max())
                    .or_else(|| Some(missed.count(client.id)).filter(|n| *n > 0));
                match numbered {
                    Some(n) if n > 0 => Unread::Count(n),
                    _ if mine_win.iter().any(|w| title_marker(&w.title)) => Unread::Dot,
                    _ => Unread::None,
                }
            };

            let presence = match (pick, item) {
                (Some(w), _) => Presence::Open {
                    con_id: w.con_id,
                    workspace: w.workspace.clone(),
                    focused: w.focused,
                    scratchpad: w.scratchpad,
                },
                (None, Some(s)) => Presence::Background {
                    address: s.address.clone(),
                },
                (None, None) if unread != Unread::None => Presence::Gone,
                (None, None) => return None,
            };
            Some(Chip {
                client,
                presence,
                unread,
            })
        })
        .collect()
}

/// The text a badge shows: the number, capped at 99+.
pub fn badge_text(n: u32) -> String {
    if n > 99 {
        "99+".to_string()
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(client: &'static str, title: &str, focused: bool) -> ClientWindow {
        ClientWindow {
            client,
            con_id: 7,
            title: title.into(),
            workspace: Some("38".into()),
            focused,
            scratchpad: false,
        }
    }

    fn item(client: &'static str, address: &str, status: SniStatus, tip: &str) -> SniPresence {
        SniPresence {
            client,
            address: address.into(),
            status,
            tooltip: tip.into(),
        }
    }

    #[test]
    fn windows_match_by_app_id_class_and_prefix() {
        assert_eq!(
            client_for_window(Some("slack"), None, None).unwrap().id,
            "slack"
        );
        assert_eq!(
            client_for_window(Some("Slack"), None, None).unwrap().id,
            "slack"
        );
        // XWayland Slack has no app_id, only class and instance.
        assert_eq!(
            client_for_window(None, Some("Slack"), Some("slack"))
                .unwrap()
                .id,
            "slack"
        );
        assert_eq!(
            client_for_window(
                Some("chrome-cifhbcnohmdccbgoicgdjpfamggdegmo-Profile_1"),
                None,
                None
            )
            .unwrap()
            .id,
            "teams"
        );
        // Another PWA is not Teams, and a name that only contains one is not a match.
        assert!(
            client_for_window(
                Some("chrome-faolnafnngnfdaknnbpnkhgohbobgegn-Profile_1"),
                None,
                None
            )
            .is_none()
        );
        assert!(client_for_window(Some("slackware-docs"), None, None).is_none());
        assert!(client_for_window(Some("firefox"), None, None).is_none());
    }

    #[test]
    fn messenger_is_the_chrome_app_window_on_messenger_dot_com() {
        // The `app_id` Chrome gives `--app=https://www.messenger.com/`.
        let id = |app_id| client_for_window(Some(app_id), None, None).map(|c| c.id);
        assert_eq!(
            id("chrome-www.messenger.com__-Profile_1"),
            Some("messenger")
        );
        assert_eq!(id("chrome-www.messenger.com__-Default"), Some("messenger"));
        // A tab of the same site in a normal browser window is not the app.
        assert_eq!(id("google-chrome"), None);
        assert_eq!(id("chrome-www.facebook.com__-Profile_1"), None);
    }

    #[test]
    fn tray_ids_and_notifications_match() {
        assert_eq!(client_for_sni("Slack_status_icon_1").unwrap().id, "slack");
        assert_eq!(client_for_sni("TelegramDesktop").unwrap().id, "telegram");
        assert!(client_for_sni("chrome_status_icon_1").is_none());
        assert_eq!(client_for_notification("Slack", None).unwrap().id, "slack");
        assert_eq!(
            client_for_notification(
                "Google Chrome",
                Some("chrome-cifhbcnohmdccbgoicgdjpfamggdegmo-Profile_1")
            )
            .unwrap()
            .id,
            "teams"
        );
        assert_eq!(
            client_for_notification("Microsoft Teams", None).unwrap().id,
            "teams"
        );
        assert!(client_for_notification("Google Chrome", Some("google-chrome")).is_none());
        assert!(client_for_notification("Claude Code", None).is_none());
    }

    #[test]
    fn reads_counts() {
        assert_eq!(tooltip_count("You have 1 notification"), Some(1));
        assert_eq!(tooltip_count("You have 12 notifications"), Some(12));
        assert_eq!(tooltip_count("Slack 4.51"), None);
        assert_eq!(tooltip_count(""), None);
        assert_eq!(title_count("(3) Discord | #general"), Some(3));
        assert_eq!(title_count("(2) Chat | Microsoft Teams"), Some(2));
        assert_eq!(
            title_count("! Activity - Frilansare Sverige - 1 new item - Slack"),
            Some(1)
        );
        assert_eq!(
            title_count("* general - Acme - 12 new items - Slack"),
            Some(12)
        );
        assert_eq!(title_count("Chat | Microsoft Teams"), None);
        assert_eq!(title_count("(a) thing"), None);
        // Titles are other people's words: no slicing inside a character.
        assert_eq!(title_count("Ålesund café new items - Slack"), None);
        assert_eq!(title_count("café 4 new items - Slack"), Some(4));
        assert_eq!(title_count("ÅÄÖ new item"), None);
        assert!(title_marker("* general - Slack"));
        assert!(title_marker("! Activity - Slack"));
        assert!(!title_marker("general - Slack"));
    }

    #[test]
    fn missed_counts_ids_once_and_clears_on_look() {
        let mut m = Missed::default();
        m.record("teams", 4);
        m.record("teams", 4); // a replacement keeps the id
        m.record("teams", 5);
        assert_eq!(m.count("teams"), 2);
        m.clear("teams");
        assert_eq!(m.count("teams"), 0);
    }

    #[test]
    fn slack_in_the_tray_shows_the_tooltip_count_with_no_window() {
        let sni = [
            item(
                "slack",
                ":1.6772/StatusNotifierItem",
                SniStatus::Active,
                "You have 1 notification",
            ),
            item(
                "slack",
                ":1.98324/StatusNotifierItem",
                SniStatus::Active,
                "You have 1 notification",
            ),
        ];
        let chips = merge(&sni, &[], &Missed::default());
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].client.id, "slack");
        assert_eq!(chips[0].unread, Unread::Count(1));
        // Two processes: the newest bus name is the one to activate.
        assert_eq!(
            chips[0].presence,
            Presence::Background {
                address: ":1.98324/StatusNotifierItem".into()
            }
        );
    }

    #[test]
    fn an_active_item_beats_a_newer_passive_one() {
        let sni = [
            item("slack", ":1.100", SniStatus::Active, ""),
            item("slack", ":1.200", SniStatus::Passive, ""),
        ];
        let chips = merge(&sni, &[], &Missed::default());
        assert_eq!(
            chips[0].presence,
            Presence::Background {
                address: ":1.100".into()
            }
        );
    }

    #[test]
    fn an_open_window_wins_over_the_tray_and_a_focused_one_shows_no_unread() {
        let sni = [item(
            "slack",
            ":1.9",
            SniStatus::Active,
            "You have 3 notifications",
        )];
        let chips = merge(
            &sni,
            &[win("slack", "! Activity - 1 new item - Slack", false)],
            &Missed::default(),
        );
        assert!(matches!(
            chips[0].presence,
            Presence::Open {
                con_id: 7,
                focused: false,
                ..
            }
        ));
        assert_eq!(chips[0].unread, Unread::Count(3));

        let chips = merge(
            &sni,
            &[win("slack", "! Activity - 1 new item - Slack", true)],
            &Missed::default(),
        );
        assert_eq!(chips[0].unread, Unread::None);
    }

    #[test]
    fn title_count_applies_when_the_tray_says_nothing() {
        let chips = merge(
            &[],
            &[win("discord", "(3) Discord | #general", false)],
            &Missed::default(),
        );
        assert_eq!(chips[0].unread, Unread::Count(3));
        let chips = merge(
            &[],
            &[win("slack", "* general - Slack", false)],
            &Missed::default(),
        );
        assert_eq!(chips[0].unread, Unread::Dot);
    }

    #[test]
    fn a_closed_teams_with_missed_notifications_still_has_a_chip() {
        let mut missed = Missed::default();
        missed.record("teams", 11);
        missed.record("teams", 12);
        let chips = merge(&[], &[], &missed);
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].client.id, "teams");
        assert_eq!(chips[0].presence, Presence::Gone);
        assert_eq!(chips[0].unread, Unread::Count(2));
    }

    #[test]
    fn nothing_running_and_nothing_waiting_draws_nothing() {
        assert!(merge(&[], &[], &Missed::default()).is_empty());
    }

    #[test]
    fn a_scratchpad_window_is_the_last_choice() {
        let mut hidden = win("slack", "Slack", false);
        hidden.scratchpad = true;
        hidden.con_id = 1;
        let mut shown = win("slack", "Slack", false);
        shown.con_id = 2;
        let chips = merge(&[], &[hidden, shown], &Missed::default());
        assert!(matches!(
            chips[0].presence,
            Presence::Open { con_id: 2, .. }
        ));
    }

    #[test]
    fn badge_caps_at_99() {
        assert_eq!(badge_text(7), "7");
        assert_eq!(badge_text(100), "99+");
    }
}
