//! Display power and backlight, reconciled off the idle loop's thread.
//!
//! Why this exists
//! ---------------
//! `swaymsg output * power on` returns in 782-820 ms on this machine, measured
//! from `presence.back: fire` to `presence.back: ok`. Every call site used to
//! run that synchronously on the idle loop's only thread, so the loop was
//! blind for most of a second at exactly the moment it most needs to be
//! responsive: the presence edge that both powers the screen and starts a face
//! attempt. The face request could not be queued until the screen command had
//! returned.
//!
//! So display changes are *desired state* published to a worker, not commands
//! executed inline. The idle loop never blocks on the compositor again.
//!
//! Reconciler, not a queue
//! -----------------------
//! Requests coalesce last-write-wins per field. If the loop asks for off and
//! then on before the worker has drained, the worker applies on and never
//! applies off. That is the point: a queue would replay a stale blank *after*
//! the screen had legitimately come back, which is both a visible glitch and,
//! if it raced an unlock, a blanked unlocked desktop.
//!
//! What this module does NOT decide
//! --------------------------------
//! Whether blanking is allowed. The invariant that outputs are never blanked
//! unless the session is confirmed locked lives with the callers, where the
//! lock state is known. This module faithfully applies what it is told,
//! including a wrong instruction. Powering *on* is unconditional and always
//! safe: it exposes nothing that was not already on screen.

use std::sync::mpsc::{self, Sender, TryRecvError};
use std::thread;
use std::time::Instant;

use crate::idle::run_cmd_timed;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Power {
    On,
    Off,
}

impl Power {
    fn arg(self) -> &'static str {
        match self {
            Power::On => "on",
            Power::Off => "off",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Request {
    scope: &'static str,
    power: Option<Power>,
    /// Backlight percentage, applied via brightnessctl.
    brightness: Option<u8>,
    queued: Instant,
}

impl Request {
    /// Fold a newer request over an older one. Newer wins per field, so a
    /// request that only sets brightness does not clear a pending power change.
    fn merge(self, newer: Request) -> Request {
        Request {
            scope: newer.scope,
            power: newer.power.or(self.power),
            brightness: newer.brightness.or(self.brightness),
            queued: self.queued,
        }
    }
}

#[derive(Clone)]
pub struct Outputs {
    tx: Sender<Request>,
}

impl Outputs {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel::<Request>();
        thread::Builder::new()
            .name("idle-outputs".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    // Drain whatever piled up while the last apply was in
                    // flight and fold it into one action.
                    let mut req = first;
                    loop {
                        match rx.try_recv() {
                            Ok(next) => req = req.merge(next),
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => break,
                        }
                    }
                    apply(req);
                }
            })
            .expect("spawn idle-outputs thread");
        Outputs { tx }
    }

    /// Request a power state. Never blocks.
    pub fn power(&self, scope: &'static str, power: Power) {
        self.send(scope, Some(power), None);
    }

    /// Request a power state and a backlight level together, so the two are
    /// applied back to back rather than with a compositor round trip between.
    pub fn power_brightness(&self, scope: &'static str, power: Power, brightness: u8) {
        self.send(scope, Some(power), Some(brightness));
    }

    /// Request a backlight level only.
    pub fn brightness(&self, scope: &'static str, brightness: u8) {
        self.send(scope, None, Some(brightness));
    }

    fn send(&self, scope: &'static str, power: Option<Power>, brightness: Option<u8>) {
        let req = Request {
            scope,
            power,
            brightness,
            queued: Instant::now(),
        };
        // A full or closed channel means the worker died. Losing a power-off
        // is cosmetic; losing a power-on would leave a dark screen, so say so
        // loudly rather than failing silently.
        if self.tx.send(req).is_err() {
            log::error!("{scope}: display worker gone, not applied");
        }
    }
}

fn apply(req: Request) {
    let waited = req.queued.elapsed();
    if let Some(power) = req.power {
        // The queue wait is logged separately from the command duration.
        // Together they are what "presence edge to first paint" is made of,
        // and the migration gate in the architecture doc needs them apart:
        // queue wait is this module's fault, command duration is sway's.
        run_cmd_timed(
            req.scope,
            "swaymsg",
            &["output", "*", "power", power.arg()],
            waited,
        );
    }
    if let Some(pct) = req.brightness {
        let value = format!("{pct}%");
        run_cmd_timed(req.scope, "brightnessctl", &["set", &value, "-n"], waited);
    }
}
