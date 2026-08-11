//! What happens to a capture once it exists: saved, copied, and shown.
//!
//! The old button spawned `grim` and forgot it. Nothing said the shot had been
//! taken, nothing showed what was in it, and getting it into both a file and
//! the clipboard meant taking it twice. All three are the same omission — the
//! capture had no follow-up — so the follow-up is one card.
//!
//! The card is a notification, not a bespoke surface. `NotificationStore`
//! already draws a picture, already lays out action buttons, already handles
//! dismissal and history, and the popup already treats a wide image as a
//! screenshot rather than an avatar (`notifications/popup.rs`). A second
//! surface doing the same job would be the notification stack with fewer
//! years on it.

use std::path::{Path, PathBuf};

use gtk4::gdk;
use gtk4::prelude::*;

use crate::notifications::store::{StoreRef, store_add};
use crate::notifications::{ImageSource, Notification, Urgency};

use super::capture::Image;

/// Where shots land. Matches what the panel button wrote before, so an
/// existing folder of screenshots keeps growing rather than being orphaned.
fn directory() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    Path::new(&home).join("Pictures/Screenshots")
}

/// `screenshot-20260811-140632.png` — sortable, and readable as a time.
///
/// Seconds resolution is enough for a person taking screenshots and wrong for
/// a script taking them in a loop, so a collision gets a counter rather than
/// silently overwriting.
fn unique_path(dir: &Path, stamp: &str) -> PathBuf {
    let first = dir.join(format!("screenshot-{stamp}.png"));
    if !first.exists() {
        return first;
    }
    for n in 2..1000 {
        let candidate = dir.join(format!("screenshot-{stamp}-{n}.png"));
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

fn stamp() -> String {
    // glib rather than a date crate: it is already linked, and the bar clock
    // formats its time the same way.
    glib::DateTime::now_local()
        .and_then(|t| t.format("%Y%m%d-%H%M%S"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// A capture as a GDK texture, which is both what the clipboard takes and
/// what encodes to PNG.
pub fn texture(image: &Image) -> gdk::MemoryTexture {
    gdk::MemoryTexture::new(
        image.width as i32,
        image.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from(&image.pixels),
        (image.width * 4) as usize,
    )
}

/// Write the capture to `~/Pictures/Screenshots`, returning where it went.
pub fn save(image: &Image) -> Result<PathBuf, String> {
    let dir = directory();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = unique_path(&dir, &stamp());
    texture(image)
        .save_to_png(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Put the capture on the clipboard as an image.
///
/// GDK's clipboard rather than the shell's own history ring: the ring is
/// text-only on purpose (`clipboard.rs`), and an image offer needs a live
/// source to serve `image/png` on demand, which GTK already is. The panel
/// outliving the shot is what makes that safe.
pub fn copy(image: &Image) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    display.clipboard().set_texture(&texture(image));
}

/// Save, copy, and post the card that says so.
///
/// Returns the card's id and where the file went, so the caller can tie the
/// card's buttons back to it.
///
/// One gesture producing both a file and a clipboard entry is the point: the
/// two are never in tension, and offering them as a choice would only make
/// the owner pick before knowing which they wanted.
pub fn finish(store: &StoreRef, image: &Image) -> (u32, Option<PathBuf>) {
    copy(image);

    let saved = match save(image) {
        Ok(path) => Some(path),
        Err(e) => {
            log::error!("screenshot: {e}");
            None
        }
    };

    let (body, actions, picture) = match &saved {
        Some(path) => (
            // The directory is the same every time; the file name is the part
            // worth reading, and it is the timestamp.
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            vec![
                ("annotate".to_string(), "Annotate".to_string()),
                ("open".to_string(), "Open".to_string()),
                ("delete".to_string(), "Delete".to_string()),
            ],
            Some(ImageSource::Path(path.clone())),
        ),
        // Copied but not saved: the clipboard still has it, and offering
        // Open or Delete on a file that does not exist would be a lie.
        None => (
            "Copied — could not write to disk".to_string(),
            Vec::new(),
            None,
        ),
    };

    let id = store_add(
        store,
        Notification {
            app_name: "Screenshot".into(),
            summary: format!("{} × {} copied", image.width, image.height),
            body,
            actions,
            image: picture,
            urgency: Urgency::Normal,
            // Long enough to act on, short enough not to sit there: the file
            // is on disk either way, so a missed card costs nothing.
            expire_timeout: 8000,
            // Actions leave the card up, so Annotate and Open do not make the
            // Delete button vanish underneath the pointer.
            resident: true,
            ..Default::default()
        },
    );

    (id, saved)
}

/// Read a saved shot back, for a second pass through the editor.
pub fn load_png(path: &Path) -> Result<Image, String> {
    let texture =
        gdk::Texture::from_filename(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let width = texture.width() as u32;
    let height = texture.height() as u32;

    // The downloader is what makes the format explicit; `Texture::download`
    // hands back cairo's premultiplied BGRA, which is not what the rest of
    // this module is written in.
    let mut downloader = gdk::TextureDownloader::new(&texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();

    // A PNG's rows can be padded; the rest of the module assumes they are not.
    let row = (width * 4) as usize;
    let mut pixels = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        let start = y * stride;
        pixels.extend_from_slice(&bytes[start..start + row]);
    }

    Ok(Image {
        width,
        height,
        pixels,
    })
}

/// Open a saved shot in whatever handles PNGs.
pub fn open(path: &Path) {
    let file = gtk4::gio::File::for_path(path);
    if let Err(e) = gtk4::gio::AppInfo::launch_default_for_uri(
        &file.uri(),
        None::<&gtk4::gio::AppLaunchContext>,
    ) {
        log::warn!("screenshot: could not open {}: {e}", path.display());
    }
}

/// Remove a saved shot. The card offers this because the moment you know a
/// screenshot was not worth keeping is the moment you look at it, which is now.
pub fn delete(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        log::warn!("screenshot: could not delete {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_shot_in_the_same_second_does_not_overwrite_the_first() {
        let dir = std::env::temp_dir().join(format!("swpp-shot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let first = unique_path(&dir, "20260811-140632");
        std::fs::write(&first, b"").unwrap();
        let second = unique_path(&dir, "20260811-140632");

        assert_ne!(first, second);
        assert!(second.to_string_lossy().ends_with("140632-2.png"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_first_shot_of_a_second_is_unsuffixed() {
        let dir = std::env::temp_dir().join(format!("swpp-shot-fresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = unique_path(&dir, "20260811-140632");
        assert!(
            path.to_string_lossy()
                .ends_with("screenshot-20260811-140632.png")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
