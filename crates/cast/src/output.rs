//! The extra screen: a virtual output, made, placed and removed over sway
//! IPC.
//!
//! `create_output` adds a headless output and answers only "success": sway
//! picks the name (HEADLESS-N, counting up, never reused in a session), so
//! the name is read back as the virtual output that was not there before.
//! sway gives it 1920×1080 with no refresh, to the right of the rest, and a
//! fresh workspace of its own; it is then set to [`MODE`] at the right edge
//! of the screens, where moving the pointer off the right side of the
//! desktop reaches it. `output NAME unplug` removes it, and its workspaces
//! move to the screens that are left.
//!
//! A display profile can say where it goes and how it is drawn
//! (`DisplayProfile::extra_screen`, saved from the Displays tab): the
//! profile on screen gives `create` its layout, and what it leaves out is
//! the default above. Profiles are never matched against it
//! (`displays::matcher::split`).
//!
//! The name made is written to the runtime directory with the sway socket
//! it was made on. A panel that restarts while a cast runs has lost the
//! stream that fed it, so [`sweep`] removes that output when the same sway
//! is still running. A record from another sway names an output that sway
//! never made, and is only deleted.
//!
//! Blocking: call from a worker (`spawn::spawn_work`).

use std::path::PathBuf;

use swayipc::Connection;

use swaypplet_base::naming::display::is_virtual;
use swaypplet_model::store::{ExtraScreenLayout, OutputTransform};

/// Width, height and refresh in Hz: 1080p, the size every Cast screen and
/// every TV takes. A stream backend sends at its own rate (wayvnc: 30 fps);
/// the refresh is only how often sway draws the output.
pub const MODE: (u32, u32, u32) = (1920, 1080, 60);

/// One output's name and logical rectangle, as `get_outputs` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub name: String,
    pub active: bool,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The virtual output among `after` that `before` did not have.
pub fn appeared(before: &[String], after: &[String]) -> Option<String> {
    after
        .iter()
        .find(|n| is_virtual(n) && !before.contains(n))
        .cloned()
}

/// Where the extra screen goes: against the right edge of the rightmost
/// active output before it was made, level with its top. The origin when
/// there is none.
pub fn right_of(outputs: &[Placed]) -> (i32, i32) {
    outputs
        .iter()
        .filter(|o| o.active)
        .max_by_key(|o| (o.x + o.width, -o.y))
        .map_or((0, 0), |o| (o.x + o.width, o.y))
}

/// The command that gives `name` its mode, place, scale and rotation:
/// `layout`'s where it says, else [`MODE`] right of `outputs`.
pub fn configure_command(name: &str, layout: &ExtraScreenLayout, outputs: &[Placed]) -> String {
    let (w, h, hz) = match layout.mode {
        Some([w, h, mhz]) if mhz > 0 => (w, h, format!("{}", f64::from(mhz) / 1000.0)),
        Some([w, h, _]) => (w, h, MODE.2.to_string()),
        None => (MODE.0, MODE.1, MODE.2.to_string()),
    };
    let (x, y) = layout
        .position
        .map_or_else(|| right_of(outputs), |[x, y]| (x, y));
    let scale = layout.scale.unwrap_or(1.0);
    let transform = layout.transform.unwrap_or(OutputTransform::Normal).sway();
    format!(
        "output {name} mode --custom {w}x{h}@{hz}Hz position {x} {y} scale {scale} transform {transform}"
    )
}

fn placed(conn: &mut Connection) -> Result<Vec<Placed>, String> {
    let outputs = conn
        .get_outputs()
        .map_err(|e| format!("sway ipc: get_outputs: {e}"))?;
    Ok(outputs
        .into_iter()
        .map(|o| Placed {
            name: o.name,
            active: o.active,
            x: o.rect.x,
            y: o.rect.y,
            width: o.rect.width,
            height: o.rect.height,
        })
        .collect())
}

/// The output of the focused workspace.
fn focused(conn: &mut Connection) -> Option<String> {
    conn.get_workspaces()
        .ok()?
        .into_iter()
        .find(|w| w.focused)
        .map(|w| w.output)
}

/// Where focus goes back to after `create_output`: the output that had it,
/// when sway moved it onto the new output `made`. `None` leaves it.
pub fn refocus(before: Option<&str>, after: Option<&str>, made: &str) -> Option<String> {
    let before = before.filter(|b| *b != made)?;
    (after == Some(made)).then(|| before.to_string())
}

fn run(conn: &mut Connection, cmd: &str) -> Result<(), String> {
    conn.run_command(cmd)
        .map_err(|e| e.to_string())
        .and_then(|o| {
            o.into_iter()
                .find(Result::is_err)
                .unwrap_or(Ok(()))
                .map_err(|e| e.to_string())
        })
        .map_err(|e| format!("sway ipc: `{cmd}`: {e}"))
}

/// Make the extra screen and place it, as `layout` says. Its name.
pub fn create(layout: &ExtraScreenLayout) -> Result<String, String> {
    let mut conn = swaypplet_base::sway::ipc::connect().map_err(|e| format!("sway ipc: {e}"))?;
    let before = placed(&mut conn)?;
    let names: Vec<String> = before.iter().map(|o| o.name.clone()).collect();
    let focused_before = focused(&mut conn);
    run(&mut conn, "create_output")?;
    let after: Vec<String> = placed(&mut conn)?.into_iter().map(|o| o.name).collect();
    let name =
        appeared(&names, &after).ok_or_else(|| "sway made no new headless output".to_string())?;
    record(&name);
    // While a layer surface holds the keyboard (the Helm, settings: where
    // Start is pressed), sway's seat has no window focused, and sway hands
    // the new output's workspace the focus. The next Helm would then open
    // on the TV. Give the focus back.
    let after = focused(&mut conn);
    if let Some(back) = refocus(focused_before.as_deref(), after.as_deref(), &name)
        && let Err(e) = run(&mut conn, &format!("focus output {back}"))
    {
        log::warn!("cast: {e}");
    }
    if let Err(e) = run(&mut conn, &configure_command(&name, layout, &before)) {
        // The output is there either way; at sway's own size and place it
        // still works, so this is worth a line in the log, not a failure.
        log::warn!("cast: {e}");
    }
    log::info!("cast: made {name}");
    Ok(name)
}

/// Remove the extra screen `name`. Refuses any output that is not virtual:
/// a typo here must never be able to unplug a monitor.
pub fn remove(name: &str) -> Result<(), String> {
    if !is_virtual(name) {
        return Err(format!("{name} is not a virtual output"));
    }
    forget();
    let mut conn = swaypplet_base::sway::ipc::connect().map_err(|e| format!("sway ipc: {e}"))?;
    run(&mut conn, &format!("output {name} unplug"))?;
    log::info!("cast: removed {name}");
    Ok(())
}

/// Remove an extra screen a previous panel made on this sway and left.
pub fn sweep() {
    let Some((name, socket)) = read_record() else {
        return;
    };
    forget();
    if Some(socket) != current_socket() {
        return;
    }
    let Ok(mut conn) = swaypplet_base::sway::ipc::connect() else {
        return;
    };
    let alive = placed(&mut conn).is_ok_and(|o| o.iter().any(|o| o.name == name));
    if alive && is_virtual(&name) {
        match run(&mut conn, &format!("output {name} unplug")) {
            Ok(()) => log::info!("cast: removed {name}, left by the panel before"),
            Err(e) => log::warn!("cast: {e}"),
        }
    }
}

/// The socket this process talks to sway on, as the environment names it.
/// `None` when it does not: the sweep then deletes the record and touches
/// nothing.
fn current_socket() -> Option<String> {
    ["SWAYSOCK", "I3SOCK"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .filter(|s| !s.is_empty())
}

fn record_path() -> Option<PathBuf> {
    swaypplet_base::paths::runtime_dir().map(|d| d.join("swaypplet/cast-output"))
}

fn record(name: &str) {
    let (Some(path), Some(socket)) = (record_path(), current_socket()) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, format!("{name}\n{socket}\n")) {
        log::debug!("cast: record {}: {e}", path.display());
    }
}

fn read_record() -> Option<(String, String)> {
    parse_record(&std::fs::read_to_string(record_path()?).ok()?)
}

/// The record's two lines: the output's name and the sway socket.
fn parse_record(text: &str) -> Option<(String, String)> {
    let mut lines = text.lines().map(str::trim);
    let name = lines.next().filter(|s| !s.is_empty())?.to_string();
    let socket = lines.next().filter(|s| !s.is_empty())?.to_string();
    Some((name, socket))
}

fn forget() {
    if let Some(path) = record_path() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    fn out(name: &str, x: i32, y: i32, w: i32, h: i32) -> Placed {
        Placed {
            name: name.into(),
            active: true,
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn the_new_name_is_the_virtual_output_that_was_not_there() {
        let before = names(&["eDP-1", "DP-3"]);
        assert_eq!(
            appeared(&before, &names(&["eDP-1", "DP-3", "HEADLESS-1"])),
            Some("HEADLESS-1".into())
        );
        // sway counts on after an unplug: HEADLESS-2 is gone, 3 is new.
        let before = names(&["eDP-1", "HEADLESS-1"]);
        assert_eq!(
            appeared(&before, &names(&["eDP-1", "HEADLESS-1", "HEADLESS-3"])),
            Some("HEADLESS-3".into())
        );
        // A monitor plugged in at the same moment is not the extra screen.
        assert_eq!(
            appeared(&before, &names(&["eDP-1", "HEADLESS-1", "DP-1"])),
            None
        );
        assert_eq!(appeared(&before, &before), None);
    }

    #[test]
    fn the_extra_screen_goes_right_of_the_rightmost_screen() {
        // The desk: the 28" at 0,0 (2560 logical wide), the laptop to its
        // right at 2560,149, 1440 wide at scale 2.
        let desk = [
            out("DP-3", 0, 0, 2560, 1440),
            out("eDP-1", 2560, 149, 1440, 900),
        ];
        assert_eq!(right_of(&desk), (4000, 149));
        // A disabled screen does not count.
        let mut off = out("HDMI-A-1", 4000, 0, 1920, 1080);
        off.active = false;
        let with = [desk[0].clone(), desk[1].clone(), off];
        assert_eq!(right_of(&with), (4000, 149));
        // The nested harness: only headless outputs, and the new one still
        // goes beside them, not on top.
        assert_eq!(right_of(&[out("HEADLESS-1", 0, 0, 1280, 800)]), (1280, 0));
        assert_eq!(right_of(&[]), (0, 0));
    }

    /// Against a nested sway with the Helm open (a layer surface holding
    /// the keyboard, which is when sway moves focus to a new output): the
    /// focused output is the same after `create` as before.
    ///   SWAYSOCK=<nested socket> cargo test -- --ignored create_leaves_the_focus
    #[test]
    #[ignore = "needs a nested sway with the Helm open"]
    fn create_leaves_the_focus() {
        let mut conn = swaypplet_base::sway::ipc::connect().unwrap();
        let before = focused(&mut conn);
        let name = create(&ExtraScreenLayout::default()).expect("create");
        let after = focused(&mut conn);
        remove(&name).expect("remove");
        assert_eq!(after, before, "focus moved to {name}");
    }

    #[test]
    fn focus_goes_back_only_when_sway_moved_it_to_the_new_output() {
        // The Helm was open: sway focused the new output's workspace.
        assert_eq!(
            refocus(Some("eDP-1"), Some("HEADLESS-3"), "HEADLESS-3"),
            Some("eDP-1".into())
        );
        // Nothing open: the focus stayed, and nothing is sent.
        assert_eq!(refocus(Some("eDP-1"), Some("eDP-1"), "HEADLESS-3"), None);
        // The person moved it elsewhere meanwhile: theirs.
        assert_eq!(refocus(Some("eDP-1"), Some("DP-3"), "HEADLESS-3"), None);
        // No focus to go back to.
        assert_eq!(refocus(None, Some("HEADLESS-3"), "HEADLESS-3"), None);
    }

    #[test]
    fn the_command_sets_a_custom_mode_and_a_place() {
        let desk = [
            out("DP-3", 0, 0, 2560, 1440),
            out("eDP-1", 2560, 149, 1440, 900),
        ];
        assert_eq!(
            configure_command("HEADLESS-3", &ExtraScreenLayout::default(), &desk),
            "output HEADLESS-3 mode --custom 1920x1080@60Hz position 4000 149 scale 1 transform normal"
        );
    }

    #[test]
    fn a_profiles_layout_wins_where_it_says_something() {
        let desk = [out("eDP-1", 0, 0, 1440, 900)];
        let layout = ExtraScreenLayout {
            mode: Some([1280, 720, 59940]),
            position: Some([-1280, 0]),
            scale: Some(1.5),
            transform: Some(OutputTransform::R90),
        };
        assert_eq!(
            configure_command("HEADLESS-3", &layout, &desk),
            "output HEADLESS-3 mode --custom 1280x720@59.94Hz position -1280 0 scale 1.5 transform 90"
        );
        // Only a scale: the default size, right of the screens.
        let scaled = ExtraScreenLayout {
            scale: Some(2.0),
            ..ExtraScreenLayout::default()
        };
        assert_eq!(
            configure_command("HEADLESS-3", &scaled, &desk),
            "output HEADLESS-3 mode --custom 1920x1080@60Hz position 1440 0 scale 2 transform normal"
        );
    }

    #[test]
    fn a_monitor_is_never_removed() {
        assert!(remove("eDP-1").is_err());
        assert!(remove("DP-3").is_err());
    }

    #[test]
    fn the_record_is_a_name_and_a_socket() {
        assert_eq!(
            parse_record("HEADLESS-2\n/run/user/1000/sway-ipc.1000.42.sock\n"),
            Some((
                "HEADLESS-2".into(),
                "/run/user/1000/sway-ipc.1000.42.sock".into()
            ))
        );
        assert_eq!(parse_record("HEADLESS-2\n"), None);
        assert_eq!(parse_record(""), None);
    }
}
