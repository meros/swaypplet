//! File writes other processes may read while they happen.

use std::io;
use std::path::{Path, PathBuf};

/// Write `contents` to `path` whole: into a file next to it, then renamed
/// over it, so a reader (another process, or this one after a crash
/// mid-write) sees the old file or the new one and never half of either.
/// The parent directory is created when missing. On failure the temporary
/// file is removed, so a failed write leaves nothing behind.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = temp_path(path);
    let written = std::fs::write(&tmp, contents).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// `<path>.tmp`: the whole name kept, so two files that differ only in
/// their extension never share a temporary.
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("swaypplet-base-fs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_write_creates_the_directory_and_replaces_the_file() {
        let dir = scratch("replace");
        let path = dir.join("a/b/settings.json");
        write_atomic(&path, "one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        // Nothing left beside it.
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["settings.json"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_rename_leaves_no_temporary() {
        let dir = scratch("fail");
        // A directory where the file should go: the rename fails.
        let path = dir.join("taken");
        std::fs::create_dir_all(path.join("inside")).unwrap();
        assert!(write_atomic(&path, "x").is_err());
        assert!(!temp_path(&path).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_temporary_keeps_the_whole_name() {
        assert_eq!(
            temp_path(Path::new("/r/theme-wayland-1.json")),
            PathBuf::from("/r/theme-wayland-1.json.tmp")
        );
    }
}
