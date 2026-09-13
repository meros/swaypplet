//! The palette, and where it comes from.
//!
//! `data/palette.css` ships one: gruvbox dark, warmed, the same values
//! nixos `users/modules/theme/colors.nix` hands sway and the terminals. This
//! module can write a second one from the wallpaper and hand that to
//! `theme::load_css` instead, which is what `look.tint` turns on.
//!
//! ## Tone is the contract
//!
//! The derived palette is not a new theme. It is the shipped one with its
//! hues rotated: every entry keeps its own **tone** (HCT tone is CIELAB L\*,
//! so tone is lightness) and keeps its chroma, and only the hue moves. That
//! is deliberate, because tone is what the stylesheet's contrast is made of
//! — the ratios in style.css's comments (7:1 for notification body text,
//! 5.8:1 for the smallest ink on a card, 4.8:1 for the URGENT chip) are
//! ratios between two tones, and `contrast::ratio_of_tones` returns the same
//! number whatever the hues are. Keep the tones and every measurement taken
//! against the shipped palette still holds; `the_derived_palette_keeps_the_
//! contrast_the_rules_were_measured_with` asserts it over the whole hue
//! circle rather than trusting this paragraph.
//!
//! ## Three classes of colour
//!
//! Rotating *everything* would turn @red green on a green wallpaper, which
//! is a lie the danger channel cannot afford. So each name is one of three:
//!
//! - **Decorative** (`accent`, the four other accents, the six notification
//!   hues): the whole fan rotates rigidly, by the angle that puts @accent on
//!   the wallpaper's hue. Rigid rather than per-colour, so the hues stay as
//!   far apart as gruvbox put them — the six notification hues exist to be
//!   told apart, and rotating each one *toward* the source would compress
//!   them into a band.
//! - **Semantic** (`red`, `danger`, `orange`, and the criticial card's warm
//!   ground): harmonized toward the source by at most [`SEMANTIC_CAP`]
//!   degrees, Material's `Blend.harmonize` rule. Red stays red.
//! - **Neutral** (the greys, the foreground, the borders): the source's hue
//!   at a chroma of at most [`NEUTRAL_CHROMA`], so the ground picks up a
//!   cast of the wallpaper without becoming a colour. `Tint::Accents`
//!   leaves this class alone; only `Tint::Full` moves it.
//!
//! One name that is in no class is a bug, not a colour that stays put:
//! `every_palette_name_has_a_class` fails the build on it.
//!
//! ## Who computes it
//!
//! Decoding a 4K JPEG costs 100 ms or so, and eight processes call
//! `theme::load_css` — the lock screen among them, where 100 ms is the gap
//! between the key press and the password field. So exactly one process
//! computes: the panel, off the main thread, whenever the wallpaper or the
//! setting changes. It writes `$XDG_CACHE_HOME/swaypplet/palette.css` with
//! the inputs hashed into its first line, and every process (the panel
//! included) reads that file and checks the hash. A miss is not an error and
//! never blocks: the reader takes the shipped palette and the next refresh
//! puts the derived one up.

use std::path::{Path, PathBuf};

use material_colors::color::Argb;
use material_colors::hct::Hct;
use material_colors::quantize::{Quantizer, QuantizerCelebi};
use material_colors::score::Score;

use crate::settings::store::{self, Tint};

/// The shipped palette. Also the fallback for every failure in here.
pub const SHIPPED: &str = include_str!("../data/palette.css");

/// Bump when the derivation changes, so a cache written by the old rule is a
/// miss rather than a stale palette that looks right.
const VERSION: u32 = 1;

/// The wallpaper is decoded down to this before it is quantized. 128²
/// pixels is 16 k samples, which is more than Celebi needs to find the
/// dominant colours and small enough that the quantizer is not the cost —
/// the JPEG decode is.
const SAMPLE: i32 = 128;

/// Colours the quantizer reduces the image to, before `Score` ranks them.
const MAX_COLORS: usize = 64;

/// The ceiling on a neutral's chroma. Gruvbox's own greys sit at 1–4 and its
/// cream foreground at about 15; 8 is a ground that is visibly warm or cool
/// with the wallpaper and still reads as grey beside an accent.
const NEUTRAL_CHROMA: f64 = 8.0;

/// Lines [`derive`] writes before the shipped block starts. Only the test
/// that walks the derived file's comments needs to know.
#[cfg(test)]
const DERIVED_HEADER_LINES: usize = 3;

/// How far a semantic colour may rotate toward the source. Material's blend
/// module uses 15 for the same job; 12 keeps gruvbox red (hue 28) out of
/// orange (hue 55) whatever the wallpaper is.
const SEMANTIC_CAP: f64 = 12.0;

// ── The classes ─────────────────────────────────────────────────────────

/// What happens to a name when the palette is derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// Rotates with the fan. See the module docs.
    Decorative,
    /// Harmonizes toward the source, at most [`SEMANTIC_CAP`] degrees.
    Semantic,
    /// Takes the source's hue at [`NEUTRAL_CHROMA`], and only under
    /// [`Tint::Full`].
    Neutral,
}

const DECORATIVE: [&str; 17] = [
    "accent",
    "aqua",
    "accent_tint",
    "accent_tint_hi",
    "accent_secondary",
    "yellow",
    "accent_tertiary",
    "blue",
    "accent_quaternary",
    "purple",
    "green",
    "accent_bright",
    "accent_bright_2",
    "accent_bright_3",
    "accent_bright_4",
    "accent_bright_5",
    "accent_bright_6",
];

const SEMANTIC: [&str; 7] = [
    "accent_quinary",
    "red",
    "danger",
    "danger_tint",
    "red_bright",
    "orange",
    "card_urgent",
];

const NEUTRAL: [&str; 16] = [
    "bg0_h",
    "bg0",
    "bg1",
    "bg2",
    "bg3",
    "fg",
    "fg_dim",
    "surface",
    "surface_raised",
    "surface_hover",
    "surface_track",
    "border_subtle",
    "border_strong",
    "text_faint",
    "card_dim",
    "glass_match",
];

fn class(name: &str) -> Option<Class> {
    if DECORATIVE.contains(&name) {
        Some(Class::Decorative)
    } else if SEMANTIC.contains(&name) {
        Some(Class::Semantic)
    } else if NEUTRAL.contains(&name) {
        Some(Class::Neutral)
    } else {
        None
    }
}

// ── Reading the shipped block ───────────────────────────────────────────

/// The byte range of the `#rrggbb` in an `@define-color` line, and the name
/// it belongs to. `None` for every other line, comments included — the
/// comments in palette.css talk about colours and must not be rewritten.
fn definition(line: &str) -> Option<(&str, std::ops::Range<usize>)> {
    let rest = line.trim_start().strip_prefix("@define-color")?;
    let offset = line.len() - rest.len();
    let mut words = rest.split_whitespace();
    let name = words.next()?;
    // The value, which is either `#rrggbb;` or `alpha(#rrggbb, 0.50);`. The
    // hash is the only one on the line before a `/*`, since a value cannot
    // hold two colours.
    let value_at = rest.find('#')?;
    let start = offset + value_at;
    let hex = line.get(start..start + 7)?;
    if !hex[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((name, start..start + 7))
}

/// Every name and colour the block defines, in file order.
fn entries(css: &str) -> Vec<(&str, Argb)> {
    css.lines()
        .filter_map(|line| {
            let (name, span) = definition(line)?;
            Some((name, parse_hex(&line[span])?))
        })
        .collect()
}

fn parse_hex(hex: &str) -> Option<Argb> {
    let v = u32::from_str_radix(hex.strip_prefix('#')?, 16).ok()?;
    Some(Argb::new(
        255,
        (v >> 16) as u8,
        ((v >> 8) & 0xff) as u8,
        (v & 0xff) as u8,
    ))
}

fn to_hex(c: Argb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue)
}

/// One named colour out of a palette block. `None` when the block does not
/// define it, which is what a hand-edited cache file looks like.
pub fn lookup(css: &str, name: &str) -> Option<Argb> {
    entries(css)
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| c)
}

// ── Deriving ────────────────────────────────────────────────────────────

/// The shorter way round from `from` to `to`, in degrees, signed.
fn difference(from: f64, to: f64) -> f64 {
    let d = (to - from).rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

/// `hue` rotated toward `toward` by at most `cap` degrees.
fn harmonize(hue: f64, toward: f64, cap: f64) -> f64 {
    let d = difference(hue, toward);
    (hue + d.clamp(-cap, cap)).rem_euclid(360.0)
}

/// The palette block for `source`, as a whole file: the shipped block with
/// every colour it defines rewritten in place. Rewritten rather than
/// generated, so the derived palette defines exactly the names the shipped
/// one does — including the comments that say what each is for — and a name
/// added to palette.css cannot be forgotten here.
pub fn derive(source: Argb, tint: Tint) -> String {
    if tint == Tint::Off {
        return SHIPPED.to_string();
    }
    let source = Hct::new(source);
    // The angle that puts @accent on the wallpaper's hue. Read from the
    // shipped block rather than written down, so re-theming the shipped
    // palette moves this with it.
    let anchor = lookup(SHIPPED, "accent").map_or(0.0, |c| Hct::new(c).get_hue());
    let rotation = difference(anchor, source.get_hue());

    let mut out = String::with_capacity(SHIPPED.len() + 512);
    // The comments come through unchanged, and some of them name gruvbox's
    // hue ("accentSecondary (yellow)"). After a rotation that word is the
    // colour's ancestry rather than its colour, and a reader of this file
    // deserves to be told which.
    out.push_str(&format!(
        "/* Derived from {} ({tint:?}) by src/palette.rs. Every value below is the\n   shipped palette's tone and chroma at a rotated hue, so a colour NAME in a\n   comment is the hue it was rotated FROM, not the hue it is. */\n",
        to_hex(Argb::from(source))
    ));
    for line in SHIPPED.lines() {
        match definition(line) {
            Some((name, span)) => {
                let old = parse_hex(&line[span.clone()]);
                match (old, class(name)) {
                    (Some(old), Some(class)) => {
                        let next = map(old, class, rotation, &source, tint);
                        out.push_str(&line[..span.start]);
                        out.push_str(&to_hex(next));
                        out.push_str(&line[span.end..]);
                    }
                    // A name with no class keeps its shipped value. The test
                    // says this cannot happen; if it does, a colour that
                    // stayed gruvbox is a better failure than one that
                    // turned into something nobody chose.
                    _ => out.push_str(line),
                }
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// One colour, rewritten. Tone survives every branch: see "Tone is the
/// contract".
fn map(old: Argb, class: Class, rotation: f64, source: &Hct, tint: Tint) -> Argb {
    let old = Hct::new(old);
    let (chroma, tone) = (old.get_chroma(), old.get_tone());
    let hue = match class {
        Class::Decorative => (old.get_hue() + rotation).rem_euclid(360.0),
        Class::Semantic => harmonize(old.get_hue(), source.get_hue(), SEMANTIC_CAP),
        Class::Neutral => {
            if tint != Tint::Full {
                return Argb::from(old);
            }
            source.get_hue()
        }
    };
    let chroma = match class {
        Class::Neutral => chroma.min(NEUTRAL_CHROMA),
        _ => chroma,
    };
    Argb::from(Hct::from(hue, chroma, tone))
}

// ── The source colour ───────────────────────────────────────────────────

/// The wallpaper's colour to build a theme on: Material's quantizer over the
/// image, then Material's `Score`, which drops the near-greys, the colours
/// with too little of the image behind them, and the dark yellow-greens its
/// `dislike` module calls out. An image with nothing usable in it falls back
/// to the shipped accent, which derives back to the shipped palette — a
/// monochrome wallpaper leaves the theme where it was rather than inventing
/// a hue for it.
///
/// Blocking, and it decodes an image. Call it off the main thread.
pub fn source_from_image(path: &Path) -> Option<Argb> {
    let pixels = sample(path)?;
    let quantized = QuantizerCelebi::quantize(&pixels, MAX_COLORS);
    let fallback = lookup(SHIPPED, "accent");
    let ranked = Score::score(&quantized.color_to_count, Some(4), fallback, Some(true));
    ranked.first().copied()
}

/// The image, decoded small, as opaque pixels.
fn sample(path: &Path) -> Option<Vec<Argb>> {
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_file_at_scale(path, SAMPLE, SAMPLE, true)
        .map_err(|e| log::warn!("palette: cannot read {}: {e}", path.display()))
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

// ── The cache ───────────────────────────────────────────────────────────

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("swaypplet"))
}

pub fn cache_path() -> Option<PathBuf> {
    Some(cache_dir()?.join("palette.css"))
}

/// What the derived palette is a function of. A hash rather than the parts,
/// because it goes in a CSS comment and a path with a `*/` in it would end
/// the comment early.
fn key(tint: Tint, image: &Path) -> String {
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
        format!("{tint:?}"),
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

/// The cache's first line. It carries the version so an old file is a miss,
/// and the key so the writer can tell a rewrite from a no-op; a *reader*
/// needs neither the wallpaper nor the settings to use the file, which is
/// the point — see [`current`].
fn marker(key: &str) -> String {
    format!("/* swaypplet palette v{VERSION} key={key} */")
}

fn marker_prefix() -> String {
    format!("/* swaypplet palette v{VERSION} key=")
}

/// The wallpaper the palette is derived from: the pick, else the one the
/// sway config set. `None` when neither exists, which is a host whose
/// compositor was never asked.
///
/// Blocking when there is no pick: it asks sway for its config.
pub fn wallpaper_path() -> Option<PathBuf> {
    store::current()
        .wallpaper
        .or_else(crate::settings::wallpaper::system_default)
        .map(|w| w.path)
}

/// The palette block to put in front of the rules: the cache when there is
/// one this build can read, the shipped palette otherwise.
///
/// Deliberately a bare file read. It runs in all eight processes, at CSS
/// load, on the main thread, and the lock screen is one of them — so it
/// consults neither the settings (which the caller may not have loaded yet)
/// nor sway (an IPC round trip) nor the wallpaper's mtime. The file's
/// presence *is* the setting: [`refresh`] writes it while the tint is on and
/// removes it when the tint goes off, so a reader has nothing to check.
pub fn current() -> String {
    read_cache().unwrap_or_else(|| SHIPPED.to_string())
}

/// The cache file's body, when its first line says this build wrote it.
fn read_cache() -> Option<String> {
    let text = std::fs::read_to_string(cache_path()?).ok()?;
    let (first, body) = text.split_once('\n')?;
    first
        .trim_start()
        .starts_with(&marker_prefix())
        .then(|| body.to_string())
}

/// The key in the cache file's first line, or `None` when there is no
/// readable cache. The writer compares this rather than re-deriving.
fn cached_key() -> Option<String> {
    let text = std::fs::read_to_string(cache_path()?).ok()?;
    let first = text.lines().next()?;
    let rest = first.trim_start().strip_prefix(&marker_prefix())?;
    Some(rest.split_whitespace().next()?.to_string())
}

/// Derive the palette from the current wallpaper on a worker thread, write
/// the cache, and call `done` on the main thread with whether the file
/// changed. `done(false)` means the cache was already right, or there was
/// nothing to derive.
///
/// The panel calls this; nothing else should, because two writers would take
/// turns invalidating each other's file on every wallpaper change.
pub fn refresh(done: impl FnOnce(bool) + 'static) {
    let tint = store::current().look().tint;
    crate::spawn::spawn_work(
        move || {
            if tint == Tint::Off {
                // The file's presence is the setting, so turning the tint
                // off removes it. Every process falls back to the shipped
                // palette on its next reload.
                return drop_cache();
            }
            let Some(image) = wallpaper_path() else {
                log::warn!("palette: no wallpaper to derive from");
                return false;
            };
            let key = key(tint, &image);
            if cached_key().as_deref() == Some(key.as_str()) {
                return false;
            }
            let Some(source) = source_from_image(&image) else {
                log::warn!("palette: no source colour in {}", image.display());
                return false;
            };
            log::info!(
                "palette: {} -> source {} ({tint:?})",
                image.display(),
                to_hex(source)
            );
            write_cache(&marker(&key), &derive(source, tint))
        },
        done,
    );
}

/// Remove the cache. True when there was one to remove, which is the caller's
/// cue that the palette moved.
fn drop_cache() -> bool {
    let Some(path) = cache_path() else {
        return false;
    };
    match std::fs::remove_file(&path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            log::warn!("palette: cannot remove {}: {e}", path.display());
            false
        }
    }
}

/// Write the cache next to itself and rename over it, so a reader in another
/// process sees either the old file or the new one.
fn write_cache(marker: &str, body: &str) -> bool {
    let Some(path) = cache_path() else {
        return false;
    };
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        log::warn!("palette: cannot make {}: {e}", dir.display());
        return false;
    }
    let tmp = path.with_extension("css.tmp");
    if let Err(e) = std::fs::write(&tmp, format!("{marker}\n{body}")) {
        log::warn!("palette: cannot write {}: {e}", tmp.display());
        return false;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        log::warn!("palette: cannot replace {}: {e}", path.display());
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}

// ── Following the settings ──────────────────────────────────────────────

/// What a palette is a function of, as far as this process can see without
/// asking sway: the picked wallpaper and the setting. A pick that is `None`
/// means the sway config's own `bg`, which does not change under us.
fn signature() -> (Option<PathBuf>, Tint) {
    let settings = store::current();
    let tint = settings.look().tint;
    (settings.wallpaper.map(|w| w.path), tint)
}

/// Keep the derived palette, this process's stylesheet and sway's borders in
/// step with the wallpaper and the setting, for as long as the process runs.
///
/// The panel calls this and nothing else does: it is the one process that
/// writes the cache, and two writers would take turns invalidating each
/// other's file on every wallpaper change. The others read the file — at
/// startup, or on `theme::watch`'s tick.
pub fn follow_settings() {
    apply_now();
    let last = std::rc::Rc::new(std::cell::RefCell::new(signature()));
    store::observe(move || {
        let now = signature();
        if *last.borrow() == now {
            return;
        }
        *last.borrow_mut() = now;
        apply_now();
    });
}

thread_local! {
    /// Called after every [`apply_now`], so the settings pane's swatches
    /// show what the wallpaper produced rather than what it produced last
    /// time. Panel-process only, like `follow_settings` itself.
    static WATCHERS: std::cell::RefCell<Vec<Box<dyn Fn()>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Run `cb` whenever the derived palette has been recomputed, changed or
/// not. The callback reads the result through [`current`].
pub fn observe(cb: impl Fn() + 'static) {
    WATCHERS.with(|w| w.borrow_mut().push(Box::new(cb)));
}

/// Derive if needed, then put the result on this process and on sway.
fn apply_now() {
    refresh(|changed| {
        if changed {
            crate::theme::reload();
        }
        // The borders are the compositor's, not this process's, so they
        // survive a panel restart and have to be pushed even when the cache
        // did not move — a fresh session has the sway config's gruvbox on
        // them. With no cache and no change there is nothing to say.
        if changed || read_cache().is_some() {
            apply_sway_borders(&current());
        }
        WATCHERS.with(|w| {
            for cb in w.borrow().iter() {
                cb();
            }
        });
    });
}

// ── sway's window borders ───────────────────────────────────────────────

/// The `client.*` lines that put a palette on sway's window borders, in the
/// same mapping nixos `users/modules/sway.nix` writes by hand: focused is
/// the accent with the ground as its text, inactive and unfocused are the
/// surface with muted text, urgent is the quinary accent.
///
/// Order is sway's: border, background, text, indicator, child_border.
pub fn sway_client_colors(css: &str) -> Vec<String> {
    let c = |name: &str| lookup(css, name).map(to_hex);
    let (Some(accent), Some(bg0), Some(bg1), Some(muted), Some(urgent)) = (
        c("accent"),
        c("bg0"),
        c("bg1"),
        c("fg_dim"),
        c("accent_quinary"),
    ) else {
        log::warn!("palette: a block with no accents in it, leaving sway's borders alone");
        return Vec::new();
    };
    vec![
        format!("client.focused {accent} {accent} {bg0} {accent} {accent}"),
        format!("client.focused_inactive {muted} {bg1} {muted} {muted} {muted}"),
        format!("client.unfocused {bg1} {bg1} {muted} {bg0} {bg1}"),
        format!("client.urgent {urgent} {urgent} {bg0} {urgent} {urgent}"),
    ]
}

/// Put the palette on sway's borders. The panel calls this after a refresh;
/// with `Tint::Off` the shipped palette goes back, which is what the sway
/// config itself set (cross-repo-guard.nix proves the two agree), so turning
/// the setting off restores the borders rather than leaving them where the
/// last wallpaper put them.
pub fn apply_sway_borders(css: &str) {
    for command in sway_client_colors(css) {
        crate::sway_ipc::run_command(&command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use material_colors::contrast::ratio_of_tones;

    /// Sources spread over the hue circle, plus the two degenerate ones: a
    /// grey wallpaper and a fully saturated one.
    fn sources() -> Vec<Argb> {
        let mut out: Vec<Argb> = (0..24)
            .map(|i| Argb::from(Hct::from(f64::from(i) * 15.0, 48.0, 55.0)))
            .collect();
        out.push(Argb::new(255, 128, 128, 128));
        out.push(Argb::new(255, 255, 0, 0));
        out
    }

    #[test]
    fn every_palette_name_has_a_class() {
        let shipped: Vec<&str> = entries(SHIPPED).into_iter().map(|(n, _)| n).collect();
        assert!(
            shipped.len() >= 40,
            "only {} colours parsed out of palette.css — the parser missed some",
            shipped.len()
        );
        for name in &shipped {
            assert!(
                class(name).is_some(),
                "palette.css defines @{name}, which src/palette.rs does not classify. \
                 Add it to DECORATIVE, SEMANTIC or NEUTRAL."
            );
        }
        for name in DECORATIVE.iter().chain(&SEMANTIC).chain(&NEUTRAL) {
            assert!(
                shipped.contains(name),
                "src/palette.rs classifies @{name}, which palette.css no longer defines"
            );
        }
    }

    #[test]
    fn the_derived_block_defines_what_the_shipped_one_does() {
        for source in sources() {
            for tint in [Tint::Accents, Tint::Full] {
                let derived = derive(source, tint);
                let want: Vec<&str> = entries(SHIPPED).into_iter().map(|(n, _)| n).collect();
                let got: Vec<&str> = entries(&derived).into_iter().map(|(n, _)| n).collect();
                assert_eq!(want, got, "{tint:?} from {}", to_hex(source));
                // The alphas are part of the rules' arithmetic and are not
                // the palette's to change.
                assert_eq!(
                    SHIPPED.matches("alpha(").count(),
                    derived.matches("alpha(").count()
                );
            }
        }
    }

    /// The claim the whole design rests on: a derived palette is the shipped
    /// one with the hues moved, so every contrast ratio style.css was tuned
    /// against survives. The pairs are the ones its comments name.
    #[test]
    fn the_derived_palette_keeps_the_contrast_the_rules_were_measured_with() {
        // (ink, ground, the floor the stylesheet claims)
        let pairs = [
            ("fg", "bg0", 7.0),
            ("fg", "bg1", 7.0),
            ("fg_dim", "bg0", 4.5),
            ("text_faint", "bg0", 2.5),
            ("accent_bright", "card_dim", 5.0),
            ("accent_bright_2", "card_dim", 5.0),
            ("accent_bright_3", "card_dim", 5.0),
            ("accent_bright_4", "card_dim", 5.0),
            ("accent_bright_5", "card_dim", 5.0),
            ("accent_bright_6", "card_dim", 5.0),
            ("bg0_h", "red_bright", 4.5),
            ("accent", "bg0", 3.0),
        ];
        let tone = |css: &str, name: &str| Hct::new(lookup(css, name).unwrap()).get_tone();
        for (ink, ground, floor) in pairs {
            let shipped = ratio_of_tones(tone(SHIPPED, ink), tone(SHIPPED, ground));
            assert!(
                shipped >= floor,
                "the SHIPPED palette already fails @{ink} on @{ground}: {shipped:.2} < {floor}"
            );
            // The floor a derived palette has to clear is the shipped one's
            // own measurement, not the rounder number the comment quotes.
            // 2 % of slack for the trip through 8-bit sRGB.
            let floor = floor.max(shipped * 0.98);
            for source in sources() {
                for tint in [Tint::Accents, Tint::Full] {
                    let css = derive(source, tint);
                    let got = ratio_of_tones(tone(&css, ink), tone(&css, ground));
                    assert!(
                        got >= floor,
                        "@{ink} on @{ground} is {got:.2} (floor {floor}) with {tint:?} \
                         from {}, where the shipped palette gives {shipped:.2}",
                        to_hex(source)
                    );
                }
            }
        }
    }

    /// The six notification hues are a channel: one sender, one colour. They
    /// have to stay told apart whatever the wallpaper is, which is what the
    /// rigid rotation buys and what harmonizing each one separately would
    /// have cost.
    #[test]
    fn the_six_notification_hues_stay_apart() {
        let names = [
            "accent_bright",
            "accent_bright_2",
            "accent_bright_3",
            "accent_bright_4",
            "accent_bright_5",
            "accent_bright_6",
        ];
        let shipped: Vec<f64> = names
            .iter()
            .map(|n| Hct::new(lookup(SHIPPED, n).unwrap()).get_hue())
            .collect();
        let closest = |hues: &[f64]| {
            let mut min: f64 = 360.0;
            for (i, a) in hues.iter().enumerate() {
                for b in &hues[i + 1..] {
                    min = min.min(difference(*a, *b).abs());
                }
            }
            min
        };
        let floor = closest(&shipped);
        for source in sources() {
            let css = derive(source, Tint::Full);
            let hues: Vec<f64> = names
                .iter()
                .map(|n| Hct::new(lookup(&css, n).unwrap()).get_hue())
                .collect();
            let got = closest(&hues);
            assert!(
                got >= floor - 1.0,
                "two sender hues are {got:.1}° apart from source {} — gruvbox's own \
                 closest pair is {floor:.1}°",
                to_hex(source)
            );
        }
    }

    /// Red is a word, not a decoration.
    #[test]
    fn danger_stays_red() {
        for source in sources() {
            for name in ["red", "danger", "accent_quinary", "red_bright"] {
                let css = derive(source, Tint::Full);
                let hue = Hct::new(lookup(&css, name).unwrap()).get_hue();
                let shipped = Hct::new(lookup(SHIPPED, name).unwrap()).get_hue();
                let moved = difference(shipped, hue).abs();
                assert!(
                    moved <= SEMANTIC_CAP + 1.0,
                    "@{name} moved {moved:.1}° for source {}",
                    to_hex(source)
                );
            }
        }
    }

    #[test]
    fn a_grey_wallpaper_leaves_the_accents_where_they_were() {
        // Not literally identical: a grey source has no usable hue, so
        // `source_from_image` falls back to the shipped accent and the
        // rotation is zero. Fed the shipped accent directly, the decorative
        // half has to come back unchanged.
        let accent = lookup(SHIPPED, "accent").unwrap();
        let css = derive(accent, Tint::Accents);
        for name in DECORATIVE {
            let before = lookup(SHIPPED, name).unwrap();
            let after = lookup(&css, name).unwrap();
            let moved = difference(Hct::new(before).get_hue(), Hct::new(after).get_hue()).abs();
            assert!(moved < 1.0, "@{name} moved {moved:.1}° for a zero rotation");
        }
        // And Accents leaves every neutral exactly alone.
        for name in NEUTRAL {
            assert_eq!(
                lookup(SHIPPED, name),
                lookup(&css, name),
                "@{name} changed under Tint::Accents"
            );
        }
    }

    #[test]
    fn off_is_the_shipped_file_byte_for_byte() {
        assert_eq!(derive(Argb::new(255, 255, 0, 0), Tint::Off), SHIPPED);
    }

    #[test]
    fn a_comment_that_mentions_a_colour_is_not_rewritten() {
        // palette.css's comments cite hex values on purpose (what a colour
        // was derived from, what it is measured against). Only the value of
        // a definition moves.
        let css = derive(Argb::new(255, 20, 90, 200), Tint::Full);
        // Past `derive`'s own header, which is the one comment in the file
        // that is not the shipped one's.
        assert_eq!(
            css.lines().nth(DERIVED_HEADER_LINES),
            SHIPPED.lines().next(),
            "DERIVED_HEADER_LINES is {DERIVED_HEADER_LINES}, but the shipped \
             block does not start there"
        );
        for line in css.lines().skip(DERIVED_HEADER_LINES) {
            assert!(!line.ends_with(' '), "trailing space: {line:?}");
            let trimmed = line.trim_start();
            if trimmed.starts_with("/*") || trimmed.starts_with('*') {
                assert!(
                    SHIPPED.contains(trimmed),
                    "a comment line was rewritten: {line}"
                );
            }
        }
    }

    #[test]
    fn the_border_commands_are_sways_five_fields() {
        let commands = sway_client_colors(SHIPPED);
        assert_eq!(commands.len(), 4);
        for command in &commands {
            let words: Vec<&str> = command.split_whitespace().collect();
            assert_eq!(words.len(), 6, "{command}");
            assert!(words[0].starts_with("client."));
            for hex in &words[1..] {
                assert!(hex.len() == 7 && hex.starts_with('#'), "{command}");
            }
        }
        // Focused is the accent, which is what sway.nix sets by hand.
        assert!(commands[0].contains(&to_hex(lookup(SHIPPED, "accent").unwrap())));
    }

    #[test]
    fn the_key_moves_with_every_input() {
        let a = Path::new("/tmp/a.png");
        let b = Path::new("/tmp/b.png");
        assert_ne!(key(Tint::Full, a), key(Tint::Accents, a));
        assert_ne!(key(Tint::Full, a), key(Tint::Full, b));
        assert_eq!(key(Tint::Full, a), key(Tint::Full, a));
        // The marker is one line, whatever the path was.
        assert_eq!(marker(&key(Tint::Full, a)).lines().count(), 1);
    }

    /// The harness's way in: derive a palette from an image and write it
    /// where a run can pick it up, without a panel and without touching the
    /// real cache.
    ///
    /// ```text
    /// SWPP_PALETTE_IMAGE=~/Pictures/wallpapers/x.jpg \
    /// SWPP_PALETTE_OUT=/tmp/p/swaypplet/palette.css SWPP_PALETTE_TINT=full \
    ///   cargo test -- --ignored write_derived_palette
    /// XDG_CACHE_HOME=/tmp/p dev/render.sh --mode panel
    /// ```
    #[test]
    #[ignore = "harness tool: writes a palette from SWPP_PALETTE_IMAGE"]
    fn write_derived_palette() {
        let image =
            PathBuf::from(std::env::var_os("SWPP_PALETTE_IMAGE").expect("SWPP_PALETTE_IMAGE"));
        let out = PathBuf::from(std::env::var_os("SWPP_PALETTE_OUT").expect("SWPP_PALETTE_OUT"));
        let tint = match std::env::var("SWPP_PALETTE_TINT").as_deref() {
            Ok("accents") => Tint::Accents,
            _ => Tint::Full,
        };
        let source = source_from_image(&image).expect("no source colour in the image");
        println!("source {} from {}", to_hex(source), image.display());
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        std::fs::write(
            &out,
            format!("{}\n{}", marker(&key(tint, &image)), derive(source, tint)),
        )
        .unwrap();
        println!("wrote {}", out.display());
    }

    #[test]
    fn the_hue_arithmetic_wraps() {
        assert!((difference(350.0, 10.0) - 20.0).abs() < 1e-9);
        assert!((difference(10.0, 350.0) + 20.0).abs() < 1e-9);
        assert!((harmonize(350.0, 10.0, 5.0) - 355.0).abs() < 1e-9);
        assert!((harmonize(0.0, 180.0, 12.0) - 12.0).abs() < 1e-9);
        // Already there: no movement, and no drift past the target.
        assert!((harmonize(100.0, 104.0, 12.0) - 104.0).abs() < 1e-9);
    }
}
