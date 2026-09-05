//! ext-idle-notify-v1 client: one notification per timeout tier, events
//! forwarded to the idle main loop. Runs on its own thread.
//!
//! `get_idle_notification` (not `get_input_idle_notification`) so that
//! idle-inhibit-v1 clients (video players, the compositor's own inhibitors)
//! keep suppressing timeouts, same as swayidle.
//!
//! The tiers' durations are not fixed: the main loop hands a new
//! [`Timeouts`] down the channel `start` returns when the settings file
//! changes (`idle/mod.rs`), and the thread destroys its notifications and
//! creates them again at the new lengths. That is why the loop below polls
//! the socket with a timeout instead of `blocking_dispatch`: a thread parked
//! in a blocking read has no way to notice the channel.

use std::os::fd::AsRawFd;
use std::sync::mpsc::{self, Receiver, Sender};

use wayland_client::backend::WaylandError;
use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

use super::Ev;
use crate::settings::store::Idle;

/// How long the socket poll waits before checking the channel. The only
/// latency it adds is between a settings edit and the re-arm, on top of the
/// main loop's own file poll.
const POLL_MS: i32 = 500;

/// The four idle tiers.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Timeout {
    Dim,
    Lock,
    /// Activity detector while locked. Not a blank tier any more: it only
    /// says "input stopped" / "input resumed", and the blank deadline in
    /// mod.rs is armed and cleared off it.
    LockIdle,
    Suspend,
}

impl Timeout {
    const ALL: [Timeout; 4] = [
        Timeout::Dim,
        Timeout::Lock,
        Timeout::LockIdle,
        Timeout::Suspend,
    ];

    /// Milliseconds, zero meaning the tier is off.
    fn ms(self, t: &Timeouts) -> u32 {
        match self {
            Timeout::Dim => t.dim_ms,
            Timeout::Lock => t.lock_ms,
            // Not a setting: it is how the blank deadline learns that input
            // stopped, and 30 s is short enough that the deadline it arms is
            // what the user set to within a rounding error.
            Timeout::LockIdle => 30_000,
            Timeout::Suspend => t.suspend_ms,
        }
    }
}

/// The three user-facing tiers' lengths, from the settings file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Timeouts {
    pub dim_ms: u32,
    pub lock_ms: u32,
    pub suspend_ms: u32,
}

impl From<&Idle> for Timeouts {
    fn from(idle: &Idle) -> Self {
        Timeouts {
            dim_ms: idle.dim_after_s.saturating_mul(1000),
            lock_ms: idle.lock_after_s.saturating_mul(1000),
            suspend_ms: idle.suspend_after_s.saturating_mul(1000),
        }
    }
}

struct App {
    tx: Sender<Ev>,
    seat: Option<WlSeat>,
    notifier: Option<ExtIdleNotifierV1>,
    /// The live notifications, destroyed and rebuilt on a re-arm.
    armed: Vec<ExtIdleNotificationV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind::<WlSeat, _, _>(name, 1, qh, ()));
                }
                "ext_idle_notifier_v1" if state.notifier.is_none() => {
                    state.notifier =
                        Some(registry.bind::<ExtIdleNotifierV1, _, _>(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, Timeout> for App {
    fn event(
        state: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        tier: &Timeout,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let ev = match event {
            ext_idle_notification_v1::Event::Idled => Ev::Idled(*tier),
            ext_idle_notification_v1::Event::Resumed => Ev::Resumed(*tier),
            _ => return,
        };
        let _ = state.tx.send(ev);
    }
}

wayland_client::delegate_noop!(App: ignore WlSeat);
wayland_client::delegate_noop!(App: ignore ExtIdleNotifierV1);

/// Start watching at `initial`. The returned sender re-arms the tiers; a
/// send that fails means the thread is gone, which it only is after a
/// fatal error the main loop has already been told about.
pub fn start(tx: Sender<Ev>, initial: Timeouts) -> Sender<Timeouts> {
    let (update_tx, update_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("idle-wayland".into())
        .spawn(move || {
            if let Err(e) = watch(tx.clone(), initial, update_rx) {
                let _ = tx.send(Ev::Fatal(format!("idle notify watcher: {e}")));
            }
        })
        .expect("spawn idle-wayland thread");
    update_tx
}

impl App {
    /// Replace every notification with one at the new length. A tier at
    /// zero gets none, which is how "never" is spelled to the compositor.
    fn arm(&mut self, qh: &QueueHandle<Self>, timeouts: &Timeouts) {
        for old in self.armed.drain(..) {
            old.destroy();
        }
        let (Some(seat), Some(notifier)) = (&self.seat, &self.notifier) else {
            return;
        };
        for tier in Timeout::ALL {
            let ms = tier.ms(timeouts);
            if ms == 0 {
                continue;
            }
            self.armed
                .push(notifier.get_idle_notification(ms, seat, qh, tier));
        }
        log::info!(
            "idle: watching {} timeouts via ext-idle-notify-v1 (dim {}s, lock {}s, suspend {}s; 0 is off)",
            self.armed.len(),
            timeouts.dim_ms / 1000,
            timeouts.lock_ms / 1000,
            timeouts.suspend_ms / 1000
        );
    }
}

fn watch(
    tx: Sender<Ev>,
    initial: Timeouts,
    updates: Receiver<Timeouts>,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());

    let mut app = App {
        tx,
        seat: None,
        notifier: None,
        armed: Vec::new(),
    };
    queue.roundtrip(&mut app)?;

    if app.seat.is_none() {
        return Err("no wl_seat advertised".into());
    }
    if app.notifier.is_none() {
        return Err("compositor lacks ext-idle-notify-v1".into());
    }
    app.arm(&qh, &initial);

    loop {
        // Only the newest re-arm matters; a burst of edits is one rebuild.
        let mut latest = None;
        while let Ok(t) = updates.try_recv() {
            latest = Some(t);
        }
        if let Some(t) = latest {
            app.arm(&qh, &t);
        }

        queue.flush()?;
        queue.dispatch_pending(&mut app)?;

        // `None` means events are already queued and the dispatch above
        // will see them on the next pass; there is nothing to wait for.
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut pfd = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN | libc::POLLERR,
                revents: 0,
            };
            // SAFETY: `pfd` is a valid, initialised pollfd for the whole
            // call, and the fd is borrowed from the guard, which keeps the
            // connection open for as long as the borrow lives.
            let ready = unsafe { libc::poll(&mut pfd, 1, POLL_MS) };
            if ready > 0 {
                match guard.read() {
                    Ok(_) => {}
                    Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.into()),
                }
            } else if ready < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() != std::io::ErrorKind::Interrupted {
                    return Err(err.into());
                }
            }
            // A timeout drops the guard unread, which cancels the read.
        }
        queue.dispatch_pending(&mut app)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_is_zero_and_the_lock_idle_tier_is_not_a_setting() {
        let never = Timeouts::from(&Idle {
            dim_after_s: 0,
            lock_after_s: 0,
            suspend_after_s: 0,
            ..Idle::default()
        });
        assert_eq!(Timeout::Dim.ms(&never), 0);
        assert_eq!(Timeout::Lock.ms(&never), 0);
        assert_eq!(Timeout::Suspend.ms(&never), 0);
        assert_eq!(Timeout::LockIdle.ms(&never), 30_000);

        let shipped = Timeouts::from(&Idle::default());
        assert_eq!(Timeout::Dim.ms(&shipped), 240_000);
        assert_eq!(Timeout::Lock.ms(&shipped), 300_000);
        assert_eq!(Timeout::Suspend.ms(&shipped), 1_200_000);
    }
}
