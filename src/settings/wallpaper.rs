//! The wallpaper as something the panel can set and put back. The tab that
//! shows it is `look_pane.rs`; this is what the tab, the CLI and the panel's
//! startup share.
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
//! reaches a playing video over IPC instead (`services::wallpaper_battery`).

use std::path::{Path, PathBuf};

use super::store::{self, Wallpaper, WallpaperMode};

/// Where the candidates live: the directory Nix copies the shipped
/// wallpapers into. Anything else is reachable with Browse.
pub(super) fn candidates_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Pictures").join("wallpapers"))
}

fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "avif" | "jxl" | "bmp" | "tiff" | "tif"
        )
    })
}

/// The images in the candidates directory, sorted by name.
pub(super) fn candidates() -> Vec<PathBuf> {
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

/// The command sway takes. The path is quoted, so a space in it survives
/// sway's splitter; a path with a `"` in it is refused rather than sent
/// half-parsed.
fn command(w: &Wallpaper) -> Option<String> {
    let path = w.path.to_str()?;
    if path.contains('"') {
        log::warn!("wallpaper: refusing a path with a quote in it: {path}");
        return None;
    }
    Some(format!("output * bg \"{path}\" {}", w.mode.as_str()))
}

pub fn apply(w: &Wallpaper) {
    if let Some(cmd) = command(w) {
        crate::sway::ipc::run_command(&cmd);
    }
    let still = w.path.clone();
    let speed = video_speed();
    crate::spawn::spawn_work(move || follow_video(&still, speed), |()| ());
}

/// [`apply`] for a process with no main loop (`swaypplet settings`): one
/// connection, one command, and sway's answer. `speed` is
/// `look.video_speed`, which a thread off the panel's main one cannot read
/// from the live settings.
pub fn apply_blocking(w: &Wallpaper, speed: f64) -> Result<(), String> {
    let cmd = command(w).ok_or("path not sendable")?;
    let outcomes = crate::sway::ipc::connect()
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

/// The video `still` stands for: a file beside it with the same name and a
/// video extension.
pub(crate) fn video_for(still: &Path) -> Option<PathBuf> {
    ["mp4", "webm", "mkv"]
        .iter()
        .map(|ext| still.with_extension(ext))
        .find(|p| p.is_file())
}

/// The unit's environment file: `VIDEO=<path>`, the one thing it plays,
/// and `SPEED=<multiple>`, how fast (mpv's `speed`).
fn video_env() -> PathBuf {
    glib::user_runtime_dir()
        .join("swaypplet")
        .join("wallpaper-video.env")
}

/// `look.video_speed` from the live settings: the panel's main thread only.
fn video_speed() -> f64 {
    store::with(|s| s.look().video_speed)
}

/// The environment file's text for `video` played at `speed`.
fn env_text(video: &Path, speed: f64) -> String {
    format!("VIDEO={}\nSPEED={speed}\n", video.display())
}

/// The video an environment file's text names.
fn env_video(text: &str) -> Option<&str> {
    text.lines().find_map(|l| l.strip_prefix("VIDEO="))
}

/// What following `video` at `speed` does to a unit whose environment file
/// reads `old`: the text to write, if it changed, and the `systemctl` verb.
/// Only another video restarts the unit. A speed alone does not: the player
/// takes a new one over IPC (`services::wallpaper_battery`), and the file
/// is for its next start.
fn env_change(old: Option<&str>, video: &Path, speed: f64) -> (Option<String>, &'static str) {
    let text = env_text(video, speed);
    let same_video = old
        .and_then(env_video)
        .is_some_and(|v| Path::new(v) == video);
    let write = (old != Some(text.as_str())).then_some(text);
    // `start` is a no-op on a running unit; a new file needs a restart.
    (write, if same_video { "start" } else { "restart" })
}

/// `text` with its speed line made `speed`, the video left as it is.
fn with_speed(text: &str, speed: f64) -> String {
    let mut out: String = text
        .lines()
        .filter(|l| !l.starts_with("SPEED="))
        .map(|l| format!("{l}\n"))
        .collect();
    out.push_str(&format!("SPEED={speed}\n"));
    out
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
fn follow_video(still: &Path, speed: f64) {
    let Some(video) = video_for(still) else {
        systemctl("stop");
        return;
    };
    let env = video_env();
    let old = std::fs::read_to_string(&env).ok();
    let (write, verb) = env_change(old.as_deref(), &video, speed);
    if let Some(text) = write
        && let Err(e) = write_env(&env, &text)
    {
        log::warn!("wallpaper: {}: {e}", env.display());
        return;
    }
    systemctl(verb);
}

/// Put a new `look.video_speed` in the environment file, for the unit's
/// next start; the playing video takes it over IPC. Nothing when no video
/// was ever picked (no file), and never a restart. Blocking: a small file
/// read and write.
pub(crate) fn sync_video_speed(speed: f64) {
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
pub(crate) fn freeze_video(frozen: bool) {
    let verb = if frozen { "freeze" } else { "thaw" };
    match std::process::Command::new("systemctl")
        .args(["--user", verb, VIDEO_UNIT])
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(s) if s.success() => {}
        Ok(s) => log::debug!("wallpaper: systemctl {verb} {VIDEO_UNIT}: {s}"),
        Err(e) => log::warn!("wallpaper: systemctl {verb} {VIDEO_UNIT}: {e}"),
    }
}

fn systemctl(verb: &str) {
    match std::process::Command::new("systemctl")
        .args(["--user", "--no-block", verb, VIDEO_UNIT])
        .status()
    {
        Ok(s) if s.success() => {}
        Ok(s) => log::warn!("wallpaper: systemctl {verb} {VIDEO_UNIT}: {s}"),
        Err(e) => log::warn!("wallpaper: systemctl {verb} {VIDEO_UNIT}: {e}"),
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
        crate::spawn::spawn_work(
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
    match crate::sway::ipc::config_text() {
        Ok(text) => parse_bg(&text),
        Err(e) => {
            log::warn!("wallpaper: {e}");
            None
        }
    }
}

/// The `bg` line out of a sway config, as `system_default` documents.
fn parse_bg(config: &str) -> Option<Wallpaper> {
    let mut block: Option<String> = None;
    let mut fallback = None;
    for raw in config.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words = split_words(line);
        if line == "}" {
            block = None;
            continue;
        }
        let (output, rest) = match (block.as_deref(), words.first().map(String::as_str)) {
            (None, Some("output")) if words.last().is_some_and(|w| w == "{") => {
                block = words.get(1).cloned();
                continue;
            }
            (None, Some("output")) => (words.get(1)?.clone(), &words[2..]),
            (Some(name), Some(_)) => (name.to_string(), &words[..]),
            _ => continue,
        };
        if rest.first().map(String::as_str) != Some("bg") {
            continue;
        }
        let path = expand_home(rest.get(1)?);
        let mode = rest
            .get(2)
            .and_then(|m| WallpaperMode::parse(m))
            .unwrap_or_default();
        let found = Wallpaper { path, mode };
        if output == "*" {
            return Some(found);
        }
        fallback.get_or_insert(found);
    }
    fallback
}

/// A config line into its words, with double quotes grouping and dropped.
fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_form_home_manager_writes_is_read() {
        let config = "\
set $mod Mod4

output \"*\" {
  bg /home/meros/Pictures/wallpapers/alcohol-ink-4k.jpg fill
}

seat \"*\" {
  xcursor_theme Adwaita 24
}
";
        let w = parse_bg(config).unwrap();
        assert_eq!(
            w.path,
            PathBuf::from("/home/meros/Pictures/wallpapers/alcohol-ink-4k.jpg")
        );
        assert_eq!(w.mode, WallpaperMode::Fill);
    }

    #[test]
    fn the_one_liner_and_quoted_paths_are_read() {
        let w = parse_bg("output * bg \"/tmp/my wallpaper.png\" fit\n").unwrap();
        assert_eq!(w.path, PathBuf::from("/tmp/my wallpaper.png"));
        assert_eq!(w.mode, WallpaperMode::Fit);
        // A missing mode is sway's default.
        let w = parse_bg("output * bg /tmp/a.png\n").unwrap();
        assert_eq!(w.mode, WallpaperMode::Fill);
    }

    #[test]
    fn the_wildcard_output_wins_over_a_named_one() {
        let config = "output eDP-1 bg /a.png fill\noutput * bg /b.png fill\n";
        assert_eq!(parse_bg(config).unwrap().path, PathBuf::from("/b.png"));
        // With no wildcard, the first named one is better than nothing.
        assert_eq!(
            parse_bg("output eDP-1 bg /a.png fill\n").unwrap().path,
            PathBuf::from("/a.png")
        );
        assert!(parse_bg("output * resolution 1920x1080\n").is_none());
        assert!(parse_bg("# output * bg /a.png fill\n").is_none());
    }

    #[test]
    fn the_command_quotes_the_path_and_refuses_a_quote_in_it() {
        let w = Wallpaper {
            path: PathBuf::from("/tmp/a b.png"),
            mode: WallpaperMode::Tile,
        };
        assert_eq!(command(&w).unwrap(), "output * bg \"/tmp/a b.png\" tile");
        let bad = Wallpaper {
            path: PathBuf::from("/tmp/a\"b.png"),
            mode: WallpaperMode::Fill,
        };
        assert!(command(&bad).is_none());
    }

    #[test]
    fn only_images_are_candidates() {
        assert!(is_image(Path::new("/x/a.JPG")));
        assert!(!is_image(Path::new("/x/a.mp4")));
        assert!(is_image(Path::new("/x/a.png")));
        assert!(!is_image(Path::new("/x/a.txt")));
        assert!(!is_image(Path::new("/x/noext")));
    }

    /// A still with a same-named video beside it stands for the video; one
    /// without, or with only a differently named video, does not.
    #[test]
    fn a_still_stands_for_the_video_named_like_it() {
        let dir = std::env::temp_dir().join(format!("swpp-video-for-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let still = dir.join("beach.jpg");
        let other = dir.join("fields.jpg");
        for f in [&still, &other, &dir.join("beach.mp4"), &dir.join("waves.mp4")] {
            std::fs::write(f, b"").unwrap();
        }
        assert_eq!(video_for(&still), Some(dir.join("beach.mp4")));
        assert_eq!(video_for(&other), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The file names the video and its speed; only another video restarts
    /// the unit, and a file already right is not written again.
    #[test]
    fn the_environment_file_carries_the_speed_and_a_speed_alone_restarts_nothing() {
        let beach = Path::new("/w/beach.mp4");
        assert_eq!(env_text(beach, 0.5), "VIDEO=/w/beach.mp4\nSPEED=0.5\n");
        assert_eq!(env_text(beach, 1.0), "VIDEO=/w/beach.mp4\nSPEED=1\n");
        // Nothing yet: written, and started afresh.
        let (write, verb) = env_change(None, beach, 1.0);
        assert_eq!(write.as_deref(), Some("VIDEO=/w/beach.mp4\nSPEED=1\n"));
        assert_eq!(verb, "restart");
        // The same file: left alone, and a running unit with it.
        let (write, verb) = env_change(Some("VIDEO=/w/beach.mp4\nSPEED=1\n"), beach, 1.0);
        assert_eq!((write, verb), (None, "start"));
        // Another speed: written, but no restart.
        let (write, verb) = env_change(Some("VIDEO=/w/beach.mp4\nSPEED=1\n"), beach, 2.0);
        assert_eq!(write.as_deref(), Some("VIDEO=/w/beach.mp4\nSPEED=2\n"));
        assert_eq!(verb, "start");
        // A file from before the speed, with the same video: no restart.
        let (write, verb) = env_change(Some("VIDEO=/w/beach.mp4\n"), beach, 1.0);
        assert!(write.is_some());
        assert_eq!(verb, "start");
        // Another video: a restart.
        let (_, verb) = env_change(Some("VIDEO=/w/waves.mp4\nSPEED=1\n"), beach, 1.0);
        assert_eq!(verb, "restart");
    }

    #[test]
    fn a_new_speed_keeps_the_video_line() {
        assert_eq!(
            with_speed("VIDEO=/w/beach.mp4\nSPEED=1\n", 0.75),
            "VIDEO=/w/beach.mp4\nSPEED=0.75\n"
        );
        assert_eq!(
            with_speed("VIDEO=/w/beach.mp4\n", 2.0),
            "VIDEO=/w/beach.mp4\nSPEED=2\n"
        );
    }
}
