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
        crate::sway_ipc::run_command(&cmd);
    }
}

/// [`apply`] for a process with no main loop (`swaypplet settings`): one
/// connection, one command, and sway's answer.
pub fn apply_blocking(w: &Wallpaper) -> Result<(), String> {
    let cmd = command(w).ok_or("path not sendable")?;
    let outcomes = crate::sway_ipc::connect()
        .and_then(|mut c| c.run_command(&cmd))
        .map_err(|e| format!("sway ipc: {e}"))?;
    outcomes
        .into_iter()
        .find_map(Result::err)
        .map_or(Ok(()), |e| Err(format!("sway: {e}")))
}

/// Replay the saved pick at panel startup.
pub fn apply_saved() {
    if let Some(w) = store::current().wallpaper {
        log::info!("wallpaper: replaying {}", w.path.display());
        apply(&w);
    }
}

/// What the config sway loaded says the wallpaper is: the `bg` under
/// `output *` (block form or one-liner), or failing that the first `bg` of
/// any output. `None` when the config sets none, or sway could not be asked.
///
/// Blocking (one IPC round trip); the pane calls it on a worker.
pub fn system_default() -> Option<Wallpaper> {
    match crate::sway_ipc::config_text() {
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
        assert!(is_image(Path::new("/x/a.png")));
        assert!(!is_image(Path::new("/x/a.txt")));
        assert!(!is_image(Path::new("/x/noext")));
    }
}
