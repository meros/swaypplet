//! The wallpaper as something the panel can set and put back. The tab that
//! shows it is `settings-look`; this is what the tab, `swaypplet settings`
//! and the panel's startup share. The parsing and the environment file's
//! text are pure, in `swaypplet_model::wallpaper`.
//!
//! sway owns the wallpaper: `output * bg <path> <mode>` from the config,
//! which Nix writes (`users/modules/sway.nix`). A pick here is the same
//! command sent live over IPC, so it lands on every output at once and the
//! lock screen follows for free — the compositor draws the lock's backdrop
//! from the background layer (`glass-config.nix`, "session-lock"). The
//! greeter is another sway with its own config and is not reached.
//!
//! The system default is not guessed and not stored: [`system_default`]
//! reads the config sway actually loaded and finds the `bg` line, which is
//! what Reset applies. `apply_saved` replays the pick when the panel starts,
//! after the config has run. One thing it cannot follow is a `swaymsg
//! reload`, which re-runs the config's line; the next panel start puts the
//! pick back.
//!
//! A still can stand for a video: a file beside it with the same name and a
//! video extension (`beach-waves-4k.jpg` and `beach-waves-4k.mp4`). Picking
//! that still starts the video (the `wallpaper-video` user unit, mpvpaper,
//! which plays the file named in its environment file) over it; picking any
//! other stops it, so the still shows. The still stays sway's `bg` either
//! way: it is what the lock screen and the greeter fall back to. The same
//! file carries `look.video_speed` for the unit's start; a change of speed
//! reaches a playing video over IPC instead (`ambient::wallpaper_battery`).

use std::path::{Path, PathBuf};

use swaypplet_model::store::{self, Wallpaper};
use swaypplet_model::wallpaper::{command, env_change, is_image, parse_bg, with_speed};

use crate::systemd::{Unit, Verb, dry_session};

/// Where the candidates live: the directory Nix copies the shipped
/// wallpapers into. Anything else is reachable with Browse.
pub fn candidates_dir() -> Option<PathBuf> {
    Some(
        swaypplet_base::paths::home()?
            .join("Pictures")
            .join("wallpapers"),
    )
}

/// The images in the candidates directory, sorted by name.
pub fn candidates() -> Vec<PathBuf> {
    let Some(dir) = candidates_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_image(p))
        .collect();
    paths.sort();
    paths
}

// ── Applying ────────────────────────────────────────────────────────────

pub fn apply(w: &Wallpaper) {
    if let Some(cmd) = command(w) {
        swaypplet_base::sway::ipc::run_command(&cmd);
    }
    let still = w.path.clone();
    let speed = video_speed();
    swaypplet_base::spawn::spawn_work(move || follow_video(&still, speed), |()| ());
}

/// [`apply`] for a process with no main loop (`swaypplet settings`): one
/// connection, one command, and sway's answer. `speed` is
/// `look.video_speed`, which a thread off the panel's main one cannot read
/// from the live settings.
pub fn apply_blocking(w: &Wallpaper, speed: f64) -> Result<(), String> {
    let cmd = command(w).ok_or("path not sendable")?;
    let outcomes = swaypplet_base::sway::ipc::connect()
        .and_then(|mut c| c.run_command(&cmd))
        .map_err(|e| format!("sway ipc: {e}"))?;
    follow_video(&w.path, speed);
    outcomes
        .into_iter()
        .find_map(Result::err)
        .map_or(Ok(()), |e| Err(format!("sway: {e}")))
}

// ── The video a still stands for ────────────────────────────────────────

/// The user unit that plays the wallpaper video (nixos
/// `users/modules/wallpaper-video.nix`).
const VIDEO_UNIT: &str = "wallpaper-video.service";

fn video_unit() -> Unit {
    Unit::user(VIDEO_UNIT)
}

/// The video `still` stands for: a file beside it with the same name and a
/// video extension.
pub fn video_for(still: &Path) -> Option<PathBuf> {
    ["mp4", "webm", "mkv"]
        .iter()
        .map(|ext| still.with_extension(ext))
        .find(|p| p.is_file())
}

/// The unit's environment file: `VIDEO=<path>`, the one thing it plays,
/// and `SPEED=<multiple>`, how fast (mpv's `speed`).
fn video_env() -> PathBuf {
    // glib's answer without a runtime directory, kept: the cache directory.
    swaypplet_base::paths::runtime_dir()
        .or_else(swaypplet_base::paths::cache_dir)
        .unwrap_or_else(glib::user_runtime_dir)
        .join("swaypplet")
        .join("wallpaper-video.env")
}

/// `look.video_speed` from the live settings: the panel's main thread only.
fn video_speed() -> f64 {
    store::with(|s| s.look().video_speed)
}

fn write_env(env: &Path, text: &str) -> std::io::Result<()> {
    env.parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(env, text))
}

/// Start the video `still` stands for, or stop the one playing. A video
/// already playing the same file is left alone, so replaying a pick (panel
/// start, `settings apply` on reload, the battery service putting the
/// wallpaper back) does not restart it; nor does a new `speed`, which only
/// waits in the file for the next start. Blocking: two small file
/// operations and a `systemctl` call.
///
/// Nothing at all in a test run: the environment file lives in the
/// person's own runtime directory, and the unit is their desktop's.
fn follow_video(still: &Path, speed: f64) {
    if dry_session() {
        log::info!("wallpaper: video for {} (dry run)", still.display());
        return;
    }
    let Some(video) = video_for(still) else {
        queue(Verb::Stop);
        return;
    };
    let env = video_env();
    let old = std::fs::read_to_string(&env).ok();
    let (write, restart) = env_change(old.as_deref(), &video, speed);
    if let Some(text) = write
        && let Err(e) = write_env(&env, &text)
    {
        log::warn!("wallpaper: {}: {e}", env.display());
        return;
    }
    // `start` is a no-op on a running unit; a new file needs a restart.
    queue(if restart { Verb::Restart } else { Verb::Start });
}

/// Put a new `look.video_speed` in the environment file, for the unit's
/// next start; the playing video takes it over IPC. Nothing when no video
/// was ever picked (no file), and never a restart. Blocking: a small file
/// read and write.
pub fn sync_video_speed(speed: f64) {
    let env = video_env();
    let Ok(old) = std::fs::read_to_string(&env) else {
        return;
    };
    let text = with_speed(&old, speed);
    if text != old
        && let Err(e) = write_env(&env, &text)
    {
        log::warn!("wallpaper: {}: {e}", env.display());
    }
}

/// Freeze the video's unit (systemd's cgroup freezer), or thaw it. mpvpaper
/// polls on a 10 ms timer whether mpv plays or not, about 200 wake-ups a
/// second paused; frozen, it runs not at all, and its last frame stays on
/// screen. Blocking: `systemctl` returns once the unit is frozen or thawed
/// (13 ms measured). A unit that is not running cannot be frozen, which is
/// the normal state of a session without the video: logged at debug only.
pub fn freeze_video(frozen: bool) {
    let verb = if frozen { Verb::Freeze } else { Verb::Thaw };
    if let Err(e) = video_unit().run(verb) {
        log::debug!("wallpaper: {e}");
    }
}

fn queue(verb: Verb) {
    if let Err(e) = video_unit().queue(verb) {
        log::warn!("wallpaper: {e}");
    }
}

/// Replay the saved pick at panel startup.
pub fn apply_saved() {
    if let Some(w) = store::current().wallpaper {
        log::info!("wallpaper: replaying {}", w.path.display());
        apply(&w);
    } else {
        // No pick: sway's config line stands, and only the video it may
        // stand for is this process's to start.
        let speed = video_speed();
        swaypplet_base::spawn::spawn_work(
            move || {
                if let Some(w) = system_default() {
                    follow_video(&w.path, speed);
                }
            },
            |()| (),
        );
    }
}

/// What the config sway loaded says the wallpaper is: the `bg` under
/// `output *` (block form or one-liner), or failing that the first `bg` of
/// any output. `None` when the config sets none, or sway could not be asked.
///
/// Blocking (one IPC round trip); the pane calls it on a worker.
pub fn system_default() -> Option<Wallpaper> {
    match swaypplet_base::sway::ipc::config_text() {
        Ok(text) => parse_bg(&text),
        Err(e) => {
            log::warn!("wallpaper: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A still with a same-named video beside it stands for the video; one
    /// without, or with only a differently named video, does not.
    #[test]
    fn a_still_stands_for_the_video_named_like_it() {
        let dir = std::env::temp_dir().join(format!("swpp-video-for-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let still = dir.join("beach.jpg");
        let other = dir.join("fields.jpg");
        for f in [
            &still,
            &other,
            &dir.join("beach.mp4"),
            &dir.join("waves.mp4"),
        ] {
            std::fs::write(f, b"").unwrap();
        }
        assert_eq!(video_for(&still), Some(dir.join("beach.mp4")));
        assert_eq!(video_for(&other), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
