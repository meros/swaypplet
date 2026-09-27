//! The xkb names the Input tab offers: every layout and variant, and the
//! options for the layout switch, read from xkeyboard-config's
//! `rules/evdev.lst`.
//!
//! The file is the list setxkbmap and every desktop's layout picker read. It
//! is plain text in four `! section` blocks; a line is a name, whitespace,
//! and a description, and a variant's description starts with its layout
//! (`dvorak          se: Swedish (Dvorak)`).
//!
//! NixOS has no `/usr/share/X11/xkb`, so the package build names the file
//! (`SWAYPPLET_XKB_RULES`, flake.nix); `XKB_CONFIG_ROOT` and the FHS paths
//! come first so a dev build on another distribution finds it too. No file
//! means an empty catalogue: the tab still lists and reorders the layouts in
//! force, it only cannot offer new ones by name.

use std::path::PathBuf;
use std::sync::OnceLock;

/// One layout or one variant of a layout, as the picker lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// `se`, or `se(dvorak)` for a variant: what `input.layouts` holds.
    pub code: String,
    /// `Swedish`, `Swedish (Dvorak)`.
    pub description: String,
}

/// One xkb option: `grp:win_space_toggle` and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XkbOption {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Default)]
pub struct Catalogue {
    /// Every layout, each followed by its variants.
    pub layouts: Vec<Layout>,
    /// Every option, in the file's order.
    pub options: Vec<XkbOption>,
}

impl Catalogue {
    /// The options of one group, `grp` for the layout switch.
    pub fn options_in(&self, group: &str) -> impl Iterator<Item = &XkbOption> {
        let prefix = format!("{group}:");
        self.options
            .iter()
            .filter(move |o| o.name.starts_with(&prefix))
    }

    /// The description of a code, `se(dvorak)` → `Swedish (Dvorak)`.
    pub fn describe(&self, code: &str) -> Option<&str> {
        self.layouts
            .iter()
            .find(|l| l.code == code)
            .map(|l| l.description.as_str())
    }

    /// The code of a description, which is all `get_inputs` reports of a
    /// keyboard's layouts (`xkb_layout_names`).
    pub fn code_of(&self, description: &str) -> Option<&str> {
        self.layouts
            .iter()
            .find(|l| l.description == description)
            .map(|l| l.code.as_str())
    }

    /// Layouts whose code or description contains every word of `query`,
    /// case-folded, in the file's order.
    pub fn search<'a>(&'a self, query: &'a str) -> impl Iterator<Item = &'a Layout> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.layouts.iter().filter(move |l| {
            let hay = format!("{} {}", l.code, l.description).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
    }
}

/// Parse `evdev.lst`. A variant whose layout the file never lists is kept
/// at the end rather than dropped.
pub fn parse(text: &str) -> Catalogue {
    let mut section = "";
    let mut layouts: Vec<Layout> = Vec::new();
    let mut variants: Vec<(String, Layout)> = Vec::new();
    let mut options = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix('!') {
            section = name.trim();
            continue;
        }
        let line = line.trim();
        let Some((name, description)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let description = description.trim();
        match section {
            "layout" => layouts.push(Layout {
                code: name.to_string(),
                description: description.to_string(),
            }),
            "variant" => {
                let Some((layout, description)) = description.split_once(": ") else {
                    continue;
                };
                variants.push((
                    layout.to_string(),
                    Layout {
                        code: format!("{layout}({name})"),
                        description: description.to_string(),
                    },
                ));
            }
            // A group's own header line (`grp  Switching to another layout`)
            // has no colon and is not an option.
            "option" if name.contains(':') => options.push(XkbOption {
                name: name.to_string(),
                description: description.to_string(),
            }),
            _ => {}
        }
    }
    let mut ordered = Vec::with_capacity(layouts.len() + variants.len());
    for layout in layouts {
        let code = layout.code.clone();
        ordered.push(layout);
        let (mine, rest): (Vec<_>, Vec<_>) = variants.into_iter().partition(|(of, _)| *of == code);
        ordered.extend(mine.into_iter().map(|(_, v)| v));
        variants = rest;
    }
    ordered.extend(variants.into_iter().map(|(_, v)| v));
    Catalogue {
        layouts: ordered,
        options,
    }
}

/// `se(dvorak)` → (`se`, `dvorak`); `se` → (`se`, ``).
pub fn split(code: &str) -> (&str, &str) {
    match code.split_once('(') {
        Some((layout, variant)) => (layout, variant.trim_end_matches(')')),
        None => (code, ""),
    }
}

fn rules_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(root) = std::env::var_os("XKB_CONFIG_ROOT") {
        candidates.push(PathBuf::from(root).join("rules/evdev.lst"));
    }
    if let Some(built) = option_env!("SWAYPPLET_XKB_RULES") {
        candidates.push(PathBuf::from(built));
    }
    candidates.push(PathBuf::from("/usr/share/X11/xkb/rules/evdev.lst"));
    candidates.push(PathBuf::from(
        "/run/current-system/sw/share/X11/xkb/rules/evdev.lst",
    ));
    candidates.into_iter().find(|p| p.is_file())
}

/// The catalogue, read once per process on first use (47 kB of text, a
/// millisecond), and only when the Input tab is built.
pub fn catalogue() -> &'static Catalogue {
    static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
    CATALOGUE.get_or_init(|| match rules_path() {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(text) => parse(&text),
            Err(e) => {
                log::warn!("input: cannot read {}: {e}", path.display());
                Catalogue::default()
            }
        },
        None => {
            log::info!("input: no xkb rules/evdev.lst; the layout picker is empty");
            Catalogue::default()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
! model
  pc105           Generic 105-key PC

! layout
  us              English (US)
  se              Swedish

! variant
  chr             us: Cherokee
  dvorak          se: Swedish (Dvorak)
  nodeadkeys      se: Swedish (no dead keys)
  intl            zz: Orphan (intl.)

! option
  grp                  Switching to another layout
  grp:win_space_toggle Win+Space
  grp:alt_shift_toggle Alt+Shift
  caps:escape          Make Caps Lock an additional Esc
  ctrl:grouptoggle_capscontrol Caps Lock as Ctrl,  Left Control switches to another layout
";

    #[test]
    fn layouts_come_with_their_variants_after_them() {
        let c = parse(SAMPLE);
        let codes: Vec<&str> = c.layouts.iter().map(|l| l.code.as_str()).collect();
        assert_eq!(
            codes,
            [
                "us",
                "us(chr)",
                "se",
                "se(dvorak)",
                "se(nodeadkeys)",
                "zz(intl)"
            ]
        );
        assert_eq!(c.describe("se(dvorak)"), Some("Swedish (Dvorak)"));
        assert_eq!(c.code_of("Swedish"), Some("se"));
        // No model leaked into the layouts.
        assert_eq!(c.describe("pc105"), None);
    }

    #[test]
    fn options_skip_the_group_headers_and_keep_a_name_past_the_column() {
        let c = parse(SAMPLE);
        let grp: Vec<&str> = c.options_in("grp").map(|o| o.name.as_str()).collect();
        assert_eq!(grp, ["grp:win_space_toggle", "grp:alt_shift_toggle"]);
        let long = c
            .options
            .iter()
            .find(|o| o.name == "ctrl:grouptoggle_capscontrol")
            .unwrap();
        assert_eq!(
            long.description,
            "Caps Lock as Ctrl,  Left Control switches to another layout"
        );
        assert_eq!(c.options_in("caps").count(), 1);
    }

    #[test]
    fn search_matches_every_word_in_code_or_description() {
        let c = parse(SAMPLE);
        let hits = |q| c.search(q).map(|l| l.code.clone()).collect::<Vec<_>>();
        assert_eq!(hits("swedish dvorak"), ["se(dvorak)"]);
        assert_eq!(hits("SE("), ["se(dvorak)", "se(nodeadkeys)"]);
        assert_eq!(hits("").len(), 6);
    }

    #[test]
    fn a_code_splits_into_layout_and_variant() {
        assert_eq!(split("se"), ("se", ""));
        assert_eq!(split("us(dvorak)"), ("us", "dvorak"));
    }

    /// The real file, when this machine has one: the parse must not come
    /// back empty on the format the package ships.
    #[test]
    fn the_installed_rules_parse_when_present() {
        let Some(path) = rules_path() else { return };
        let c = parse(&std::fs::read_to_string(path).unwrap());
        assert!(c.layouts.len() > 500, "{}", c.layouts.len());
        assert!(c.describe("se").is_some());
        assert!(c.options_in("grp").count() > 10);
    }
}
