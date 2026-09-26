//! What the launcher has been used for, so what you use rises.
//!
//! One number per item: a score that gains 1 on every launch and halves every
//! [`HALF_LIFE_S`]. Stored with the time it was last brought up to date, so
//! decay costs nothing until the score is read: `score · 2^(−Δt/half-life)`.
//! That is Firefox's frecency reduced to its one continuous idea, without the
//! visit-type buckets a launcher has no use for.
//!
//! The ranking blends it with the match elephant gives: a row keeps its
//! place in elephant's order and moves up by [`WEIGHT`] places per natural
//! log of its score, so ten launches this week lift an app about seven rows
//! and a thing used once a month ago barely moves. A good match still beats a
//! habit: frecency reorders what matched and never adds what did not.
//!
//! One store per process (the Helm card and the standalone launcher share
//! it), in memory once loaded. The file is read on a worker the first time a
//! launcher is built and written on a worker [`SAVE_DEBOUNCE_MS`] after the
//! last launch, atomically, and never larger than [`MAX_ENTRIES`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::services::elephant::SearchResult;

/// The time a score takes to halve: a week, so last month's habit fades
/// and this week's shows.
pub const HALF_LIFE_S: f64 = 7.0 * 24.0 * 3600.0;

/// Places a row moves up per unit of `ln(1 + score)`.
pub const WEIGHT: f64 = 3.0;

/// Items the file keeps, the lowest scores dropped first.
pub const MAX_ENTRIES: usize = 300;

/// A score this low is forgotten when the store is written: about seven
/// half-lives after a single launch.
const FORGET_BELOW: f64 = 0.01;

const SAVE_DEBOUNCE_MS: u64 = 2000;

/// One item: its score as of `at`, and enough of the row to draw it with
/// the query empty, before elephant has answered anything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub score: f64,
    /// Unix seconds.
    pub at: f64,
    pub provider: String,
    pub identifier: String,
    pub text: String,
    #[serde(default)]
    pub subtext: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub actions: Vec<String>,
}

impl Entry {
    /// The score at `now`.
    pub fn score_at(&self, now: f64) -> f64 {
        decayed(self.score, now - self.at)
    }

    fn result(&self) -> SearchResult {
        SearchResult {
            identifier: self.identifier.clone(),
            text: self.text.clone(),
            subtext: self.subtext.clone(),
            icon: self.icon.clone(),
            provider: self.provider.clone(),
            score: 0,
            actions: self.actions.clone(),
        }
    }
}

/// `score` after `dt` seconds.
pub fn decayed(score: f64, dt: f64) -> f64 {
    score * (-dt.max(0.0) / HALF_LIFE_S).exp2()
}

/// What a row is remembered by.
pub fn key(provider: &str, identifier: &str) -> String {
    format!("{provider}\t{identifier}")
}

/// Whether launching this row says anything about the next launch. A
/// calculator result, a clipboard entry or a window is gone by then.
pub fn remembers(provider: &str) -> bool {
    !matches!(
        provider,
        "calc" | "calculator" | "clipboard" | "windows" | "symbols" | "unicode"
    ) && !provider.starts_with("swaypplet-")
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Store {
    pub entries: HashMap<String, Entry>,
}

impl Store {
    /// Count a launch of `r` at `now`.
    pub fn record(&mut self, r: &SearchResult, now: f64) {
        let k = key(&r.provider, &r.identifier);
        let e = self.entries.entry(k).or_insert_with(|| Entry {
            score: 0.0,
            at: now,
            provider: r.provider.clone(),
            identifier: r.identifier.clone(),
            text: String::new(),
            subtext: String::new(),
            icon: String::new(),
            actions: Vec::new(),
        });
        e.score = e.score_at(now) + 1.0;
        e.at = now;
        // The newest look of the row: an app renamed by an update shows as
        // it is now.
        e.text = r.text.clone();
        e.subtext = r.subtext.clone();
        e.icon = r.icon.clone();
        e.actions = r.actions.clone();
    }

    /// The score of `r` at `now`, zero when it has none. A hash lookup, so
    /// ranking a result set is linear in its length.
    pub fn score(&self, r: &SearchResult, now: f64) -> f64 {
        self.entries
            .get(&key(&r.provider, &r.identifier))
            .map_or(0.0, |e| e.score_at(now))
    }

    /// `results` in elephant's order, each moved up by its frecency.
    /// Stable: rows with equal standing keep elephant's order.
    pub fn rank(&self, results: Vec<SearchResult>, now: f64) -> Vec<SearchResult> {
        let mut keyed: Vec<(f64, usize, SearchResult)> = results
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                let lift = WEIGHT * self.score(&r, now).ln_1p();
                (i as f64 - lift, i, r)
            })
            .collect();
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        keyed.into_iter().map(|(_, _, r)| r).collect()
    }

    /// The most used items at `now`, best first, for the empty query.
    pub fn top(&self, n: usize, now: f64) -> Vec<SearchResult> {
        let mut all: Vec<&Entry> = self.entries.values().collect();
        all.sort_by(|a, b| b.score_at(now).total_cmp(&a.score_at(now)));
        all.into_iter()
            .filter(|e| e.score_at(now) >= FORGET_BELOW)
            .take(n)
            .map(Entry::result)
            .collect()
    }

    /// The store as it is written: faded items dropped, at most
    /// [`MAX_ENTRIES`], the best kept.
    pub fn pruned(&self, now: f64) -> Store {
        let mut all: Vec<(&String, &Entry)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.score_at(now) >= FORGET_BELOW)
            .collect();
        all.sort_by(|a, b| b.1.score_at(now).total_cmp(&a.1.score_at(now)));
        Store {
            entries: all
                .into_iter()
                .take(MAX_ENTRIES)
                .map(|(k, e)| (k.clone(), e.clone()))
                .collect(),
        }
    }
}

// ── The process's store ─────────────────────────────────────────────────

thread_local! {
    /// `None` until the file has been read.
    static STORE: RefCell<Option<Store>> = const { RefCell::new(None) };
    static LOADING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SAVE: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
}

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("swaypplet").join("launcher-frecency.json"))
}

/// Read the file on a worker, once per process. Until it lands the launcher
/// ranks as elephant does, which is what it did before this module.
pub fn load() {
    if LOADING.with(|l| l.replace(true)) {
        return;
    }
    crate::spawn::spawn_work(
        || {
            path()
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|b| serde_json::from_slice::<Store>(&b).ok())
                .unwrap_or_default()
        },
        |store| {
            STORE.with(|s| {
                let mut s = s.borrow_mut();
                // A launch recorded before the file arrived is kept.
                let mut merged = store;
                if let Some(early) = s.take() {
                    merged.entries.extend(early.entries);
                }
                *s = Some(merged);
            });
        },
    );
}

/// Read the store, if it has loaded.
pub fn with<T>(f: impl FnOnce(&Store) -> T) -> Option<T> {
    STORE.with(|s| s.borrow().as_ref().map(f))
}

/// Count a launch, and schedule the write.
pub fn record(r: &SearchResult) {
    if !remembers(&r.provider) {
        return;
    }
    STORE.with(|s| {
        s.borrow_mut()
            .get_or_insert_with(Store::default)
            .record(r, now())
    });
    schedule_save();
}

/// Forget every launch, in memory and on disk.
pub fn forget() {
    STORE.with(|s| *s.borrow_mut() = Some(Store::default()));
    if let Some(p) = path() {
        crate::spawn::spawn_work(
            move || {
                let _ = std::fs::remove_file(p);
            },
            |()| {},
        );
    }
}

fn schedule_save() {
    SAVE.with(|s| {
        if let Some(id) = s.borrow_mut().take() {
            crate::spawn::remove_source(id);
        }
    });
    let id =
        glib::timeout_add_local_once(std::time::Duration::from_millis(SAVE_DEBOUNCE_MS), || {
            SAVE.with(|s| s.borrow_mut().take());
            let Some(store) = with(|s| s.pruned(now())) else {
                return;
            };
            let Some(p) = path() else { return };
            crate::spawn::spawn_work(move || write(&p, &store), |()| {});
        });
    SAVE.with(|s| *s.borrow_mut() = Some(id));
}

/// Write next to the file and rename over it.
fn write(p: &std::path::Path, store: &Store) {
    let Ok(json) = serde_json::to_vec(store) else {
        return;
    };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, p).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str) -> SearchResult {
        SearchResult {
            identifier: id.into(),
            text: id.into(),
            subtext: String::new(),
            icon: String::new(),
            provider: "desktopapplications".into(),
            score: 0,
            actions: Vec::new(),
        }
    }

    #[test]
    fn a_score_halves_every_half_life() {
        assert!((decayed(8.0, HALF_LIFE_S) - 4.0).abs() < 1e-9);
        assert!((decayed(8.0, 3.0 * HALF_LIFE_S) - 1.0).abs() < 1e-9);
        // A clock that went backwards does not grow a score.
        assert_eq!(decayed(2.0, -100.0), 2.0);
    }

    #[test]
    fn launches_add_up_and_fade() {
        let mut s = Store::default();
        let t0 = 1_000_000.0;
        s.record(&app("a"), t0);
        s.record(&app("a"), t0);
        assert!((s.score(&app("a"), t0) - 2.0).abs() < 1e-9);
        // A week later the two launches count as one, and a third makes two.
        s.record(&app("a"), t0 + HALF_LIFE_S);
        assert!((s.score(&app("a"), t0 + HALF_LIFE_S) - 2.0).abs() < 1e-9);
        assert_eq!(s.score(&app("never"), t0), 0.0);
    }

    #[test]
    fn a_habit_lifts_a_row_but_keeps_the_rest_in_order() {
        let mut s = Store::default();
        let t = 5_000_000.0;
        for _ in 0..10 {
            s.record(&app("d"), t);
        }
        let ranked = s.rank(
            ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
                .map(app)
                .to_vec(),
            t,
        );
        let ids: Vec<&str> = ranked.iter().map(|r| r.identifier.as_str()).collect();
        // ln(11)·3 ≈ 7.2 places: d (at 3) goes to the top; the rest keep
        // elephant's order.
        assert_eq!(ids, ["d", "a", "b", "c", "e", "f", "g", "h", "i", "j"]);
        // Used once, a row at 9 moves up about two places.
        let mut s = Store::default();
        s.record(&app("j"), t);
        let ranked = s.rank(
            ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
                .map(app)
                .to_vec(),
            t,
        );
        assert_eq!(ranked[7].identifier, "j");
    }

    #[test]
    fn the_top_is_best_first_and_the_file_is_bounded() {
        let mut s = Store::default();
        let t = 9_000_000.0;
        for i in 0..(MAX_ENTRIES + 50) {
            let r = app(&format!("app{i}"));
            for _ in 0..=(i % 5) {
                s.record(&r, t);
            }
        }
        let top = s.top(3, t);
        assert_eq!(top.len(), 3);
        assert!(s.score(&top[0], t) >= s.score(&top[2], t));
        assert_eq!(s.pruned(t).entries.len(), MAX_ENTRIES);
        // Faded to nothing, forgotten.
        assert!(s.pruned(t + 100.0 * HALF_LIFE_S).entries.is_empty());
    }

    #[test]
    fn the_store_round_trips_through_json() {
        let mut s = Store::default();
        s.record(&app("firefox"), 1234.5);
        let back: Store = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn only_lasting_things_are_remembered() {
        assert!(remembers("desktopapplications"));
        assert!(remembers("runner"));
        assert!(!remembers("calc"));
        assert!(!remembers("clipboard"));
        assert!(!remembers("swaypplet-window"));
    }
}
