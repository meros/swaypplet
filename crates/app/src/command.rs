//! What a `swaypplet <word> …` client asks the running panel for.
//!
//! Every invocation of the binary that is not one of `main.rs`'s own
//! subcommands reaches the panel process as a command line, over D-Bus.
//! This reads it; `run` does what it says.

use swaypplet_osd::OsdCommand;

/// One request to the panel.
#[derive(Debug, Clone, PartialEq)]
pub enum PanelCommand {
    /// No word: activate, which shows or hides the panel.
    Activate,
    /// `toggle`: Super+Space (the nixos repo's sway.nix). The same as
    /// [`PanelCommand::Activate`], named.
    Toggle,
    Launcher,
    Jump(JumpAction),
    /// `label …`: the rest of the words, for `swaypplet_label`.
    Label(Vec<String>),
    Pin(PinAction),
    /// `screenshot [kind]`, for `Shot::parse`.
    Screenshot(Option<String>),
    /// `report [area]`, for `Area::parse`.
    Report(Option<String>),
    Keybinds(KeybindsAction),
    Osd(OsdCommand),
    /// `osd` with words it does not know.
    BadOsd(Vec<String>),
    /// A word the panel does not know. It activates, as no word does.
    Unknown(String),
}

/// `jump …`: Super+Tab and the keys of sway's `swaypplet-jump` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpAction {
    Step,
    StepBack,
    /// From the mode, which holds `Super+Escape` and `Super+p` while the
    /// card is up.
    Cancel,
    Pin,
}

/// `pin …`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PinAction {
    /// The focused window.
    Focused,
    /// `pin region`: drag a rectangle on the frozen screen.
    SelectRegion,
    /// `pin region x,y,w,h`: that rectangle on the focused output, no
    /// selector. For scripts, and for the harness, which has no pointer to
    /// drag with.
    Region((f64, f64, f64, f64)),
}

/// `keybinds …`. The overlay is driven by key press and release, so the
/// client says which edge it saw rather than asking for a toggle it cannot
/// reason about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeybindsAction {
    Show,
    Hide,
    Reload,
    Toggle,
}

impl PanelCommand {
    /// Read the words after the program's name.
    pub fn parse(words: &[String]) -> PanelCommand {
        let arg = |i: usize| words.get(i).map(String::as_str);
        let Some(word) = arg(0) else {
            return PanelCommand::Activate;
        };
        match word {
            "launcher" => PanelCommand::Launcher,
            "jump" => PanelCommand::Jump(match arg(1) {
                Some("step-back") => JumpAction::StepBack,
                Some("cancel") => JumpAction::Cancel,
                Some("pin") => JumpAction::Pin,
                _ => JumpAction::Step,
            }),
            "label" => PanelCommand::Label(words[1..].to_vec()),
            "pin" => PanelCommand::Pin(match arg(1) {
                Some("region") => arg(2)
                    .and_then(parse_area)
                    .map_or(PinAction::SelectRegion, PinAction::Region),
                _ => PinAction::Focused,
            }),
            "screenshot" => PanelCommand::Screenshot(arg(1).map(str::to_string)),
            "report" => PanelCommand::Report(arg(1).map(str::to_string)),
            "keybinds" => PanelCommand::Keybinds(match arg(1).unwrap_or("toggle") {
                "show" => KeybindsAction::Show,
                "hide" => KeybindsAction::Hide,
                "reload" => KeybindsAction::Reload,
                _ => KeybindsAction::Toggle,
            }),
            "osd" => {
                let rest = &words[1..];
                OsdCommand::parse(rest)
                    .map_or_else(|| PanelCommand::BadOsd(rest.to_vec()), PanelCommand::Osd)
            }
            "toggle" => PanelCommand::Toggle,
            other => PanelCommand::Unknown(other.to_string()),
        }
    }
}

/// `x,y,w,h`, each a number.
fn parse_area(text: &str) -> Option<(f64, f64, f64, f64)> {
    let v: Vec<f64> = text
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    (v.len() == 4).then(|| (v[0], v[1], v[2], v[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> PanelCommand {
        let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        PanelCommand::parse(&words)
    }

    #[test]
    fn each_word_is_its_request() {
        assert_eq!(parse(""), PanelCommand::Activate);
        assert_eq!(parse("toggle"), PanelCommand::Toggle);
        assert_eq!(parse("launcher"), PanelCommand::Launcher);
        assert_eq!(parse("jump"), PanelCommand::Jump(JumpAction::Step));
        assert_eq!(
            parse("jump step-back"),
            PanelCommand::Jump(JumpAction::StepBack)
        );
        assert_eq!(parse("jump cancel"), PanelCommand::Jump(JumpAction::Cancel));
        assert_eq!(parse("jump pin"), PanelCommand::Jump(JumpAction::Pin));
        assert_eq!(parse("jump what"), PanelCommand::Jump(JumpAction::Step));
        assert_eq!(
            parse("label show 3"),
            PanelCommand::Label(vec!["show".into(), "3".into()])
        );
        assert_eq!(parse("label"), PanelCommand::Label(Vec::new()));
        assert_eq!(parse("screenshot"), PanelCommand::Screenshot(None));
        assert_eq!(
            parse("screenshot region"),
            PanelCommand::Screenshot(Some("region".into()))
        );
        assert_eq!(
            parse("report window"),
            PanelCommand::Report(Some("window".into()))
        );
        assert_eq!(
            parse("keybinds"),
            PanelCommand::Keybinds(KeybindsAction::Toggle)
        );
        assert_eq!(
            parse("keybinds show"),
            PanelCommand::Keybinds(KeybindsAction::Show)
        );
        assert_eq!(
            parse("keybinds hide"),
            PanelCommand::Keybinds(KeybindsAction::Hide)
        );
        assert_eq!(
            parse("keybinds reload"),
            PanelCommand::Keybinds(KeybindsAction::Reload)
        );
        assert_eq!(
            parse("frobnicate"),
            PanelCommand::Unknown("frobnicate".into())
        );
    }

    #[test]
    fn pin_takes_a_rectangle_or_asks_for_one() {
        assert_eq!(parse("pin"), PanelCommand::Pin(PinAction::Focused));
        assert_eq!(
            parse("pin region"),
            PanelCommand::Pin(PinAction::SelectRegion)
        );
        assert_eq!(
            parse("pin region 10,20,300,200.5"),
            PanelCommand::Pin(PinAction::Region((10.0, 20.0, 300.0, 200.5)))
        );
        // Anything but four numbers is a request to drag one.
        assert_eq!(
            parse("pin region 10,20,300"),
            PanelCommand::Pin(PinAction::SelectRegion)
        );
        assert_eq!(parse("pin other"), PanelCommand::Pin(PinAction::Focused));
    }

    #[test]
    fn osd_words_go_to_the_osd_or_are_reported() {
        assert_eq!(
            parse("osd --output-volume raise"),
            PanelCommand::Osd(OsdCommand::OutputVolumeRaise)
        );
        assert_eq!(
            parse("osd --volume up"),
            PanelCommand::BadOsd(vec!["--volume".into(), "up".into()])
        );
    }
}
