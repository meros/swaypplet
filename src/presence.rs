//! Human presence sensor — the one walk-away signal this machine reports.
//!
//! The Integrated Sensor Hub publishes a HID sensor with usage 0x200011
//! (human presence) as an IIO device named "prox": `in_proximity0_raw` reads 1
//! while someone is detected and 0 once they leave, `in_attention_input`
//! tracks engagement and sits at 100 while facing the machine. Both sample at
//! 10 Hz (measured 2026-08-12, ThinkPad X9-14 Gen 1).
//!
//! Why this sensor carries the feature: the IR camera (Himax HM1092) has no
//! Linux driver, and the power button never reaches the OS at all — the EC
//! consumes the press and raises no ACPI query — so neither is available as a
//! lock trigger. This one needs no vendor driver.
//!
//! The IIO index moves across boots, so the device is resolved by name.
//!
//! Transitions are reported raw. The sensor does its own filtering in the
//! ISH, so debouncing here was a second layer over one that already exists,
//! and it cost the thing presence is for: a 10 s absence hold meant the screen
//! stayed lit for ten seconds after you left, and a 1 s return hold delayed
//! every wake and every face-unlock attempt.
//!
//! An older note here measured 1 → 0 → 0 (attention 100) → 1 across 8 s for a
//! step away and back, which is what the debounce was built for. If a glance
//! aside starts locking the session, that blip is back and the fix is a hold
//! on the *absence* edge only — never on the return, which must stay
//! immediate.

use std::fs;
use std::path::PathBuf;

const IIO_DEVICES: &str = "/sys/bus/iio/devices";
const RAW: &str = "in_proximity0_raw";
const ATTENTION: &str = "in_attention_input";

pub struct Presence {
    dir: PathBuf,
    /// Last reported state; seeded by the first reading without reporting it.
    state: Option<bool>,
}

impl Presence {
    /// Resolve the sensor by IIO name. `None` on a machine without one, which
    /// leaves every caller on its pre-presence behaviour.
    pub fn detect() -> Option<Self> {
        let entries = match fs::read_dir(IIO_DEVICES) {
            Ok(entries) => entries,
            Err(e) => {
                log::info!("presence: no IIO devices ({e}) — presence rules off");
                return None;
            }
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            let Ok(name) = fs::read_to_string(dir.join("name")) else {
                continue;
            };
            if name.trim() != "prox" || !dir.join(RAW).exists() {
                continue;
            }
            log::info!("presence: sensor at {}", dir.display());
            return Some(Self {
                dir,
                state: None,
            });
        }
        log::info!("presence: no IIO device named \"prox\" — presence rules off");
        None
    }

    fn read_i32(&self, file: &str) -> Option<i32> {
        fs::read_to_string(self.dir.join(file))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// Last settled state. `None` until the first reading lands.
    pub fn state(&self) -> Option<bool> {
        self.state
    }

    /// Instantaneous reading. For readouts that redraw anyway;
    /// anything that acts on presence should use [`Self::poll`].
    pub fn read(&self) -> Option<bool> {
        Some(self.read_i32(RAW)? != 0)
    }

    /// Engagement, 0-100, unsmoothed and undebounced. Display only.
    pub fn attention(&self) -> Option<i32> {
        self.read_i32(ATTENTION)
    }

    /// Read once. Returns `Some(new_state)` only on a settled transition, so
    /// callers can drive this from an existing tick without tracking edges.
    /// Report a change in presence, or `None` when nothing changed.
    ///
    /// Edge-triggered: every caller acts on transitions, and the first reading
    /// is the starting state rather than a transition — reporting it would
    /// fire "user back" at every service start, which on the lock screen would
    /// arm a face attempt against whoever locked the machine a moment ago.
    pub fn poll(&mut self) -> Option<bool> {
        let present = self.read_i32(RAW)? != 0;

        if self.state.is_none() {
            self.state = Some(present);
            return None;
        }

        if Some(present) == self.state {
            return None;
        }

        self.state = Some(present);
        Some(present)
    }
}
