//! The keybinding overlay: a glass sheet the compositor writes itself.
//!
//! The old overlay was a `foot` terminal running `cat` on a text file, shown
//! and hidden through sway window rules and two transient systemd units. It
//! looked like a terminal because it was one, and its contents were curated by
//! hand, which opened `keybinds-help.nix` with the instruction to "update the
//! matching line here in the same commit". That instruction is the bug: by
//! 2026-08 the sheet still advertised four screens per task (`1234 / qwer /
//! asdf / zxcv`) against a grid that had been two screens since the screens c
//! and d were retired.
//!
//! So the sheet is derived instead. Sway's IPC hands back the config it
//! actually loaded ([`sway_ipc::config_text`]); every `bindsym` in it becomes a
//! row, and the only hand-written part is presentation: which section a command
//! belongs in, and how to say it in fewer characters than the command itself.
//! A binding whose command matches nothing still renders, under `OTHER`, with
//! its command shortened but intact. Nothing can go missing, which is the whole
//! point — a cheat sheet that silently omits a binding is worse than no cheat
//! sheet.
//!
//! The surface is the shell's own glass, centered, keyboard-free: it appears
//! while Super is held and leaves when it is released, so it has no business
//! taking focus.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;

use crate::anim;
use crate::bar::workspaces::generic_label;
use crate::layer_shell::{self, LayerShellConfig};
use crate::sway_ipc;

// ── The sheet's structure ───────────────────────────────────────────────

/// Where a binding lands on the sheet. Order is print order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Section {
    Tasks,
    Workspaces,
    Windows,
    Apps,
    Media,
    System,
    Other,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::Tasks => "TASKS",
            Section::Workspaces => "WORKSPACES",
            Section::Windows => "WINDOWS",
            Section::Apps => "APPS",
            Section::Media => "MEDIA / DISPLAY",
            Section::System => "SYSTEM",
            Section::Other => "OTHER",
        }
    }

    const ALL: [Section; 7] = [
        Section::Tasks,
        Section::Workspaces,
        Section::Windows,
        Section::Apps,
        Section::Media,
        Section::System,
        Section::Other,
    ];
}

/// One `bindsym` line, split but not yet interpreted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    pub mods: Vec<String>,
    pub key: String,
    pub command: String,
}

/// The whole sheet: sections in print order, each with its rows.
pub(crate) type Sheet = Vec<(Section, Vec<Row>)>;

/// One printed line: the keys on the left, what they do on the right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub keys: String,
    pub label: String,
}

// ── Parsing ─────────────────────────────────────────────────────────────

/// Pull the `bindsym` lines out of a sway config.
///
/// Only depth 0 counts. Bindings nested in a `mode { … }` or `bar { … }`
/// block belong to a keyboard state this sheet does not describe, and
/// printing them beside the default-mode bindings would claim they are
/// reachable when they are not.
pub(crate) fn parse(config: &str) -> Vec<Binding> {
    let mut depth = 0i32;
    let mut out = Vec::new();

    for line in config.lines() {
        let line = line.trim();

        if line.starts_with("bindsym")
            && depth == 0
            && let Some(binding) = parse_bindsym(line)
        {
            out.push(binding);
        }

        // Cheap brace tracking. Sway's own emitted config puts at most one
        // brace on a line, which is why this does not need to lex strings.
        depth += line.matches('{').count() as i32;
        depth -= line.matches('}').count() as i32;
        depth = depth.max(0);
    }

    out
}

fn parse_bindsym(line: &str) -> Option<Binding> {
    let mut tokens = line.split_whitespace();
    if tokens.next()? != "bindsym" {
        return None;
    }

    // `--release`, `--locked`, `--to-code`, `--no-warn`: modifiers on when the
    // binding fires, not on what it is bound to.
    let combo = tokens.by_ref().find(|t| !t.starts_with("--"))?;
    let command = tokens.collect::<Vec<_>>().join(" ");
    if command.is_empty() {
        return None;
    }

    let mut parts: Vec<&str> = combo.split('+').collect();
    let key = parts.pop()?.to_string();
    let mods = parts.into_iter().map(str::to_string).collect();

    Some(Binding { mods, key, command })
}

// ── Saying it shorter ───────────────────────────────────────────────────

fn pretty_mods(mods: &[String]) -> String {
    // Fixed order, so ⌘⇧ never renders as ⇧⌘ depending on how the config
    // happened to spell the combo.
    const ORDER: [(&str, &str); 6] = [
        ("Mod4", "⌘"),
        ("Ctrl", "⌃"),
        ("Control", "⌃"),
        ("Mod1", "⌥"),
        ("Alt", "⌥"),
        ("Shift", "⇧"),
    ];

    let mut out = String::new();
    for (name, glyph) in ORDER {
        if mods.iter().any(|m| m == name) && !out.contains(glyph) {
            out.push_str(glyph);
        }
    }
    // Anything unrecognised still shows, spelled out, rather than vanishing.
    for m in mods {
        if !ORDER.iter().any(|(name, _)| name == m) {
            out.push_str(m);
        }
    }
    out
}

fn pretty_key(key: &str) -> String {
    let named = match key {
        "Return" | "KP_Enter" => "↵",
        "BackSpace" => "⌫",
        "Escape" => "⎋",
        "space" => "Space",
        "backslash" => "\\",
        "bracketleft" => "[",
        "bracketright" => "]",
        "comma" => ",",
        "period" => ".",
        "minus" => "-",
        "equal" => "=",
        "semicolon" => ";",
        "apostrophe" => "'",
        "grave" => "`",
        "slash" => "/",
        "Left" => "←",
        "Right" => "→",
        "Up" => "↑",
        "Down" => "↓",
        "Tab" => "⇥",
        "Delete" => "⌦",
        "Print" => "Print",
        "XF86AudioRaiseVolume" => "Vol+",
        "XF86AudioLowerVolume" => "Vol−",
        "XF86AudioMute" => "Mute",
        "XF86AudioMicMute" => "MicMute",
        "XF86AudioPlay" => "Play",
        "XF86AudioPause" => "Pause",
        "XF86AudioNext" => "Next",
        "XF86AudioPrev" => "Prev",
        "XF86AudioStop" => "Stop",
        "XF86MonBrightnessUp" => "Bright+",
        "XF86MonBrightnessDown" => "Bright−",
        "XF86ScreenSaver" => "ScreenSaver",
        "XF86Sleep" => "Sleep",
        "Caps_Lock" => "CapsLock",
        "Num_Lock" => "NumLock",
        "Scroll_Lock" => "ScrollLock",
        other => other,
    };
    named.to_string()
}

/// Drop the `/nix/store/<hash>-` noise so a command reads as the script it is.
///
/// Store paths are most of the width of a generated sway config and none of
/// the meaning: `/nix/store/wf53…-task-switch 1:t1a` is `task-switch 1:t1a`.
pub(crate) fn shorten(command: &str) -> String {
    command
        .split_whitespace()
        .map(|token| {
            if !token.contains("/nix/store/") {
                return token.to_string();
            }
            let short = token.rsplit('/').next().unwrap_or(token);
            // `<32 chars of hash>-name` for a script written straight into the
            // store; a package's `bin/foo` is already just the name.
            match short.split_once('-') {
                Some((hash, name)) if hash.len() == 32 => name.to_string(),
                _ => short.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_exec(command: &str) -> &str {
    let rest = command
        .strip_prefix("exec ")
        .map(|rest| rest.strip_prefix("--no-startup-id ").unwrap_or(rest))
        .unwrap_or(command);
    // sway quotes a command containing `&&` or `;` as a whole. Those quotes
    // belong to sway's parser, not to the command, and leaving them on strands
    // an unbalanced `'` at the end of the printed label.
    for quote in ['\'', '"'] {
        if let Some(inner) = rest.strip_prefix(quote).and_then(|r| r.strip_suffix(quote)) {
            return inner;
        }
    }
    rest
}

/// The workspace a `workspace number 19:wb` style argument points at, as the
/// bar already draws it (`󰖟 b`), so the sheet and the bar agree on names.
fn workspace_label(arg: &str) -> String {
    match arg.split_once(':') {
        Some((num, _)) => match num.parse::<i32>() {
            Ok(n) => generic_label(n, arg).to_string(),
            Err(_) => arg.to_string(),
        },
        None => arg.to_string(),
    }
}

/// What a command does, and which family it belongs to.
///
/// The family is how rows collapse: four `focus` bindings that differ only in
/// direction print as one row with four keys. `None` means the row stands
/// alone.
fn describe(command: &str) -> (Section, String, Option<&'static str>) {
    // Unquote before shortening: sway wraps a whole command in quotes when it
    // contains `&&`, and shortening a store path inside that wrapper would
    // leave the closing quote stranded on a token that no longer opens one.
    let cmd = shorten(strip_exec(command)).trim().to_string();
    let words: Vec<&str> = cmd.split_whitespace().collect();

    match words.as_slice() {
        // ── Tasks ──
        ["task-switch", arg] => {
            let (task, screen) = task_of(arg);
            match task {
                Some(n) => (
                    Section::Tasks,
                    format!("Task {n} screens"),
                    Some(match n {
                        1 => "task1",
                        2 => "task2",
                        3 => "task3",
                        _ => "task4",
                    }),
                ),
                None => (Section::Tasks, format!("Task workspace {screen}"), None),
            }
        }
        ["task-swap", _] => (
            Section::Tasks,
            "Swap task contents".into(),
            Some("task-swap"),
        ),
        ["task-find"] => (Section::Tasks, "Find task → jump".into(), None),
        ["task-rename"] => (Section::Tasks, "Rename current task".into(), None),
        ["task-move-ws", _] => (
            Section::Tasks,
            "Move workspace to output".into(),
            Some("task-move-ws"),
        ),

        // ── Workspaces ──
        ["workspace", "number", arg] => (Section::Workspaces, workspace_label(arg), None),
        ["workspace", arg] => (Section::Workspaces, workspace_label(arg), None),
        ["move", "container", "to", "workspace", "number", _]
        | ["move", "container", "to", "workspace", _] => (
            Section::Workspaces,
            "Move window to workspace".into(),
            Some("move-to-ws"),
        ),

        // ── Windows ──
        ["focus", dir] if is_direction(dir) => (Section::Windows, "Focus".into(), Some("focus")),
        ["move", dir] if is_direction(dir) => {
            (Section::Windows, "Move window".into(), Some("move"))
        }
        ["resize", ..] => (Section::Windows, "Resize".into(), Some("resize")),
        ["layout", "toggle", "split"] => (Section::Windows, "Toggle split".into(), None),
        ["layout", mode] => (Section::Windows, capitalize(mode), None),
        ["fullscreen", ..] => (Section::Windows, "Fullscreen".into(), None),
        ["floating", "toggle"] => (Section::Windows, "Float toggle".into(), None),
        ["sticky", "toggle"] => (Section::Windows, "Sticky toggle".into(), None),
        ["kill"] => (Section::Windows, "Kill window".into(), None),
        ["scratchpad", "show"] => (Section::Windows, "Scratchpad show".into(), None),
        ["move", "scratchpad"] => (Section::Windows, "Move to scratchpad".into(), None),

        // ── Apps ──
        ["chrome-profile-launcher", ..] => (Section::Apps, "Chrome profiles".into(), None),
        ["swaypplet-launcher", ..] => (Section::Apps, "Launcher".into(), None),
        ["swaypplet-toggle", ..] => (Section::Apps, "Control centre".into(), None),
        ["alacritty", ..] | ["foot", ..] | ["kitty", ..] => {
            (Section::Apps, "Terminal".into(), None)
        }

        // ── Media / display ──
        ["swaypplet-osd", "--output-volume", _] => {
            (Section::Media, "Volume".into(), Some("osd-output-volume"))
        }
        ["swaypplet-osd", "--input-volume", _] => (Section::Media, "Mic mute".into(), None),
        ["swaypplet-osd", "--brightness", _] => {
            (Section::Media, "Brightness".into(), Some("osd-brightness"))
        }
        ["swaypplet-osd", flag] => (
            Section::Media,
            capitalize(flag.trim_start_matches("--")),
            None,
        ),
        ["playerctl", "play-pause"] => (Section::Media, "Play / pause".into(), Some("play-pause")),
        ["playerctl", action] => (Section::Media, capitalize(action), None),
        ["grim", ..] if cmd.contains("wl-copy") => {
            (Section::Media, "Screenshot region → clipboard".into(), None)
        }
        ["grim", ..] => (Section::Media, "Screenshot region → file".into(), None),

        // ── System ──
        ["loginctl", "lock-session"] => (Section::System, "Lock screen".into(), Some("lock")),
        ["reload"] => (Section::System, "Reload sway".into(), None),
        ["exit"] => (Section::System, "Exit sway".into(), None),

        // ── Everything else, verbatim ──
        _ => (Section::Other, cmd, None),
    }
}

fn is_direction(word: &str) -> bool {
    matches!(word, "left" | "right" | "up" | "down")
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `1:t1a` → task 1, screen `a`.
fn task_of(arg: &str) -> (Option<u32>, String) {
    let name = arg.split_once(':').map_or(arg, |(_, n)| n);
    let rest = match name.strip_prefix('t') {
        Some(rest) => rest,
        None => return (None, name.to_string()),
    };
    let mut chars = rest.chars();
    match chars.next().and_then(|c| c.to_digit(10)) {
        Some(n) => (Some(n), chars.as_str().to_string()),
        None => (None, name.to_string()),
    }
}

// ── Laying it out ───────────────────────────────────────────────────────

/// Turn bindings into printed rows, one list per section.
///
/// Collapsing happens here: bindings sharing a modifier set and a family
/// (`⌘ ←`, `⌘ ↑`, `⌘ ↓`, `⌘ →` all being `focus`) become one row whose keys
/// are joined. Ungrouped bindings keep their own row, in config order, which
/// for the workspace list is alphabetical by key and reads as a table.
pub(crate) fn sheet(bindings: &[Binding]) -> Sheet {
    /// A row under construction: its identity, its modifier prefix, and the
    /// keys collected for it so far.
    struct Pending {
        section: Section,
        family: Option<&'static str>,
        mods: String,
        keys: Vec<String>,
        label: String,
    }

    let mut pending: Vec<Pending> = Vec::new();

    for binding in bindings {
        let (section, label, family) = describe(&binding.command);
        let mods = pretty_mods(&binding.mods);
        let key = pretty_key(&binding.key);

        // A family collapses only within one modifier set: `⌘ ←` (focus) and
        // `⌘⇧ ←` (move) are different gestures and different rows.
        let existing = family.and_then(|family| {
            pending
                .iter_mut()
                .find(|p| p.section == section && p.family == Some(family) && p.mods == mods)
        });

        match existing {
            Some(row) => row.keys.push(key),
            None => pending.push(Pending {
                section,
                family,
                mods,
                keys: vec![key],
                label,
            }),
        }
    }

    let mut by_section: Sheet = Vec::new();
    for section in Section::ALL {
        let mut rows: Vec<Row> = pending
            .iter()
            .filter(|p| p.section == section)
            .map(|p| {
                // A family that swallowed most of the keyboard (every letter
                // moving a window to its workspace) is not a key list any
                // more; printing all 28 would set the column's width alone.
                let mut keys = p.keys.clone();
                sort_arrows(&mut keys);
                let keys = if keys.len() > MAX_KEYS_PER_ROW {
                    "key".to_string()
                } else {
                    keys.join(" ")
                };
                Row {
                    keys: if p.mods.is_empty() {
                        keys
                    } else {
                        format!("{} {}", p.mods, keys)
                    },
                    label: p.label.clone(),
                }
            })
            .collect();
        sort_rows(section, &mut rows);
        if !rows.is_empty() {
            by_section.push((section, rows));
        }
    }
    by_section
}

/// Put a section's rows in the order they will be read in.
///
/// Config order is alphabetical by key, which scatters a family across the
/// section: `Task 1`, `Find task`, `Task 3`, `Task 2`. Everywhere else the
/// question is "what can I do", so rows sort by plain modifier first and then
/// by description — which also lines up `Task 1..4` by number for free.
///
/// Workspaces are the exception. That section is a lookup table answering
/// "where does this key go", so it sorts by key, and its descriptions are
/// glyphs that sort meaninglessly anyway.
fn sort_rows(section: Section, rows: &mut [Row]) {
    if section == Section::Workspaces {
        rows.sort_by(|a, b| {
            mod_rank(&a.keys)
                .cmp(&mod_rank(&b.keys))
                .then_with(|| a.keys.cmp(&b.keys))
        });
    } else {
        rows.sort_by(|a, b| {
            mod_rank(&a.keys)
                .cmp(&mod_rank(&b.keys))
                .then_with(|| a.label.cmp(&b.label))
        });
    }
}

/// A row of arrow keys reads in compass order. Config order is alphabetical
/// by keysym (`Down Left Right Up`), which is the one order that looks like a
/// mistake to anyone who has seen a direction pad.
fn sort_arrows(keys: &mut [String]) {
    const COMPASS: [&str; 4] = ["←", "↑", "↓", "→"];
    if !keys.iter().all(|k| COMPASS.contains(&k.as_str())) {
        return;
    }
    keys.sort_by_key(|k| COMPASS.iter().position(|c| c == k).unwrap_or(COMPASS.len()));
}

/// Rows with fewer modifiers come first: the bare key is the one reached
/// most, and its shifted and controlled variants read as its variations.
fn mod_rank(keys: &str) -> (usize, String) {
    let prefix: String = keys.chars().take_while(|c| "⌘⌃⌥⇧".contains(*c)).collect();
    (prefix.chars().count(), prefix)
}

// ── The surface ─────────────────────────────────────────────────────────

pub struct Keybinds {
    body: gtk4::Box,
    reveal: anim::Reveal,
    loaded: Rc<Cell<bool>>,
    /// Super is still down and the sheet is still wanted. Cleared by any
    /// release, so a fetch or a hold that lands afterwards reveals nothing.
    pending_show: Rc<Cell<bool>>,
    /// The hold has lasted long enough to mean "show me", as opposed to
    /// being the leading edge of `⌘ b`.
    held: Rc<Cell<bool>>,
    hold_timer: Rc<RefCell<Option<glib::SourceId>>>,
    fetching: Rc<Cell<bool>>,
    rows: Rc<RefCell<Sheet>>,
}

/// How long Super must be down before the sheet appears.
///
/// Super is also the modifier on every binding, so a reveal without a hold
/// would flash the sheet on the leading edge of `⌘ b`. The delay lives here
/// rather than in the watcher that reports the press, which is what lets the
/// session drop the two transient systemd units it used to arm.
const HOLD_MS: u32 = 1000;

/// Above this many keys a collapsed row prints `key` instead of the list.
const MAX_KEYS_PER_ROW: usize = 8;

/// Sections per printed column. Three keeps the sheet wider than tall on a
/// 16:10 panel, which is the shape the eye scans fastest.
const COLUMNS: usize = 3;

impl Keybinds {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        static CONFIG: LayerShellConfig = LayerShellConfig {
            namespace: "swaypplet-keybinds",
            layer: gtk4_layer_shell::Layer::Overlay,
            exclusive: false,
            default_width: None,
            default_height: None,
            anchors: &[],
            margins: &[],
            // Held-Super is not a focus gesture: the sheet reads and leaves.
            keyboard_mode: gtk4_layer_shell::KeyboardMode::None,
        };

        let window = layer_shell::create_layer_window(app, &CONFIG);
        window.set_resizable(false);
        window.set_decorated(false);

        let wrapper = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .build();

        let card = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();
        card.add_css_class("glass-card");
        card.add_css_class("keybinds-card");

        let body = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(34)
            .build();
        body.add_css_class("keybinds-body");

        card.append(&body);
        wrapper.append(&card);
        window.set_child(Some(&wrapper));

        let reveal = anim::Reveal::new(&window, &card).content(&body);

        Rc::new(Keybinds {
            body,
            reveal,
            loaded: Rc::new(Cell::new(false)),
            pending_show: Rc::new(Cell::new(false)),
            held: Rc::new(Cell::new(false)),
            hold_timer: Rc::new(RefCell::new(None)),
            fetching: Rc::new(Cell::new(false)),
            rows: Rc::new(RefCell::new(Vec::new())),
        })
    }

    /// Ask for the sheet: Super went down.
    ///
    /// Two things have to finish before anything appears — the hold has to
    /// last [`HOLD_MS`], and the config has to arrive — and they run at once.
    /// The first press pays a socket round trip on a worker thread, which the
    /// hold covers; every later press has the sheet already parsed and only
    /// waits out the hold. Either can be cancelled by a release.
    pub fn show(self: &Rc<Self>) {
        self.pending_show.set(true);

        if !self.held.get() && self.hold_timer.borrow().is_none() {
            let this = self.clone();
            let id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(u64::from(HOLD_MS)),
                move || {
                    this.hold_timer.replace(None);
                    this.held.set(true);
                    this.try_reveal();
                },
            );
            self.hold_timer.replace(Some(id));
        }

        if self.loaded.get() || self.fetching.replace(true) {
            return;
        }

        let this = self.clone();
        crate::spawn::spawn_work(sway_ipc::config_text, move |config| {
            this.fetching.set(false);
            match config {
                Ok(config) => {
                    *this.rows.borrow_mut() = sheet(&parse(&config));
                    this.rebuild();
                    this.loaded.set(true);
                    this.try_reveal();
                }
                Err(e) => log::warn!("keybinds: {e}"),
            }
        });
    }

    fn try_reveal(&self) {
        if self.pending_show.get() && self.held.get() && self.loaded.get() {
            self.reveal.show();
        }
    }

    /// Super came up, or another key was pressed. Either way the sheet is not
    /// wanted, including by anything still in flight.
    pub fn hide(&self) {
        self.pending_show.set(false);
        self.held.set(false);
        if let Some(id) = self.hold_timer.replace(None) {
            id.remove();
        }
        self.reveal.hide();
    }

    /// For a keyboard-less caller (a click, a script). Skips the hold: an
    /// explicit toggle has already expressed the intent the hold tests for.
    pub fn toggle(self: &Rc<Self>) {
        if self.reveal.is_shown() {
            self.hide();
        } else {
            self.held.set(true);
            self.show();
        }
    }

    /// Drop the parsed sheet, so the next show re-reads sway's config. For
    /// `reload`, which is the only thing that changes the answer.
    pub fn invalidate(&self) {
        self.loaded.set(false);
    }

    fn rebuild(&self) {
        while let Some(child) = self.body.first_child() {
            self.body.remove(&child);
        }

        let sections = self.rows.borrow();

        // Each section goes in whichever column is currently shortest, costed
        // as its rows plus its heading. Filling left-to-right instead would
        // hand one column the 21-row workspace table and leave the sheet a
        // third full — the sections are independent, so nothing is lost by
        // letting a later one start an earlier column.
        let columns: Vec<gtk4::Box> = (0..COLUMNS).map(|_| new_column()).collect();
        let mut heights = vec![0usize; COLUMNS];

        for (section, rows) in sections.iter() {
            let shortest = heights
                .iter()
                .enumerate()
                .min_by_key(|(i, h)| (**h, *i))
                .map_or(0, |(i, _)| i);
            columns[shortest].append(&section_widget(*section, rows));
            heights[shortest] += rows.len() + 2;
        }

        for (column, height) in columns.iter().zip(&heights) {
            // A column nothing landed in would still claim its min-width.
            if *height > 0 {
                self.body.append(column);
            }
        }
    }
}

fn new_column() -> gtk4::Box {
    let column = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    column.add_css_class("keybinds-column");
    column
}

fn section_widget(section: Section, rows: &[Row]) -> gtk4::Box {
    let group = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    group.add_css_class("keybinds-section");

    let heading = gtk4::Label::builder()
        .label(section.title())
        .xalign(0.0)
        .build();
    heading.add_css_class("keybinds-heading");
    group.append(&heading);

    // A GtkGrid, not two boxes: the key column has to line up across every
    // row in a section, and only the grid measures it once for all of them.
    // The key/label gap is column spacing on the grid, so a long key list
    // pushes its own description instead of widening the whole column.
    let grid = gtk4::Grid::builder()
        .row_spacing(2)
        .column_spacing(12)
        .build();
    grid.add_css_class("keybinds-grid");

    for (i, row) in rows.iter().enumerate() {
        let keys = gtk4::Label::builder().label(&row.keys).xalign(0.0).build();
        keys.add_css_class("keybinds-keys");

        let label = gtk4::Label::builder()
            .label(&row.label)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        label.add_css_class("keybinds-label");

        grid.attach(&keys, 0, i as i32, 1, 1);
        grid.attach(&label, 1, i as i32, 1, 1);
    }

    group.append(&grid);
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags_modifiers_and_command() {
        let b = parse_bindsym("bindsym --release Mod4+Shift+q kill").unwrap();
        assert_eq!(b.mods, vec!["Mod4", "Shift"]);
        assert_eq!(b.key, "q");
        assert_eq!(b.command, "kill");
    }

    #[test]
    fn bindings_inside_a_block_are_not_default_mode() {
        let config = "\
bindsym Mod4+a kill
mode \"resize\" {
    bindsym Left resize shrink width 10px
}
bindsym Mod4+b reload
";
        let keys: Vec<String> = parse(config).into_iter().map(|b| b.key).collect();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn store_paths_shorten_to_the_script_name() {
        assert_eq!(
            shorten("exec /nix/store/wf53rrki04h6dbmswygk9bsrjx3az69z-task-switch 1:t1a"),
            "exec task-switch 1:t1a"
        );
        assert_eq!(
            shorten("exec /nix/store/0i2v-sway-1.12/bin/swaymsg reload"),
            "exec swaymsg reload"
        );
    }

    #[test]
    fn modifier_order_is_fixed_regardless_of_config_spelling() {
        let a = pretty_mods(&["Shift".into(), "Mod4".into()]);
        let b = pretty_mods(&["Mod4".into(), "Shift".into()]);
        assert_eq!(a, b);
        assert_eq!(a, "⌘⇧");
    }

    #[test]
    fn a_task_workspace_names_its_task() {
        assert_eq!(task_of("1:t1a"), (Some(1), "a".to_string()));
        assert_eq!(task_of("13:t4a"), (Some(4), "a".to_string()));
        assert_eq!(task_of("19:wb"), (None, "wb".to_string()));
    }

    #[test]
    fn directional_bindings_collapse_into_one_row() {
        let config = "\
bindsym Mod4+Left focus left
bindsym Mod4+Right focus right
bindsym Mod4+Up focus up
bindsym Mod4+Down focus down
";
        let sheet = sheet(&parse(config));
        let (section, rows) = &sheet[0];
        assert_eq!(*section, Section::Windows);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].keys, "⌘ ← ↑ ↓ →");
        assert_eq!(rows[0].label, "Focus");
    }

    #[test]
    fn the_same_family_under_different_modifiers_stays_two_rows() {
        let config = "\
bindsym Mod4+Left focus left
bindsym Mod4+Shift+Left move left
";
        let sheet = sheet(&parse(config));
        let rows = &sheet[0].1;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].keys, "⌘ ←");
        assert_eq!(rows[1].keys, "⌘⇧ ←");
    }

    #[test]
    fn an_unrecognised_command_still_prints() {
        // A real store hash is 32 characters; `shorten` keys on that length so a
        // hyphenated package name never loses its first word.
        let config = "bindsym Mod4+j exec /nix/store/wf53rrki04h6dbmswygk9bsrjx3az69z-something-odd --flag\n";
        let sheet = sheet(&parse(config));
        let (section, rows) = &sheet[0];
        assert_eq!(*section, Section::Other);
        assert_eq!(rows[0].keys, "⌘ j");
        assert_eq!(rows[0].label, "something-odd --flag");
    }

    #[test]
    fn workspace_rows_borrow_the_bars_glyphs() {
        let config = "bindsym Mod4+b workspace number 19:wb\n";
        let rows = &sheet(&parse(config))[0].1;
        assert_eq!(rows[0].label, "󰖟 b");
    }
}
