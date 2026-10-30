//! Durations and sizes as the panel words them. Pure: the caller passes the
//! number, so a label is testable and never reads the clock itself.
//!
//! Each surface has its own wording, chosen for the room it has (a card
//! header, a settings row, a chip), so each is a style here rather than
//! one answer forced on all of them.

/// How [`ago`] words an age.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ago {
    /// "just now" under 90 s, then to the nearest unit: "2 min ago",
    /// "5 h ago", "3 days ago". Coarse on purpose: nobody acts on the
    /// difference between 61 and 62 minutes (the backup row).
    Rounded,
    /// "just now", then whole units: "3 min ago", "5 h ago", "2 d ago".
    Floored,
    /// "just now", "3m ago", "5h ago", "2d ago" (the notification list).
    Short,
    /// "now", "3m", "5h", "2d", for a header beside the app name.
    Bare,
}

/// `secs` ago, in `style`.
pub fn ago(secs: u64, style: Ago) -> String {
    let s = secs;
    if style == Ago::Rounded {
        return match s {
            0..=89 => "just now".to_string(),
            90..=5399 => format!("{} min ago", (s + 30) / 60),
            5400..=169_199 => format!("{} h ago", (s + 1800) / 3600),
            _ => format!("{} days ago", (s + 43_200) / 86_400),
        };
    }
    let (now, [min, h, d], suffix) = match style {
        Ago::Floored => ("just now", [" min", " h", " d"], " ago"),
        Ago::Short => ("just now", ["m", "h", "d"], " ago"),
        _ => ("now", ["m", "h", "d"], ""),
    };
    match s {
        0..60 => now.to_string(),
        60..3600 => format!("{}{min}{suffix}", s / 60),
        3600..86_400 => format!("{}{h}{suffix}", s / 3600),
        _ => format!("{}{d}{suffix}", s / 86_400),
    }
}

/// How [`hm`] words a span of time. Every style counts whole minutes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hm {
    /// "<1m", "12m", "1h", "1h 5m" (the task popover).
    Compact,
    /// "45m", "1h 0m", "13h 20m": always both units past the hour, as
    /// a clock would (the battery estimate).
    Clock,
    /// "5 min", "1 h 5 min", "2 d 3 h": the two largest units (uptime).
    Prose,
    /// "12m", then whole hours "3h": the largest unit only (the task chip).
    Largest,
}

/// `secs` as a span, in `style`.
pub fn hm(secs: u64, style: Hm) -> String {
    let mins = secs / 60;
    let (h, m) = (mins / 60, mins % 60);
    match style {
        Hm::Compact => match (h, m) {
            (0, 0) => "<1m".to_string(),
            (0, m) => format!("{m}m"),
            (h, 0) => format!("{h}h"),
            (h, m) => format!("{h}h {m}m"),
        },
        Hm::Clock if h == 0 => format!("{m}m"),
        Hm::Clock => format!("{h}h {m}m"),
        Hm::Prose => match (h / 24, h % 24) {
            (0, 0) => format!("{m} min"),
            (0, h) => format!("{h} h {m} min"),
            (d, h) => format!("{d} d {h} h"),
        },
        Hm::Largest if h == 0 => format!("{m}m"),
        Hm::Largest => format!("{h}h"),
    }
}

/// Bytes in the largest binary unit that keeps a whole part, one decimal:
/// "512 B", "1.5 KiB", "3.2 GiB".
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_as_each_surface_words_them() {
        let table: &[(u64, Ago, &str)] = &[
            (0, Ago::Rounded, "just now"),
            (89, Ago::Rounded, "just now"),
            (90, Ago::Rounded, "2 min ago"),
            (5399, Ago::Rounded, "90 min ago"),
            (5400, Ago::Rounded, "2 h ago"),
            (169_199, Ago::Rounded, "47 h ago"),
            (169_200, Ago::Rounded, "2 days ago"),
            (59, Ago::Floored, "just now"),
            (60, Ago::Floored, "1 min ago"),
            (3599, Ago::Floored, "59 min ago"),
            (3600, Ago::Floored, "1 h ago"),
            (86_399, Ago::Floored, "23 h ago"),
            (86_400 * 3, Ago::Floored, "3 d ago"),
            (30, Ago::Short, "just now"),
            (150, Ago::Short, "2m ago"),
            (7200, Ago::Short, "2h ago"),
            (172_800, Ago::Short, "2d ago"),
            (59, Ago::Bare, "now"),
            (60, Ago::Bare, "1m"),
            (3600, Ago::Bare, "1h"),
            (86_400, Ago::Bare, "1d"),
        ];
        for &(secs, style, want) in table {
            assert_eq!(ago(secs, style), want, "{secs} s as {style:?}");
        }
    }

    #[test]
    fn spans_read_as_each_surface_words_them() {
        let table: &[(u64, Hm, &str)] = &[
            (59, Hm::Compact, "<1m"),
            (12 * 60 + 59, Hm::Compact, "12m"),
            (3600, Hm::Compact, "1h"),
            (3900, Hm::Compact, "1h 5m"),
            (45 * 60, Hm::Clock, "45m"),
            (3600, Hm::Clock, "1h 0m"),
            (13 * 3600 + 20 * 60, Hm::Clock, "13h 20m"),
            (300, Hm::Prose, "5 min"),
            (3900, Hm::Prose, "1 h 5 min"),
            (2 * 86_400 + 3 * 3600 + 120, Hm::Prose, "2 d 3 h"),
            (12 * 60, Hm::Largest, "12m"),
            (3 * 3600 + 59 * 60, Hm::Largest, "3h"),
            (30 * 3600, Hm::Largest, "30h"),
        ];
        for &(secs, style, want) in table {
            assert_eq!(hm(secs, style), want, "{secs} s as {style:?}");
        }
    }

    #[test]
    fn sizes_take_the_largest_whole_unit() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
        assert_eq!(bytes(u64::MAX), "16777216.0 TiB");
    }
}
