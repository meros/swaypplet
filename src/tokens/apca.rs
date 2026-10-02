//! APCA 0.0.98G, and the contrast requirements of §5 as tests: every text
//! token over the glass body, computed through the material the way
//! `liquid_glass.frag` does it, over white, mid-grey and black behind.

use super::Rgb;

/// Lc of `text` on `bg`, signed as APCA signs it (positive: dark text).
pub fn apca(text: Rgb, bg: Rgb) -> f64 {
    let y = |c: Rgb| {
        0.212_672_9 * c.0.powf(2.4) + 0.715_152_2 * c.1.powf(2.4) + 0.072_175 * c.2.powf(2.4)
    };
    let clamp = |y: f64| {
        if y < 0.022 {
            y + (0.022 - y).powf(1.414)
        } else {
            y
        }
    };
    let (yt, yb) = (clamp(y(text)), clamp(y(bg)));
    if (yb - yt).abs() < 0.0005 {
        return 0.0;
    }
    let out = if yb > yt {
        let s = (yb.powf(0.56) - yt.powf(0.57)) * 1.14;
        if s < 0.1 { 0.0 } else { s - 0.027 }
    } else {
        let s = (yb.powf(0.65) - yt.powf(0.62)) * 1.14;
        if s > -0.1 { 0.0 } else { s + 0.027 }
    };
    out * 100.0
}

#[cfg(test)]
mod tests {
    use super::super::every;
    use super::*;
    use crate::tokens::material::glass_body;
    use crate::tokens::{
        Accent, Contrast, ON_STATUS, categorical, levels, material, scales, status,
    };

    const BEHIND: [Rgb; 3] = [Rgb::WHITE, Rgb(0.5, 0.5, 0.5), Rgb::BLACK];

    /// §5: every text token, through the material, over what can be behind
    /// the glass. Grey and black backdrops (a desktop, a terminal) are held
    /// to the full targets; a pure white page, which the dark material
    /// cannot fully overcome, to 15 less, and to 5 less at high contrast.
    #[test]
    fn text_meets_its_contrast_over_every_backdrop() {
        let mut failures = Vec::new();
        for inputs in every::input() {
            let s = scales(inputs);
            let lv = levels(inputs.mode, inputs.contrast);
            let m = material(inputs);
            let t = crate::tokens::material::targets(inputs.mode, inputs.contrast);
            let fg = s.neutral[11];
            for behind in BEHIND {
                let ground = glass_body(behind, &m);
                let slack = match (behind == Rgb::WHITE, inputs.contrast) {
                    (false, _) => 0.0,
                    (true, Contrast::Standard) => 15.0,
                    (true, Contrast::High) => 5.0,
                };
                for (name, color, alpha, need) in [
                    ("fg", fg, 1.0, t.fg - slack),
                    ("fg-muted", fg, lv.muted, t.muted - slack),
                    ("fg-faint", fg, lv.faint, t.faint - slack),
                    ("accent", s.accent[10], 1.0, t.accent - slack),
                ] {
                    let lc = apca(color.over(alpha, ground), ground).abs();
                    if lc < need {
                        failures.push(format!(
                            "{inputs:?} {name} over {} behind: Lc {lc:.0} < {need}",
                            behind.css()
                        ));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} failures:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn on_colours_meet_their_contrast() {
        let mut failures = Vec::new();
        for inputs in every::input() {
            let s = scales(inputs);
            let lc = apca(s.on_accent, s.accent_bg).abs();
            if lc < 60.0 {
                failures.push(format!("{inputs:?} on-accent: Lc {lc:.0}"));
            }
        }
        for inputs in every::input() {
            let st = status(inputs);
            for (name, bg) in [
                ("success", st.success_bg),
                ("warning", st.warning_bg),
                ("danger", st.danger_bg),
            ] {
                let lc = apca(ON_STATUS, bg).abs();
                if lc < 60.0 {
                    failures.push(format!("{inputs:?} on-status on {name}: Lc {lc:.0}"));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} failures:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// A categorical colour marks identity with a dot, a rail or a short
    /// label; Lc 45 over grey and black behind, the large-text level.
    #[test]
    fn categorical_colours_read_on_the_glass() {
        let mut failures = Vec::new();
        // The accent does not reach the categorical set; the neutral does,
        // through the glass body.
        for inputs in every::input()
            .into_iter()
            .filter(|i| i.accent == Accent::Aqua)
        {
            let m = material(inputs);
            let need = crate::tokens::material::targets(inputs.mode, inputs.contrast).categorical;
            let ink = scales(inputs).neutral[0];
            for behind in [Rgb(0.5, 0.5, 0.5), Rgb::BLACK] {
                let ground = glass_body(behind, &m);
                for (i, (text, fill)) in categorical(inputs).iter().enumerate() {
                    let lc = apca(*text, ground).abs();
                    if lc < need {
                        failures.push(format!(
                            "{inputs:?} cat-{} over {}: Lc {lc:.0}",
                            i + 1,
                            behind.css()
                        ));
                    }
                    let on = apca(Rgb::WHITE, *fill).abs().max(apca(ink, *fill).abs());
                    if on < 45.0 {
                        failures.push(format!("{inputs:?} cat-{}-bg label: Lc {on:.0}", i + 1));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} failures:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
