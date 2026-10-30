//! The wallpaper's pure parts: which files are pictures, the command sway
//! takes, the `bg` line out of a sway config, and the video unit's
//! environment file. Setting it, the video and the panel's startup replay
//! are the service, `swaypplet_system::wallpaper`, whose docs say how the
//! pieces fit.

use std::path::Path;

use super::store::{Wallpaper, WallpaperMode};

/// Whether `path` names a picture, by its extension.
pub fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "avif" | "jxl" | "bmp" | "tiff" | "tif"
        )
    })
}

/// The command sway takes. The path is quoted, so a space in it survives
/// sway's splitter; a path with a `"` in it is refused rather than sent
/// half-parsed.
pub fn command(w: &Wallpaper) -> Option<String> {
    let path = w.path.to_str()?;
    if path.contains('"') {
        log::warn!("wallpaper: refusing a path with a quote in it: {path}");
        return None;
    }
    Some(format!("output * bg \"{path}\" {}", w.mode.as_str()))
}

// ── The video unit's environment file ───────────────────────────────────

/// The environment file's text for `video` played at `speed`.
fn env_text(video: &Path, speed: f64) -> String {
    format!("VIDEO={}\nSPEED={speed}\n", video.display())
}

/// The video an environment file's text names.
fn env_video(text: &str) -> Option<&str> {
    text.lines().find_map(|l| l.strip_prefix("VIDEO="))
}

/// What following `video` at `speed` does to a unit whose environment file
/// reads `old`: the text to write, if it changed, and whether the unit
/// restarts (otherwise it is only started, a no-op on a running one). Only
/// another video restarts it. A speed alone does not: the player takes a
/// new one over IPC (`ambient::wallpaper_battery`), and the file is for its
/// next start.
pub fn env_change(old: Option<&str>, video: &Path, speed: f64) -> (Option<String>, bool) {
    let text = env_text(video, speed);
    let same_video = old
        .and_then(env_video)
        .is_some_and(|v| Path::new(v) == video);
    let write = (old != Some(text.as_str())).then_some(text);
    (write, !same_video)
}

/// `text` with its speed line made `speed`, the video left as it is.
pub fn with_speed(text: &str, speed: f64) -> String {
    let mut out: String = text
        .lines()
        .filter(|l| !l.starts_with("SPEED="))
        .map(|l| format!("{l}\n"))
        .collect();
    out.push_str(&format!("SPEED={speed}\n"));
    out
}

// ── The config's own wallpaper ──────────────────────────────────────────

/// The `bg` line out of a sway config: the one under `output *` (block
/// form or one-liner), or failing that the first `bg` of any output.
/// `None` when the config sets none.
pub fn parse_bg(config: &str) -> Option<Wallpaper> {
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
        let path = swaypplet_base::paths::expand_home(rest.get(1)?);
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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
    fn a_config_line_splits_on_spaces_outside_quotes() {
        assert_eq!(
            split_words("output \"*\"  bg \"/a b.png\" fill"),
            ["output", "*", "bg", "/a b.png", "fill"]
        );
        assert!(split_words("   ").is_empty());
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

    /// The file names the video and its speed; only another video restarts
    /// the unit, and a file already right is not written again.
    #[test]
    fn the_environment_file_carries_the_speed_and_a_speed_alone_restarts_nothing() {
        let beach = Path::new("/w/beach.mp4");
        assert_eq!(env_text(beach, 0.5), "VIDEO=/w/beach.mp4\nSPEED=0.5\n");
        assert_eq!(env_text(beach, 1.0), "VIDEO=/w/beach.mp4\nSPEED=1\n");
        // Nothing yet: written, and started afresh.
        let (write, restart) = env_change(None, beach, 1.0);
        assert_eq!(write.as_deref(), Some("VIDEO=/w/beach.mp4\nSPEED=1\n"));
        assert!(restart);
        // The same file: left alone, and a running unit with it.
        let (write, restart) = env_change(Some("VIDEO=/w/beach.mp4\nSPEED=1\n"), beach, 1.0);
        assert_eq!((write, restart), (None, false));
        // Another speed: written, but no restart.
        let (write, restart) = env_change(Some("VIDEO=/w/beach.mp4\nSPEED=1\n"), beach, 2.0);
        assert_eq!(write.as_deref(), Some("VIDEO=/w/beach.mp4\nSPEED=2\n"));
        assert!(!restart);
        // A file from before the speed, with the same video: no restart.
        let (write, restart) = env_change(Some("VIDEO=/w/beach.mp4\n"), beach, 1.0);
        assert!(write.is_some());
        assert!(!restart);
        // Another video: a restart.
        let (_, restart) = env_change(Some("VIDEO=/w/waves.mp4\nSPEED=1\n"), beach, 1.0);
        assert!(restart);
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
