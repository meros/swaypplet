//! The backlight, through `brightnessctl`: what the panel's slider and the
//! OSD's brightness keys read and set.
//!
//! Every call starts a process and may block on it, so none of them belongs
//! on the GTK thread.

use std::process::Command;

/// The backlight as a percentage, or `None` when `brightnessctl` fails or
/// says something this does not read.
pub fn read() -> Option<u32> {
    let out = Command::new("brightnessctl")
        .arg("-m")
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// The percentage in `brightnessctl -m`'s answer, whose first line is
/// `device,class,current,percentage%,max`.
pub fn parse(machine: &str) -> Option<u32> {
    let line = machine.lines().next()?;
    // Field 3 (0-indexed) is "NN%".
    let pct_field = line.split(',').nth(3)?;
    pct_field.trim().trim_end_matches('%').parse::<u32>().ok()
}

/// Set the backlight to `pct` percent. Not waited for: the slider sends one
/// of these per step of a drag.
pub fn set(pct: u32) {
    let _ = Command::new("brightnessctl")
        .args(["set", &format!("{pct}%")])
        .spawn();
}

/// Move the backlight `step` percent up or down, and wait until it has,
/// so a [`read`] after it sees the new level.
pub fn adjust(step: u32, up: bool) {
    let sign = if up { '+' } else { '-' };
    let _ = Command::new("brightnessctl")
        .args(["set", &format!("{step}%{sign}")])
        .output();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_percentage_is_the_fourth_field_of_the_first_line() {
        assert_eq!(
            parse("intel_backlight,backlight,9600,50%,19200\n"),
            Some(50)
        );
        assert_eq!(parse("intel_backlight,backlight,0,0%,19200"), Some(0));
        assert_eq!(parse("a,backlight,1,100%,1\nb,leds,0,0%,1\n"), Some(100));
    }

    #[test]
    fn an_answer_it_cannot_read_is_none() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("intel_backlight,backlight,9600"), None);
        assert_eq!(parse("intel_backlight,backlight,9600,half,19200"), None);
    }
}
