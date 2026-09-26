//! The glass material following the theme's colour fade (docs/design-system.md
//! §4), and the one thread every material push goes through.
//!
//! A mode switch moves six material values (`tokens::material`). Sent in one
//! command, the glass jumps while the stylesheet beside it fades over `page`;
//! so [`send`] walks from what the compositor has to the new material over
//! the same duration and curve as `theme::fade`, [`STEP`] apart, with the
//! exact end value last.
//!
//! The walk runs on its own thread ([`worker`]) against the wall clock, not
//! on GTK's frame clock. A frame that restyles a panel full of rows takes
//! tens of milliseconds, and the glass is the compositor's: it has no reason
//! to stutter with it, or to cost the main thread anything. The thread is
//! also the one place a push is sent from, so pushes run in order, a newer
//! one replaces a waiting one, and the last value sent is the last value
//! set. (An idle-priority completion on the main loop, the first design,
//! starved for the whole fade while the frames kept the loop busy.)

use std::cell::RefCell;
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use super::glass::{System, Tuning};
use crate::tokens::{Rgb, tint};

/// Between pushes: 20 a second. Each push re-arranges every glass
/// namespace's layer surfaces in sway, and the material moves slowly.
const STEP: Duration = Duration::from_millis(50);

/// A fade, as the thread plays it and the main thread remembers it.
#[derive(Clone)]
struct Walk {
    system: System,
    from: Tuning,
    to: Tuning,
    start: Instant,
    span: Duration,
}

impl Walk {
    fn progress(&self, now: Instant) -> f64 {
        now.saturating_duration_since(self.start).as_secs_f64() / self.span.as_secs_f64()
    }

    /// Where the glass is at `now`.
    fn at(&self, now: Instant) -> Tuning {
        let t = self.progress(now);
        if t >= 1.0 {
            self.to.clone()
        } else {
            between(&self.from, &self.to, crate::anim::standard(t))
        }
    }
}

enum Msg {
    Now(Box<(System, Tuning)>),
    Fade(Box<Walk>),
}

thread_local! {
    /// What the compositor has, or is walking to: the start of the next fade.
    static LAST: RefCell<Option<Tuning>> = const { RefCell::new(None) };
    static WALK: RefCell<Option<Walk>> = const { RefCell::new(None) };
}

/// The one thread that talks to sway about the material.
fn worker() -> &'static Sender<Msg> {
    static TX: OnceLock<Sender<Msg>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = channel::<Msg>();
        if let Err(e) = std::thread::Builder::new()
            .name("glass-push".into())
            .spawn(move || serve(&rx))
        {
            log::warn!("glass: no push thread: {e}");
        }
        tx
    })
}

fn run(system: &System, tuning: &Tuning) {
    let cmd = system.command(tuning);
    if cmd.is_empty() {
        return;
    }
    if let Err(e) = crate::sway::ipc::run_command_blocking(&cmd) {
        log::warn!("{e}");
    }
}

/// The thread's loop: the newest message wins, a fade steps until it ends or
/// a newer message replaces it.
fn serve(rx: &Receiver<Msg>) {
    let mut next = rx.recv().ok();
    while let Some(msg) = next.take() {
        let mut msg = msg;
        while let Ok(newer) = rx.try_recv() {
            msg = newer;
        }
        match msg {
            Msg::Now(push) => run(&push.0, &push.1),
            Msg::Fade(walk) => loop {
                let now = Instant::now();
                run(&walk.system, &walk.at(now));
                if walk.progress(now) >= 1.0 {
                    break;
                }
                match rx.recv_timeout(STEP) {
                    Ok(newer) => {
                        next = Some(newer);
                        break;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            },
        }
        if next.is_none() {
            next = rx.recv().ok();
        }
    }
}

/// The compositor already has `tuning` (the sway config put it there), so a
/// later fade can start from it without anything being sent.
pub fn assume(tuning: Tuning) {
    LAST.with(|l| *l.borrow_mut() = Some(tuning));
}

/// Push `tuning` now, stopping any fade: a live edit in the Glass pane, a
/// reset, the startup replay.
pub fn direct(system: &System, tuning: &Tuning) {
    WALK.with(|w| w.borrow_mut().take());
    LAST.with(|l| *l.borrow_mut() = Some(tuning.clone()));
    let _ = worker().send(Msg::Now(Box::new((system.clone(), tuning.clone()))));
}

/// Fade from where the glass is to `to` over `page`, or push it at once
/// when there is nothing to fade from or motion is off.
pub fn send(system: &System, to: Tuning) {
    let now = Instant::now();
    // A fade still running starts the next one from where it has got to.
    let from = WALK
        .with(|w| w.borrow_mut().take())
        .map(|w| w.at(now))
        .or_else(|| LAST.with(|l| l.borrow().clone()));
    let ms = crate::anim::ms(crate::tokens::motion::PAGE);
    LAST.with(|l| *l.borrow_mut() = Some(to.clone()));
    match from.filter(|f| *f != to && ms > 1.0) {
        Some(from) => {
            let walk = Walk {
                system: system.clone(),
                from,
                to,
                start: now,
                span: Duration::from_secs_f64(ms / 1000.0),
            };
            WALK.with(|w| *w.borrow_mut() = Some(walk.clone()));
            let _ = worker().send(Msg::Fade(Box::new(walk)));
        }
        None => {
            let _ = worker().send(Msg::Now(Box::new((system.clone(), to))));
        }
    }
}

/// The fill a material shows, sentinels resolved: `none` and a negative
/// alpha mean the card's own paint, and every glass card paints the fill
/// key (`tokens::SURFACE_KEY`), so that is what "its own" is.
fn fill(m: &super::glass::Material) -> (Rgb, f64) {
    let key = crate::tokens::SURFACE_KEY;
    let colour = m.fill_rgb().map_or(key.0, |(r, g, b)| Rgb(r, g, b));
    let alpha = if m.fill_alpha < 0.0 {
        key.1
    } else {
        m.fill_alpha
    };
    (colour, alpha)
}

/// `photochromic` `t` of the way from `a` to `b`, along what it does rather
/// than along the number. Positive is a ceiling whose strength is `1/p` (a
/// small `p` is the *strongest* cap, so a straight line from 0.35 to −0.43
/// darkened the glass half way through a switch to light); zero is off;
/// negative is a lift of `-p`. A ceiling fades out by its strength, a lift
/// grows from nothing, and a switch from one to the other does the first
/// half, then the second.
fn photochromic(a: f64, b: f64, t: f64) -> f64 {
    let ceiling = |p: f64| if p > 0.0 { 1.0 / p } else { 0.0 };
    let lift = |p: f64| if p < 0.0 { -p } else { 0.0 };
    let from_ceiling = |s: f64| if s > 0.0 { 1.0 / s } else { 0.0 };
    let mix = |x: f64, y: f64, t: f64| x + (y - x) * t;
    match (a > 0.0, b > 0.0, a < 0.0, b < 0.0) {
        // Ceiling to ceiling (or to or from off): by strength.
        (_, _, false, false) => from_ceiling(mix(ceiling(a), ceiling(b), t)),
        // Lift to lift (or to or from off): straight.
        (false, false, _, _) => -mix(lift(a), lift(b), t),
        // Across: out, then in.
        _ if t < 0.5 => {
            let s = 2.0 * t;
            if a > 0.0 {
                from_ceiling(mix(ceiling(a), 0.0, s))
            } else {
                -mix(lift(a), 0.0, s)
            }
        }
        _ => {
            let s = 2.0 * t - 1.0;
            if b > 0.0 {
                from_ceiling(mix(0.0, ceiling(b), s))
            } else {
                -mix(0.0, lift(b), s)
            }
        }
    }
}

/// `t` of the way from `a` to `b`: the ends exactly, and between them every
/// number straight except `photochromic` (see there), the fill through
/// OKLCH (`tint::blend`) with its sentinels resolved, and what is a word (a
/// surface or grain kind) at `b`'s from the first frame.
fn between(a: &Tuning, b: &Tuning, t: f64) -> Tuning {
    if t <= 0.0 {
        return a.clone();
    }
    if t >= 1.0 {
        return b.clone();
    }
    let n = |x: f64, y: f64| x + (y - x) * t;
    let mut out = b.clone();
    let (m, ma, mb) = (&mut out.material, &a.material, &b.material);
    macro_rules! numbers {
        ($($f:ident),*) => { $( m.$f = n(ma.$f, mb.$f); )* };
    }
    numbers!(
        roughness,
        refraction,
        dispersion,
        samples,
        reflection,
        lensing,
        frost_radius,
        absorb,
        absorb_floor,
        haze,
        specular,
        edge_light,
        noise,
        frost,
        shine,
        reflect_blur,
        grain_scale,
        grain_strength,
        grain_angle,
        grain_aspect,
        energy_comp,
        iridescence,
        edge_glow,
        wave_amplitude
    );
    m.photochromic = photochromic(ma.photochromic, mb.photochromic, t);
    let ((ca, aa), (cb, ab)) = (fill(ma), fill(mb));
    let c = tint::blend(ca, cb, t);
    m.set_fill_rgb(Some((c.0, c.1, c.2)));
    m.fill_alpha = n(aa, ab);
    out.bezel_scale = n(a.bezel_scale, b.bezel_scale);
    out.crest_scale = n(a.crest_scale, b.crest_scale);
    out
}

#[cfg(test)]
mod tests {
    use super::super::glass::Material;
    use super::*;

    fn tuning(fill: &str, alpha: f64, frost: f64) -> Tuning {
        let mut m: Material = super::super::preset::ALL[0].material();
        m.fill_color = fill.to_string();
        m.fill_alpha = alpha;
        m.frost = frost;
        Tuning {
            material: m,
            bezel_scale: 1.0,
            thickness_ratio: 0.0,
            crest_scale: 1.0,
        }
    }

    #[test]
    fn the_ends_are_exact_and_the_middle_is_between() {
        let (a, b) = (tuning("#32302f", 0.50, 0.33), tuning("#ebe9e4", 0.65, 0.45));
        assert_eq!(between(&a, &b, 1.0), b);
        let start = between(&a, &b, 0.0);
        assert_eq!(start.material.fill_alpha, a.material.fill_alpha);
        assert_eq!(start.material.fill_color, a.material.fill_color);
        let mid = between(&a, &b, 0.5);
        assert!((mid.material.fill_alpha - 0.575).abs() < 1e-9);
        assert!((mid.material.frost - 0.39).abs() < 1e-9);
        let l = |t: &Tuning| {
            let (r, g, b) = t.material.fill_rgb().unwrap();
            crate::tokens::Oklch::from(Rgb(r, g, b)).0
        };
        assert!(l(&a) < l(&mid) && l(&mid) < l(&b));
    }

    /// The shipped dark material leaves the fill to the card; a fade from it
    /// starts from the card's own paint, the key, not from a jump.
    #[test]
    fn an_unset_fill_fades_from_the_key() {
        let (a, b) = (tuning("none", -1.0, 0.3), tuning("#ebe9e4", 0.5, 0.3));
        let early = between(&a, &b, 0.05);
        let (r, g, bl) = early.material.fill_rgb().unwrap();
        let key = crate::tokens::SURFACE_KEY;
        assert!((Rgb(r, g, bl).css() == key.0.css()) || (r - key.0.0).abs() < 0.05);
        assert!((early.material.fill_alpha - 0.5).abs() < 1e-9);
        assert_eq!(between(&a, &b, 0.0), a);
        assert_eq!(between(&a, &b, 1.0), b);
    }

    #[test]
    fn a_ceiling_fades_out_before_a_lift_fades_in() {
        // Never a small positive number: that is the strongest ceiling.
        for i in 0..=100 {
            let t = f64::from(i) / 100.0;
            let p = photochromic(0.35, -0.43, t);
            assert!(p <= 0.0 || p >= 0.35 - 1e-9, "t {t}: {p}");
        }
        assert!((photochromic(0.35, -0.43, 0.0) - 0.35).abs() < 1e-9);
        assert!((photochromic(0.35, -0.43, 1.0) + 0.43).abs() < 1e-9);
        assert_eq!(photochromic(0.35, -0.43, 0.5), 0.0);
        assert!((photochromic(-0.43, -0.50, 0.5) + 0.465).abs() < 1e-9);
    }

    /// The whole walk through the shader's model of the glass body: a
    /// switch from the dark material to the light one gets lighter at every
    /// step, over a black, a grey and a white backdrop. No dip on the way.
    #[test]
    fn dark_to_light_brightens_the_body_at_every_step() {
        use crate::tokens::{Contrast, Inputs, Mode};
        let body = |t: &Tuning, behind: Rgb| {
            let m = &t.material;
            let (fill_color, fill_alpha) = fill(m);
            let model = crate::tokens::material::Material {
                fill_color,
                fill_alpha,
                absorb: m.absorb,
                photochromic: m.photochromic,
                edge_light: m.edge_light,
                frost: m.frost,
            };
            crate::tokens::Oklch::from(crate::tokens::material::glass_body(behind, &model)).0
        };
        let with = |mode: Mode| {
            let tm = crate::tokens::material(Inputs {
                mode,
                contrast: Contrast::Standard,
                ..Inputs::default()
            });
            let mut t = tuning(&tm.fill_color.css(), tm.fill_alpha, tm.frost);
            t.material.absorb = tm.absorb;
            t.material.photochromic = tm.photochromic;
            t.material.edge_light = tm.edge_light;
            t
        };
        // The shipped dark material, sentinels and all, and the light one.
        let mut dark = with(Mode::Dark);
        dark.material.fill_color = "none".into();
        dark.material.fill_alpha = -1.0;
        let light = with(Mode::Light);
        for behind in [Rgb::BLACK, Rgb(0.5, 0.5, 0.5), Rgb::WHITE] {
            let mut prev = body(&dark, behind);
            for i in 1..=50 {
                let now = body(&between(&dark, &light, f64::from(i) / 50.0), behind);
                assert!(
                    now >= prev - 0.005,
                    "over {}: step {i} {prev:.3} -> {now:.3}",
                    behind.css()
                );
                prev = now;
            }
        }
    }
}
