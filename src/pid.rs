//! The panel's pid file. The panel writes it (`app`); the standalone bar's
//! start button (`bar::start`) signals the pid in it.

/// Pid file lives in the per-user runtime dir (mode 0700), so no other user
/// can forge or clobber it. The /tmp fallback matches the wrapper scripts'
/// `$XDG_RUNTIME_DIR/swaypplet.pid` with the same /tmp fallback.
pub fn file_path() -> std::path::PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    std::path::Path::new(&dir).join("swaypplet.pid")
}
