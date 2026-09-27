//! Keyboard, touchpad and mouse settings on the compositor: the `input`
//! section as sway `input type:…` commands.
//!
//! Applied by device type (`type:keyboard`, `type:touchpad`,
//! `type:pointer`). Sway keeps a type's settings and gives them to a device
//! that is plugged in later, so a hotplug needs nothing from here; a
//! `swaymsg reload` does, because it reads the config again and drops every
//! runtime `input` command. The watcher thread below re-applies on both
//! anyway (a device `added` and a workspace `reload`): one path, and it
//! does not depend on that detail of sway.
//!
//! A knob the section leaves `None` is never sent, so the sway config's
//! `input` blocks stay in force for it. A knob that goes back to `None`
//! (Reset, `swaypplet settings reset input`) cannot be sent back to a value
//! this process never knew, so it costs a `reload`: sway reads its config
//! again and the watcher puts the knobs still set back on top.
//!
//! Zero work at rest: the watcher blocks on the IPC socket, and the store
//! observer compares one small struct when a setting changes.

use std::cell::RefCell;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::schema::{AccelProfile, ClickMethod, Input, ScrollMethod};
use crate::settings::store;
use crate::settings::xkb;
use crate::sway::ipc;

const KEYBOARD: &str = "input type:keyboard";
const TOUCHPAD: &str = "input type:touchpad";
const POINTER: &str = "input type:pointer";

/// A wait after a hotplug or a reload, so a dock that brings five devices
/// is one apply rather than five.
const SETTLE_MS: u64 = 300;

// ── Commands ────────────────────────────────────────────────────────────

/// An enum's serde spelling, which is sway's word for it.
fn word<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn toggle(on: bool) -> &'static str {
    if on { "enabled" } else { "disabled" }
}

/// Every knob `input` sets, each with its commands in the order sway must
/// get them. A knob is the unit [`changed`] compares, so a drag on the
/// repeat rate never sends the layouts again (each xkb command compiles a
/// new keymap).
pub fn knobs(input: &Input) -> Vec<(&'static str, Vec<String>)> {
    let mut out: Vec<(&'static str, Vec<String>)> = Vec::new();
    if let Some(layouts) = &input.layouts {
        let (names, variants): (Vec<&str>, Vec<&str>) =
            layouts.iter().map(|code| xkb::split(code)).unzip();
        // Variants cleared first and set last: at every step the layouts
        // and the variants sway holds form a keymap that compiles, which a
        // new layout list beside the old variant list need not.
        let mut cmds = vec![
            format!("{KEYBOARD} xkb_variant \"\""),
            format!("{KEYBOARD} xkb_layout \"{}\"", names.join(",")),
        ];
        if variants.iter().any(|v| !v.is_empty()) {
            cmds.push(format!("{KEYBOARD} xkb_variant \"{}\"", variants.join(",")));
        }
        out.push(("layouts", cmds));
    }
    if input.layout_switch.is_some() || input.caps.is_some() {
        // One xkb_options line holds both, so either one set sends both.
        let options: Vec<&str> = [&input.layout_switch, &input.caps]
            .into_iter()
            .filter_map(|o| o.as_deref())
            .filter(|o| !o.is_empty())
            .collect();
        out.push((
            "xkb_options",
            vec![format!("{KEYBOARD} xkb_options \"{}\"", options.join(","))],
        ));
    }
    if let Some(ms) = input.repeat_delay_ms {
        out.push((
            "repeat_delay",
            vec![format!("{KEYBOARD} repeat_delay {ms}")],
        ));
    }
    if let Some(rate) = input.repeat_rate {
        out.push((
            "repeat_rate",
            vec![format!("{KEYBOARD} repeat_rate {rate}")],
        ));
    }
    pointer_knobs(
        &mut out,
        TOUCHPAD,
        [
            "touchpad_speed",
            "touchpad_accel_profile",
            "touchpad_natural_scroll",
        ],
        input.touchpad_speed,
        input.touchpad_accel_profile,
        input.touchpad_natural_scroll,
    );
    if let Some(on) = input.touchpad_tap {
        out.push((
            "touchpad_tap",
            vec![format!("{TOUCHPAD} tap {}", toggle(on))],
        ));
    }
    if let Some(on) = input.touchpad_dwt {
        out.push((
            "touchpad_dwt",
            vec![format!("{TOUCHPAD} dwt {}", toggle(on))],
        ));
    }
    if let Some(m) = input.touchpad_click_method {
        out.push((
            "touchpad_click_method",
            vec![format!("{TOUCHPAD} click_method {}", word(&m))],
        ));
    }
    if let Some(m) = input.touchpad_scroll_method {
        out.push((
            "touchpad_scroll_method",
            vec![format!("{TOUCHPAD} scroll_method {}", word(&m))],
        ));
    }
    pointer_knobs(
        &mut out,
        POINTER,
        ["mouse_speed", "mouse_accel_profile", "mouse_natural_scroll"],
        input.mouse_speed,
        input.mouse_accel_profile,
        input.mouse_natural_scroll,
    );
    out
}

/// The three knobs a touchpad and a mouse share.
fn pointer_knobs(
    out: &mut Vec<(&'static str, Vec<String>)>,
    target: &str,
    names: [&'static str; 3],
    speed: Option<f64>,
    profile: Option<AccelProfile>,
    natural: Option<bool>,
) {
    if let Some(s) = speed {
        out.push((names[0], vec![format!("{target} pointer_accel {s:.2}")]));
    }
    if let Some(p) = profile {
        out.push((
            names[1],
            vec![format!("{target} accel_profile {}", word(&p))],
        ));
    }
    if let Some(on) = natural {
        out.push((
            names[2],
            vec![format!("{target} natural_scroll {}", toggle(on))],
        ));
    }
}

/// Every command for `input`, in order.
pub fn commands(input: &Input) -> Vec<String> {
    knobs(input).into_iter().flat_map(|(_, c)| c).collect()
}

/// What going from `old` to `new` takes: the commands of every knob that
/// is new or different, and whether a knob was dropped, which only a
/// `reload` can undo.
pub fn changed(old: &Input, new: &Input) -> (Vec<String>, bool) {
    let before = knobs(old);
    let after = knobs(new);
    let dropped = before
        .iter()
        .any(|(name, _)| !after.iter().any(|(n, _)| n == name));
    let cmds = after
        .into_iter()
        .filter(|knob| !before.contains(knob))
        .flat_map(|(_, c)| c)
        .collect();
    (cmds, dropped)
}

// ── Applying ────────────────────────────────────────────────────────────

/// The section some layer sets, or `None` when neither file has one: then
/// there is nothing to send and the sway config is the whole story.
fn in_force() -> Option<Input> {
    store::with(|s| s.input.clone()).or_else(|| store::system().input.clone())
}

thread_local! {
    /// What was last sent, for [`changed`].
    static SENT: RefCell<Input> = RefCell::new(Input::default());
}

fn apply_all() {
    let input = in_force().unwrap_or_default();
    let cmds = commands(&input);
    SENT.with(|s| *s.borrow_mut() = input);
    if !cmds.is_empty() {
        log::info!("input: applying {} command(s)", cmds.len());
        ipc::run_commands(cmds);
    }
}

fn follow_edit() {
    let now = in_force().unwrap_or_default();
    let (cmds, dropped) = SENT.with(|s| {
        let mut sent = s.borrow_mut();
        if *sent == now {
            return (Vec::new(), false);
        }
        let out = changed(&sent, &now);
        *sent = now;
        out
    });
    if dropped {
        // Everything still set comes back through the reload event.
        log::info!("input: a setting went back to the sway config; reloading it");
        ipc::run_command("reload");
    } else if !cmds.is_empty() {
        ipc::run_commands(cmds);
    }
}

/// Apply the section now, follow edits to it, and apply it again after a
/// hotplug or a reload. Once per process, from the panel's startup.
pub fn follow() {
    apply_all();
    store::observe(follow_edit);

    let (tx, rx) = async_channel::bounded::<()>(1);
    std::thread::Builder::new()
        .name("sway-input".into())
        .spawn(move || watch(tx))
        .expect("spawn sway-input thread");
    glib::MainContext::default().spawn_local(async move {
        while rx.recv().await.is_ok() {
            glib::timeout_future(std::time::Duration::from_millis(SETTLE_MS)).await;
            // The burst that arrived during the wait is covered by this one.
            while rx.try_recv().is_ok() {}
            apply_all();
        }
    });
}

/// The event thread: a device added, or the config read again.
fn watch(tx: async_channel::Sender<()>) {
    let mut backoff = crate::service::Backoff::new();
    loop {
        let started = std::time::Instant::now();
        let result = ipc::connect()
            .and_then(|c| c.subscribe([swayipc::EventType::Input, swayipc::EventType::Workspace]));
        let events = match result {
            Ok(events) => events,
            Err(e) => {
                let delay = backoff.next_delay(started.elapsed());
                log::warn!("input: {e}; reconnecting in {delay:?}");
                std::thread::sleep(delay);
                continue;
            }
        };
        for event in events {
            let wanted = match event {
                Ok(swayipc::Event::Input(e)) => matches!(e.change, swayipc::InputChange::Added),
                Ok(swayipc::Event::Workspace(w)) => w.change == swayipc::WorkspaceChange::Reload,
                Ok(_) => false,
                Err(e) => {
                    log::warn!("input: event stream: {e}");
                    break;
                }
            };
            // A full channel already holds a pending apply; that is enough.
            if wanted && let Err(async_channel::TrySendError::Closed(())) = tx.try_send(()) {
                return; // the GTK side is gone
            }
        }
        let delay = backoff.next_delay(started.elapsed());
        std::thread::sleep(delay);
    }
}

// ── Devices ─────────────────────────────────────────────────────────────

/// A connected device, as the Input tab lists it and reads its current
/// values from. Only what `get_inputs` reports; `None` where it does not.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Device {
    /// `1:1:AT_Translated_Set_2_keyboard`: what a per-device `input`
    /// command names.
    pub identifier: String,
    pub name: String,
    /// sway's type: `keyboard`, `touchpad`, `pointer`, `switch` …
    pub kind: String,
    /// Layout descriptions (`Swedish`), which is all sway reports.
    pub layouts: Vec<String>,
    pub repeat_delay_ms: Option<u32>,
    pub repeat_rate: Option<u32>,
    pub tap: Option<bool>,
    pub natural_scroll: Option<bool>,
    pub speed: Option<f64>,
    pub accel_profile: Option<AccelProfile>,
    pub dwt: Option<bool>,
    pub click_method: Option<ClickMethod>,
    pub scroll_method: Option<ScrollMethod>,
}

/// The reply of `get_inputs`, one [`Device`] per entry. A keyboard that
/// is also a pointer is listed twice by sway, once per capability, and
/// stays that way here.
pub fn parse_devices(reply: &Value) -> Vec<Device> {
    let Some(list) = reply.as_array() else {
        return Vec::new();
    };
    list.iter()
        .map(|d| {
            let str_of = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let u32_of = |k: &str| d.get(k).and_then(Value::as_u64).map(|v| v as u32);
            let li = d.get("libinput");
            let flag = |k: &str| {
                li.and_then(|l| l.get(k))
                    .and_then(Value::as_str)
                    .map(|s| s == "enabled")
            };
            fn parsed<T: for<'de> Deserialize<'de>>(v: Option<&Value>) -> Option<T> {
                v.and_then(|v| serde_json::from_value(v.clone()).ok())
            }
            Device {
                identifier: str_of("identifier"),
                name: str_of("name"),
                kind: str_of("type"),
                layouts: d
                    .get("xkb_layout_names")
                    .and_then(Value::as_array)
                    .map(|l| {
                        l.iter()
                            .filter_map(|n| n.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                repeat_delay_ms: u32_of("repeat_delay"),
                repeat_rate: u32_of("repeat_rate"),
                tap: flag("tap"),
                natural_scroll: flag("natural_scroll"),
                speed: li
                    .and_then(|l| l.get("accel_speed"))
                    .and_then(Value::as_f64),
                accel_profile: parsed(li.and_then(|l| l.get("accel_profile"))),
                dwt: flag("dwt"),
                click_method: parsed(li.and_then(|l| l.get("click_method"))),
                scroll_method: parsed(li.and_then(|l| l.get("scroll_method"))),
            }
        })
        .collect()
}

/// The connected devices, read on a worker thread; `then` runs on the GTK
/// thread, with nothing when sway cannot be asked.
pub fn devices(then: impl FnOnce(Vec<Device>) + 'static) {
    crate::spawn::spawn_work(
        || match ipc::inputs_json() {
            Ok(reply) => parse_devices(&reply),
            Err(e) => {
                log::warn!("input: {e}");
                Vec::new()
            }
        },
        then,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Input {
        Input {
            layouts: Some(vec!["se".into(), "us(dvorak)".into()]),
            layout_switch: Some("grp:win_space_toggle".into()),
            caps: Some("caps:escape".into()),
            repeat_delay_ms: Some(200),
            repeat_rate: Some(40),
            touchpad_tap: Some(false),
            touchpad_natural_scroll: Some(true),
            touchpad_speed: Some(0.2),
            touchpad_accel_profile: Some(AccelProfile::Flat),
            touchpad_dwt: Some(true),
            touchpad_click_method: Some(ClickMethod::Clickfinger),
            touchpad_scroll_method: Some(ScrollMethod::TwoFinger),
            mouse_speed: Some(-0.35),
            mouse_accel_profile: Some(AccelProfile::Adaptive),
            mouse_natural_scroll: Some(false),
        }
    }

    #[test]
    fn every_knob_becomes_the_exact_sway_command() {
        assert_eq!(
            commands(&all()),
            [
                r#"input type:keyboard xkb_variant """#,
                r#"input type:keyboard xkb_layout "se,us""#,
                r#"input type:keyboard xkb_variant ",dvorak""#,
                r#"input type:keyboard xkb_options "grp:win_space_toggle,caps:escape""#,
                "input type:keyboard repeat_delay 200",
                "input type:keyboard repeat_rate 40",
                "input type:touchpad pointer_accel 0.20",
                "input type:touchpad accel_profile flat",
                "input type:touchpad natural_scroll enabled",
                "input type:touchpad tap disabled",
                "input type:touchpad dwt enabled",
                "input type:touchpad click_method clickfinger",
                "input type:touchpad scroll_method two_finger",
                "input type:pointer pointer_accel -0.35",
                "input type:pointer accel_profile adaptive",
                "input type:pointer natural_scroll disabled",
            ]
        );
    }

    #[test]
    fn nothing_set_sends_nothing_so_the_sway_config_stands() {
        assert!(commands(&Input::default()).is_empty());
    }

    #[test]
    fn layouts_without_a_variant_leave_the_variants_cleared() {
        let input = Input {
            layouts: Some(vec!["se".into(), "us".into()]),
            ..Input::default()
        };
        assert_eq!(
            commands(&input),
            [
                r#"input type:keyboard xkb_variant """#,
                r#"input type:keyboard xkb_layout "se,us""#,
            ]
        );
    }

    #[test]
    fn one_xkb_option_set_sends_the_line_with_only_that_one() {
        let caps_only = Input {
            caps: Some("ctrl:nocaps".into()),
            ..Input::default()
        };
        assert_eq!(
            commands(&caps_only),
            [r#"input type:keyboard xkb_options "ctrl:nocaps""#]
        );
        // Chosen as "none" is an empty option list, which is still sent.
        let none = Input {
            layout_switch: Some(String::new()),
            ..Input::default()
        };
        assert_eq!(commands(&none), [r#"input type:keyboard xkb_options """#]);
    }

    #[test]
    fn a_change_sends_only_its_own_knob() {
        let old = all();
        let new = Input {
            repeat_rate: Some(50),
            ..all()
        };
        assert_eq!(
            changed(&old, &new),
            (
                vec!["input type:keyboard repeat_rate 50".to_string()],
                false
            )
        );
        // Caps changes the one options line, both options on it.
        let new = Input {
            caps: Some("caps:swapescape".into()),
            ..all()
        };
        assert_eq!(
            changed(&old, &new).0,
            [r#"input type:keyboard xkb_options "grp:win_space_toggle,caps:swapescape""#]
        );
        assert_eq!(changed(&old, &old), (Vec::new(), false));
    }

    #[test]
    fn a_knob_going_back_to_the_config_asks_for_a_reload() {
        let old = all();
        let new = Input {
            touchpad_tap: None,
            ..all()
        };
        assert_eq!(changed(&old, &new), (Vec::new(), true));
        assert!(changed(&old, &Input::default()).1);
        assert!(!changed(&Input::default(), &old).1);
    }

    #[test]
    fn get_inputs_parses_into_devices() {
        let reply = serde_json::json!([
            {
                "identifier": "1:1:AT_Translated_Set_2_keyboard",
                "name": "AT Translated Set 2 keyboard",
                "type": "keyboard",
                "repeat_delay": 200,
                "repeat_rate": 40,
                "xkb_layout_names": ["Swedish", "English (US)"],
                "xkb_active_layout_index": 0,
                "libinput": {"send_events": "enabled"}
            },
            {
                "identifier": "1267:12699:ELAN0678:00_04F3:3195_Touchpad",
                "name": "ELAN0678:00 04F3:3195 Touchpad",
                "type": "touchpad",
                "libinput": {
                    "send_events": "enabled", "tap": "disabled",
                    "accel_speed": 0.2, "accel_profile": "flat",
                    "natural_scroll": "enabled", "dwt": "enabled",
                    "click_method": "clickfinger", "scroll_method": "two_finger"
                }
            },
            {"identifier": "0:0:wlr_virtual_keyboard_v1", "name": "virtual", "type": "keyboard"}
        ]);
        let devices = parse_devices(&reply);
        assert_eq!(devices.len(), 3);
        let kb = &devices[0];
        assert_eq!(kb.kind, "keyboard");
        assert_eq!(kb.layouts, ["Swedish", "English (US)"]);
        assert_eq!((kb.repeat_delay_ms, kb.repeat_rate), (Some(200), Some(40)));
        assert_eq!(kb.tap, None);
        let tp = &devices[1];
        assert_eq!(tp.tap, Some(false));
        assert_eq!(tp.natural_scroll, Some(true));
        assert_eq!(tp.speed, Some(0.2));
        assert_eq!(tp.accel_profile, Some(AccelProfile::Flat));
        assert_eq!(tp.click_method, Some(ClickMethod::Clickfinger));
        assert_eq!(tp.scroll_method, Some(ScrollMethod::TwoFinger));
        assert_eq!(devices[2].repeat_rate, None);
        assert!(parse_devices(&serde_json::json!({"error": "x"})).is_empty());
    }
}
