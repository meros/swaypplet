//! The wallpaper's colours, for the tint input (docs/design-system.md §2.2).
//!
//! This module's whole job is up to three colours of the wallpaper: the
//! primary (what the image is about), the ground (what most of it is) and a
//! secondary (another colour it has). `theme::inputs` reads their hues and
//! hands them to the token generator as a `tokens::Palette` inside `Tint`,
//! and `tokens::tint` decides what each colour family does with them.
//! Nothing here knows what a colour is for, and nothing here takes a
//! lightness: the tokens own every tone.
//!
//! ## Who computes it
//!
//! Decoding a 4K JPEG costs 100 ms or so, and eight processes build the
//! stylesheet: the lock screen among them, where 100 ms is the gap between
//! the key press and the password field. So exactly one process samples: the
//! panel, off the main thread, whenever the wallpaper or the setting changes.
//! It writes `$XDG_CACHE_HOME/swaypplet/wallpaper-source` (one line: a
//! version, a key over the image's path, mtime and size, and the three
//! colours), and every process reads that line. A missing or foreign file is
//! not an error and never blocks: the tint is off until the sample lands.

use std::path::{Path, PathBuf};

use material_colors::color::Argb;
use material_colors::quantize::{Quantizer, QuantizerCelebi};
use material_colors::score::Score;

use crate::settings::store::{self, Tint};
use crate::tokens::{Oklch, Palette, Rgb, tint};

/// Bump when the sampling changes, so a cache written by the old rule is a
/// miss rather than a stale colour that looks right.
const VERSION: u32 = 3;

/// The wallpaper is decoded down to this before it is quantized. 128²
/// pixels is 16 k samples, which is more than Celebi needs to find the
/// dominant colours and small enough that the quantizer is not the cost:
/// the JPEG decode is.
const SAMPLE: i32 = 128;

/// Colours the quantizer reduces the image to, before `Score` ranks them.
const MAX_COLORS: usize = 64;

/// The OKLCH chroma below which a colour of the image is a grey and gives no
/// hue: about HCT chroma 5, which is where `Score` draws the same line.
const GREY: f64 = 0.03;

/// A hue's share of the image counts every colour within this many degrees
/// of it, as `Score` does, so a gradient is one hue and not a dozen.
const WINDOW: f64 = 15.0;

/// The ground must cover at least this share of the image. Less, and no hue
/// dominates (a grey photo with a coloured detail); the ground is then the
/// primary, as the single-hue tint was.
const GROUND_SHARE: f64 = 0.20;

/// The secondary must cover at least this share, so a speck is not a colour.
const SECONDARY_SHARE: f64 = 0.05;

/// The colours the tokens take from one image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub primary: Rgb,
    pub ground: Rgb,
    pub secondary: Option<Rgb>,
}

impl Sample {
    /// The hues, in whole OKLCH degrees.
    fn palette(self) -> Palette {
        Palette {
            primary: hue_of(self.primary),
            ground: hue_of(self.ground),
            secondary: self.secondary.map(hue_of),
        }
    }
}

/// What `Score` answers when nothing in the image is usable. Only compared
/// against, never used: a monochrome wallpaper has no hue to give, and then
/// the tint stays off rather than inventing one.
fn no_colour() -> Argb {
    Argb::new(255, 0x68, 0x9d, 0x6a)
}

// ── The colours ─────────────────────────────────────────────────────────

/// The wallpaper's colours to build a tint on: Material's quantizer over the
/// image, then Material's `Score` for the primary and the secondary (it drops
/// the near-greys, the colours with too little of the image behind them, and
/// the dark yellow-greens its `dislike` module calls out, and hands back
/// hues at least 15° apart), and the population for the ground. `None` for
/// an image with nothing usable in it.
///
/// Blocking, and it decodes an image. Call it off the main thread.
pub fn sample_image(path: &Path) -> Option<Sample> {
    let pixels = sample(path)?;
    let quantized = QuantizerCelebi::quantize(&pixels, MAX_COLORS);
    let ranked = Score::score(
        &quantized.color_to_count,
        Some(4),
        Some(no_colour()),
        Some(true),
    );
    let rgb = |c: &Argb| {
        Rgb(
            f64::from(c.red) / 255.0,
            f64::from(c.green) / 255.0,
            f64::from(c.blue) / 255.0,
        )
    };
    let ranked: Vec<Rgb> = ranked
        .iter()
        .filter(|c| **c != no_colour())
        .map(rgb)
        .collect();
    let counts: Vec<(Rgb, u32)> = quantized
        .color_to_count
        .iter()
        .map(|(c, n)| (rgb(c), *n))
        .collect();
    pick(&counts, &ranked)
}

/// The share of the image within [`WINDOW`] of `hue`, greys left out of the
/// hue but counted in the whole.
fn share(counts: &[(Rgb, u32)], hue: f64) -> f64 {
    let total: u32 = counts.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return 0.0;
    }
    let near: u32 = counts
        .iter()
        .filter(|(c, _)| {
            let o = Oklch::from(*c);
            o.1 >= GREY && tint::difference(o.2, hue).abs() <= WINDOW
        })
        .map(|(_, n)| n)
        .sum();
    f64::from(near) / f64::from(total)
}

/// The three colours out of the quantized image and `Score`'s ranking.
/// Pure, so the rules are tested on made-up images.
fn pick(counts: &[(Rgb, u32)], ranked: &[Rgb]) -> Option<Sample> {
    let primary = *ranked.first()?;
    let hue = |c: Rgb| Oklch::from(c).2;
    let secondary = ranked[1..].iter().copied().find(|c| {
        tint::difference(hue(primary), hue(*c)).abs() >= tint::SECONDARY_APART
            && share(counts, hue(*c)) >= SECONDARY_SHARE
    });
    // The ground: the chromatic colour with the most of the image around its
    // hue, if that is enough of the image to be a ground at all.
    let ground = counts
        .iter()
        .filter(|(c, _)| Oklch::from(*c).1 >= GREY)
        .map(|(c, n)| (*c, share(counts, hue(*c)), *n))
        .max_by(|a, b| a.1.total_cmp(&b.1).then(a.2.cmp(&b.2)))
        .filter(|(_, sh, _)| *sh >= GROUND_SHARE)
        .map_or(primary, |(c, _, _)| c);
    Some(Sample {
        primary,
        ground,
        secondary,
    })
}

/// The image, decoded small, as opaque pixels.
fn sample(path: &Path) -> Option<Vec<Argb>> {
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_file_at_scale(path, SAMPLE, SAMPLE, true)
        .map_err(|e| log::warn!("wallpaper: cannot read {}: {e}", path.display()))
        .ok()?;
    let channels = pixbuf.n_channels() as usize;
    let stride = pixbuf.rowstride() as usize;
    let (w, h) = (pixbuf.width() as usize, pixbuf.height() as usize);
    let bytes = pixbuf.read_pixel_bytes();
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = y * stride;
        for x in 0..w {
            let i = row + x * channels;
            let (r, g, b) = (*bytes.get(i)?, *bytes.get(i + 1)?, *bytes.get(i + 2)?);
            // A transparent pixel is not a colour of the image; a PNG
            // wallpaper with a hole in it should not vote grey.
            if channels == 4 && bytes.get(i + 3).is_some_and(|a| *a < 128) {
                continue;
            }
            pixels.push(Argb::new(255, r, g, b));
        }
    }
    Some(pixels)
}

/// The hue the tokens take from `source`, in whole OKLCH degrees.
fn hue_of(source: Rgb) -> u16 {
    (Oklch::from(source).2.round() as u16) % 360
}

// ── The cache ───────────────────────────────────────────────────────────

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("swaypplet"))
}

fn cache_path() -> Option<PathBuf> {
    Some(cache_dir()?.join("wallpaper-source"))
}

/// What the colours are a function of: the image, by path, mtime and size.
/// The setting is not in it: the colours are the same at either reach.
fn key(image: &Path) -> String {
    let meta = std::fs::metadata(image).ok();
    let mtime = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let len = meta.as_ref().map_or(0, std::fs::Metadata::len);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in [
        VERSION.to_string(),
        image.display().to_string(),
        mtime.to_string(),
        len.to_string(),
    ] {
        for b in part.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The cache's one line.
fn line(key: &str, s: Sample) -> String {
    format!(
        "swaypplet-wallpaper v{VERSION} key={key} primary={} ground={} secondary={}\n",
        s.primary.css(),
        s.ground.css(),
        s.secondary.map_or_else(|| "none".to_string(), |c| c.css()),
    )
}

/// A `#rrggbb` word.
fn colour(word: &str) -> Option<Rgb> {
    let hex = word.strip_prefix('#')?;
    let v = u32::from_str_radix(hex, 16)
        .ok()
        .filter(|_| hex.len() == 6)?;
    Some(Rgb::hex(v))
}

/// The key and the colours out of a cache line this build wrote.
fn parse(text: &str) -> Option<(String, Sample)> {
    let mut words = text.split_whitespace();
    (words.next()? == "swaypplet-wallpaper").then_some(())?;
    (words.next()? == format!("v{VERSION}")).then_some(())?;
    let key = words.next()?.strip_prefix("key=")?.to_string();
    let primary = colour(words.next()?.strip_prefix("primary=")?)?;
    let ground = colour(words.next()?.strip_prefix("ground=")?)?;
    let secondary = match words.next()?.strip_prefix("secondary=")? {
        "none" => None,
        word => Some(colour(word)?),
    };
    Some((
        key,
        Sample {
            primary,
            ground,
            secondary,
        },
    ))
}

fn read_cache() -> Option<(String, Sample)> {
    parse(&std::fs::read_to_string(cache_path()?).ok()?)
}

/// The wallpaper's hues, as the last sample left them. A bare file read: it
/// runs in every process that builds the stylesheet, on the main thread,
/// the lock screen among them, so it asks neither sway nor the image.
pub fn palette() -> Option<Palette> {
    read_cache().map(|(_, s)| s.palette())
}

/// Write the cache next to itself and rename over it, so a reader in another
/// process sees either the old file or the new one.
fn write_cache(text: &str) -> bool {
    let Some(path) = cache_path() else {
        return false;
    };
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        log::warn!("wallpaper: cannot make {}: {e}", dir.display());
        return false;
    }
    let tmp = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp, text) {
        log::warn!("wallpaper: cannot write {}: {e}", tmp.display());
        return false;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        log::warn!("wallpaper: cannot replace {}: {e}", path.display());
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}

// ── Following the settings ──────────────────────────────────────────────

/// The picked wallpaper, if the settings hold one.
///
/// Main thread only, and that is the whole reason this is not folded into
/// [`wallpaper_path`]: `store::current` reads a thread-local, so on a worker
/// it answers with a default-constructed `Settings`: no pick, no error, no
/// way to tell. Sampling used to resolve the wallpaper inside the worker and
/// so keyed every colour on the sway config's original `bg` line; the theme
/// then never followed a wallpaper change.
fn picked_wallpaper() -> Option<PathBuf> {
    store::current().wallpaper.map(|w| w.path)
}

/// `picked` when the caller found one, else the one the sway config set.
///
/// Blocking when there is no pick: it asks sway for its config. Call it off
/// the main thread, and read `picked` on the main thread (see above).
fn wallpaper_path(picked: Option<PathBuf>) -> Option<PathBuf> {
    picked.or_else(|| crate::settings::wallpaper::system_default().map(|w| w.path))
}

/// Sample the current wallpaper on a worker thread if the cache does not
/// already hold it, and reload this process's stylesheet when it moved.
/// With the tint off nothing is sampled; the cache stays for when it comes
/// back on.
fn refresh() {
    // Both reads happen here, on the thread the settings live on. The worker
    // gets plain data and can reach nothing that would answer it wrongly.
    if store::current().look().tint == Tint::Off {
        return;
    }
    let picked = picked_wallpaper();
    crate::spawn::spawn_work(
        move || {
            let Some(image) = wallpaper_path(picked) else {
                log::warn!("wallpaper: none to take a tint from");
                return false;
            };
            let key = key(&image);
            if read_cache().is_some_and(|(k, _)| k == key) {
                return false;
            }
            let Some(sample) = sample_image(&image) else {
                log::info!("wallpaper: no usable colour in {}", image.display());
                return false;
            };
            log::info!("wallpaper: {} -> {:?}", image.display(), sample.palette());
            write_cache(&line(&key, sample))
        },
        |changed| {
            if changed {
                crate::theme::reload();
            }
        },
    );
}

/// Keep the sampled colours in step with the wallpaper and the setting, for as
/// long as the process runs.
///
/// The panel calls this and nothing else does: it is the one process that
/// writes the cache, and two writers would take turns invalidating each
/// other's file on every wallpaper change. The others read the file on
/// `theme::watch`'s tick or at startup.
pub fn follow_settings() {
    // The palette file an older build derived; nothing reads it any more.
    if let Some(dir) = cache_dir() {
        let _ = std::fs::remove_file(dir.join("palette.css"));
    }
    refresh();
    let signature = || (picked_wallpaper(), store::current().look().tint);
    let last = std::rc::Rc::new(std::cell::RefCell::new(signature()));
    store::observe(move || {
        let now = signature();
        if *last.borrow() == now {
            return;
        }
        *last.borrow_mut() = now;
        refresh();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_line_reads_back() {
        for secondary in [None, Some(Rgb::hex(0xd65d0e))] {
            let sample = Sample {
                primary: Rgb::hex(0x3a6ea5),
                ground: Rgb::hex(0x203040),
                secondary,
            };
            let text = line(&key(Path::new("/tmp/a.png")), sample);
            assert_eq!(text.lines().count(), 1);
            let (k, back) = parse(&text).unwrap();
            assert_eq!(k, key(Path::new("/tmp/a.png")));
            assert_eq!(back, sample);
            // Another build's file is a miss, not a colour.
            assert!(parse(&text.replace(&format!("v{VERSION}"), "v2")).is_none());
        }
        assert!(parse("swaypplet-wallpaper v2 key=0 source=#3a6ea5").is_none());
    }

    fn at(hue: f64) -> Rgb {
        Rgb::from(Oklch(0.6, 0.12, hue))
    }

    /// A blue sky with a red boat: the accent is the boat, the ground the
    /// sky, and the sky is far enough from the boat to be the secondary too.
    #[test]
    fn the_ground_is_the_area_and_the_primary_the_ranking() {
        let (sky, boat) = (at(250.0), at(25.0));
        let counts = [(sky, 800), (boat, 60), (Rgb(0.5, 0.5, 0.5), 140)];
        let s = pick(&counts, &[boat, sky]).unwrap();
        assert_eq!(s.primary, boat);
        assert_eq!(s.ground, sky);
        assert_eq!(s.secondary, Some(sky));
    }

    #[test]
    fn a_second_colour_needs_distance_and_area() {
        let (a, near, far, speck) = (at(250.0), at(280.0), at(60.0), at(140.0));
        // 30° away: the same colour, no secondary.
        let s = pick(&[(a, 700), (near, 300)], &[a, near]).unwrap();
        assert_eq!(s.secondary, None);
        // Far enough but a speck: no secondary.
        let s = pick(&[(a, 980), (speck, 20)], &[a, speck]).unwrap();
        assert_eq!(s.secondary, None);
        // Far and big enough.
        let s = pick(&[(a, 700), (far, 300)], &[a, far]).unwrap();
        assert_eq!(s.secondary, Some(far));
    }

    /// A grey photo with one coloured detail has no ground of its own; the
    /// ground is then the primary, which is the single-hue tint.
    #[test]
    fn no_dominant_hue_leaves_the_ground_on_the_primary() {
        let detail = at(140.0);
        let counts = [(Rgb(0.4, 0.4, 0.4), 900), (detail, 100)];
        let s = pick(&counts, &[detail]).unwrap();
        assert_eq!(s.ground, detail);
        assert_eq!(s.palette(), Palette::single(hue_of(detail)));
        assert!(pick(&counts, &[]).is_none());
    }

    #[test]
    fn the_key_moves_with_the_image() {
        let (a, b) = (Path::new("/tmp/a.png"), Path::new("/tmp/b.png"));
        assert_ne!(key(a), key(b));
        assert_eq!(key(a), key(a));
    }

    #[test]
    fn the_hue_is_whole_degrees_on_the_circle() {
        // sRGB red is OKLCH hue 29.2; a hue just under 360 wraps to 0.
        assert_eq!(hue_of(Rgb::hex(0xff0000)), 29);
        assert!(
            (0..24).all(|i| hue_of(Rgb::from(Oklch(0.6, 0.1, f64::from(i) * 15.0 + 0.4))) < 360)
        );
        assert_eq!(hue_of(Rgb::from(Oklch(0.6, 0.1, 359.7))), 0);
    }

    /// The harness's way in: sample an image and print what the tint would
    /// take from it, without a panel.
    ///
    /// ```text
    /// SWPP_WALLPAPER_IMAGE=~/Pictures/wallpapers/x.jpg \
    ///   cargo test -- --ignored print_wallpaper_source --nocapture
    /// ```
    #[test]
    #[ignore = "harness tool: samples SWPP_WALLPAPER_IMAGE"]
    fn print_wallpaper_source() {
        let image =
            PathBuf::from(std::env::var_os("SWPP_WALLPAPER_IMAGE").expect("SWPP_WALLPAPER_IMAGE"));
        match sample_image(&image) {
            Some(s) => println!("{s:?} {:?} from {}", s.palette(), image.display()),
            None => println!("no usable colour in {}", image.display()),
        }
    }
}
