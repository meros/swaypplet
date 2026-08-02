//! Tray icon resolution — theme name first, ARGB32 pixmap fallback.
//!
//! Qt/Electron apps typically provide only one of the two sources, and the
//! spec says to prefer names over pixmaps when both exist.

use gtk4::gdk;
use system_tray::item::{IconPixmap, Status, StatusNotifierItem};

/// Waybar's tray icon-size.
pub const ICON_SIZE: i32 = 15;

pub fn apply(image: &gtk4::Image, sni: &StatusNotifierItem) {
    image.set_pixel_size(ICON_SIZE);

    // NeedsAttention swaps in the attention icon where the app provides one.
    let attention = sni.status == Status::NeedsAttention;
    let name = if attention && sni.attention_icon_name.is_some() {
        sni.attention_icon_name.as_deref()
    } else {
        sni.icon_name.as_deref()
    };
    let pixmaps = if attention && sni.attention_icon_pixmap.is_some() {
        sni.attention_icon_pixmap.as_deref()
    } else {
        sni.icon_pixmap.as_deref()
    };

    if let Some(name) = name.filter(|n| !n.is_empty()) {
        let path = std::path::Path::new(name);
        // Some Electron apps hand a filesystem path instead of a theme name.
        if path.is_absolute() {
            if path.exists() {
                image.set_from_file(Some(path));
                return;
            }
        } else if theme_has_icon(name, sni.icon_theme_path.as_deref()) {
            image.set_icon_name(Some(name));
            return;
        }
    }

    if let Some(px) = pixmaps.and_then(|p| pick_pixmap(p, ICON_SIZE)) {
        image.set_paintable(Some(&texture(px)));
        return;
    }

    image.set_icon_name(Some("image-missing"));
}

/// Theme lookup, honoring the item's private IconThemePath (Qt apps ship
/// their icons in one instead of installing hicolor entries).
fn theme_has_icon(name: &str, extra_path: Option<&str>) -> bool {
    let Some(display) = gdk::Display::default() else {
        return false;
    };
    let theme = gtk4::IconTheme::for_display(&display);
    if let Some(path) = extra_path.filter(|p| !p.is_empty()) {
        let path = std::path::Path::new(path);
        if !theme.search_path().iter().any(|p| p == path) {
            theme.add_search_path(path);
        }
    }
    theme.has_icon(name)
}

fn texture(px: &IconPixmap) -> gdk::MemoryTexture {
    let bytes = premultiply(px.pixels.clone());
    gdk::MemoryTexture::new(
        px.width,
        px.height,
        gdk::MemoryFormat::A8r8g8b8Premultiplied,
        &glib::Bytes::from_owned(bytes),
        px.width as usize * 4,
    )
}

/// SNI pixmaps are unpremultiplied ARGB32 in network byte order — bytes
/// [A,R,G,B] — which is already GDK's A8R8G8B8 byte layout; only the
/// alpha premultiply is missing.
fn premultiply(mut pixels: Vec<u8>) -> Vec<u8> {
    for px in pixels.chunks_exact_mut(4) {
        let a = u16::from(px[0]);
        for c in &mut px[1..] {
            *c = ((u16::from(*c) * a) / 255) as u8;
        }
    }
    pixels
}

/// Smallest candidate covering `target`, else the largest available (the
/// scaler downsizes better than it upsizes). Entries whose buffer doesn't
/// match their claimed dimensions are skipped.
fn pick_pixmap(pixmaps: &[IconPixmap], target: i32) -> Option<&IconPixmap> {
    let valid = pixmaps.iter().filter(|p| {
        p.width > 0 && p.height > 0 && p.pixels.len() == p.width as usize * p.height as usize * 4
    });
    valid
        .clone()
        .filter(|p| p.width.min(p.height) >= target)
        .min_by_key(|p| p.width.min(p.height))
        .or_else(|| valid.max_by_key(|p| p.width.min(p.height)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixmap(size: i32) -> IconPixmap {
        IconPixmap {
            width: size,
            height: size,
            pixels: vec![0; (size * size * 4) as usize],
        }
    }

    #[test]
    fn premultiply_scales_color_channels_by_alpha() {
        // Opaque white, half-transparent white, fully transparent white.
        let out = premultiply(vec![
            255, 255, 255, 255, //
            128, 255, 255, 255, //
            0, 255, 255, 255,
        ]);
        assert_eq!(out[0..4], [255, 255, 255, 255]);
        assert_eq!(out[4..8], [128, 128, 128, 128]);
        assert_eq!(out[8..12], [0, 0, 0, 0]);
    }

    #[test]
    fn picks_smallest_pixmap_covering_the_target() {
        let candidates = [pixmap(64), pixmap(16), pixmap(22)];
        assert_eq!(pick_pixmap(&candidates, 15).unwrap().width, 16);
    }

    #[test]
    fn falls_back_to_largest_when_all_are_too_small() {
        let candidates = [pixmap(8), pixmap(12)];
        assert_eq!(pick_pixmap(&candidates, 15).unwrap().width, 12);
    }

    #[test]
    fn skips_malformed_and_empty_entries() {
        let broken = IconPixmap {
            width: 32,
            height: 32,
            pixels: vec![0; 16], // buffer doesn't match the claimed size
        };
        assert_eq!(pick_pixmap(&[broken, pixmap(10)], 15).unwrap().width, 10);
        assert!(pick_pixmap(&[], 15).is_none());
    }
}
