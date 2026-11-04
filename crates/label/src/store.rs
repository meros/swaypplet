//! The saved workspace names, shared by everything in the process that
//! shows or edits one: the toast that appears on a switch (`lib.rs`) and the
//! bar, which draws the name of the workspace a screen is on and lets you
//! change it (`swaypplet-bar`, `name_editor.rs`).
//!
//! State lives in `$XDG_STATE_HOME/swaypplet/workspace-labels.json`. It is
//! state and not a setting: a person fills it in all day, and nothing in it
//! has a Nix default. The file is written whole through a rename, and a
//! file monitor reads a change another process made (the bar can run on its
//! own), so every reader sees the same names whichever process wrote them.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use gio::prelude::*;
use serde_json::{Value, json};

/// Whether the toast shows by itself when you switch to a named workspace.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// Never on its own. The bar still shows the name.
    Hidden,
    /// For a moment after you settle on a workspace that has a name.
    #[default]
    Auto,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Hidden => "hidden",
            Mode::Auto => "auto",
        }
    }

    /// `always` is the mode this one replaced: a pill that stayed up on
    /// every named workspace. It was the loudest of the three, so a file
    /// that still says it gets the quiet default.
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "hidden" => Some(Mode::Hidden),
            "auto" | "always" => Some(Mode::Auto),
            _ => None,
        }
    }

    pub fn next(self) -> Mode {
        match self {
            Mode::Hidden => Mode::Auto,
            Mode::Auto => Mode::Hidden,
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq, Clone)]
pub(crate) struct Data {
    pub mode: Mode,
    /// Workspace name → label. Never holds an empty label.
    pub labels: BTreeMap<String, String>,
}

impl Data {
    pub fn parse(raw: &[u8]) -> Data {
        let Ok(value) = serde_json::from_slice::<Value>(raw) else {
            return Data::default();
        };
        Data {
            mode: value["mode"]
                .as_str()
                .and_then(Mode::parse)
                .unwrap_or_default(),
            labels: value["labels"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(ws, text)| {
                    let text = text.as_str()?.trim();
                    (!text.is_empty()).then(|| (ws.clone(), text.to_string()))
                })
                .collect(),
        }
    }

    pub fn to_json(&self) -> String {
        let text = json!({ "mode": self.mode.as_str(), "labels": self.labels });
        serde_json::to_string_pretty(&text).unwrap_or_default()
    }

    fn path() -> PathBuf {
        swaypplet_base::paths::state_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("swaypplet")
            .join("workspace-labels.json")
    }

    fn load() -> Data {
        std::fs::read(Self::path())
            .map(|raw| Data::parse(&raw))
            .unwrap_or_default()
    }

    /// Written whole through a rename, so a crash mid-write leaves the old
    /// file and never half of the new one.
    fn save(&self) {
        let path = Self::path();
        if let Err(e) = swaypplet_base::fs::write_atomic(&path, self.to_json()) {
            log::warn!("label: could not save {}: {e}", path.display());
        }
    }
}

struct Store {
    data: RefCell<Data>,
    subscribers: RefCell<Vec<Rc<dyn Fn()>>>,
    /// Asks whoever draws the editor (the bar) to open it for a workspace.
    editors: RefCell<Vec<Rc<dyn Fn(&str, &str)>>>,
    /// Held for its signal.
    _monitor: RefCell<Option<gio::FileMonitor>>,
}

thread_local! {
    static STORE: Rc<Store> = Store::new();
}

impl Store {
    fn new() -> Rc<Store> {
        let store = Rc::new(Store {
            data: RefCell::new(Data::load()),
            subscribers: RefCell::new(Vec::new()),
            editors: RefCell::new(Vec::new()),
            _monitor: RefCell::new(None),
        });
        let file = gio::File::for_path(Data::path());
        if let Ok(monitor) = file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            let weak = Rc::downgrade(&store);
            monitor.connect_changed(move |_, _, _, _| {
                let Some(store) = weak.upgrade() else { return };
                let fresh = Data::load();
                if *store.data.borrow() == fresh {
                    return;
                }
                *store.data.borrow_mut() = fresh;
                store.notify();
            });
            *store._monitor.borrow_mut() = Some(monitor);
        }
        store
    }

    fn notify(&self) {
        let subscribers: Vec<_> = self.subscribers.borrow().clone();
        for f in subscribers {
            f();
        }
    }
}

fn with<R>(f: impl FnOnce(&Store) -> R) -> R {
    STORE.with(|s| f(s))
}

/// The name saved for the workspace called `workspace`, if it has one.
pub fn name_of(workspace: &str) -> Option<String> {
    with(|s| s.data.borrow().labels.get(workspace).cloned())
}

/// Save `text` as the name of `workspace`; empty text removes it.
pub fn set_name(workspace: &str, text: &str) {
    let text = text.trim();
    with(|s| {
        {
            let mut data = s.data.borrow_mut();
            let before = data.clone();
            if text.is_empty() {
                data.labels.remove(workspace);
            } else {
                data.labels.insert(workspace.to_string(), text.to_string());
            }
            if *data == before {
                return;
            }
            data.save();
        }
        s.notify();
    });
}

pub fn mode() -> Mode {
    with(|s| s.data.borrow().mode)
}

pub fn set_mode(mode: Mode) {
    with(|s| {
        {
            let mut data = s.data.borrow_mut();
            if data.mode == mode {
                return;
            }
            data.mode = mode;
            data.save();
        }
        s.notify();
    });
}

/// Call `f` whenever a name or the mode changes, here or in another process.
pub fn connect_changed(f: impl Fn() + 'static) {
    with(|s| s.subscribers.borrow_mut().push(Rc::new(f)));
}

/// Register what opens the editor for a workspace's name: called with the
/// output that has focus and the workspace to name.
pub fn connect_edit(f: impl Fn(&str, &str) + 'static) {
    with(|s| s.editors.borrow_mut().push(Rc::new(f)));
}

/// Ask every registered editor to open for `workspace` on `output`.
pub(crate) fn request_edit(output: &str, workspace: &str) {
    let editors: Vec<_> = with(|s| s.editors.borrow().clone());
    for f in editors {
        f(output, workspace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_round_trips_and_keeps_swedish_letters() {
        let mut data = Data {
            mode: Mode::Hidden,
            ..Data::default()
        };
        data.labels.insert("3".into(), "Fakturering åäö".into());
        assert_eq!(Data::parse(data.to_json().as_bytes()), data);
    }

    #[test]
    fn a_bad_file_reads_as_the_default() {
        assert_eq!(Data::parse(b"not json"), Data::default());
        assert_eq!(Data::default().mode, Mode::Auto);
    }

    #[test]
    fn empty_and_wrong_typed_labels_are_dropped_and_an_unknown_mode_is_auto() {
        let data = Data::parse(br#"{"mode":"loud","labels":{"1":"  ","2":5,"3":" ok "}}"#);
        assert_eq!(data.mode, Mode::Auto);
        assert_eq!(data.labels.len(), 1);
        assert_eq!(data.labels["3"], "ok");
    }

    #[test]
    fn the_retired_always_mode_reads_as_auto() {
        assert_eq!(Mode::parse("always"), Some(Mode::Auto));
        assert_eq!(Mode::parse("hidden"), Some(Mode::Hidden));
        assert_eq!(Mode::Hidden.next(), Mode::Auto);
        assert_eq!(Mode::Auto.next(), Mode::Hidden);
    }
}
