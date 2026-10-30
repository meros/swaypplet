//! The shell's appearance, published to apps
//! (docs/prior-art/theming/xdg-portal-appearance.md).
//!
//! The panel writes the inputs on screen to the GNOME settings that
//! xdg-desktop-portal-gtk and -gnome serve as `org.freedesktop.appearance`:
//! the mode as `color-scheme` (libadwaita, GTK4, Firefox, Chromium and
//! Electron follow it), the accent as the nearest of GNOME 47's nine
//! `accent-color` names, contrast as `high-contrast`, and motion as
//! `enable-animations`. GTK3 apps ignore `color-scheme`, so `gtk-theme`
//! moves between the light and the dark member of the theme family that is
//! set, when it is one this module knows (`gtk_theme`); any other theme is
//! the person's and stays.
//!
//! [`appearance`] and the two functions under it are pure and tested. The
//! writes are on a worker thread, which reads every key first and writes
//! only what differs, so a publish that changes nothing costs five reads
//! and no change signal in any app.
//!
//! Qt is not covered: the session styles Qt through Kvantum, which has no
//! dark-and-light pair to switch between.

use std::cell::RefCell;
use std::sync::mpsc;

use gio::prelude::*;

use crate::tokens::{Contrast, Inputs, Mode, Oklch, Rgb};

/// What apps are told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appearance {
    pub mode: Mode,
    /// `org.gnome.desktop.interface color-scheme`.
    pub color_scheme: &'static str,
    /// `org.gnome.desktop.interface accent-color`.
    pub accent: &'static str,
    /// `org.gnome.desktop.a11y.interface high-contrast`.
    pub high_contrast: bool,
    /// `org.gnome.desktop.interface enable-animations`; its opposite is
    /// `org.gnome.desktop.a11y.interface reduced-motion`, where the schema
    /// has it.
    pub animations: bool,
}

/// GNOME 47's accent names and the colours libadwaita 1.6 gives them
/// (`adw_accent_color_to_rgba`), slate last.
const GNOME_ACCENTS: [(&str, u32); 9] = [
    ("blue", 0x3584e4),
    ("teal", 0x2190a4),
    ("green", 0x3a944a),
    ("yellow", 0xc88800),
    ("orange", 0xed5b00),
    ("red", 0xe62d42),
    ("pink", 0xd56199),
    ("purple", 0x9141ac),
    ("slate", 0x6f8396),
];

/// Below this OKLCH chroma an accent has no hue worth matching: slate.
const GREY_CHROMA: f64 = 0.04;

/// The theme families whose light and dark members are known, as
/// (light, dark).
const GTK_THEMES: [(&str, &str); 2] = [("adw-gtk3", "adw-gtk3-dark"), ("Adwaita", "Adwaita-dark")];

/// The inputs as apps are told them. The accent is the shell's own accent
/// in dark mode, tint included, so the name apps get does not change when
/// the mode does.
pub fn appearance(inputs: Inputs) -> Appearance {
    let dark = Inputs {
        mode: Mode::Dark,
        ..inputs
    };
    Appearance {
        mode: inputs.mode,
        color_scheme: match inputs.mode {
            Mode::Dark => "prefer-dark",
            Mode::Light => "prefer-light",
        },
        accent: nearest_accent(crate::tokens::scales(dark).accent[8]),
        high_contrast: inputs.contrast == Contrast::High,
        animations: inputs.motion > 0,
    }
}

/// The GNOME accent name nearest in hue to `c`, or slate when `c` is
/// nearly grey.
pub fn nearest_accent(c: Rgb) -> &'static str {
    let Oklch(_, chroma, hue) = Oklch::from(c);
    if chroma < GREY_CHROMA {
        return "slate";
    }
    GNOME_ACCENTS[..8]
        .iter()
        .map(|(name, hex)| {
            let their = Oklch::from(Rgb::hex(*hex)).2;
            (*name, crate::tokens::tint::difference(hue, their).abs())
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or("slate", |(name, _)| name)
}

/// The `gtk-theme` for `mode` in the family `current` belongs to, or `None`
/// when `current` is no family this module knows.
pub fn gtk_theme(current: &str, mode: Mode) -> Option<&'static str> {
    GTK_THEMES
        .iter()
        .find(|(light, dark)| current == *light || current == *dark)
        .map(|(light, dark)| match mode {
            Mode::Dark => *dark,
            Mode::Light => *light,
        })
}

const INTERFACE: &str = "org.gnome.desktop.interface";
const A11Y: &str = "org.gnome.desktop.a11y.interface";

/// Every key but `gtk-theme`, as GVariant text. `reduced-motion` is a
/// boolean here; [`as_stored`] gives it the type the installed schema has.
fn keys(a: Appearance) -> [(&'static str, &'static str, String); 5] {
    let quoted = |s: &str| format!("'{s}'");
    [
        (INTERFACE, "color-scheme", quoted(a.color_scheme)),
        (INTERFACE, "accent-color", quoted(a.accent)),
        (INTERFACE, "enable-animations", a.animations.to_string()),
        (A11Y, "high-contrast", a.high_contrast.to_string()),
        (A11Y, "reduced-motion", (!a.animations).to_string()),
    ]
}

/// `value` in the type of `now`, the key's current value. GNOME 48 made
/// `reduced-motion` an enum (`'no-preference'`, `'reduce'`) where it had
/// been a boolean; a boolean written to the enum is refused, so every
/// publish found the key different and failed to set it.
fn as_stored(key: &str, value: String, now: &str) -> String {
    if key == "reduced-motion" && now.starts_with('\'') {
        let reduce = value == "true";
        return if reduce {
            "'reduce'"
        } else {
            "'no-preference'"
        }
        .into();
    }
    value
}

/// What [`write_appearance`] needs from where the keys live: read a key as GVariant
/// text (`None` when the store does not have it), write one, put one back
/// to its default, and wait until every write has reached the database.
trait Keys {
    fn read(&self, schema: &str, key: &str) -> Option<String>;
    fn write(&self, schema: &str, key: &str, value: &str);
    fn reset(&self, schema: &str, key: &str);
    fn sync(&self);
}

/// Write what differs from `a`, and nothing else; then read every written
/// key back, and write again what did not land.
///
/// A write can be acknowledged and still not land. dconf-service loads the
/// database once and diffs each change against that copy, so after a second
/// dconf-service has written the same file (a nested session's, under
/// `dbus-run-session`; dev/render.sh runs a whole panel that way) the
/// session's service still believes what it last wrote, and drops a write of
/// that value as a no-op: apps stayed on the other service's dark mode while
/// the shell went light. A reset is a change against any copy, so a reset
/// and the write again commit both.
fn write_appearance(store: &impl Keys, a: Appearance) {
    let mut wanted = Vec::new();
    for (schema, key, value) in keys(a) {
        // The dconf store reads an unset key as nothing, which differs from
        // every value, so it writes: correct, since unset is the schema
        // default and not what the shell shows.
        if let Some(now) = store.read(schema, key) {
            let value = as_stored(key, value, &now);
            if now != value {
                wanted.push((schema, key, value));
            }
        }
    }
    if let Some(now) = store.read(INTERFACE, "gtk-theme") {
        let now = now.trim_matches('\'');
        if let Some(next) = gtk_theme(now, a.mode)
            && next != now
        {
            wanted.push((INTERFACE, "gtk-theme", format!("'{next}'")));
        }
    }
    if wanted.is_empty() {
        return;
    }
    for (schema, key, value) in &wanted {
        store.write(schema, key, value);
    }
    store.sync();
    let mut nudged = false;
    for (schema, key, value) in &wanted {
        if store.read(schema, key).as_deref() != Some(value.as_str()) {
            log::info!("theme: apps: {schema} {key} did not take {value}; writing it again");
            store.reset(schema, key);
            store.sync();
            store.write(schema, key, value);
            nudged = true;
        }
    }
    if nudged {
        store.sync();
    }
}

/// Where the keys are written: GSettings when the schemas are installed,
/// else the `dconf` command, else nowhere (logged once, when the worker
/// starts).
enum Store {
    Gio(Vec<(&'static str, gio::Settings)>),
    Dconf,
    Nowhere,
}

impl Store {
    fn open() -> Store {
        let source = gio::SettingsSchemaSource::default();
        let lookup = |id| source.as_ref().and_then(|s| s.lookup(id, true));
        // Without the dconf GIO module, GSettings falls back to a memory
        // backend that no other process sees: a write there reaches no app.
        let memory = gio::SettingsBackend::default().type_().name() == "GMemorySettingsBackend";
        if memory {
            log::info!("theme: GSettings has no persistent backend here");
        }
        if !memory && let (Some(interface), Some(a11y)) = (lookup(INTERFACE), lookup(A11Y)) {
            let open = |schema: &gio::SettingsSchema| {
                gio::Settings::new_full(schema, None::<&gio::SettingsBackend>, None)
            };
            log::info!("theme: apps follow the appearance through GSettings");
            return Store::Gio(vec![(INTERFACE, open(&interface)), (A11Y, open(&a11y))]);
        }
        let dconf = std::process::Command::new("dconf")
            .arg("help")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok();
        if dconf {
            log::info!("theme: no GNOME schemas installed; apps follow through `dconf write`");
            Store::Dconf
        } else {
            log::warn!(
                "theme: neither the GNOME schemas nor `dconf` is available; apps will not follow the shell's appearance"
            );
            Store::Nowhere
        }
    }

    fn dconf_path(schema: &str, key: &str) -> String {
        format!("/{}/{key}", schema.replace('.', "/"))
    }

    fn settings(&self, schema: &str) -> Option<&gio::Settings> {
        match self {
            Store::Gio(settings) => settings
                .iter()
                .find(|(id, _)| *id == schema)
                .map(|(_, s)| s),
            _ => None,
        }
    }

    fn dconf(args: &[&str]) -> Result<(), String> {
        std::process::Command::new("dconf")
            .args(args)
            .status()
            .map_err(|e| e.to_string())
            .and_then(|s| s.success().then_some(()).ok_or(s.to_string()))
    }
}

impl Keys for Store {
    fn read(&self, schema: &str, key: &str) -> Option<String> {
        match self {
            Store::Gio(_) => {
                let s = self.settings(schema)?;
                if !s.settings_schema()?.has_key(key) {
                    return None;
                }
                Some(s.value(key).print(true).to_string())
            }
            Store::Dconf => {
                let out = std::process::Command::new("dconf")
                    .args(["read", &Self::dconf_path(schema, key)])
                    .output()
                    .ok()?;
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
            }
            Store::Nowhere => None,
        }
    }

    fn write(&self, schema: &str, key: &str, value: &str) {
        let result = match self {
            Store::Gio(_) => {
                let Some(s) = self.settings(schema) else {
                    return;
                };
                glib::Variant::parse(None, value)
                    .map_err(|e| e.to_string())
                    .and_then(|v| s.set_value(key, &v).map_err(|e| e.to_string()))
            }
            Store::Dconf => Self::dconf(&["write", &Self::dconf_path(schema, key), value]),
            Store::Nowhere => Ok(()),
        };
        match result {
            Ok(()) => log::debug!("theme: apps: {schema} {key} = {value}"),
            Err(e) => log::warn!("theme: apps: cannot set {schema} {key} to {value}: {e}"),
        }
    }

    fn reset(&self, schema: &str, key: &str) {
        match self {
            Store::Gio(_) => {
                if let Some(s) = self.settings(schema) {
                    s.reset(key);
                }
            }
            Store::Dconf => {
                if let Err(e) = Self::dconf(&["reset", &Self::dconf_path(schema, key)]) {
                    log::warn!("theme: apps: cannot reset {schema} {key}: {e}");
                }
            }
            Store::Nowhere => {}
        }
    }

    /// GSettings writes asynchronously; this waits for them. A `dconf write`
    /// has landed when the command returns.
    fn sync(&self) {
        if matches!(self, Store::Gio(_)) {
            gio::Settings::sync();
        }
    }
}

thread_local! {
    /// The worker's queue, in the one process that publishes.
    static QUEUE: RefCell<Option<mpsc::Sender<Appearance>>> = const { RefCell::new(None) };
}

/// Whether a process on the bus at `address` speaks for the session's apps.
/// Not when the user's bus exists (`bus`, `$XDG_RUNTIME_DIR/bus`) and this
/// process is on another one: that is a nested panel under
/// `dbus-run-session` (dev/render.sh, dev/frame-bench.sh), whose dconf-service
/// still writes the session's database, so every harness run used to set the
/// apps to the harness's mode, and leave the session's dconf-service with a
/// stale copy that dropped the panel's next write ([`write_appearance`]).
/// `SWAYPPLET_APPS=on` or `off` overrides.
fn speaks_for_apps(
    setting: Option<&str>,
    address: Option<&str>,
    bus: &str,
    bus_exists: bool,
) -> bool {
    match setting {
        Some("off") => return false,
        Some("on") => return true,
        _ => {}
    }
    let ours = format!("unix:path={bus}");
    match address {
        Some(address) if bus_exists => address
            .split(';')
            .any(|a| a.split(',').next() == Some(ours.as_str())),
        _ => true,
    }
}

/// Start publishing from this process. The panel calls this, once.
pub(super) fn start() {
    let bus = glib::user_runtime_dir().join("bus");
    let speaks = speaks_for_apps(
        std::env::var("SWAYPPLET_APPS").ok().as_deref(),
        std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().as_deref(),
        &bus.to_string_lossy(),
        bus.exists(),
    );
    if !speaks {
        log::info!("theme: on a private D-Bus, not the session's; apps are left alone");
        return;
    }
    let (tx, rx) = mpsc::channel::<Appearance>();
    let spawned = std::thread::Builder::new()
        .name("theme-apps".into())
        .spawn(move || {
            let store = Store::open();
            while let Ok(mut a) = rx.recv() {
                // Only the newest matters.
                while let Ok(newer) = rx.try_recv() {
                    a = newer;
                }
                write_appearance(&store, a);
            }
        });
    match spawned {
        Ok(_) => QUEUE.with(|q| *q.borrow_mut() = Some(tx)),
        Err(e) => log::warn!("theme: cannot start the apps publisher: {e}"),
    }
}

/// Tell apps about `inputs`, when this process publishes and the setting
/// is on. Returns at once; the worker does the reads and the writes.
pub(super) fn publish(inputs: Inputs) {
    if !crate::settings::store::with(|s| s.look().apps_follow) {
        return;
    }
    QUEUE.with(|q| {
        if let Some(tx) = q.borrow().as_ref() {
            let _ = tx.send(appearance(inputs));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::{Accent, Palette, Tint};

    #[test]
    fn the_mode_is_the_color_scheme() {
        let dark = appearance(Inputs::default());
        assert_eq!(dark.color_scheme, "prefer-dark");
        let light = appearance(Inputs {
            mode: Mode::Light,
            ..Inputs::default()
        });
        assert_eq!(light.color_scheme, "prefer-light");
    }

    #[test]
    fn contrast_and_motion_map_to_the_a11y_keys() {
        let high = appearance(Inputs {
            contrast: Contrast::High,
            ..Inputs::default()
        });
        assert!(high.high_contrast);
        assert!(!appearance(Inputs::default()).high_contrast);
        for (motion, on) in [(100, true), (50, true), (0, false)] {
            let a = appearance(Inputs {
                motion,
                ..Inputs::default()
            });
            assert_eq!(a.animations, on, "motion {motion}");
            let reduced = &keys(a)[4];
            assert_eq!(reduced.2, (!on).to_string());
        }
    }

    #[test]
    fn every_gnome_accent_is_its_own_nearest() {
        for (name, hex) in GNOME_ACCENTS {
            assert_eq!(nearest_accent(Rgb::hex(hex)), name);
        }
    }

    #[test]
    fn a_grey_accent_is_slate() {
        assert_eq!(nearest_accent(Rgb::hex(0x808080)), "slate");
        assert_eq!(nearest_accent(Rgb::hex(0x7a7f85)), "slate");
    }

    /// The shell's six accents, each the nearest in hue: gruvbox's aqua is
    /// green, its blue is teal and its purple is pink, which is what those
    /// colours are.
    #[test]
    fn the_shells_accents_map_by_hue_in_both_modes() {
        let expected = [
            (Accent::Aqua, "green"),
            (Accent::Yellow, "yellow"),
            (Accent::Blue, "teal"),
            (Accent::Purple, "pink"),
            (Accent::Orange, "orange"),
            (Accent::Red, "red"),
        ];
        for (accent, name) in expected {
            for mode in Mode::ALL {
                let a = appearance(Inputs {
                    accent,
                    mode,
                    ..Inputs::default()
                });
                assert_eq!(a.accent, name, "{accent:?} in {mode:?}");
            }
        }
    }

    #[test]
    fn a_tinted_accent_follows_the_wallpapers_hue() {
        let tinted = |hue: u16| {
            appearance(Inputs {
                tint: Tint::Accents(Palette::single(hue)),
                ..Inputs::default()
            })
            .accent
        };
        let hue_of = |name: &str| {
            let (_, hex) = GNOME_ACCENTS.iter().find(|(n, _)| *n == name).unwrap();
            Oklch::from(Rgb::hex(*hex)).2.round() as u16
        };
        for name in ["blue", "green", "orange", "purple"] {
            assert_eq!(tinted(hue_of(name)), name);
        }
    }

    /// The session's dconf-service as it behaves after another one wrote
    /// the database: `file` is what readers see, `cache` what the service
    /// believes, and a write equal to the cache is dropped.
    #[derive(Default)]
    struct StaleService {
        file: std::cell::RefCell<std::collections::HashMap<String, String>>,
        cache: std::cell::RefCell<std::collections::HashMap<String, Option<String>>>,
        writes: std::cell::Cell<usize>,
    }

    impl StaleService {
        fn set(&self, key: &str, value: Option<&str>) {
            let mut cache = self.cache.borrow_mut();
            if cache.get(key).cloned().flatten().as_deref() == value {
                return;
            }
            cache.insert(key.into(), value.map(Into::into));
            let mut file = self.file.borrow_mut();
            for (k, v) in cache.iter() {
                match v {
                    Some(v) => file.insert(k.clone(), v.clone()),
                    None => file.remove(k),
                };
            }
        }
    }

    impl Keys for StaleService {
        fn read(&self, _: &str, key: &str) -> Option<String> {
            let defaults = [
                ("color-scheme", "'default'"),
                ("accent-color", "'blue'"),
                ("enable-animations", "true"),
                ("high-contrast", "false"),
                ("reduced-motion", "'no-preference'"),
                ("gtk-theme", "'Adwaita'"),
            ];
            let default = defaults
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string());
            self.file.borrow().get(key).cloned().or(default)
        }
        fn write(&self, _: &str, key: &str, value: &str) {
            self.writes.set(self.writes.get() + 1);
            self.set(key, Some(value));
        }
        fn reset(&self, _: &str, key: &str) {
            self.set(key, None);
        }
        fn sync(&self) {}
    }

    /// The live failure of 2026-10-02: the panel last wrote light, a nested
    /// panel's dconf-service wrote dark behind the session's, and the
    /// panel's write of light again was dropped, so apps stayed dark under
    /// a light shell.
    #[test]
    fn a_write_the_stale_service_drops_is_written_again() {
        let light = appearance(Inputs {
            mode: Mode::Light,
            ..Inputs::default()
        });
        let service = StaleService::default();
        service
            .file
            .borrow_mut()
            .insert("gtk-theme".into(), "'adw-gtk3'".into());
        write_appearance(&service, light);
        assert_eq!(service.read("", "color-scheme").unwrap(), "'prefer-light'");
        // Another dconf-service writes the file; this one's copy goes stale.
        service
            .file
            .borrow_mut()
            .insert("color-scheme".into(), "'prefer-dark'".into());
        service
            .file
            .borrow_mut()
            .insert("gtk-theme".into(), "'adw-gtk3-dark'".into());
        write_appearance(&service, light);
        assert_eq!(service.read("", "color-scheme").unwrap(), "'prefer-light'");
        assert_eq!(service.read("", "gtk-theme").unwrap(), "'adw-gtk3'");
    }

    /// Nothing differs, nothing is written: no change signal in any app.
    #[test]
    fn a_publish_that_changes_nothing_writes_nothing() {
        let service = StaleService::default();
        let dark = appearance(Inputs::default());
        write_appearance(&service, dark);
        let writes = service.writes.get();
        assert!(writes > 0);
        write_appearance(&service, dark);
        assert_eq!(service.writes.get(), writes);
    }

    /// GNOME 48's `reduced-motion` is an enum; older schemas have a boolean.
    #[test]
    fn reduced_motion_takes_the_schemas_type() {
        assert_eq!(
            as_stored("reduced-motion", "true".into(), "'no-preference'"),
            "'reduce'"
        );
        assert_eq!(
            as_stored("reduced-motion", "false".into(), "'reduce'"),
            "'no-preference'"
        );
        assert_eq!(as_stored("reduced-motion", "true".into(), "false"), "true");
        assert_eq!(as_stored("high-contrast", "true".into(), "false"), "true");
    }

    /// A panel on the session's bus publishes; one on a private bus beside
    /// it (a harness under `dbus-run-session`) does not.
    #[test]
    fn only_the_sessions_bus_speaks_for_apps() {
        let bus = "/run/user/1000/bus";
        let session = Some("unix:path=/run/user/1000/bus");
        let private = Some("unix:path=/tmp/dbus-XYZ,guid=abc");
        assert!(speaks_for_apps(None, session, bus, true));
        assert!(!speaks_for_apps(None, private, bus, true));
        // No user bus at all (a non-systemd session): the bus there is it.
        assert!(speaks_for_apps(None, private, bus, false));
        assert!(speaks_for_apps(None, None, bus, true));
        assert!(speaks_for_apps(Some("on"), private, bus, true));
        assert!(!speaks_for_apps(Some("off"), session, bus, true));
    }

    #[test]
    fn gtk_theme_switches_only_a_known_family() {
        assert_eq!(gtk_theme("adw-gtk3-dark", Mode::Light), Some("adw-gtk3"));
        assert_eq!(gtk_theme("adw-gtk3", Mode::Dark), Some("adw-gtk3-dark"));
        assert_eq!(gtk_theme("Adwaita", Mode::Dark), Some("Adwaita-dark"));
        assert_eq!(gtk_theme("Nordic", Mode::Light), None);
    }
}
