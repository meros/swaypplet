//! One capture per set of windows, however many pictures show it.
//!
//! A pin and the bar's peek of the same workspace used to start a capture
//! each: the same windows, the same size, twice the compositor's copies and
//! twice the scaling on the worker. Here every picture subscribes, and the
//! subscribers of one set of windows at one size share one `live::Stream`.
//! Each frame becomes one texture, which every subscriber shows.
//!
//! A subscription is a [`Feed`]. Dropping the last one of a set drops its
//! stream, which stops the capture; nothing here ticks.
//!
//! Frames coalesce. The worker queues them as they come; the main thread,
//! on each wake, takes everything queued and shows only the newest frame
//! of each window. A main thread that fell behind (a long layout, a busy
//! animation) then catches up in one step instead of turning a backlog of
//! stale frames into textures it throws away.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gtk4::glib;

use super::card::{self, Live};
use super::live::{self, Want};

/// The pictures sharing one stream, by subscription id, and the stream's
/// frame cap (0 is none). Generic over the stream so the bookkeeping is a
/// unit test without a compositor.
struct Entry<S> {
    stream: S,
    fps: u32,
    subscribers: Vec<(u64, Weak<RefCell<Live>>)>,
}

/// What a new subscriber means for its key's stream.
#[derive(Debug, PartialEq)]
enum Join {
    /// Nothing captures this yet: start a stream.
    Start,
    /// A stream runs, at a lower cap than this subscriber asks for: replace
    /// it with a faster one.
    Faster,
    /// A stream runs, fast enough.
    Shared,
}

/// Every running stream, by [`key`].
struct Hub<S> {
    entries: HashMap<String, Entry<S>>,
}

impl<S> Default for Hub<S> {
    fn default() -> Self {
        Hub {
            entries: HashMap::new(),
        }
    }
}

impl<S> Hub<S> {
    /// Add subscriber `id` to `key`, whose stream `start` makes when none
    /// runs. `Faster` leaves replacing the stream to the caller, outside
    /// any borrow of the hub, because dropping a stream is not free.
    fn join(
        &mut self,
        key: &str,
        id: u64,
        fps: u32,
        live: Weak<RefCell<Live>>,
        start: impl FnOnce() -> S,
    ) -> Join {
        match self.entries.get_mut(key) {
            Some(entry) => {
                entry.subscribers.push((id, live));
                if slower(entry.fps, fps) {
                    Join::Faster
                } else {
                    Join::Shared
                }
            }
            None => {
                self.entries.insert(
                    key.to_string(),
                    Entry {
                        stream: start(),
                        fps,
                        subscribers: vec![(id, live)],
                    },
                );
                Join::Start
            }
        }
    }

    /// Put `stream` in place of `key`'s, at cap `fps`; the old one comes
    /// back for the caller to drop.
    fn replace(&mut self, key: &str, fps: u32, stream: S) -> Option<S> {
        let entry = self.entries.get_mut(key)?;
        entry.fps = fps;
        Some(std::mem::replace(&mut entry.stream, stream))
    }

    /// Take subscriber `id` off `key`. The stream comes back, for the
    /// caller to drop, when that was its last subscriber.
    fn leave(&mut self, key: &str, id: u64) -> Option<S> {
        let entry = self.entries.get_mut(key)?;
        entry.subscribers.retain(|(sub, _)| *sub != id);
        if entry.subscribers.is_empty() {
            self.entries.remove(key).map(|e| e.stream)
        } else {
            None
        }
    }

    /// The pictures of `key` still alive.
    fn subscribers(&self, key: &str) -> Vec<Rc<RefCell<Live>>> {
        self.entries
            .get(key)
            .map(|e| {
                e.subscribers
                    .iter()
                    .filter_map(|(_, w)| w.upgrade())
                    .collect()
            })
            .unwrap_or_default()
    }
}

thread_local! {
    static HUB: RefCell<Hub<live::Stream>> = RefCell::default();
    static NEXT: Cell<u64> = const { Cell::new(0) };
}

/// A picture's share of a capture. Drop it to stop receiving frames.
pub struct Feed {
    key: String,
    id: u64,
}

impl Drop for Feed {
    fn drop(&mut self) {
        // The stream leaves the map before it is dropped, so its Drop never
        // runs inside a borrow of the map.
        let gone = HUB.with(|h| h.borrow_mut().leave(&self.key, self.id));
        log::info!(target: "swaypplet::feed", "leave {} last={}", self.key, gone.is_some());
        drop(gone);
    }
}

/// Whether a stream capped at `have` gives fewer frames than `want` asks.
fn slower(have: u32, want: u32) -> bool {
    match (have, want) {
        (0, _) => false,
        (_, 0) => true,
        (h, w) => h < w,
    }
}

/// The capture's identity: every window with its piece and its size, in
/// any order. The frame cap is not in it; a subscriber that wants more
/// frames speeds the shared stream up instead.
fn key(wants: &[Want]) -> String {
    let mut parts: Vec<String> = wants
        .iter()
        .map(|w| format!("{}|{:?}|{:?}", w.id, w.crop, w.size))
        .collect();
    parts.sort();
    parts.join(",")
}

/// Feed `live`'s pictures from a capture of `wants`, at most `fps` frames a
/// second per window (0: every frame). `None` when there is nothing to
/// capture.
pub fn subscribe(wants: Vec<Want>, fps: u32, live: &Rc<RefCell<Live>>) -> Option<Feed> {
    if wants.is_empty() {
        return None;
    }
    let key = key(&wants);
    let id = NEXT.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    });
    let joined = HUB.with(|h| {
        h.borrow_mut().join(&key, id, fps, Rc::downgrade(live), || {
            start(&key, &wants, fps)
        })
    });
    log::info!(target: "swaypplet::feed", "join {key} {joined:?}");
    if joined == Join::Faster {
        // Faster for everyone. The old stream stops as it is replaced; the
        // pictures keep their last frame until the new one sends.
        let stream = start(&key, &wants, fps);
        let old = HUB.with(|h| h.borrow_mut().replace(&key, fps, stream));
        drop(old);
    }
    Some(Feed { key, id })
}

fn start(key: &str, wants: &[Want], fps: u32) -> live::Stream {
    let (tx, rx) = async_channel::unbounded::<live::Frame>();
    let key = key.to_string();
    glib::spawn_future_local(async move {
        while let Ok(first) = rx.recv().await {
            for frame in newest(first, &rx) {
                fan_out(&key, frame);
            }
        }
    });
    live::Stream::start_wants(wants.to_vec(), fps, tx)
}

/// `first` and everything queued behind it, keeping each window's newest
/// frame only, in the order the windows first appeared.
fn newest(first: live::Frame, rx: &async_channel::Receiver<live::Frame>) -> Vec<live::Frame> {
    let mut frames = vec![first];
    while let Ok(frame) = rx.try_recv() {
        match frames.iter_mut().find(|f| f.id == frame.id) {
            Some(older) => *older = frame,
            None => frames.push(frame),
        }
    }
    frames
}

/// One frame to every picture of its window, as one texture.
fn fan_out(key: &str, frame: live::Frame) {
    let subscribers = HUB.with(|h| h.borrow().subscribers(key));
    if subscribers.is_empty() {
        return;
    }
    // One line per frame shown, for dev/frame-bench.sh --pin to count.
    log::debug!(target: "swaypplet::feed", "frame {} {}x{}", frame.id, frame.width, frame.height);
    let (id, texture) = card::remember(frame);
    for live in subscribers {
        live.borrow().show(&id, &texture);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn want(id: &str, crop: Option<live::Crop>, w: u32) -> Want {
        Want {
            id: id.to_string(),
            crop,
            size: live::Size::Draw(w, w),
        }
    }

    #[test]
    fn the_key_ignores_window_order_but_not_size_or_piece() {
        let a = vec![want("x", None, 400), want("y", None, 200)];
        let b = vec![want("y", None, 200), want("x", None, 400)];
        assert_eq!(key(&a), key(&b));
        assert_ne!(key(&a), key(&[want("x", None, 400), want("y", None, 100)]));
        assert_ne!(
            key(&a[..1]),
            key(&[want("x", Some((0.0, 0.0, 0.5, 0.5)), 400)])
        );
    }

    /// A stand-in stream: which one it is.
    type Hub = super::Hub<u32>;

    #[test]
    fn a_pin_keeps_its_stream_when_the_peek_of_it_closes() {
        let (pin, peek) = (
            Rc::<RefCell<Live>>::default(),
            Rc::<RefCell<Live>>::default(),
        );
        let mut hub = Hub::default();
        assert_eq!(
            hub.join("ws", 0, 30, Rc::downgrade(&pin), || 1),
            Join::Start
        );
        assert_eq!(
            hub.join("ws", 1, 30, Rc::downgrade(&peek), || unreachable!()),
            Join::Shared
        );
        assert_eq!(hub.subscribers("ws").len(), 2);
        // The peek closes: nothing stops.
        assert_eq!(hub.leave("ws", 1), None);
        assert_eq!(hub.subscribers("ws").len(), 1);
        // Leaving twice changes nothing.
        assert_eq!(hub.leave("ws", 1), None);
        // The pin hides: its stream comes back to be dropped.
        assert_eq!(hub.leave("ws", 0), Some(1));
        assert!(hub.subscribers("ws").is_empty());
        // Shown again: a new stream starts.
        assert_eq!(
            hub.join("ws", 2, 30, Rc::downgrade(&pin), || 2),
            Join::Start
        );
        assert_eq!(hub.leave("ws", 2), Some(2));
    }

    #[test]
    fn a_faster_subscriber_replaces_the_stream_for_everyone() {
        let (a, b) = (
            Rc::<RefCell<Live>>::default(),
            Rc::<RefCell<Live>>::default(),
        );
        let mut hub = Hub::default();
        hub.join("ws", 0, 15, Rc::downgrade(&a), || 1);
        assert_eq!(hub.join("ws", 1, 30, Rc::downgrade(&b), || 9), Join::Faster);
        assert_eq!(hub.replace("ws", 30, 2), Some(1));
        // Now at 30: a third at 30 shares it.
        assert_eq!(hub.join("ws", 2, 30, Rc::downgrade(&b), || 9), Join::Shared);
        hub.leave("ws", 1);
        hub.leave("ws", 2);
        assert_eq!(
            hub.leave("ws", 0),
            Some(2),
            "the replacement is the one dropped"
        );
    }

    #[test]
    fn a_dropped_picture_gets_no_frames() {
        let gone = Rc::<RefCell<Live>>::default();
        let mut hub = Hub::default();
        hub.join("ws", 0, 30, Rc::downgrade(&gone), || 1);
        drop(gone);
        assert!(hub.subscribers("ws").is_empty());
    }

    fn frame(id: &str, width: u32) -> live::Frame {
        live::Frame {
            id: id.to_string(),
            width,
            height: 1,
            pixels: Vec::new(),
        }
    }

    #[test]
    fn a_backlog_keeps_each_windows_newest_frame() {
        let (tx, rx) = async_channel::unbounded();
        for (id, w) in [("b", 2), ("a", 3), ("b", 4), ("a", 5)] {
            tx.try_send(frame(id, w)).unwrap();
        }
        let got = newest(frame("a", 1), &rx);
        let got: Vec<(&str, u32)> = got.iter().map(|f| (f.id.as_str(), f.width)).collect();
        assert_eq!(got, [("a", 5), ("b", 4)]);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn only_a_faster_subscriber_restarts_the_stream() {
        assert!(slower(30, 0));
        assert!(slower(15, 30));
        assert!(!slower(0, 30));
        assert!(!slower(30, 15));
        assert!(!slower(0, 0));
    }
}
