//! Which profile fits the connected outputs, and what applying it changes.
//!
//! Pure, so every rule is tested on made-up heads. The rules are kanshi's
//! (kanshi 1.8 `main.c`): a profile matches when each of its outputs claims
//! a different connected output and every connected output is claimed; the
//! profile on screen stays while it still matches, so a layout picked by
//! hand is not undone by the next change the compositor reports.
//!
//! Where several profiles match, the most specific wins, as shikane ranks
//! them: a serial named exactly says more than a make and model, which say
//! more than a connector, which says more than `*`. Order breaks a tie, so
//! kanshi's "first match wins" still holds between equals.

use crate::settings::store::{DisplayOutput, DisplayProfile, OutputMatch, OutputTransform};

/// A mode as the compositor lists it: width, height, refresh in mHz.
pub type Mode = (u32, u32, u32);

/// One connected output as the compositor last described it.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadState {
    pub name: String,
    pub make: String,
    pub model: String,
    pub serial: String,
    pub enabled: bool,
    pub mode: Option<Mode>,
    pub modes: Vec<Mode>,
    pub position: (i32, i32),
    pub scale: f64,
    pub transform: OutputTransform,
    /// `None` below protocol version 4, which does not report it.
    pub adaptive_sync: Option<bool>,
    /// Millimetres, `(0, 0)` when the output does not say (a projector, a
    /// headless output).
    pub physical_mm: (i32, i32),
}

impl HeadState {
    /// What identifies this output across plugs, for noticing that the set
    /// of outputs changed (and only then matching again).
    pub fn identity(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}",
            self.name, self.make, self.model, self.serial
        )
    }
}

/// `pattern` against `text`: `*` any run, `?` one character, the rest
/// literal. What kanshi's `fnmatch` does with the patterns it is given.
pub fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

fn or_unknown(s: &str) -> &str {
    if s.is_empty() { "Unknown" } else { s }
}

/// Whether `m` claims `head`.
pub fn claims(m: &OutputMatch, head: &HeadState) -> bool {
    let field = |pat: &Option<String>, value: &str| pat.as_deref().is_none_or(|p| glob(p, value));
    field(&m.name, &head.name)
        && field(&m.make, or_unknown(&head.make))
        && field(&m.model, or_unknown(&head.model))
        && field(&m.serial, or_unknown(&head.serial))
}

/// For each of `profile`'s outputs, the head it claims, when the profile
/// matches: each head claimed once and every head claimed.
pub fn assign(profile: &DisplayProfile, heads: &[HeadState]) -> Option<Vec<usize>> {
    if profile.outputs.len() != heads.len() {
        return None;
    }
    fn fill(
        outputs: &[DisplayOutput],
        heads: &[HeadState],
        taken: &mut Vec<bool>,
        out: &mut Vec<usize>,
    ) -> bool {
        let Some((first, rest)) = outputs.split_first() else {
            return true;
        };
        for (i, head) in heads.iter().enumerate() {
            if taken[i] || !claims(&first.criteria, head) {
                continue;
            }
            taken[i] = true;
            out.push(i);
            if fill(rest, heads, taken, out) {
                return true;
            }
            out.pop();
            taken[i] = false;
        }
        false
    }
    let mut taken = vec![false; heads.len()];
    let mut out = Vec::with_capacity(heads.len());
    fill(&profile.outputs, heads, &mut taken, &mut out).then_some(out)
}

/// How much `m` says about which output it wants: an exact serial 8, a
/// make or model 2 each without a glob (1 with one), a connector 1, `*` 0.
fn specificity(m: &OutputMatch) -> u32 {
    let exact = |p: &Option<String>, w: u32| match p {
        Some(s) if !s.contains(['*', '?']) => w,
        Some(s) if s.chars().any(|c| c != '*') => 1,
        _ => 0,
    };
    exact(&m.serial, 8) + exact(&m.make, 2) + exact(&m.model, 2) + exact(&m.name, 1)
}

fn profile_specificity(p: &DisplayProfile) -> u32 {
    p.outputs.iter().map(|o| specificity(&o.criteria)).sum()
}

/// The profile to show: `current` while it still matches, else the most
/// specific that matches, the earlier of two equals. Its index and its
/// assignment.
pub fn choose(
    profiles: &[DisplayProfile],
    heads: &[HeadState],
    current: Option<&str>,
) -> Option<(usize, Vec<usize>)> {
    if heads.is_empty() {
        return None;
    }
    if let Some(name) = current
        && let Some(i) = profiles.iter().position(|p| p.name == name)
        && let Some(a) = assign(&profiles[i], heads)
    {
        return Some((i, a));
    }
    profiles
        .iter()
        .enumerate()
        .filter_map(|(i, p)| assign(p, heads).map(|a| (i, a)))
        // `max_by_key` keeps the last of equals; ranking by the negated
        // index as well keeps the first.
        .max_by_key(|(i, _)| (profile_specificity(&profiles[*i]), std::cmp::Reverse(*i)))
}

/// A scale for an output no profile names, from its pixel density: about
/// 110 pixels to the logical inch, the density a desktop's type is drawn
/// for, in quarter steps between 1 and 3. `None` when the output does not
/// report its size.
pub fn dpi_scale(head: &HeadState) -> Option<f64> {
    let (w_mm, _) = head.physical_mm;
    let (w_px, _, _) = head.mode?;
    if w_mm <= 0 {
        return None;
    }
    let ppi = f64::from(w_px) / (f64::from(w_mm) / 25.4);
    Some(((ppi / 110.0) * 4.0).round().clamp(4.0, 12.0) / 4.0)
}

/// The plan for outputs no profile matches: each output the compositor
/// left at scale 1 that is new since the last match gets [`dpi_scale`];
/// everything else stays as it is.
pub fn default_plan(heads: &[HeadState], new: &[bool]) -> Option<Vec<HeadPlan>> {
    let mut any = false;
    let plans = heads
        .iter()
        .zip(new)
        .map(|(h, is_new)| {
            if !h.enabled {
                return HeadPlan::Disable;
            }
            let scale = (*is_new && (h.scale - 1.0).abs() < 1e-3)
                .then(|| dpi_scale(h))
                .flatten()
                .filter(|s| (s - 1.0).abs() > 1e-3);
            any |= scale.is_some();
            HeadPlan::Enable {
                mode: None,
                position: None,
                scale,
                transform: None,
                adaptive_sync: None,
            }
        })
        .collect();
    any.then_some(plans)
}

/// What to do to one head.
#[derive(Debug, Clone, PartialEq)]
pub enum HeadPlan {
    Disable,
    Enable {
        /// One of the head's own modes, or a custom one it does not list.
        mode: Option<ModeChoice>,
        position: Option<(i32, i32)>,
        scale: Option<f64>,
        transform: Option<OutputTransform>,
        adaptive_sync: Option<bool>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModeChoice {
    Listed(Mode),
    Custom(Mode),
}

impl ModeChoice {
    pub fn mode(self) -> Mode {
        match self {
            ModeChoice::Listed(m) | ModeChoice::Custom(m) => m,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    /// Applying it would leave no output enabled.
    NoneEnabled,
}

/// The head's listed mode for `want`: the same size at the nearest refresh
/// within 1.5 Hz (a panel that says 59.997 Hz for "60"), else a custom
/// mode.
pub fn pick_mode(head: &HeadState, want: Mode) -> ModeChoice {
    let (w, h, r) = want;
    head.modes
        .iter()
        .filter(|(mw, mh, _)| *mw == w && *mh == h)
        .min_by_key(|(_, _, mr)| mr.abs_diff(r))
        .filter(|(_, _, mr)| mr.abs_diff(r) <= 1500)
        .map_or(ModeChoice::Custom(want), |m| ModeChoice::Listed(*m))
}

/// Every head's part of applying `profile`, head by head in `heads` order.
pub fn plan(
    profile: &DisplayProfile,
    assignment: &[usize],
    heads: &[HeadState],
) -> Result<Vec<HeadPlan>, PlanError> {
    let mut plans = vec![HeadPlan::Disable; heads.len()];
    for (output, &i) in profile.outputs.iter().zip(assignment) {
        plans[i] = if output.enabled {
            HeadPlan::Enable {
                mode: output.mode.map(|[w, h, r]| pick_mode(&heads[i], (w, h, r))),
                position: output.position.map(|[x, y]| (x, y)),
                scale: output.scale,
                transform: output.transform,
                adaptive_sync: output.adaptive_sync,
            }
        } else {
            HeadPlan::Disable
        };
    }
    if plans.iter().all(|p| *p == HeadPlan::Disable) {
        return Err(PlanError::NoneEnabled);
    }
    Ok(plans)
}

/// Whether the heads already show `plans`, so applying would change
/// nothing. An apply is a modeset, which blanks a screen for a moment, so
/// one that changes nothing is never sent.
pub fn satisfied(plans: &[HeadPlan], heads: &[HeadState]) -> bool {
    plans.iter().zip(heads).all(|(plan, head)| match plan {
        HeadPlan::Disable => !head.enabled,
        HeadPlan::Enable {
            mode,
            position,
            scale,
            transform,
            adaptive_sync,
        } => {
            head.enabled
                && mode.is_none_or(|m| {
                    head.mode.is_some_and(|(w, h, r)| {
                        let (mw, mh, mr) = m.mode();
                        w == mw && h == mh && r.abs_diff(mr) <= 1500
                    })
                })
                && position.is_none_or(|p| p == head.position)
                && scale.is_none_or(|s| (s - head.scale).abs() < 1e-3)
                && transform.is_none_or(|t| t == head.transform)
                && adaptive_sync.is_none_or(|a| head.adaptive_sync.is_none_or(|h| h == a))
        }
    })
}

/// A profile that puts every output back where it is now, under `name`.
/// An output is matched by make, model and serial when it reports them and
/// no other connected output has the same three (two identical monitors
/// without serials cannot be told apart that way), else by connector.
pub fn capture(name: &str, heads: &[HeadState]) -> DisplayProfile {
    let known = |h: &HeadState| !h.make.is_empty() && h.make != "Unknown";
    let triple = |h: &HeadState| (h.make.clone(), h.model.clone(), h.serial.clone());
    let outputs = heads
        .iter()
        .map(|h| {
            let unique = heads.iter().filter(|o| triple(o) == triple(h)).count() == 1;
            let criteria = if known(h) && unique {
                OutputMatch {
                    make: Some(h.make.clone()),
                    model: Some(or_unknown(&h.model).to_string()),
                    serial: (!h.serial.is_empty() && h.serial != "Unknown")
                        .then(|| h.serial.clone()),
                    ..OutputMatch::default()
                }
            } else {
                OutputMatch {
                    name: Some(h.name.clone()),
                    ..OutputMatch::default()
                }
            };
            DisplayOutput {
                criteria,
                enabled: h.enabled,
                mode: h
                    .enabled
                    .then_some(h.mode)
                    .flatten()
                    .map(|(w, hh, r)| [w, hh, r]),
                position: h.enabled.then_some([h.position.0, h.position.1]),
                scale: h.enabled.then_some(h.scale),
                transform: h.enabled.then_some(h.transform),
                adaptive_sync: None,
            }
        })
        .collect();
    DisplayProfile {
        name: name.to_string(),
        outputs,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn head(name: &str, make: &str, model: &str) -> HeadState {
        HeadState {
            name: name.into(),
            make: make.into(),
            model: model.into(),
            serial: String::new(),
            enabled: true,
            mode: Some((1920, 1080, 60000)),
            modes: vec![
                (1920, 1080, 60000),
                (1920, 1080, 59940),
                (3840, 2160, 59997),
            ],
            position: (0, 0),
            scale: 1.0,
            transform: OutputTransform::Normal,
            adaptive_sync: Some(false),
            physical_mm: (0, 0),
        }
    }

    fn by_name(n: &str) -> OutputMatch {
        OutputMatch {
            name: Some(n.into()),
            ..OutputMatch::default()
        }
    }

    fn out(m: OutputMatch) -> DisplayOutput {
        DisplayOutput {
            criteria: m,
            enabled: true,
            mode: None,
            position: None,
            scale: None,
            transform: None,
            adaptive_sync: None,
        }
    }

    fn profile(name: &str, outputs: Vec<DisplayOutput>) -> DisplayProfile {
        DisplayProfile {
            name: name.into(),
            outputs,
        }
    }

    /// The two profiles kanshi.nix had.
    fn kanshi() -> Vec<DisplayProfile> {
        let desk = OutputMatch {
            make: Some("NON".into()),
            model: Some("28H2U".into()),
            ..OutputMatch::default()
        };
        vec![
            profile("desk-external", vec![out(by_name("eDP-1")), out(desk)]),
            profile("laptop-only", vec![out(by_name("eDP-1"))]),
        ]
    }

    #[test]
    fn globs_are_fnmatch_enough() {
        assert!(glob("NON 28H2U *", "NON 28H2U 0000000000000"));
        assert!(glob("eDP-?", "eDP-1"));
        assert!(glob("*", ""));
        assert!(glob("a*b*c", "axxbyyc"));
        assert!(!glob("a*b", "ac"));
        assert!(!glob("eDP-1", "eDP-10"));
    }

    #[test]
    fn identity_and_unknown_fields() {
        let mut h = head("DP-3", "", "");
        let m = OutputMatch {
            make: Some("Unknown".into()),
            ..OutputMatch::default()
        };
        assert!(claims(&m, &h), "a missing make matches as Unknown");
        h.make = "NON".into();
        assert!(!claims(&m, &h));
        assert!(claims(&OutputMatch::default(), &h), "no field is `*`");
    }

    #[test]
    fn kanshis_profiles_pick_by_what_is_plugged_in() {
        let laptop = head("eDP-1", "BOE", "0x0B66");
        let desk = head("DP-3", "NON", "28H2U");
        let p = kanshi();
        assert_eq!(
            choose(&p, &[laptop.clone(), desk.clone()], None).unwrap().0,
            0
        );
        // The monitor on another port: still the desk.
        let moved = HeadState {
            name: "DP-5".into(),
            ..desk.clone()
        };
        assert_eq!(
            choose(&p, &[moved, laptop.clone()], None).unwrap(),
            (0, vec![1, 0])
        );
        assert_eq!(
            choose(&p, std::slice::from_ref(&laptop), None).unwrap().0,
            1
        );
        // An output no profile claims: nothing matches, nothing is applied.
        let tv = head("HDMI-A-1", "SAM", "TV");
        assert!(choose(&p, &[laptop, tv], None).is_none());
        assert!(choose(&p, &[], None).is_none());
    }

    #[test]
    fn the_most_specific_match_wins_then_the_first_and_the_current_stays() {
        let a = head("DP-1", "X", "Y");
        let any = profile("any", vec![out(OutputMatch::default())]);
        let named = profile("named", vec![out(by_name("DP-1"))]);
        let model = profile(
            "model",
            vec![out(OutputMatch {
                make: Some("X".into()),
                model: Some("Y".into()),
                ..OutputMatch::default()
            })],
        );
        let list = vec![any.clone(), named.clone()];
        // A connector says more than `*`, wherever it sits in the list.
        assert_eq!(choose(&list, std::slice::from_ref(&a), None).unwrap().0, 1);
        // Make and model say more than a connector.
        let list = vec![named.clone(), model];
        assert_eq!(choose(&list, std::slice::from_ref(&a), None).unwrap().0, 1);
        // Equals: the first in order.
        let twin = profile("twin", vec![out(by_name("DP-1"))]);
        let list = vec![named.clone(), twin];
        assert_eq!(choose(&list, std::slice::from_ref(&a), None).unwrap().0, 0);
        // Picked by hand: kept while it matches, even when less specific.
        let list = vec![any, named];
        assert_eq!(
            choose(&list, std::slice::from_ref(&a), Some("any"))
                .unwrap()
                .0,
            0
        );
        // A current profile that stopped matching gives way.
        let b = head("DP-2", "X", "Y");
        assert_eq!(
            choose(&list, std::slice::from_ref(&b), Some("named"))
                .unwrap()
                .0,
            0
        );
    }

    #[test]
    fn an_output_no_profile_names_gets_a_scale_from_its_density() {
        // The desk monitor: 28" 3840 wide, 621 mm: 157 ppi, 1.5.
        let mut desk = head("DP-3", "NON", "28H2U");
        desk.mode = Some((3840, 2160, 60000));
        desk.physical_mm = (621, 341);
        assert_eq!(dpi_scale(&desk), Some(1.5));
        // A 24" 1080p: 92 ppi, stays at 1.
        let mut office = head("DP-4", "DEL", "P2419H");
        office.physical_mm = (527, 296);
        assert_eq!(dpi_scale(&office), Some(1.0));
        assert_eq!(dpi_scale(&head("HEADLESS-1", "", "")), None);
        // Only new outputs at scale 1 move; one set elsewhere stays.
        let plans = default_plan(&[desk.clone(), office.clone()], &[true, true]).unwrap();
        assert!(matches!(plans[0], HeadPlan::Enable { scale: Some(s), .. } if s == 1.5));
        assert!(matches!(plans[1], HeadPlan::Enable { scale: None, .. }));
        assert!(default_plan(&[desk.clone()], &[false]).is_none());
        desk.scale = 2.0;
        assert!(default_plan(&[desk], &[true]).is_none());
    }

    #[test]
    fn a_claim_backtracks_when_the_greedy_choice_blocks_another() {
        // Output 0 claims either head; output 1 only DP-1. Greedy would give
        // DP-1 to output 0 and fail.
        let p = profile(
            "two",
            vec![out(OutputMatch::default()), out(by_name("DP-1"))],
        );
        let heads = [head("DP-1", "", ""), head("DP-2", "", "")];
        assert_eq!(assign(&p, &heads), Some(vec![1, 0]));
    }

    #[test]
    fn plans_pick_listed_modes_and_refuse_to_blank_everything() {
        let h = head("DP-1", "", "");
        let mut o = out(by_name("DP-1"));
        o.mode = Some([3840, 2160, 60000]);
        o.position = Some([10, 20]);
        let p = profile("p", vec![o.clone()]);
        let plans = plan(&p, &[0], std::slice::from_ref(&h)).unwrap();
        let HeadPlan::Enable { mode, position, .. } = &plans[0] else {
            panic!("{plans:?}")
        };
        // "60" is the panel's 59.997.
        assert_eq!(*mode, Some(ModeChoice::Listed((3840, 2160, 59997))));
        assert_eq!(*position, Some((10, 20)));
        assert_eq!(
            pick_mode(&h, (1234, 567, 60000)),
            ModeChoice::Custom((1234, 567, 60000))
        );
        o.enabled = false;
        assert_eq!(
            plan(&profile("p", vec![o]), &[0], &[h]),
            Err(PlanError::NoneEnabled)
        );
    }

    #[test]
    fn a_plan_already_on_screen_is_not_sent() {
        let mut h = head("DP-1", "", "");
        let mut o = out(by_name("DP-1"));
        o.mode = Some([1920, 1080, 60000]);
        o.scale = Some(1.0);
        let p = profile("p", vec![o.clone()]);
        let plans = plan(&p, &[0], std::slice::from_ref(&h)).unwrap();
        assert!(satisfied(&plans, std::slice::from_ref(&h)));
        h.scale = 2.0;
        assert!(!satisfied(&plans, std::slice::from_ref(&h)));
        h.scale = 1.0;
        h.position = (5, 0);
        assert!(
            satisfied(&plans, std::slice::from_ref(&h)),
            "no position asked, any is fine"
        );
        h.enabled = false;
        assert!(!satisfied(&plans, &[h]));
    }

    #[test]
    fn capturing_the_layout_round_trips_through_matching() {
        let mut laptop = head("eDP-1", "BOE", "0x0B66");
        laptop.scale = 2.0;
        laptop.position = (2560, 149);
        let mut twin_a = head("DP-1", "DEL", "U2720Q");
        let twin_b = head("DP-2", "DEL", "U2720Q");
        twin_a.enabled = false;
        let heads = vec![laptop, twin_a, twin_b];
        let p = capture("desk", &heads);
        // Two identical monitors without serials are told apart by port.
        assert_eq!(p.outputs[1].criteria.name.as_deref(), Some("DP-1"));
        assert_eq!(p.outputs[0].criteria.make.as_deref(), Some("BOE"));
        assert!(!p.outputs[1].enabled && p.outputs[1].mode.is_none());
        let (_, a) = choose(std::slice::from_ref(&p), &heads, None).unwrap();
        let plans = plan(&p, &a, &heads).unwrap();
        assert!(satisfied(&plans, &heads));
    }
}
