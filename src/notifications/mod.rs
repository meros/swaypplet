pub mod dbus;
pub mod markup;
pub mod popup;
pub mod quiet;
pub mod store;

/// Urgency levels per the Desktop Notifications spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Urgency {
    Low = 0,
    #[default]
    Normal = 1,
    Critical = 2,
}

impl From<u8> for Urgency {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Low,
            2 => Self::Critical,
            _ => Self::Normal,
        }
    }
}

/// Why a notification was closed (spec values).
#[derive(Debug, Clone, Copy)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    CloseCall = 3,
}

/// Raw pixels handed over in an `image-data` hint, in the spec's
/// `(iiibiiay)` layout. Carried as plain data rather than a `GdkPixbuf`
/// because it crosses from the D-Bus thread to the GTK thread.
#[derive(Debug, Clone)]
pub struct ImageData {
    pub width: i32,
    pub height: i32,
    pub rowstride: i32,
    pub has_alpha: bool,
    pub bits_per_sample: i32,
    pub data: Vec<u8>,
}

/// Where a card's picture comes from. Names and paths are resolved by GTK at
/// render time; raw data arrives inline over D-Bus.
#[derive(Debug, Clone)]
pub enum ImageSource {
    /// A themed icon name, or a desktop-entry basename to look one up from.
    Named(String),
    Path(std::path::PathBuf),
    Data(ImageData),
}

/// A single notification.
///
/// `Default` exists so that call sites and fixtures name only the fields they
/// care about: the struct grew a tail of optional hint-derived fields, and
/// spelling all of them out at every construction hid the interesting ones.
#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub urgency: Urgency,
    pub actions: Vec<(String, String)>,
    pub expire_timeout: i32,
    pub timestamp: std::time::SystemTime,
    /// Transient notifications are not stored in history.
    pub transient: bool,
    /// Progress hint (0–100), if provided by the sender.
    pub progress: Option<u32>,
    /// replaces_id from the Notify call.
    pub replaces_id: u32,
    /// Claude session pid from the nixos hook's `claude-pid` hint
    /// (stop-notification policy, vision O2). Absent → normal rules.
    pub claude_pid: Option<i32>,
    /// Task 1–4 the session belongs to, resolved once at add time; the
    /// popup renders it as a hue-carrying "T<N>" attribution.
    pub task: Option<u8>,
    /// The session's workspace was visible when the notification arrived —
    /// the popup is suppressed (history keeps it; Critical overrides).
    pub suppressed: bool,

    /// The sending application's icon: the `app_icon` argument, or the
    /// `desktop-entry` hint when that is empty. Identity, not content.
    pub icon: Option<ImageSource>,
    /// The notification's own picture, from `image-data` / `image-path`.
    /// Content, and it outranks `icon` when the card has room for one.
    pub image: Option<ImageSource>,
    /// `category` hint (e.g. `device.added`, `im.received`), which is what
    /// routes a notification to a card template.
    pub category: Option<String>,
    /// `desktop-entry` hint: the sender naming its own `.desktop` file. The
    /// most reliable thing a notification says about which window it came
    /// from, since sway's `app_id` is the same string for Wayland clients.
    pub desktop_entry: Option<String>,
    /// `x-canonical-private-synchronous`: a tag whose notifications replace
    /// each other in place rather than stacking. Volume and brightness OSDs
    /// use it, and so do this machine's own scripts.
    pub sync_tag: Option<String>,
    /// `resident` hint: invoking an action leaves the notification open.
    /// Without it every button is terminal.
    pub resident: bool,
    /// `action-icons` hint: action keys name icons rather than being opaque.
    pub action_icons: bool,
}

impl Default for Notification {
    fn default() -> Self {
        Self {
            id: 0,
            app_name: String::new(),
            summary: String::new(),
            body: String::new(),
            urgency: Urgency::Normal,
            actions: Vec::new(),
            // The spec's "let the server decide", not "never expire".
            expire_timeout: -1,
            // Only a placeholder: every real notification stamps `now` at the
            // D-Bus boundary, and a card's age is read from this.
            timestamp: std::time::UNIX_EPOCH,
            transient: false,
            progress: None,
            replaces_id: 0,
            claude_pid: None,
            task: None,
            suppressed: false,
            icon: None,
            image: None,
            category: None,
            desktop_entry: None,
            sync_tag: None,
            resident: false,
            action_icons: false,
        }
    }
}
