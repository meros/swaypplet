//! The pre-built NixOS update, as root's services report it.
//!
//! A root service builds the next system ahead of time and writes
//! `/run/nx/prebuilt.json` (atomic replace; absent until the first run;
//! world-readable). The bar's mark reads it through [`PrebuiltService`] and
//! starts the switch with [`start_apply`]. The file's shape:
//!
//! ```json
//! { "host": "meros-laptop",
//!   "state": "idle" | "building" | "ready" | "switching" | "failed",
//!   "ready": true,
//!   "target": "auto/meros-laptop", "rev": "<40 hex>", "running": "<40 hex>",
//!   "inputs": ["nixpkgs"], "built_at": "2026-11-11T03:12:09+01:00",
//!   "detail": "short text" }
//! ```
//!
//! Only `state` is required. Everything else degrades to empty, so a file
//! from an older writer still draws; a file that is not JSON, is empty, or
//! names a state this build does not know draws nothing ([`parse`] is
//! `None`) and never panics the bar.
//!
//! The directory is watched, not the file: the writer replaces the file by
//! rename, which an inotify watch on the old inode never sees. The 30 s tick
//! only covers a directory that did not exist yet (or was recreated) when
//! the watch was set; it re-reads one small file and costs nothing else.

use std::cell::{Cell, RefCell};
use std::io::Read;
use std::path::Path;
use std::rc::Rc;

use gio::prelude::*;

use swaypplet_base::service::Observed;

use crate::systemd::{Unit, Verb};

pub const DIR: &str = "/run/nx";
pub const FILE: &str = "/run/nx/prebuilt.json";
/// The unit a click starts. The nixos side grants this user `start` on it.
pub const APPLY_UNIT: &str = "nx-apply.service";

/// A pathological file must not cost the bar memory: the real one is under
/// 1 KiB.
const MAX_BYTES: u64 = 64 * 1024;
/// Re-attach the directory watch if it is missing, and re-read.
const TICK_S: u32 = 30;
/// Characters of the commit shown in the tooltip.
const SHORT_REV: usize = 7;

/// What the writer says it is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Building,
    Ready,
    Switching,
    Failed,
}

impl State {
    fn parse(s: &str) -> Option<State> {
        Some(match s {
            "idle" => State::Idle,
            "building" => State::Building,
            "ready" => State::Ready,
            "switching" => State::Switching,
            "failed" => State::Failed,
            _ => return None,
        })
    }
}

/// The file, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prebuilt {
    pub state: State,
    pub ready: bool,
    pub target: String,
    pub rev: String,
    pub running: String,
    pub inputs: Vec<String>,
    /// Seconds since the epoch; `None` when absent or not RFC 3339.
    pub built_at: Option<u64>,
    pub detail: String,
}

/// What the bar draws for a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    /// An image newer than the running system is waiting: a click switches.
    Ready,
    Building,
    Switching,
    Failed,
}

impl Prebuilt {
    /// `None` hides the mark: nothing is waiting and nothing is going on.
    pub fn shown(&self) -> Option<Shown> {
        match self.state {
            State::Idle | State::Ready if self.ready => Some(Shown::Ready),
            State::Idle | State::Ready => None,
            State::Building => Some(Shown::Building),
            State::Switching => Some(Shown::Switching),
            State::Failed => Some(Shown::Failed),
        }
    }

    /// The tooltip, `now` in seconds since the epoch.
    pub fn tooltip(&self, shown: Shown, now: u64) -> String {
        match shown {
            Shown::Ready => {
                let mut line = String::from("Update ready");
                let rev: String = self.rev.chars().take(SHORT_REV).collect();
                if !rev.is_empty() {
                    line.push_str(": ");
                    line.push_str(&rev);
                }
                if let Some(at) = self.built_at {
                    line.push_str(&format!(
                        " (built {})",
                        swaypplet_base::fmt::ago(
                            now.saturating_sub(at),
                            swaypplet_base::fmt::Ago::Rounded
                        )
                    ));
                }
                if !self.inputs.is_empty() {
                    line.push_str(&format!("\nChanged: {}", self.inputs.join(", ")));
                }
                line.push_str("\nClick to switch.");
                line
            }
            Shown::Building => "Update building".to_string(),
            Shown::Switching => "Switching to the new system".to_string(),
            Shown::Failed if self.detail.is_empty() => "Update failed".to_string(),
            Shown::Failed => self.detail.clone(),
        }
    }
}

/// The file's text as a [`Prebuilt`]; `None` for anything that is not a
/// well-formed report (empty, not JSON, not an object, no `state`, a state
/// this build does not know).
pub fn parse(text: &str) -> Option<Prebuilt> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let object = value.as_object()?;
    let text_of = |key: &str| {
        object
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    Some(Prebuilt {
        state: State::parse(object.get("state")?.as_str()?)?,
        ready: object
            .get("ready")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        target: text_of("target"),
        rev: text_of("rev"),
        running: text_of("running"),
        inputs: object
            .get("inputs")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        built_at: parse_rfc3339(&text_of("built_at")),
        detail: text_of("detail"),
    })
}

/// `2026-11-11T03:12:09+01:00` (or `Z`, or with fractional seconds) as
/// seconds since the epoch. The workspace carries no date crate for the one
/// timestamp it reads.
fn parse_rfc3339(s: &str) -> Option<u64> {
    let (date, rest) = s.split_once(['T', 't', ' '])?;
    let mut d = date.splitn(3, '-');
    let (y, m, day): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    let split = rest.find(['Z', 'z', '+', '-']);
    let (clock, zone) = rest.split_at(split?);
    let mut c = clock.splitn(3, ':');
    let (h, mi): (i64, i64) = (c.next()?.parse().ok()?, c.next()?.parse().ok()?);
    let sec: i64 = c.next()?.split('.').next()?.parse().ok()?;
    if h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let offset = match zone {
        "Z" | "z" => 0,
        z => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let (oh, om) = z[1..].split_once(':')?;
            let (oh, om): (i64, i64) = (oh.parse().ok()?, om.parse().ok()?);
            if oh > 23 || om > 59 {
                return None;
            }
            sign * (oh * 3600 + om * 60)
        }
    };
    // Days from 1970-01-01 (Howard Hinnant's civil-date algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + mi * 60 + sec - offset).ok()
}

/// Seconds since the epoch.
pub fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The file at `path`, parsed. A missing, unreadable or malformed file is
/// `None`; only a file that exists and fails to parse is worth a log line.
fn read_file(path: &Path) -> Option<Prebuilt> {
    let mut text = String::new();
    match std::fs::File::open(path) {
        Ok(f) => f.take(MAX_BYTES).read_to_string(&mut text).ok()?,
        Err(_) => return None,
    };
    let parsed = parse(&text);
    if parsed.is_none() {
        log::debug!("nx prebuilt: ignoring unreadable {}", path.display());
    }
    parsed
}

/// Start the switch: `systemctl start --no-block nx-apply.service`. Queued,
/// because the unit is a long oneshot; blocking all the same (the polkit
/// round trip), so call it on a worker. The unit restarts this process, so
/// nothing here waits for it.
pub fn start_apply() -> Result<(), String> {
    Unit::system(APPLY_UNIT).queue(Verb::Start)
}

pub struct PrebuiltService {
    state: Observed<Option<Prebuilt>>,
    /// A dropped GFileMonitor stops watching.
    monitor: RefCell<Option<gio::FileMonitor>>,
    /// The watched directory went away: attach again on the next tick.
    lost: Cell<bool>,
}

impl PrebuiltService {
    pub fn start() -> Rc<Self> {
        let service = Rc::new(Self {
            state: Observed::new(read_file(Path::new(FILE))),
            monitor: RefCell::new(None),
            lost: Cell::new(false),
        });
        service.attach();

        let weak = Rc::downgrade(&service);
        glib::timeout_add_seconds_local(TICK_S, move || match weak.upgrade() {
            Some(service) => {
                service.attach();
                service.refresh();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        service
    }

    /// Watch [`DIR`] unless a watch is already standing. The directory may
    /// not exist yet, which is not worth more than a debug line.
    fn attach(self: &Rc<Self>) {
        if self.monitor.borrow().is_some() && !self.lost.get() {
            return;
        }
        match gio::File::for_path(DIR)
            .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            Ok(monitor) => {
                let weak = Rc::downgrade(self);
                monitor.connect_changed(move |_, file, _, event| {
                    let Some(service) = weak.upgrade() else {
                        return;
                    };
                    if event == gio::FileMonitorEvent::Deleted
                        && file.path().is_some_and(|p| p == Path::new(DIR))
                    {
                        service.lost.set(true);
                    }
                    service.refresh();
                });
                self.monitor.replace(Some(monitor));
                self.lost.set(false);
            }
            Err(e) => log::debug!("nx prebuilt: watch {DIR}: {e}"),
        }
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.state.connect_change(cb);
    }

    pub fn snapshot(&self) -> Option<Prebuilt> {
        self.state.with(Clone::clone)
    }

    fn refresh(&self) {
        self.state.set_if_changed(read_file(Path::new(FILE)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WRITTEN: &str = r#"{
      "host": "meros-laptop",
      "state": "ready",
      "ready": true,
      "target": "auto/meros-laptop",
      "rev": "0123456789abcdef0123456789abcdef01234567",
      "running": "fedcba9876543210fedcba9876543210fedcba98",
      "inputs": ["nixpkgs", "home-manager"],
      "built_at": "2026-11-11T03:12:09+01:00",
      "detail": "built in 14 min"
    }"#;

    #[test]
    fn the_writers_own_file_parses() {
        let p = parse(WRITTEN).expect("a valid file");
        assert_eq!(p.state, State::Ready);
        assert!(p.ready);
        assert_eq!(p.target, "auto/meros-laptop");
        assert_eq!(p.inputs, ["nixpkgs", "home-manager"]);
        assert_eq!(p.detail, "built in 14 min");
        // 2026-11-11 02:12:09 UTC.
        assert_eq!(p.built_at, Some(1_794_363_129));
        assert_eq!(p.shown(), Some(Shown::Ready));
    }

    #[test]
    fn the_ready_tooltip_has_rev_age_and_inputs() {
        let p = parse(WRITTEN).unwrap();
        let now = p.built_at.unwrap() + 20 * 60;
        assert_eq!(
            p.tooltip(Shown::Ready, now),
            "Update ready: 0123456 (built 20 min ago)\nChanged: nixpkgs, home-manager\nClick to switch."
        );
    }

    #[test]
    fn a_ready_tooltip_without_inputs_or_time_stays_short() {
        let p = parse(r#"{"state":"ready","ready":true,"rev":"abc"}"#).unwrap();
        assert_eq!(
            p.tooltip(Shown::Ready, 0),
            "Update ready: abc\nClick to switch."
        );
        let p = parse(r#"{"state":"ready","ready":true}"#).unwrap();
        assert_eq!(p.tooltip(Shown::Ready, 0), "Update ready\nClick to switch.");
    }

    #[test]
    fn a_built_time_in_the_future_reads_as_just_now() {
        let p = parse(WRITTEN).unwrap();
        let tip = p.tooltip(Shown::Ready, p.built_at.unwrap() - 500);
        assert!(tip.contains("(built just now)"), "{tip}");
    }

    #[test]
    fn missing_fields_degrade_to_empty() {
        let p = parse(r#"{"state":"building"}"#).unwrap();
        assert_eq!(p.state, State::Building);
        assert!(!p.ready);
        assert!(p.rev.is_empty() && p.inputs.is_empty() && p.detail.is_empty());
        assert_eq!(p.built_at, None);
        assert_eq!(p.shown(), Some(Shown::Building));
        assert_eq!(p.tooltip(Shown::Building, 0), "Update building");
    }

    #[test]
    fn fields_of_the_wrong_type_do_not_fail_the_file() {
        let p = parse(r#"{"state":"ready","ready":"yes","inputs":"nixpkgs","rev":7}"#).unwrap();
        assert!(!p.ready);
        assert!(p.inputs.is_empty());
        assert!(p.rev.is_empty());
        assert_eq!(p.shown(), None);
    }

    #[test]
    fn an_unknown_or_absent_state_draws_nothing() {
        assert_eq!(parse(r#"{"state":"paused","ready":true}"#), None);
        assert_eq!(parse(r#"{"ready":true}"#), None);
        assert_eq!(parse(r#"{"state":3}"#), None);
    }

    #[test]
    fn malformed_and_empty_files_draw_nothing() {
        for text in [
            "",
            "  \n",
            "{",
            "not json",
            "[]",
            "null",
            "\"ready\"",
            "{\"state\":",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn idle_and_not_ready_is_hidden() {
        let p = parse(r#"{"state":"idle","ready":false}"#).unwrap();
        assert_eq!(p.shown(), None);
        // Ready is the bar's word for "something is waiting" in either state.
        let p = parse(r#"{"state":"idle","ready":true}"#).unwrap();
        assert_eq!(p.shown(), Some(Shown::Ready));
        // A "ready" state that says nothing is waiting is not clickable.
        let p = parse(r#"{"state":"ready","ready":false}"#).unwrap();
        assert_eq!(p.shown(), None);
    }

    #[test]
    fn every_working_state_has_its_own_mark() {
        let shown = |s: &str| {
            parse(&format!(r#"{{"state":"{s}","ready":false}}"#))
                .unwrap()
                .shown()
        };
        assert_eq!(shown("building"), Some(Shown::Building));
        assert_eq!(shown("switching"), Some(Shown::Switching));
        assert_eq!(shown("failed"), Some(Shown::Failed));
    }

    #[test]
    fn a_failure_says_why_or_says_that_it_failed() {
        let p = parse(r#"{"state":"failed","detail":" build of meros-laptop failed "}"#).unwrap();
        assert_eq!(p.tooltip(Shown::Failed, 0), "build of meros-laptop failed");
        let p = parse(r#"{"state":"failed"}"#).unwrap();
        assert_eq!(p.tooltip(Shown::Failed, 0), "Update failed");
    }

    #[test]
    fn rfc3339_in_every_spelling_the_writers_use() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("1970-01-02T01:00:00+01:00"), Some(86_400));
        assert_eq!(
            parse_rfc3339("2026-11-11T03:12:09+01:00"),
            Some(1_794_363_129)
        );
        assert_eq!(
            parse_rfc3339("2026-11-11T02:12:09.123456Z"),
            Some(1_794_363_129)
        );
        assert_eq!(
            parse_rfc3339("2026-11-10T21:12:09-05:00"),
            Some(1_794_363_129)
        );
        // Leap day.
        assert_eq!(parse_rfc3339("2024-02-29T00:00:00Z"), Some(1_709_164_800));
    }

    #[test]
    fn rfc3339_rejects_what_is_not_a_time() {
        for s in [
            "",
            "yesterday",
            "2026-11-11",
            "2026-13-11T00:00:00Z",
            "2026-11-11T25:00:00Z",
            "2026-11-11T03:12:09",
            "2026-11-11T03:12:09+1",
            "1969-12-31T00:00:00Z",
        ] {
            assert_eq!(parse_rfc3339(s), None, "{s:?}");
        }
    }

    #[test]
    fn a_bad_time_keeps_the_rest_of_the_file() {
        let p =
            parse(r#"{"state":"ready","ready":true,"rev":"abcdef012","built_at":"soon"}"#).unwrap();
        assert_eq!(p.built_at, None);
        assert_eq!(
            p.tooltip(Shown::Ready, 0),
            "Update ready: abcdef0\nClick to switch."
        );
    }

    #[test]
    fn a_missing_file_is_none() {
        assert_eq!(read_file(Path::new("/nonexistent/nx/prebuilt.json")), None);
    }
}
