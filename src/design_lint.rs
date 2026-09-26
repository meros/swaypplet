//! The design system's enforcement (docs/design-system.md §7), as tests.
//!
//! What "every surface is on the design system" means, made checkable:
//!
//! | Rule id           | What it refuses                                              | Where |
//! |-------------------|--------------------------------------------------------------|-------|
//! | `colour-literal`  | hex, `rgb()`/`rgba()`/`hsl()`…, named colours (`white` …)    | all CSS |
//! | `colour-function` | `alpha()`, `shade()`, `mix()`, `lighter()`, `darker()`, `color-mix()` | all CSS |
//! | `at-name`         | `@name` colour references and `@define-color`                | all CSS |
//! | `token`           | `var(--x)` naming nothing the generator emits; a custom property defined outside the generator and `00-components.css` | all CSS |
//! | `primitive`       | `var(--neutral-N)` / `var(--accent-N)`                       | all CSS |
//! | `font-size`       | a `font-size` (or `font`) that is not `var(--type-*)`        | all CSS |
//! | `font-weight`     | a `font-weight` that is not `var(--w-*)`                     | all CSS |
//! | `radius`          | a corner radius that is not `var(--radius-*)` or `0`         | all CSS |
//! | `space`           | padding, margin, border-spacing off the `--space-*` scale    | all CSS |
//! | `motion`          | a transition or animation off `--motion-*` / `--dur-*`+`--ease-*` | all CSS |
//! | `surface-look`    | colour, type, shape or state motion in a surface's own file  | CSS except 00-components.css |
//! | `rust-space`      | a box spacing or widget margin that is a non-zero literal    | Rust |
//! | `rust-colour`     | a Cairo / `gdk::RGBA` colour from numeric literals            | Rust |
//! | `rust-class`      | a CSS class added in Rust that no stylesheet styles          | Rust |
//!
//! Every CSS rule reads the files in [`crate::theme::RULES`] with comments
//! removed (a comment may cite the colour a token was derived from). The
//! Rust rules read `src/**/*.rs` from disk at test time, minus `src/tokens/`
//! (the generator, where the numbers live), `src/ui/` (the components, which
//! are the one place allowed to pick classes and sizes) and anything under
//! `#[cfg(test)]`.
//!
//! ## The migration ledger
//!
//! The surfaces are being moved onto the system one at a time (§9), so the
//! lint could not pass on the day it was switched on. Instead of an `#[ignore]`
//! it carries [`LEDGER`]: one entry per file that is not migrated yet, with
//! the number of violations of each rule it still has. The tests fail when
//!
//! - a file that is **not** on the ledger violates a rule (a migrated file
//!   regressed, or a new file was written the old way);
//! - a file violates a rule its entry does not list;
//! - a count went **up** (someone added to the debt);
//! - a count went **down** or to zero (the ledger must shrink with the
//!   migration: lower the number, delete the rule, or delete the entry — the
//!   failure says exactly which line to change).
//!
//! So the ledger only ever shrinks, and a surface is migrated when its entry
//! is gone. `cargo test --release print_the_ledger -- --ignored --nocapture`
//! prints the ledger as the code stands; use it to fill in the new numbers
//! after a migration, never to raise one.

#![allow(clippy::type_complexity)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// ── Rules ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    Colour,
    ColourFn,
    AtName,
    Token,
    Primitive,
    FontSize,
    FontWeight,
    Radius,
    Space,
    Motion,
    SurfaceLook,
    RustSpace,
    RustColour,
    RustClass,
}

use Rule::*;

impl Rule {
    const CSS_COLOUR: &[Rule] = &[Colour, ColourFn, AtName];
    const CSS_TOKENS: &[Rule] = &[Token, Primitive];
    const CSS_SCALES: &[Rule] = &[FontSize, FontWeight, Radius, Space, Motion];
    const CSS_HYGIENE: &[Rule] = &[SurfaceLook];
    const RUST: &[Rule] = &[RustSpace, RustColour, RustClass];

    fn id(self) -> &'static str {
        match self {
            Colour => "colour-literal",
            ColourFn => "colour-function",
            AtName => "at-name",
            Token => "token",
            Primitive => "primitive",
            FontSize => "font-size",
            FontWeight => "font-weight",
            Radius => "radius",
            Space => "space",
            Motion => "motion",
            SurfaceLook => "surface-look",
            RustSpace => "rust-space",
            RustColour => "rust-colour",
            RustClass => "rust-class",
        }
    }

    /// What to write instead.
    fn help(self) -> &'static str {
        match self {
            Colour => {
                "a colour comes from a semantic token, `var(--fg)`, `var(--fill-2)` … (§3.2; legacy map in §9)"
            }
            ColourFn => {
                "no derived colours in a rule: pick the semantic token for the job (§3.2), or the state overlay (§3.3) for a hover"
            }
            AtName => {
                "the @define-color palette is legacy: map `@name` to its token with the table in §9"
            }
            Token => {
                "only tokens the generator emits (src/tokens/mod.rs `css`) and component-local properties in 00-components.css; a new value is a new token in the spec first"
            }
            Primitive => {
                "primitives are private to the generator: use the semantic token for the step's job (§3.1, §3.2)"
            }
            FontSize => {
                "`font-size: var(--type-*)` (§3.4), or better `ui::text` / `ui::set_text_style` in Rust"
            }
            FontWeight => "`font-weight: var(--w-regular|strong|heavy)` (§3.4)",
            Radius => "`var(--radius-control|tile|thin|card|pill)` or 0 (§3.6)",
            Space => {
                "`var(--space-1…7)`, 0, or `calc()` of space tokens (negative: `calc(-1 * var(--space-n))`) (§3.5)"
            }
            Motion => {
                "`var(--motion-*)` by meaning (§3.8), or `var(--dur-*) var(--ease-*)`; only a repeating @keyframes loop names its own period"
            }
            SurfaceLook => {
                "a surface's file only places things; colour, type, shape and state belong to a component in 00-components.css + src/ui/ (§6, §9 step 2)"
            }
            RustSpace => {
                "`ui::vbox(n)` / `ui::hbox(n)` / `ui::pad(w, n)` or `tokens::space(n)` (§3.5)"
            }
            RustColour => {
                "take the colour from `crate::tokens` (scales, status, categorical), never numbers"
            }
            RustClass => {
                "the class is styled nowhere in data/css/: a typo, or a dead class; use a ui::* component or style it"
            }
        }
    }
}

// ── The ledger ──────────────────────────────────────────────────────────

/// Files not yet migrated, and what they still violate. Shrinks only; see
/// the module docs. Paths are relative to the crate root.
#[rustfmt::skip]
pub const LEDGER: &[(&str, &[(Rule, usize)])] = &[
    // LEDGER-START (generated by `print_the_ledger` on main ec8f215; from here on only ever lowered)
    // ── Stylesheets ──
    ("data/css/01-base.css", &[(Colour, 1), (ColourFn, 1), (AtName, 4), (FontSize, 1), (Radius, 1), (SurfaceLook, 8)]),
    // The bar is migrated; these 12 are not its own: the Helm backup
    // readout (src/widgets/backup.rs) and the media-breathing keyframes the
    // panel and notifications use. They leave with the Helm and panel.
    ("data/css/02-bar.css", &[(SurfaceLook, 12)]),
    ("data/css/03-panel.css", &[(Colour, 12), (ColourFn, 32), (AtName, 92), (FontSize, 26), (FontWeight, 7), (Radius, 20), (Space, 20), (Motion, 10), (SurfaceLook, 168)]),
    ("data/css/04-network.css", &[(Colour, 10), (ColourFn, 21), (AtName, 59), (FontSize, 35), (FontWeight, 9), (Radius, 12), (Space, 31), (Motion, 3), (SurfaceLook, 131)]),
    ("data/css/08-auth.css", &[(Colour, 1), (ColourFn, 21), (AtName, 53), (FontSize, 10), (FontWeight, 9), (Radius, 7), (Space, 21), (Motion, 15), (SurfaceLook, 91)]),
    ("data/css/09-helm.css", &[(Colour, 1), (ColourFn, 16), (AtName, 63), (FontSize, 13), (FontWeight, 6), (Radius, 9), (Space, 40), (Motion, 5), (SurfaceLook, 99)]),
    ("data/css/10-lock.css", &[(Colour, 3), (ColourFn, 3), (AtName, 2), (FontSize, 2), (FontWeight, 2), (Space, 1), (SurfaceLook, 9)]),
    ("data/css/12-screenshot.css", &[(AtName, 18), (FontSize, 1), (Radius, 3), (Space, 3), (SurfaceLook, 23)]),
    ("data/css/13-switcher.css", &[(ColourFn, 1), (AtName, 7), (FontSize, 3), (Radius, 2), (Space, 3), (SurfaceLook, 14)]),
    ("data/css/14-elevation-face.css", &[(Colour, 1), (ColourFn, 6), (AtName, 31), (FontSize, 3), (Radius, 7), (Space, 4), (Motion, 5), (SurfaceLook, 44)]),
    ("data/css/15-settings.css", &[(Colour, 1), (ColourFn, 11), (AtName, 38), (FontSize, 12), (FontWeight, 5), (Radius, 8), (Space, 14), (SurfaceLook, 68)]),
    ("data/css/16-jump.css", &[(Colour, 2), (ColourFn, 9), (AtName, 26), (FontSize, 16), (FontWeight, 5), (Radius, 5), (Space, 12), (Motion, 3), (SurfaceLook, 57)]),
    // ── Rust ──
    ("src/auth_field.rs", &[(RustClass, 1)]),
    ("src/avatar.rs", &[(RustColour, 1)]),
    // Migrated bar: segment identity classes (bar-backup, bar-battery,
    // bar-clock, bar-presence) and presence-away that no rule styles.
    ("src/bar/backup.rs", &[(RustClass, 1)]),
    ("src/bar/battery.rs", &[(RustClass, 1)]),
    ("src/bar/clock.rs", &[(RustClass, 1)]),
    ("src/bar/presence.rs", &[(RustClass, 2)]),
    ("src/face_ring.rs", &[(RustClass, 2)]),
    ("src/jump/pin.rs", &[(RustSpace, 3), (RustClass, 1)]),
    ("src/lock/ui.rs", &[(RustSpace, 5), (RustColour, 1)]),
    ("src/panel.rs", &[(RustSpace, 14), (RustClass, 1)]),
    ("src/polkit/cue.rs", &[(RustSpace, 2), (RustClass, 1)]),
    ("src/polkit/dialog.rs", &[(RustSpace, 2), (RustClass, 1)]),
    ("src/preview.rs", &[(RustSpace, 1), (RustClass, 1)]),
    ("src/screenshot/annotate.rs", &[(RustSpace, 1)]),
    ("src/screenshot/select.rs", &[(RustColour, 10)]),
    ("src/screenshot/window.rs", &[(RustSpace, 2), (RustClass, 1)]),
    ("src/settings/alerts_pane.rs", &[(RustSpace, 1)]),
    ("src/settings/bar_pane.rs", &[(RustSpace, 1)]),
    ("src/settings/glass_pane.rs", &[(RustSpace, 13)]),
    ("src/settings/idle_pane.rs", &[(RustSpace, 3)]),
    ("src/settings/look_pane.rs", &[(RustSpace, 1), (RustClass, 1)]),
    ("src/settings/mod.rs", &[(RustSpace, 2)]),
    ("src/settings/ui.rs", &[(RustSpace, 4)]),
    ("src/widgets/audio.rs", &[(RustSpace, 11), (RustClass, 6)]),
    ("src/widgets/backup.rs", &[(RustSpace, 4), (RustClass, 1)]),
    ("src/widgets/bluetooth.rs", &[(RustSpace, 9), (RustClass, 11)]),
    ("src/widgets/brightness.rs", &[(RustSpace, 3), (RustClass, 1)]),
    ("src/widgets/clipboard.rs", &[(RustSpace, 5)]),
    ("src/widgets/display.rs", &[(RustSpace, 7), (RustClass, 2)]),
    ("src/widgets/media.rs", &[(RustSpace, 6)]),
    ("src/widgets/network/interfaces.rs", &[(RustSpace, 5)]),
    ("src/widgets/network/mod.rs", &[(RustSpace, 23)]),
    ("src/widgets/network/vpn.rs", &[(RustSpace, 5)]),
    ("src/widgets/network/wifi.rs", &[(RustSpace, 24)]),
    ("src/widgets/power.rs", &[(RustSpace, 8), (RustClass, 3)]),
    ("src/widgets/tiles.rs", &[(RustSpace, 1), (RustClass, 1)]),
    ("src/widgets/users.rs", &[(RustSpace, 3)]),
    // LEDGER-END
];

// ── Allowances ──────────────────────────────────────────────────────────

/// Values in 00-components.css that sit off the space scale on purpose.
/// Each is geometry of a knob against its track, not spacing between
/// things, so no space token is its value. (file-less: components only.)
const COMPONENT_SPACE_EXCEPTIONS: &[(&str, &str, &str, &str)] = &[
    (
        "scale.ui-slider slider",
        "margin",
        "-6px 0",
        "the 16 px knob overhangs the 6 px track: (16 − 6) / 2 + 1 above and below",
    ),
    (
        "switch.ui-switch slider",
        "margin",
        "3px",
        "the 16 px knob inset in the 22 px switch: (22 − 16) / 2",
    ),
];

/// Classes GTK's own theme gives meaning to, which Rust may add without a
/// rule in data/css/ (a widget that should look like GTK's own, or a state
/// GTK reads). Keep this to what is actually used.
const GTK_CLASSES: &[&str] = &[];

// ── CSS parsing ─────────────────────────────────────────────────────────

/// One `property: value` with where it sits.
#[derive(Debug, Clone)]
struct Decl {
    line: usize,
    /// The rule's selector; inside @keyframes, the step (`0%`, `from`).
    selector: String,
    /// The @keyframes this sits in, if any.
    keyframes: Option<String>,
    prop: String,
    value: String,
}

#[derive(Debug, Default)]
struct Sheet {
    decls: Vec<Decl>,
    /// Top-level `@…;` statements (`@define-color`, `@import`).
    statements: Vec<(usize, String)>,
    selectors: Vec<String>,
    keyframes: Vec<String>,
}

/// Comments replaced by spaces, newlines kept, so line numbers still hold.
fn strip_css_comments(css: &str) -> String {
    let b = css.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let (mut i, mut quote) = (0, None::<u8>);
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            out.push(c);
            if c == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1]);
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = css[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
            for &x in &b[i..end] {
                out.push(if x == b'\n' { b'\n' } else { b' ' });
            }
            i = end;
            continue;
        }
        if c == b'"' || c == b'\'' {
            quote = Some(c);
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8(out).expect("comment stripping keeps UTF-8 boundaries")
}

fn parse(css: &str) -> Sheet {
    let src = strip_css_comments(css);
    let b = src.as_bytes();
    let line_at = |idx: usize| src[..idx].bytes().filter(|&c| c == b'\n').count() + 1;
    let mut sheet = Sheet::default();
    let mut stack: Vec<String> = Vec::new();
    let (mut start, mut i, mut paren, mut quote) = (0usize, 0usize, 0i32, None::<u8>);

    let chunk = |sheet: &mut Sheet, stack: &[String], from: usize, to: usize| {
        let raw = &src[from..to];
        let text = raw.trim();
        if text.is_empty() {
            return;
        }
        let lead = raw.len() - raw.trim_start().len();
        let line = line_at(from + lead);
        if stack.is_empty() {
            sheet.statements.push((line, text.to_string()));
            return;
        }
        let Some(colon) = text.find(':') else { return };
        let keyframes = stack
            .iter()
            .find_map(|p| p.strip_prefix("@keyframes").map(|n| n.trim().to_string()));
        let selector = stack
            .iter()
            .rev()
            .find(|p| !p.starts_with('@'))
            .cloned()
            .unwrap_or_default();
        let value = text[colon + 1..].trim();
        let value = value.strip_suffix("!important").unwrap_or(value).trim();
        sheet.decls.push(Decl {
            line,
            selector: collapse(&selector),
            keyframes,
            prop: text[..colon].trim().to_ascii_lowercase(),
            value: collapse(value),
        });
    };

    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' | b'\'' => quote = Some(c),
            b'(' => paren += 1,
            b')' => paren -= 1,
            b'{' if paren == 0 => {
                let prelude = collapse(src[start..i].trim());
                let in_keyframes = stack.iter().any(|p| p.starts_with("@keyframes"));
                if let Some(name) = prelude.strip_prefix("@keyframes") {
                    sheet.keyframes.push(name.trim().to_string());
                } else if !prelude.starts_with('@') && !in_keyframes {
                    sheet.selectors.push(prelude.clone());
                }
                stack.push(prelude);
                start = i + 1;
            }
            b';' if paren == 0 => {
                chunk(&mut sheet, &stack, start, i);
                start = i + 1;
            }
            b'}' if paren == 0 => {
                chunk(&mut sheet, &stack, start, i);
                stack.pop();
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    sheet
}

/// Whitespace runs to one space.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// String literals emptied, so a font name cannot look like a colour.
fn without_strings(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut quote = None;
    for c in v.chars() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                out.push(c);
            }
            Some(_) => {}
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                }
                out.push(c);
            }
        }
    }
    out
}

/// The maximal identifier runs of `v`, with the character before each.
fn words(v: &str) -> Vec<(Option<char>, &str)> {
    let mut out = Vec::new();
    let mut prev = None;
    let mut start = None;
    for (i, c) in v.char_indices() {
        match (start, is_ident(c)) {
            (None, true) => start = Some((i, prev)),
            (Some((s, p)), false) => {
                out.push((p, &v[s..i]));
                start = None;
            }
            _ => {}
        }
        prev = Some(c);
    }
    if let Some((s, p)) = start {
        out.push((p, &v[s..]));
    }
    out
}

/// Names of the functions called in `v` (`alpha` in `alpha(@fg, .5)`).
fn functions(v: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for (i, _) in v.match_indices('(') {
        let head = &v[..i];
        let s = head
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_ident(*c))
            .last()
            .map(|(s, _)| s);
        if let Some(s) = s {
            out.push(&v[s..i]);
        }
    }
    out
}

/// Every `var(--name…)` name in `v`, without the dashes.
fn vars(v: &str) -> Vec<&str> {
    v.match_indices("var(")
        .filter_map(|(i, _)| {
            let rest = v[i + 4..].trim_start().strip_prefix("--")?;
            let end = rest.find(|c: char| !is_ident(c)).unwrap_or(rest.len());
            Some(&rest[..end])
        })
        .collect()
}

/// Split at `sep` outside parentheses.
fn split_top(v: &str, sep: fn(char) -> bool) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in v.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if depth == 0 && sep(c) => {
                out.push(v[start..i].trim());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(v[start..].trim());
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

fn parts(v: &str) -> Vec<&str> {
    split_top(v, char::is_whitespace)
}

fn items(v: &str) -> Vec<&str> {
    split_top(v, |c| c == ',')
}

/// `var(--prefix…)` and nothing else.
fn is_var_with(part: &str, prefix: &str) -> bool {
    let Some(inner) = part.strip_prefix("var(").and_then(|r| r.strip_suffix(')')) else {
        return false;
    };
    inner
        .trim()
        .strip_prefix("--")
        .is_some_and(|n| n.starts_with(prefix) && n.chars().all(is_ident))
}

// ── CSS rules ───────────────────────────────────────────────────────────

const NAMED_COLOURS: &[&str] = &[
    "white", "black", "red", "green", "blue", "yellow", "orange", "purple", "gray", "grey",
    "silver", "pink", "cyan", "magenta", "lime", "navy", "teal", "maroon", "olive", "aqua",
    "fuchsia", "gold", "brown", "violet", "indigo",
];
const COLOUR_FUNCTIONS: &[&str] = &[
    "rgb", "rgba", "hsl", "hsla", "hwb", "lab", "lch", "oklab", "oklch",
];
const DERIVED_FUNCTIONS: &[&str] = &["alpha", "shade", "mix", "lighter", "darker", "color-mix"];
const KEYWORD_EASINGS: &[&str] = &[
    "ease",
    "ease-in",
    "ease-out",
    "ease-in-out",
    "linear",
    "step-start",
    "step-end",
];

/// Values that reset a property to nothing, which a surface file may write
/// to clear GTK's own paint without choosing a look.
const NEUTRAL: &[&str] = &["none", "transparent", "0", "inherit", "initial", "unset"];

/// Properties that are a component's business (§6, §9 step 2). A surface
/// file may set them only to a [`NEUTRAL`] value.
///
/// Deliberately *not* here, so a surface file may set them: `opacity` (how
/// a whole surface or part of it fades in, out or dims for a state the
/// surface owns: the bar on an inactive output, a card while it animates;
/// GTK has no token for it), `transform` and `-gtk-icon-transform` (motion
/// of a surface's own layout), `-gtk-icon-size`/`-gtk-icon-style`,
/// `min-width`/`min-height`, `border-width`/`border-style`, `outline-offset`,
/// and every margin and padding (checked against the space scale instead).
const SURFACE_FORBIDDEN: &[&str] = &[
    // colour
    "color",
    "background",
    "background-color",
    "background-image",
    "border-color",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "box-shadow",
    "outline",
    "outline-color",
    "caret-color",
    "text-shadow",
    "-gtk-icon-shadow",
    "text-decoration-color",
    "-gtk-icon-palette",
    // type
    "font",
    "font-size",
    "font-weight",
    "font-family",
    "font-style",
    "letter-spacing",
    "font-feature-settings",
    "text-transform",
    "line-height",
    // shape
    "border-radius",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
];

/// What a surface file may transition or animate: its own layout, never a
/// colour (a colour change on state is `--motion-state` on a component).
const SURFACE_MOTION_PROPS: &[&str] = &[
    "opacity",
    "transform",
    "-gtk-icon-transform",
    "min-width",
    "min-height",
    "margin",
    "margin-top",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "padding",
    "padding-top",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "border-spacing",
];

#[derive(Debug, Clone)]
struct Violation {
    file: String,
    line: usize,
    rule: Rule,
    text: String,
    why: String,
}

/// The token names the generator emits, without the dashes.
fn emitted_tokens() -> BTreeSet<String> {
    crate::tokens::css(crate::tokens::Inputs::default())
        .lines()
        .filter_map(|l| l.trim().strip_prefix("--"))
        .filter_map(|l| l.split(':').next())
        .map(str::to_string)
        .collect()
}

fn is_primitive(name: &str) -> bool {
    ["neutral-", "accent-"].iter().any(|p| {
        name.strip_prefix(p)
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    })
}

fn is_time(p: &str) -> bool {
    let n = p.strip_suffix("ms").or_else(|| p.strip_suffix('s'));
    n.is_some_and(|n| {
        !n.is_empty()
            && n.trim_start_matches('-')
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'.')
    })
}

fn is_number(p: &str) -> bool {
    !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

fn is_easing(p: &str) -> bool {
    KEYWORD_EASINGS.contains(&p) || p.starts_with("cubic-bezier(") || p.starts_with("steps(")
}

/// A space value part: 0, a space token, or a calc() of space tokens.
fn space_part_ok(p: &str) -> bool {
    if p == "0" || is_var_with(p, "space-") {
        return true;
    }
    let Some(inner) = p.strip_prefix("calc(").and_then(|r| r.strip_suffix(')')) else {
        return false;
    };
    let names = vars(inner);
    if names.is_empty() || !names.iter().all(|n| n.starts_with("space-")) {
        return false;
    }
    // What is left once the var()s are gone: numbers and arithmetic.
    let mut rest = inner.to_string();
    while let Some(i) = rest.find("var(") {
        let end = rest[i..].find(')').map_or(rest.len(), |e| i + e + 1);
        rest.replace_range(i..end, " ");
    }
    rest.chars()
        .all(|c| c.is_ascii_digit() || " .+-*/()".contains(c))
}

fn css_violations() -> Vec<Violation> {
    let tokens = emitted_tokens();
    let sheets: Vec<(&str, Sheet)> = crate::theme::RULES
        .iter()
        .map(|(name, css)| (*name, parse(css)))
        .collect();
    let keyframes: BTreeSet<&str> = sheets
        .iter()
        .flat_map(|(_, s)| s.keyframes.iter().map(String::as_str))
        .collect();
    // Component-local custom properties: defined in 00-components.css.
    let local: BTreeSet<String> = sheets
        .iter()
        .filter(|(n, _)| *n == "00-components.css")
        .flat_map(|(_, s)| s.decls.iter())
        .filter_map(|d| d.prop.strip_prefix("--").map(str::to_string))
        .collect();

    let mut out = Vec::new();
    for (name, sheet) in &sheets {
        let file = format!("data/css/{name}");
        let components = *name == "00-components.css";
        let mut push = |line: usize, rule: Rule, text: String, why: String| {
            out.push(Violation {
                file: file.clone(),
                line,
                rule,
                text,
                why,
            });
        };

        for (line, st) in &sheet.statements {
            if st.starts_with("@define-color") {
                push(
                    *line,
                    AtName,
                    st.clone(),
                    "defines a palette colour in a rule file".into(),
                );
            }
        }

        for d in &sheet.decls {
            let shown = format!("{}: {}", d.prop, d.value);
            let v = without_strings(&d.value);

            // ── colour ──
            {
                let mut lits = Vec::new();
                for (i, _) in v.match_indices('#') {
                    let hex: String = v[i + 1..]
                        .chars()
                        .take_while(char::is_ascii_hexdigit)
                        .collect();
                    let next = v[i + 1 + hex.len()..].chars().next();
                    if [3, 4, 6, 8].contains(&hex.len()) && !next.is_some_and(is_ident) {
                        lits.push(format!("#{hex}"));
                    }
                }
                for (prev, w) in words(&v) {
                    if NAMED_COLOURS.contains(&w.to_ascii_lowercase().as_str())
                        && !matches!(prev, Some('#' | '@' | '-'))
                    {
                        lits.push(w.to_string());
                    }
                }
                let fns = functions(&v);
                lits.extend(
                    fns.iter()
                        .filter(|f| COLOUR_FUNCTIONS.contains(&f.to_ascii_lowercase().as_str()))
                        .map(|f| format!("{f}()")),
                );
                if !lits.is_empty() {
                    push(d.line, Colour, shown.clone(), lits.join(", "));
                }
                let derived: Vec<String> = fns
                    .iter()
                    .filter(|f| DERIVED_FUNCTIONS.contains(&f.to_ascii_lowercase().as_str()))
                    .map(|f| format!("{f}()"))
                    .collect();
                if !derived.is_empty() {
                    push(d.line, ColourFn, shown.clone(), derived.join(", "));
                }
                let ats: Vec<String> = v
                    .match_indices('@')
                    .filter_map(|(i, _)| {
                        let n: String = v[i + 1..].chars().take_while(|c| is_ident(*c)).collect();
                        (!n.is_empty()).then(|| format!("@{n}"))
                    })
                    .collect();
                if !ats.is_empty() {
                    push(d.line, AtName, shown.clone(), ats.join(", "));
                }
            }

            // ── tokens ──
            if let Some(defined) = d.prop.strip_prefix("--")
                && !components
            {
                push(
                    d.line,
                    Token,
                    shown.clone(),
                    format!(
                        "defines the custom property --{defined} outside the generator and 00-components.css"
                    ),
                );
            }
            let mut unknown = Vec::new();
            let mut primitives = Vec::new();
            for n in vars(&v) {
                if is_primitive(n) {
                    primitives.push(format!("--{n}"));
                } else if !tokens.contains(n) && !local.contains(n) {
                    unknown.push(format!("--{n}"));
                }
            }
            if !unknown.is_empty() {
                push(
                    d.line,
                    Token,
                    shown.clone(),
                    format!("no such token: {}", unknown.join(", ")),
                );
            }
            if !primitives.is_empty() {
                push(d.line, Primitive, shown.clone(), primitives.join(", "));
            }

            // ── scales ──
            let p = d.prop.as_str();
            let inherit = v == "inherit";
            if (p == "font-size" && !inherit && !is_var_with(&v, "type-"))
                || (p == "font" && !inherit)
            {
                push(
                    d.line,
                    FontSize,
                    shown.clone(),
                    "not a --type-* token".into(),
                );
            }
            if p == "font-weight" && !inherit && !is_var_with(&v, "w-") {
                push(
                    d.line,
                    FontWeight,
                    shown.clone(),
                    "not a --w-* token".into(),
                );
            }
            if p.starts_with("border") && p.ends_with("radius") && !inherit {
                let bad: Vec<&str> = parts(&v)
                    .into_iter()
                    .filter(|x| *x != "0" && !is_var_with(x, "radius-"))
                    .collect();
                if !bad.is_empty() {
                    push(
                        d.line,
                        Radius,
                        shown.clone(),
                        format!("off the scale: {}", bad.join(" ")),
                    );
                }
            }
            if p.starts_with("padding") || p.starts_with("margin") || p == "border-spacing" {
                let excepted = components
                    && COMPONENT_SPACE_EXCEPTIONS
                        .iter()
                        .any(|(sel, prop, val, _)| *sel == d.selector && *prop == p && *val == v);
                let bad: Vec<&str> = parts(&v)
                    .into_iter()
                    .filter(|x| !space_part_ok(x))
                    .collect();
                if !bad.is_empty() && !excepted {
                    push(
                        d.line,
                        Space,
                        shown.clone(),
                        format!("off the scale: {}", bad.join(" ")),
                    );
                }
            }
            if matches!(p, "transition" | "animation") {
                for item in items(&v) {
                    if item == "none" {
                        continue;
                    }
                    let ps = parts(item);
                    let lit_time = ps.iter().any(|x| is_time(x));
                    let lit_ease = ps.iter().any(|x| is_easing(x));
                    let has = |pre: &str| ps.iter().any(|x| is_var_with(x, pre));
                    let tokened = has("motion-") || (has("dur-") && has("ease-"));
                    let looping = p == "animation"
                        && ps.iter().any(|x| *x == "infinite" || is_number(x))
                        && ps.iter().any(|x| keyframes.contains(*x));
                    if looping {
                        continue; // an attention loop names its own period (§3.8)
                    }
                    if lit_time || lit_ease || !tokened {
                        push(
                            d.line,
                            Motion,
                            shown.clone(),
                            format!("`{item}` is off the motion scale"),
                        );
                        break;
                    }
                }
            }
            if matches!(
                p,
                "transition-duration"
                    | "transition-timing-function"
                    | "animation-duration"
                    | "animation-timing-function"
            ) && parts(&v).iter().any(|x| is_time(x) || is_easing(x))
            {
                push(
                    d.line,
                    Motion,
                    shown.clone(),
                    "a literal duration or curve".into(),
                );
            }

            // ── surface hygiene ──
            if !components {
                if SURFACE_FORBIDDEN.contains(&p) && !NEUTRAL.contains(&v.as_str()) {
                    let what = if d.keyframes.is_some() {
                        "animates"
                    } else {
                        "sets"
                    };
                    push(
                        d.line,
                        SurfaceLook,
                        shown.clone(),
                        format!("a surface file {what} `{p}`"),
                    );
                }
                if matches!(p, "transition" | "transition-property") && v != "none" {
                    let bad: Vec<String> = items(&v)
                        .into_iter()
                        .map(|item| {
                            parts(item)
                                .into_iter()
                                .find(|x| {
                                    x.chars()
                                        .next()
                                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '-')
                                        && !is_time(x)
                                        && !is_easing(x)
                                        && !x.starts_with("var(")
                                })
                                .unwrap_or("all")
                                .to_string()
                        })
                        .filter(|prop| !SURFACE_MOTION_PROPS.contains(&prop.as_str()))
                        .collect();
                    if !bad.is_empty() {
                        push(
                            d.line,
                            SurfaceLook,
                            shown.clone(),
                            format!("a surface file transitions {}", bad.join(", ")),
                        );
                    }
                }
            }
        }
    }
    out
}

// ── Rust scan ───────────────────────────────────────────────────────────

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let src = root().join("src");
    let mut out = Vec::new();
    walk(&src, &mut out);
    out.retain(|p| {
        let rel = p.strip_prefix(&src).unwrap();
        !rel.starts_with("tokens")
            && !rel.starts_with("ui")
            && rel != Path::new("design_lint.rs")
            && p.file_name().is_some_and(|n| n != "tests.rs")
    });
    out
}

/// Comments blanked (newlines kept), and every `#[cfg(test)]` item blanked.
fn rust_code(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for x in &mut out[from..to] {
            if *x != b'\n' {
                *x = b' ';
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = src[i..].find('\n').map_or(b.len(), |e| i + e);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = src[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
                blank(&mut out, i, end);
                i = end;
            }
            b'"' => {
                // A string (raw strings in this codebase hold no quotes
                // that would confuse this).
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'\'' => {
                // A char literal ('x', '\n', '"'), or a lifetime.
                if b.get(i + 1) == Some(&b'\\') {
                    i += src[i + 2..].find('\'').map_or(b.len(), |e| e + 3);
                } else if b.get(i + 2) == Some(&b'\'') {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    let mut code = String::from_utf8(out).expect("blanking keeps UTF-8 boundaries");
    // #[cfg(test)] items: from the attribute to the end of the item.
    let mut from = 0;
    while let Some(at) = code[from..].find("#[cfg(test)]").map(|a| from + a) {
        let rest = &code[at..];
        let (brace, semi) = (rest.find('{'), rest.find(';'));
        let end = match (brace, semi) {
            (Some(bi), Some(si)) if si < bi => at + si + 1,
            (Some(bi), _) => at + bi + matching(&rest[bi..]).unwrap_or(rest.len() - bi),
            (None, Some(si)) => at + si + 1,
            (None, None) => code.len(),
        };
        let mut bytes = code.into_bytes();
        blank(&mut bytes, at, end);
        code = String::from_utf8(bytes).unwrap();
        from = end;
    }
    code
}

/// Length up to and including the bracket closing the one `s` starts with.
fn matching(s: &str) -> Option<usize> {
    let open = s.chars().next()?;
    let close = match open {
        '(' => ')',
        '{' => '}',
        '[' => ']',
        _ => return None,
    };
    let (mut depth, mut quote) = (0i32, false);
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if quote {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == '"' {
                quote = false;
            }
        } else if c == '"' {
            quote = true;
        } else if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

/// Every call of `needle` (which ends in `(`): the byte offset and the
/// argument text.
fn calls<'a>(code: &'a str, needle: &str) -> Vec<(usize, &'a str)> {
    let mut out = Vec::new();
    for (at, _) in code.match_indices(needle) {
        let paren = at + needle.len() - 1;
        if let Some(len) = matching(&code[paren..]) {
            out.push((at, &code[paren + 1..paren + len - 1]));
        }
    }
    out
}

/// A Rust expression made only of number literals and arithmetic.
fn numeric(e: &str) -> bool {
    let mut s = e.trim().to_string();
    for suffix in ["f64", "f32", "i32", "u32", "i64", "u8", "usize"] {
        s = s.replace(suffix, "");
    }
    s.chars().any(|c| c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_digit() || " ._+-*/()".contains(c))
}

fn nonzero(e: &str) -> bool {
    numeric(e) && e.chars().any(|c| ('1'..='9').contains(&c))
}

fn string_literals(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = args.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            let s = i + 1;
            i = s;
            while i < b.len() && b[i] != b'"' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            out.push(&args[s..i.min(args.len())]);
        }
        i += 1;
    }
    out
}

/// Every class any selector in data/css/ names.
fn styled_classes() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (_, css) in crate::theme::RULES {
        for sel in parse(css).selectors {
            let s: Vec<char> = sel.chars().collect();
            let mut i = 0;
            while i < s.len() {
                if s[i] == '.'
                    && s.get(i + 1)
                        .is_some_and(|c| c.is_ascii_alphabetic() || *c == '-' || *c == '_')
                {
                    let n: String = s[i + 1..].iter().take_while(|c| is_ident(**c)).collect();
                    i += n.len();
                    out.insert(n);
                }
                i += 1;
            }
        }
    }
    out
}

fn rust_violations() -> Vec<Violation> {
    let styled = styled_classes();
    let root = root();
    let mut out = Vec::new();
    for path in rust_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        let code = rust_code(&src);
        let file = path.strip_prefix(&root).unwrap().display().to_string();
        let line_at = |i: usize| code[..i].bytes().filter(|&c| c == b'\n').count() + 1;
        let snippet = |_at: usize, args: &str, needle: &str| {
            let s = collapse(&format!("{needle}{args})"));
            if s.chars().count() > 90 {
                format!("{}…", s.chars().take(90).collect::<String>())
            } else {
                s
            }
        };
        let mut push = |at: usize, rule: Rule, text: String, why: String| {
            out.push(Violation {
                file: file.clone(),
                line: line_at(at),
                rule,
                text,
                why,
            });
        };

        // Spacing and margins.
        for needle in [
            ".spacing(",
            "set_spacing(",
            ".margin_top(",
            ".margin_bottom(",
            ".margin_start(",
            ".margin_end(",
            "set_margin_top(",
            "set_margin_bottom(",
            "set_margin_start(",
            "set_margin_end(",
        ] {
            for (at, args) in calls(&code, needle) {
                if nonzero(args) {
                    push(
                        at,
                        RustSpace,
                        snippet(at, args, needle),
                        "a literal spacing or margin".into(),
                    );
                }
            }
        }
        for (at, args) in calls(&code, "Box::new(") {
            let a = items_rs(args);
            if a.len() == 2 && a[0].contains("Orientation") && nonzero(a[1]) {
                push(
                    at,
                    RustSpace,
                    snippet(at, args, "Box::new("),
                    "a literal box spacing".into(),
                );
            }
        }

        // Colour literals.
        for needle in ["set_source_rgb(", "set_source_rgba(", "RGBA::new("] {
            for (at, args) in calls(&code, needle) {
                let a = items_rs(args);
                if a.len() >= 3 && a[..3].iter().any(|x| numeric(x)) {
                    push(
                        at,
                        RustColour,
                        snippet(at, args, needle),
                        "a colour from numbers".into(),
                    );
                }
            }
        }
        for (at, args) in calls(&code, "RGBA::parse(") {
            if !string_literals(args).is_empty() {
                push(
                    at,
                    RustColour,
                    snippet(at, args, "RGBA::parse("),
                    "a colour from a string".into(),
                );
            }
        }

        // Classes.
        for needle in ["add_css_class(", ".css_classes(", "set_css_classes("] {
            for (at, args) in calls(&code, needle) {
                for class in string_literals(args) {
                    if class.contains('{') || class.is_empty() {
                        continue; // built at runtime (format!)
                    }
                    if !styled.contains(class) && !GTK_CLASSES.contains(&class) {
                        push(
                            at,
                            RustClass,
                            snippet(at, args, needle),
                            format!("`.{class}` is styled nowhere in data/css/"),
                        );
                    }
                }
            }
        }
    }
    out
}

/// Split Rust call arguments at top-level commas.
fn items_rs(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in args.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(args[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(args[start..].trim());
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

// ── Checking against the ledger ─────────────────────────────────────────

fn all_violations() -> &'static [Violation] {
    static ALL: OnceLock<Vec<Violation>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut v = css_violations();
        v.extend(rust_violations());
        v
    })
}

fn counts(rules: &[Rule]) -> BTreeMap<String, BTreeMap<Rule, Vec<&'static Violation>>> {
    let mut out: BTreeMap<String, BTreeMap<Rule, Vec<&Violation>>> = BTreeMap::new();
    for v in all_violations().iter().filter(|v| rules.contains(&v.rule)) {
        out.entry(v.file.clone())
            .or_default()
            .entry(v.rule)
            .or_default()
            .push(v);
    }
    out
}

fn listing(vs: &[&Violation]) -> String {
    let mut s = String::new();
    for v in vs {
        let _ = writeln!(
            s,
            "    {}:{} [{}] {}\n        {}",
            v.file,
            v.line,
            v.rule.id(),
            v.text,
            v.why
        );
    }
    s
}

/// The rules in `rules`, held to the ledger.
fn check(rules: &[Rule]) {
    let found = counts(rules);
    let mut problems = Vec::new();

    for (file, by_rule) in &found {
        let entry = LEDGER.iter().find(|(f, _)| f == file).map(|(_, r)| *r);
        for (rule, vs) in by_rule {
            let allowed = entry
                .and_then(|r| r.iter().find(|(x, _)| x == rule))
                .map(|(_, n)| *n);
            match allowed {
                None => problems.push(format!(
                    "NEW: {file} breaks [{}] {} time(s), and the ledger {}.\n  Fix: {}\n{}",
                    rule.id(),
                    vs.len(),
                    if entry.is_some() { "does not list that rule for it" } else { "has no entry for it (migrated files stay clean)" },
                    rule.help(),
                    listing(vs)
                )),
                Some(n) if vs.len() > n => problems.push(format!(
                    "UP: {file} [{}] was {n} on the ledger, is {} now: new debt. Every one in the file:\n  Fix: {}\n{}",
                    rule.id(),
                    vs.len(),
                    rule.help(),
                    listing(vs)
                )),
                Some(n) if vs.len() < n => problems.push(format!(
                    "DOWN: {file} [{}] went from {n} to {}. Shrink the ledger in src/design_lint.rs: \
                     in the entry for \"{file}\", change `({rule:?}, {n})` to `({rule:?}, {})`.",
                    rule.id(),
                    vs.len(),
                    vs.len()
                )),
                Some(_) => {}
            }
        }
    }
    for (file, rs) in LEDGER {
        for (rule, n) in rs.iter().filter(|(r, _)| rules.contains(r)) {
            let now = found
                .get(*file)
                .and_then(|m| m.get(rule))
                .map_or(0, Vec::len);
            if now == 0 {
                let left = all_violations().iter().filter(|v| v.file == *file).count();
                problems.push(if left == 0 {
                    format!(
                        "DONE: {file} has no violations left. Delete its whole entry \
                         `(\"{file}\", &[...])` from LEDGER in src/design_lint.rs."
                    )
                } else {
                    format!(
                        "DONE: {file} no longer breaks [{}] (ledger said {n}). Delete `({rule:?}, {n})` \
                         from its entry in LEDGER in src/design_lint.rs.",
                        rule.id()
                    )
                });
            }
        }
    }

    if !problems.is_empty() {
        let mut summary = String::new();
        for (file, by_rule) in &found {
            let parts: Vec<String> = by_rule
                .iter()
                .map(|(r, v)| format!("{} {}", r.id(), v.len()))
                .collect();
            let _ = writeln!(summary, "  {file}: {}", parts.join(", "));
        }
        panic!(
            "design lint (docs/design-system.md §7): {} problem(s).\n\n{}\n\nCounts now, per file, for these rules:\n{summary}",
            problems.len(),
            problems.join("\n\n")
        );
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

/// §7.1: colours come from `var()`. No hex, `rgb()`, named colours,
/// `alpha()`/`shade()`/`mix()`, or `@name` from the legacy palette.
#[test]
fn css_colours_come_from_tokens() {
    check(Rule::CSS_COLOUR);
}

/// §7.2: every `var(--x)` is a token the generator emits (or a
/// component-local property of 00-components.css); never a primitive.
#[test]
fn css_names_only_semantic_tokens() {
    check(Rule::CSS_TOKENS);
}

/// §7.3: type, weight, radius, space and motion on their scales.
#[test]
fn css_sizes_are_on_the_scales() {
    check(Rule::CSS_SCALES);
}

/// §6 / §9 step 2: a surface's own file places things; its look is a
/// component's.
#[test]
fn surface_css_only_places_things() {
    check(Rule::CSS_HYGIENE);
}

/// §7.5: Rust spacing from `tokens::space`, colours from `tokens`, and every
/// class it adds styled somewhere.
#[test]
fn rust_takes_space_colour_and_classes_from_the_system() {
    check(Rule::RUST);
}

/// Every ledger entry names a file that exists, once, with no empty or
/// zero rows, so the ledger cannot carry dead weight.
#[test]
fn the_ledger_names_real_files() {
    let mut seen = BTreeSet::new();
    for (file, rules) in LEDGER {
        assert!(
            root().join(file).is_file(),
            "LEDGER names {file}, which does not exist: delete the entry"
        );
        assert!(
            seen.insert(*file),
            "LEDGER lists {file} twice: merge the entries"
        );
        assert!(
            !rules.is_empty(),
            "LEDGER entry for {file} lists no rules: delete it"
        );
        for (r, n) in *rules {
            assert!(*n > 0, "LEDGER: {file} ({r:?}, 0): delete that row");
        }
    }
}

/// The already-migrated pieces are clean on their own, whatever the ledger
/// says: the components, the OSD, and the builders.
#[test]
fn migrated_surfaces_are_clean() {
    for file in [
        "data/css/00-components.css",
        "data/css/05-osd.css",
        "src/osd.rs",
    ] {
        let vs: Vec<&Violation> = all_violations().iter().filter(|v| v.file == file).collect();
        assert!(
            vs.is_empty(),
            "{file} is migrated and must stay clean:\n{}",
            listing(&vs)
        );
        assert!(
            !LEDGER.iter().any(|(f, _)| *f == file),
            "{file} is migrated; it has no place on the ledger"
        );
    }
}

/// Spec and code cannot drift: every scale token the generator emits is
/// named in docs/design-system.md.
#[test]
fn every_scale_token_is_documented() {
    let doc = include_str!("../docs/design-system.md");
    let mut missing = Vec::new();
    for name in emitted_tokens() {
        if !["motion-", "type-", "radius-", "space-", "w-"]
            .iter()
            .any(|p| name.starts_with(p))
        {
            continue;
        }
        let needle = format!("--{name}");
        let named = doc
            .match_indices(&needle)
            .any(|(i, _)| !doc[i + needle.len()..].chars().next().is_some_and(is_ident));
        if !named {
            missing.push(needle);
        }
    }
    assert!(
        missing.is_empty(),
        "the generator emits tokens docs/design-system.md never names: {}.\n\
         Document each in §3 (its value and what it is for), or stop emitting it.",
        missing.join(", ")
    );
}

/// The linter, on examples, so a rule that stops catching what it should
/// fails here rather than silently passing everything.
#[test]
fn the_linter_catches_what_it_should() {
    let s = parse(
        "/* #ffffff */ .a { color: #fff; background: alpha(@fg, 0.5); }\n\
         .b { font-size: 13px; padding: calc(var(--space-2) * 2) 0; margin: -4px; }\n\
         .c { transition: color var(--motion-state); animation: pulse 1s ease infinite; }\n\
         @keyframes pulse { from { opacity: 0; } to { opacity: 1; } }\n\
         .d { font-family: \"Red Hat\"; border-radius: 0 var(--radius-tile); }",
    );
    assert_eq!(s.decls.len(), 11, "{:#?}", s.decls);
    assert_eq!(s.decls[0].line, 1);
    assert_eq!(s.decls[2].line, 2);
    assert_eq!(s.keyframes, ["pulse"]);
    assert_eq!(s.decls[7].keyframes.as_deref(), Some("pulse"));
    assert!(space_part_ok("calc(var(--space-2) * 2)"));
    assert!(space_part_ok("calc(-1 * var(--space-3))"));
    assert!(!space_part_ok("-4px"));
    assert!(!space_part_ok("calc(var(--fg) * 2)"));
    assert!(is_var_with("var(--type-body)", "type-"));
    assert!(!is_var_with("var(--type-body) 2px", "type-"));
    assert_eq!(functions("alpha(@fg, 0.5)"), ["alpha"]);
    assert_eq!(
        vars("calc(var(--space-7) + var(--space-5))"),
        ["space-7", "space-5"]
    );
    assert!(
        words(&without_strings("\"Red Hat\", sans"))
            .iter()
            .all(|(_, w)| *w != "Red")
    );
    assert!(is_primitive("neutral-3") && is_primitive("accent-12"));
    assert!(!is_primitive("accent-bg") && !is_primitive("accent"));
    assert!(nonzero("6") && nonzero("-4") && !nonzero("0") && !nonzero("space(3)"));
    assert!(numeric("0.408") && !numeric("r as f64 / 255.0") && !numeric("c.0"));
    let code = rust_code(
        "fn a() { b.set_spacing(6); } // .spacing(4)\n#[cfg(test)]\nmod t { fn x() { y.spacing(8); } }",
    );
    assert_eq!(calls(&code, ".spacing(").len(), 0);
    assert_eq!(calls(&code, "set_spacing(").len(), 1);
}

/// Prints [`LEDGER`] as the code stands. Run after migrating a surface to
/// read off the lower numbers; a reviewer rejects a number that grew.
#[test]
#[ignore]
fn print_the_ledger() {
    let found = counts(&[
        Colour,
        ColourFn,
        AtName,
        Token,
        Primitive,
        FontSize,
        FontWeight,
        Radius,
        Space,
        Motion,
        SurfaceLook,
        RustSpace,
        RustColour,
        RustClass,
    ]);
    let mut s = String::new();
    for (file, by_rule) in &found {
        let rows: Vec<String> = by_rule
            .iter()
            .map(|(r, v)| format!("({r:?}, {})", v.len()))
            .collect();
        let _ = writeln!(s, "    (\"{file}\", &[{}]),", rows.join(", "));
    }
    println!("{s}");
    for v in all_violations() {
        eprintln!(
            "{}:{} [{}] {} -- {}",
            v.file,
            v.line,
            v.rule.id(),
            v.text,
            v.why
        );
    }
}
