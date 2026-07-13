//! ext-idle-notify-v1 client: one notification per timeout tier, events
//! forwarded to the idle main loop. Runs on its own thread with a blocking
//! wayland event queue.
//!
//! `get_idle_notification` (not `get_input_idle_notification`) so that
//! idle-inhibit-v1 clients (video players, the compositor's own inhibitors)
//! keep suppressing timeouts, same as swayidle.

use std::sync::mpsc::Sender;

use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

use super::Ev;

/// The four idle tiers. Timings ported from the old swayidle config.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Timeout {
    Dim,
    Lock,
    Reblank,
    Suspend,
}

impl Timeout {
    const ALL: [Timeout; 4] = [
        Timeout::Dim,
        Timeout::Lock,
        Timeout::Reblank,
        Timeout::Suspend,
    ];

    fn ms(self) -> u32 {
        match self {
            Timeout::Dim => 240_000,
            Timeout::Lock => 300_000,
            Timeout::Reblank => 30_000,
            Timeout::Suspend => 1_200_000,
        }
    }
}

struct App {
    tx: Sender<Ev>,
    seat: Option<WlSeat>,
    notifier: Option<ExtIdleNotifierV1>,
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
        if let wl_registry::Event::Global { name, interface, .. } = event {
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

pub fn start(tx: Sender<Ev>) {
    std::thread::Builder::new()
        .name("idle-wayland".into())
        .spawn(move || {
            if let Err(e) = watch(tx.clone()) {
                let _ = tx.send(Ev::Fatal(format!("idle notify watcher: {e}")));
            }
        })
        .expect("spawn idle-wayland thread");
}

fn watch(tx: Sender<Ev>) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());

    let mut app = App {
        tx,
        seat: None,
        notifier: None,
    };
    queue.roundtrip(&mut app)?;

    let seat = app.seat.clone().ok_or("no wl_seat advertised")?;
    let notifier = app
        .notifier
        .clone()
        .ok_or("compositor lacks ext-idle-notify-v1")?;
    for tier in Timeout::ALL {
        notifier.get_idle_notification(tier.ms(), &seat, &qh, tier);
    }
    log::info!("idle: watching {} timeouts via ext-idle-notify-v1", Timeout::ALL.len());

    loop {
        queue.blocking_dispatch(&mut app)?;
    }
}
