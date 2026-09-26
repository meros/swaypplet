//! The stylesheet fading from one set of inputs to the next (docs/design-system.md
//! §2.3), over the `page` motion token.
//!
//! A mode switch, a new tint or a new wallpaper used to replace every colour
//! in one frame. Reparsing the whole document on every frame of a fade would
//! cost a full parse per frame, so the fade is split in two:
//!
//! - The main provider gets the **new** document at once, so every rule and
//!   every token that is not a colour (a shadow, a length) is final from the
//!   first frame.
//! - A second provider, above it, holds only the colour tokens that changed,
//!   as a `:root` block of custom properties, and on each frame gets their
//!   value that far along. It starts at the old colours, so the first frame
//!   looks like the last one did, and is emptied when the fade ends, which
//!   leaves the new document on screen exactly as a reload without a fade
//!   would have.
//!
//! The lanes are read back out of the generated `tokens.css` rather than out
//! of the generator, so this module knows two value shapes and nothing about
//! what a token is for: `#rrggbb`, and `color-mix(in srgb, #rrggbb N%,
//! transparent)` (`emit.rs`'s `mixed`). A changed token of any other shape
//! (the light mode's `shadow-float`) takes its new value on the first frame.

use std::cell::RefCell;
use std::time::Instant;

use gdk4::Display;
use gtk4::CssProvider;

use crate::tokens::{Rgb, tint};

/// One colour token's value.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Value {
    Solid(Rgb),
    /// A colour at a share, 0..1: `color-mix(in srgb, c N%, transparent)`.
    Mixed(Rgb, f64),
}

/// One changing token: where it comes from and goes to, and both ends
/// verbatim so the endpoints are the generator's own bytes.
#[derive(Clone, Debug)]
struct Lane {
    name: String,
    from: Value,
    to: Value,
    from_raw: String,
    to_raw: String,
}

/// `--name: value;` pairs of a `tokens.css`, in order.
fn declarations(css: &str) -> impl Iterator<Item = (&str, &str)> {
    css.lines().filter_map(|line| {
        let (name, value) = line.trim().strip_prefix("--")?.split_once(':')?;
        Some((name.trim(), value.trim().strip_suffix(';')?.trim()))
    })
}

fn hex(word: &str) -> Option<Rgb> {
    let h = word.strip_prefix('#')?;
    (h.len() == 6).then_some(())?;
    Some(Rgb::hex(u32::from_str_radix(h, 16).ok()?))
}

fn value(raw: &str) -> Option<Value> {
    if let Some(c) = hex(raw) {
        return Some(Value::Solid(c));
    }
    let inner = raw
        .strip_prefix("color-mix(in srgb, ")?
        .strip_suffix(", transparent)")?;
    let (color, share) = inner.split_once(' ')?;
    let share: f64 = share.strip_suffix('%')?.parse().ok()?;
    Some(Value::Mixed(hex(color)?, share / 100.0))
}

/// The colour tokens that differ between two `tokens.css`, each with both
/// ends. `current` holds what a fade still running has on screen, so a new
/// fade starts from there rather than jumping back to the old target.
fn lanes(from: &str, to: &str, current: &[(String, Value)]) -> Vec<Lane> {
    let old: std::collections::HashMap<&str, &str> = declarations(from).collect();
    declarations(to)
        .filter_map(|(name, to_raw)| {
            let on_screen = current.iter().find(|(n, _)| n == name).map(|(_, v)| *v);
            let from_raw = *old.get(name)?;
            if from_raw == to_raw && on_screen.is_none() {
                return None;
            }
            let from = on_screen.or_else(|| value(from_raw))?;
            let to = value(to_raw)?;
            let from_raw = match on_screen {
                Some(v) => render(v),
                None => from_raw.to_string(),
            };
            Some(Lane {
                name: name.to_string(),
                from,
                to,
                from_raw,
                to_raw: to_raw.to_string(),
            })
        })
        .collect()
}

fn render(v: Value) -> String {
    match v {
        Value::Solid(c) => c.css(),
        Value::Mixed(c, share) => format!(
            "color-mix(in srgb, {} {:.1}%, transparent)",
            c.css(),
            share * 100.0
        ),
    }
}

/// A lane at eased progress `t`. A solid and a mixed end meet as a mix at a
/// share, a solid being a share of 1.
fn at(lane: &Lane, t: f64) -> Value {
    let parts = |v: Value| match v {
        Value::Solid(c) => (c, 1.0, false),
        Value::Mixed(c, s) => (c, s, true),
    };
    let ((a, sa, ma), (b, sb, mb)) = (parts(lane.from), parts(lane.to));
    let c = tint::blend(a, b, t);
    if ma || mb {
        Value::Mixed(c, sa + (sb - sa) * t)
    } else {
        Value::Solid(c)
    }
}

/// The overlay's text at eased progress `t`: the ends verbatim, anything
/// between rendered.
fn overlay(lanes: &[Lane], t: f64) -> String {
    let mut out = String::from(":root {\n");
    for lane in lanes {
        let v = if t <= 0.0 {
            lane.from_raw.clone()
        } else if t >= 1.0 {
            lane.to_raw.clone()
        } else {
            render(at(lane, t))
        };
        out.push_str("  --");
        out.push_str(&lane.name);
        out.push_str(": ");
        out.push_str(&v);
        out.push_str(";\n");
    }
    out.push_str("}\n");
    out
}

// ── The running fade ────────────────────────────────────────────────────

/// A step that took longer than this to reach the next frame is followed by
/// a frame with no step. A step restyles every widget in the process (a
/// custom property on `:root` invalidates them all), which is 3–6 ms with
/// the bar alone and about 60 ms with the panel's launcher list open; under
/// that load, every other frame stays free for input and for everything
/// else that moves, and the fade takes fewer, larger steps.
const SLOW: std::time::Duration = std::time::Duration::from_millis(25);

struct Run {
    lanes: Vec<Lane>,
    /// What the overlay shows now, for a fade that starts on top of this one.
    shown: Vec<(String, Value)>,
    driver: crate::anim::Frames,
    /// When the last step was taken, and whether the tick before this one
    /// was let go (see [`SLOW`]).
    last: Option<Instant>,
    rested: bool,
    frames: u32,
    parse_us: u128,
    worst_us: u128,
}

thread_local! {
    static OVERLAY: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    static RUN: RefCell<Option<Run>> = const { RefCell::new(None) };
}

fn overlay_provider() -> Option<CssProvider> {
    OVERLAY.with(|o| {
        if o.borrow().is_none() {
            let provider = CssProvider::new();
            gtk4::style_context_add_provider_for_display(
                &Display::default()?,
                &provider,
                // Above the stylesheet it overrides, and nothing else.
                gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
            );
            *o.borrow_mut() = Some(provider);
        }
        o.borrow().clone()
    })
}

/// Stop a running fade where it is: the overlay is emptied and the main
/// provider's document shows. Idempotent.
fn finish() {
    let Some(run) = RUN.with(|r| r.borrow_mut().take()) else {
        return;
    };
    run.driver.cancel();
    if let Some(p) = OVERLAY.with(|o| o.borrow().clone()) {
        p.load_from_string("");
    }
    if run.frames > 0 {
        log::info!(
            "theme: faded {} colour tokens over {} frames, overlay parse mean {:.2} ms, worst {:.2} ms",
            run.lanes.len(),
            run.frames,
            run.parse_us as f64 / f64::from(run.frames) / 1000.0,
            run.worst_us as f64 / 1000.0,
        );
    }
}

/// Show `from`'s colours now and fade them to `to`'s, which the main
/// provider already holds. Call it *before* loading `to` into the main
/// provider, so the frame that shows the new rules shows them at the old
/// colours. With motion off, or no window on screen whose frame clock could
/// drive it, nothing fades: the main provider's document shows as it is.
pub(super) fn start(from: &str, to: &str) {
    let current = RUN
        .with(|r| r.borrow().as_ref().map(|run| run.shown.clone()))
        .unwrap_or_default();
    finish();
    let ms = crate::anim::ms(crate::tokens::motion::PAGE);
    let lanes = lanes(from, to, &current);
    if lanes.is_empty() || ms <= 1.0 {
        return;
    }
    let Some(provider) = overlay_provider() else {
        return;
    };
    let Some(driver) = crate::anim::Frames::start(ms, |t| frame(crate::anim::standard(t)), finish)
    else {
        return;
    };
    provider.load_from_string(&overlay(&lanes, 0.0));
    let shown = lanes.iter().map(|l| (l.name.clone(), l.from)).collect();
    RUN.with(|r| {
        *r.borrow_mut() = Some(Run {
            lanes,
            shown,
            driver,
            last: None,
            rested: false,
            frames: 0,
            parse_us: 0,
            worst_us: 0,
        });
    });
}

fn frame(t: f64) {
    let Some(provider) = OVERLAY.with(|o| o.borrow().clone()) else {
        return;
    };
    RUN.with(|r| {
        let mut r = r.borrow_mut();
        let Some(run) = r.as_mut() else { return };
        let now = Instant::now();
        if !run.rested && run.last.is_some_and(|l| now - l > SLOW) {
            run.rested = true;
            run.last = Some(now);
            return;
        }
        run.rested = false;
        run.last = Some(now);
        let text = overlay(&run.lanes, t);
        let clock = Instant::now();
        provider.load_from_string(&text);
        let us = clock.elapsed().as_micros();
        run.frames += 1;
        run.parse_us += us;
        run.worst_us = run.worst_us.max(us);
        run.shown = run
            .lanes
            .iter()
            .map(|l| (l.name.clone(), at(l, t)))
            .collect();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::{Inputs, Mode, Oklch, Palette, Tint, css};

    fn light() -> Inputs {
        Inputs {
            mode: Mode::Light,
            ..Inputs::default()
        }
    }

    #[test]
    fn a_mode_switch_fades_every_colour_that_changes() {
        let (a, b) = (css(Inputs::default()), css(light()));
        let lanes = lanes(&a, &b, &[]);
        let names: Vec<&str> = lanes.iter().map(|l| l.name.as_str()).collect();
        for name in ["fg", "fg-muted", "accent-bg", "neutral-3", "cat-1", "knob"] {
            assert!(names.contains(&name), "{name} does not fade: {names:?}");
        }
        // Every changed declaration is either a lane or not a colour.
        let old: std::collections::HashMap<_, _> = declarations(&a).collect();
        for (name, raw) in declarations(&b) {
            if old.get(name) != Some(&raw) && !names.contains(&name) {
                assert!(
                    value(raw).is_none(),
                    "--{name}: {raw} changed but does not fade"
                );
            }
        }
        // Nothing that stays the same is sent.
        assert!(!names.contains(&"on-status"));
        assert!(super::lanes(&a, &a, &[]).is_empty());
    }

    #[test]
    fn the_ends_are_the_generators_own_bytes() {
        let tinted = Inputs {
            tint: Tint::Full(Palette::single(250)),
            ..light()
        };
        for (from, to) in [
            (Inputs::default(), light()),
            (light(), Inputs::default()),
            (Inputs::default(), tinted),
        ] {
            let (a, b) = (css(from), css(to));
            let set = lanes(&a, &b, &[]);
            let start: std::collections::HashMap<_, _> = declarations(&overlay(&set, 0.0))
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect();
            let end: std::collections::HashMap<_, _> = declarations(&overlay(&set, 1.0))
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect();
            let (old, new): (
                std::collections::HashMap<_, _>,
                std::collections::HashMap<_, _>,
            ) = (declarations(&a).collect(), declarations(&b).collect());
            for lane in &set {
                assert_eq!(start[&lane.name], old[lane.name.as_str()]);
                assert_eq!(end[&lane.name], new[lane.name.as_str()]);
            }
        }
    }

    /// Off, at the end of a fade, is the untinted token set byte for byte:
    /// the overlay is emptied and the main provider holds `css(Off)`, so
    /// the only thing to hold is that the overlay's last frame agrees with
    /// it wherever it says anything.
    #[test]
    fn a_fade_back_to_off_lands_on_the_shipped_tokens() {
        let tinted = Inputs {
            tint: Tint::Full(Palette::single(40)),
            ..Inputs::default()
        };
        let (a, b) = (css(tinted), css(Inputs::default()));
        let last = overlay(&lanes(&a, &b, &[]), 1.0);
        for (name, raw) in declarations(&last) {
            assert!(
                b.contains(&format!("  --{name}: {raw};")),
                "--{name}: {raw}"
            );
        }
    }

    #[test]
    fn lightness_runs_one_way_and_hue_the_short_way() {
        let (a, b) = (css(Inputs::default()), css(light()));
        for lane in lanes(&a, &b, &[]) {
            let l = |t: f64| match at(&lane, t) {
                Value::Solid(c) | Value::Mixed(c, _) => Oklch::from(c).0,
            };
            let rising = l(1.0) >= l(0.0);
            let mut prev = l(0.0);
            for i in 1..=20 {
                let now = l(f64::from(i) / 20.0);
                // Two hundredths of slack: the 8-bit ends and gamut
                // clipping of chroma move lightness by less than that.
                assert!(
                    if rising {
                        now >= prev - 0.02
                    } else {
                        now <= prev + 0.02
                    },
                    "--{}: {prev:.3} -> {now:.3}",
                    lane.name
                );
                prev = now;
            }
        }
        // 350° to 10° goes through 0°, not through 180°.
        let mid = Oklch::from(tint::blend(
            Rgb::from(Oklch(0.6, 0.1, 350.0)),
            Rgb::from(Oklch(0.6, 0.1, 10.0)),
            0.5,
        ));
        assert!(tint::difference(mid.2, 0.0).abs() < 3.0, "{mid:?}");
    }

    #[test]
    fn a_fade_on_top_of_a_fade_starts_where_the_screen_is() {
        let (a, b) = (css(Inputs::default()), css(light()));
        let first = lanes(&a, &b, &[]);
        let fg = first.iter().find(|l| l.name == "fg").unwrap();
        let mid = at(fg, 0.5);
        // Back to dark, half way: fg starts from the half-way colour even
        // though its old target and its new one are the same bytes.
        let second = lanes(&b, &a, &[("fg".to_string(), mid)]);
        let fg2 = second.iter().find(|l| l.name == "fg").unwrap();
        assert_eq!(fg2.from, mid);
        let back = lanes(&a, &a, &[("fg".to_string(), mid)]);
        assert_eq!(back.len(), 1);
    }
}
