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

/// Every key but `gtk-theme`, as GVariant text.
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

    /// The key's value as GVariant text, or `None` when this store does not
    /// have it (a schema older than the key).
    fn read(&self, schema: &str, key: &str) -> Option<String> {
        match self {
            Store::Gio(settings) => {
                let (_, s) = settings.iter().find(|(id, _)| *id == schema)?;
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
            Store::Gio(settings) => {
                let Some((_, s)) = settings.iter().find(|(id, _)| *id == schema) else {
                    return;
                };
                glib::Variant::parse(None, value)
                    .map_err(|e| e.to_string())
                    .and_then(|v| s.set_value(key, &v).map_err(|e| e.to_string()))
            }
            Store::Dconf => std::process::Command::new("dconf")
                .args(["write", &Self::dconf_path(schema, key), value])
                .status()
                .map_err(|e| e.to_string())
                .and_then(|s| s.success().then_some(()).ok_or(s.to_string())),
            Store::Nowhere => Ok(()),
        };
        match result {
            Ok(()) => log::debug!("theme: apps: {schema} {key} = {value}"),
            Err(e) => log::warn!("theme: apps: cannot set {schema} {key} to {value}: {e}"),
        }
    }

    /// Write what differs from `a`, and nothing else.
    fn publish(&self, a: Appearance) {
        let mut wrote = false;
        for (schema, key, value) in keys(a) {
            // The dconf store reads an unset key as nothing, which differs
            // from every value, so it writes: correct, since unset is the
            // schema default and not what the shell shows.
            if let Some(now) = self.read(schema, key)
                && now != value
            {
                self.write(schema, key, &value);
                wrote = true;
            }
        }
        if let Some(now) = self.read(INTERFACE, "gtk-theme") {
            let now = now.trim_matches('\'');
            if let Some(next) = gtk_theme(now, a.mode)
                && next != now
            {
                self.write(INTERFACE, "gtk-theme", &format!("'{next}'"));
                wrote = true;
            }
        }
        if wrote && matches!(self, Store::Gio(_)) {
            gio::Settings::sync();
        }
    }
}

thread_local! {
    /// The worker's queue, in the one process that publishes.
    static QUEUE: RefCell<Option<mpsc::Sender<Appearance>>> = const { RefCell::new(None) };
}

/// Start publishing from this process. The panel calls this, once.
pub(super) fn start() {
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
                store.publish(a);
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

    #[test]
    fn gtk_theme_switches_only_a_known_family() {
        assert_eq!(gtk_theme("adw-gtk3-dark", Mode::Light), Some("adw-gtk3"));
        assert_eq!(gtk_theme("adw-gtk3", Mode::Dark), Some("adw-gtk3-dark"));
        assert_eq!(gtk_theme("Adwaita", Mode::Dark), Some("Adwaita-dark"));
        assert_eq!(gtk_theme("Nordic", Mode::Light), None);
    }
}
