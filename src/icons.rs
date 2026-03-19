// Volume / speaker
pub const SPEAKER_HIGH: &str = "󰕾";
pub const SPEAKER_MED: &str = "󰖀";
pub const SPEAKER_LOW: &str = "󰕿";
pub const SPEAKER_MUTED: &str = "󰖁";

// Microphone
pub const MIC: &str = "󰍬";
pub const MIC_MUTED: &str = "󰍭";

// Brightness
pub const BRIGHTNESS: &str = "󰃟";

// Notifications
#[allow(dead_code)]
pub const NOTIFICATION: &str = "󰂚";
pub const NOTIFICATION_CLEAR: &str = "󰂛";
pub const CLOSE: &str = "󰅖";

// Keyboard indicators
pub const CAPS_ON: &str = "A";
pub const CAPS_OFF: &str = "a";
pub const NUM_ON: &str = "1";
pub const NUM_OFF: &str = "#";

/// Pick the right volume/mic icon based on level and mute state.
pub fn volume_icon(volume: f64, muted: bool, is_mic: bool) -> &'static str {
    if is_mic {
        return if muted { MIC_MUTED } else { MIC };
    }
    if muted {
        SPEAKER_MUTED
    } else if volume < 0.34 {
        SPEAKER_LOW
    } else if volume < 0.67 {
        SPEAKER_MED
    } else {
        SPEAKER_HIGH
    }
}
