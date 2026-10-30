//! Where files live: the XDG base directories, the home directory and the
//! runtime directory, read from the environment in one place.
//!
//! Each function has one policy. A variable that is set but empty counts as
//! unset, as the XDG spec asks. A directory under the home falls back to
//! `$HOME/<default>`, and is `None` when there is no home either, so the
//! caller decides what a process without one does (most never meet it: a
//! sway session always has both).
//!
//! The directories are the bases, without `swaypplet` under them, because
//! not every caller's file is under that name (`swaypplet-autofix`, the
//! elephant socket, mpvpaper's socket).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// `$HOME`.
pub fn home() -> Option<PathBuf> {
    home_in(&env)
}

/// `$XDG_CONFIG_HOME`, else `~/.config`.
pub fn config_dir() -> Option<PathBuf> {
    xdg_in(&env, "XDG_CONFIG_HOME", ".config")
}

/// `$XDG_STATE_HOME`, else `~/.local/state`.
pub fn state_dir() -> Option<PathBuf> {
    xdg_in(&env, "XDG_STATE_HOME", ".local/state")
}

/// `$XDG_CACHE_HOME`, else `~/.cache`.
pub fn cache_dir() -> Option<PathBuf> {
    xdg_in(&env, "XDG_CACHE_HOME", ".cache")
}

/// `$XDG_RUNTIME_DIR`. No fallback: a directory that stands in for it has
/// none of its guarantees (mode 0700, emptied at logout), so each caller
/// says what it uses instead, if anything.
pub fn runtime_dir() -> Option<PathBuf> {
    var(&env, "XDG_RUNTIME_DIR").map(PathBuf::from)
}

/// `~/rest` with the home directory put in; any other path as it is, and
/// `~/rest` too when there is no home to put in.
pub fn expand_home(path: &str) -> PathBuf {
    expand_home_in(&env, path)
}

/// A file in the runtime directory (or `/tmp` without one) named for the
/// compositor this process runs under: `<name>-<display>.<ext>`, where
/// `<display>` is the last component of `$WAYLAND_DISPLAY`, or `unset`
/// without one. Per compositor, because the runtime directory is shared
/// across them: a nested session (the render harness) or the greeter must
/// not take the live panel's file. The last component, because
/// `WAYLAND_DISPLAY` may be an absolute path, and its slashes would put the
/// file in a directory that does not exist.
pub fn per_display_file(name: &str, ext: &str, unset: &str) -> PathBuf {
    per_display_file_in(&env, name, ext, unset)
}

fn env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// The variable, when set and not empty.
fn var(lookup: &dyn Fn(&str) -> Option<OsString>, name: &str) -> Option<OsString> {
    lookup(name).filter(|v| !v.is_empty())
}

fn home_in(lookup: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    var(lookup, "HOME").map(PathBuf::from)
}

fn xdg_in(
    lookup: &dyn Fn(&str) -> Option<OsString>,
    name: &str,
    under_home: &str,
) -> Option<PathBuf> {
    var(lookup, name)
        .map(PathBuf::from)
        .or_else(|| Some(home_in(lookup)?.join(under_home)))
}

fn expand_home_in(lookup: &dyn Fn(&str) -> Option<OsString>, path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = home_in(lookup)
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

fn per_display_file_in(
    lookup: &dyn Fn(&str) -> Option<OsString>,
    name: &str,
    ext: &str,
    unset: &str,
) -> PathBuf {
    let dir = var(lookup, "XDG_RUNTIME_DIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    let display = var(lookup, "WAYLAND_DISPLAY")
        .and_then(|d| {
            Path::new(&d)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| unset.to_string());
    dir.join(format!("{name}-{display}.{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn an_xdg_variable_wins_over_the_home() {
        let lookup = with(&[("HOME", "/home/u"), ("XDG_STATE_HOME", "/s")]);
        assert_eq!(
            xdg_in(&lookup, "XDG_STATE_HOME", ".local/state"),
            Some(PathBuf::from("/s"))
        );
    }

    #[test]
    fn an_unset_or_empty_xdg_variable_falls_back_to_the_home() {
        for lookup in [
            with(&[("HOME", "/home/u")]),
            with(&[("HOME", "/home/u"), ("XDG_CONFIG_HOME", "")]),
        ] {
            assert_eq!(
                xdg_in(&lookup, "XDG_CONFIG_HOME", ".config"),
                Some(PathBuf::from("/home/u/.config"))
            );
        }
    }

    #[test]
    fn no_home_and_no_variable_is_no_directory() {
        assert_eq!(xdg_in(&with(&[]), "XDG_CACHE_HOME", ".cache"), None);
        assert_eq!(
            xdg_in(&with(&[("HOME", "")]), "XDG_CACHE_HOME", ".cache"),
            None
        );
        assert_eq!(home_in(&with(&[("HOME", "")])), None);
    }

    #[test]
    fn a_tilde_path_gets_the_home() {
        let lookup = with(&[("HOME", "/home/u")]);
        assert_eq!(
            expand_home_in(&lookup, "~/Pictures/a.jpg"),
            PathBuf::from("/home/u/Pictures/a.jpg")
        );
        assert_eq!(
            expand_home_in(&lookup, "/abs/a.jpg"),
            PathBuf::from("/abs/a.jpg")
        );
        assert_eq!(expand_home_in(&lookup, "~user/a"), PathBuf::from("~user/a"));
        assert_eq!(expand_home_in(&with(&[]), "~/a"), PathBuf::from("~/a"));
    }

    #[test]
    fn a_per_display_file_is_named_by_the_display_leaf() {
        let plain = with(&[
            ("XDG_RUNTIME_DIR", "/run/user/1"),
            ("WAYLAND_DISPLAY", "wayland-1"),
        ]);
        assert_eq!(
            per_display_file_in(&plain, "swaypplet/theme", "json", "wayland-0"),
            PathBuf::from("/run/user/1/swaypplet/theme-wayland-1.json")
        );
        // An absolute display names a socket elsewhere; only its leaf is
        // the name, so the file stays in the runtime directory.
        let absolute = with(&[
            ("XDG_RUNTIME_DIR", "/run/user/1"),
            ("WAYLAND_DISPLAY", "/tmp/nested/wayland-9"),
        ]);
        assert_eq!(
            per_display_file_in(&absolute, "swaypplet-dmenu", "sock", "default"),
            PathBuf::from("/run/user/1/swaypplet-dmenu-wayland-9.sock")
        );
        assert_eq!(
            per_display_file_in(&with(&[]), "swaypplet-dmenu", "sock", "default"),
            PathBuf::from("/tmp/swaypplet-dmenu-default.sock")
        );
    }
}
