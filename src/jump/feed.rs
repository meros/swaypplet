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
use super::live::{self, Crop};

struct Entry {
    stream: live::Stream,
    /// The stream's frame cap; 0 is none.
    fps: u32,
    subscribers: Vec<(u64, Weak<RefCell<Live>>)>,
}

thread_local! {
    static HUB: RefCell<HashMap<String, Entry>> = RefCell::new(HashMap::new());
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
        let gone = HUB.with(|h| {
            let mut hub = h.borrow_mut();
            let entry = hub.get_mut(&self.key)?;
            entry.subscribers.retain(|(id, _)| *id != self.id);
            if entry.subscribers.is_empty() {
                hub.remove(&self.key)
            } else {
                None
            }
        });
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

/// The capture's identity: the windows (order does not matter), the piece,
/// and the frames' longer edge. The frame cap is not in it; a subscriber
/// that wants more frames speeds the shared stream up instead.
fn key(windows: &[String], crop: Option<Crop>, max_edge: u32) -> String {
    let mut ids = windows.to_vec();
    ids.sort();
    format!("{}|{crop:?}|{max_edge}", ids.join(","))
}

/// Feed `live`'s pictures from a capture of `windows` (or of `crop` of the
/// one window, when given), at most `max_edge` pixels on a side and `fps`
/// frames a second per window (0: every frame). `None` when there is
/// nothing to capture.
pub fn subscribe(
    windows: Vec<String>,
    crop: Option<Crop>,
    max_edge: u32,
    fps: u32,
    live: &Rc<RefCell<Live>>,
) -> Option<Feed> {
    if windows.is_empty() || (crop.is_some() && windows.len() != 1) {
        return None;
    }
    let key = key(&windows, crop, max_edge);
    let id = NEXT.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    });
    let restart = HUB.with(|h| {
        let mut hub = h.borrow_mut();
        match hub.get_mut(&key) {
            Some(entry) => {
                entry.subscribers.push((id, Rc::downgrade(live)));
                slower(entry.fps, fps)
            }
            None => {
                let stream = start(&key, &windows, crop, max_edge, fps);
                hub.insert(
                    key.clone(),
                    Entry {
                        stream,
                        fps,
                        subscribers: vec![(id, Rc::downgrade(live))],
                    },
                );
                false
            }
        }
    });
    if restart {
        // Faster for everyone. The old stream stops as it is replaced; the
        // pictures keep their last frame until the new one sends.
        let stream = start(&key, &windows, crop, max_edge, fps);
        let old = HUB.with(|h| {
            h.borrow_mut().get_mut(&key).map(|entry| {
                entry.fps = fps;
                std::mem::replace(&mut entry.stream, stream)
            })
        });
        drop(old);
    }
    Some(Feed { key, id })
}

fn start(
    key: &str,
    windows: &[String],
    crop: Option<Crop>,
    max_edge: u32,
    fps: u32,
) -> live::Stream {
    let (tx, rx) = async_channel::unbounded::<live::Frame>();
    let key = key.to_string();
    glib::spawn_future_local(async move {
        while let Ok(first) = rx.recv().await {
            for frame in newest(first, &rx) {
                fan_out(&key, frame);
            }
        }
    });
    match crop {
        Some(crop) => live::Stream::start_region(windows[0].clone(), crop, max_edge, fps, tx),
        None => live::Stream::start(windows.to_vec(), max_edge, fps, tx),
    }
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
    let subscribers: Vec<Rc<RefCell<Live>>> = HUB.with(|h| {
        h.borrow()
            .get(key)
            .map(|e| e.subscribers.iter().filter_map(|(_, w)| w.upgrade()).collect())
            .unwrap_or_default()
    });
    if subscribers.is_empty() {
        return;
    }
    let (id, texture) = card::remember(frame);
    for live in subscribers {
        live.borrow().show(&id, &texture);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_ignores_window_order_but_not_size_or_piece() {
        let a = vec!["x".to_string(), "y".to_string()];
        let b = vec!["y".to_string(), "x".to_string()];
        assert_eq!(key(&a, None, 800), key(&b, None, 800));
        assert_ne!(key(&a, None, 800), key(&a, None, 384));
        assert_ne!(
            key(&a[..1], None, 800),
            key(&a[..1], Some((0.0, 0.0, 0.5, 0.5)), 800)
        );
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
