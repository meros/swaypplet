pub mod dbus;
pub mod popup;
pub mod store;

/// Urgency levels per the Desktop Notifications spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Low = 0,
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

/// A single notification.
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
}
