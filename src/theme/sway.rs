//! sway's window borders, from the tokens.
//!
//! The borders are the compositor's, so they are sent over IPC rather than
//! styled: `client.*` commands built from the same scales as the stylesheet,
//! so they follow the mode, the accent, the neutral and the tint. nixos
//! `users/modules/sway.nix` sets the dark gruvbox ones in the config, which
//! is what a session shows before the panel is up.

use crate::tokens::{self, Inputs, Rgb};

/// The four `client.*` lines, mapped by meaning:
///
/// - **focused**: `--accent-bg`, with `--on-accent` as its title text: the
///   window that is "on".
/// - **focused_inactive**: the raised surface (`--neutral-4`) with the
///   secondary text step (`--neutral-11`) as border and text: focused on
///   another output, present but quiet.
/// - **unfocused**: the raised surface, secondary text, and the ground
///   (`--neutral-3`) as the split indicator.
/// - **urgent**: `--danger-bg` with `--on-status`.
///
/// Order is sway's: border, background, text, indicator, child_border.
pub fn client_colors(inputs: Inputs) -> Vec<String> {
    let s = tokens::scales(inputs);
    let st = tokens::status(inputs);
    let (ground, raised, muted) = (s.neutral[2], s.neutral[3], s.neutral[10]);
    let line = |class: &str, c: [Rgb; 5]| {
        let [a, b, c, d, e] = c.map(Rgb::css);
        format!("client.{class} {a} {b} {c} {d} {e}")
    };
    let (accent, on) = (s.accent_bg, s.on_accent);
    let (urgent, on_urgent) = (st.danger_bg, tokens::ON_STATUS);
    vec![
        line("focused", [accent, accent, on, accent, accent]),
        line("focused_inactive", [muted, raised, muted, muted, muted]),
        line("unfocused", [raised, raised, muted, ground, raised]),
        line("urgent", [urgent, urgent, on_urgent, urgent, urgent]),
    ]
}

/// Put the tokens on sway's borders.
pub fn apply_borders(inputs: Inputs) {
    crate::sway::ipc::run_commands(client_colors(inputs));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_border_commands_are_sways_five_fields() {
        let commands = client_colors(Inputs::default());
        assert_eq!(commands.len(), 4);
        for command in &commands {
            let words: Vec<&str> = command.split_whitespace().collect();
            assert_eq!(words.len(), 6, "{command}");
            assert!(words[0].starts_with("client."));
            for hex in &words[1..] {
                assert!(hex.len() == 7 && hex.starts_with('#'), "{command}");
            }
        }
        // Untinted dark gruvbox aqua: focused is gruvbox's aqua, which is
        // what sway.nix sets by hand.
        assert!(commands[0].starts_with("client.focused #689d6a #689d6a"));
    }

    #[test]
    fn the_borders_follow_the_tint() {
        let tinted = client_colors(Inputs {
            tint: tokens::Tint::Accents(265),
            ..Inputs::default()
        });
        assert_ne!(tinted[0], client_colors(Inputs::default())[0]);
    }
}
