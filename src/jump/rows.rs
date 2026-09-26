//! One row per place: the workspaces the switcher can go to, each with its
//! caption (the chord that reaches it, the bar's label, what is on it) and
//! the command that switches to it.

use crate::bar::workspaces::generic_label;
use crate::keybinds::Binding;

use super::place::Place;

// ── Geometry ────────────────────────────────────────────────────────────

/// A workspace picture, 16:10 like the panel: the bar's peek draws one this
/// size. A workspace on a portrait or ultrawide output is letterboxed inside
/// it (`scene::fit`).
pub const PREVIEW_W: i32 = 540;
pub const PREVIEW_H: i32 = 338;

/// The list stops here. Nothing is lost by it: every workspace on this machine
/// is one direct chord away, and the chord column on each row says which. A
/// ninth row would be a place you have not visited in eight switches, which is
/// not what "back to what I was doing" means.
pub const MAX_ROWS: usize = 8;

/// How many rows a ring of `places` produces.
///
/// `places[0]` is where you are and is never drawn, so a ring of one produces
/// no rows at all - which is what makes `Super+Tab` a silent no-op on a fresh
/// session rather than a switcher with nothing in it. `rows` applies the same
/// rule; this is its statement for the tests.
#[cfg(test)]
pub fn row_count(places: usize) -> usize {
    places.saturating_sub(1).min(MAX_ROWS)
}

// ── A row ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The key that reaches this place directly, without this surface.
    /// `None` when no plain-Super binding targets it.
    pub chord: Option<String>,
    /// The bar's own label, so the two surfaces name a workspace identically.
    pub label: String,
    /// What is on it: distinct app names, or "empty".
    pub detail: String,
    pub windows: usize,
    /// On a screen other than the one the switcher is drawn on.
    pub other_output: bool,
    /// What commit runs.
    pub command: String,
}

/// Build the rows for a ring, skipping the head.
pub fn rows(
    places: &[Place],
    bindings: &[Binding],
    apps: &dyn Fn(&str) -> Vec<String>,
    focused_output: &str,
) -> Vec<Row> {
    places
        .iter()
        .skip(1)
        .take(MAX_ROWS)
        .map(|p| row(p, bindings, apps, focused_output))
        .collect()
}

/// One place's row. For the head too, which the switcher can walk back to.
pub fn row(
    p: &Place,
    bindings: &[Binding],
    apps: &dyn Fn(&str) -> Vec<String>,
    focused_output: &str,
) -> Row {
    let names = apps(&p.name);
    Row {
        chord: chord_for(bindings, p),
        label: label_for(p),
        detail: detail_for(&names),
        windows: names.len(),
        other_output: p.output != focused_output,
        command: crate::bar::workspaces::switch_command(p.num, &p.name),
    }
}

/// The label under a tile, with the chord taken out when the label ends in
/// it. The bar's generic labels are a glyph and the workspace's key
/// (`"󰗃 y"`), and the tile already prints the key in its badge, so the caption
/// would say it twice. A task label (`"1¹"`) keeps its screen mark.
pub fn caption_label(row: &Row) -> &str {
    let Some(chord) = row.chord.as_deref() else {
        return &row.label;
    };
    match row.label.strip_suffix(chord) {
        Some(rest) if rest.is_empty() || rest.ends_with(' ') => rest.trim_end(),
        _ => &row.label,
    }
}

/// The label the bar would draw for this workspace.
pub(crate) fn label_for(p: &Place) -> String {
    // Task workspaces are 1..=16 by the table's own numbering; everything else
    // takes the generic table's glyph, falling back to the raw name.
    if (1..=16).contains(&p.num) {
        let task = ((p.num - 1) / 4) + 1;
        let screen = ((p.num - 1) % 4) as usize;
        const SUP: [&str; 4] = ["\u{00b9}", "\u{00b2}", "\u{00b3}", "\u{2074}"];
        format!("{task}{}", SUP[screen])
    } else {
        generic_label(p.num, &p.name).to_string()
    }
}

/// The one key that reaches this place directly.
///
/// Plain Super only. `Super+Shift+<key>` moves a container to that workspace,
/// which is a different verb, and printing it here would teach the wrong
/// chord to anyone reading the column to learn them.
fn chord_for(bindings: &[Binding], p: &Place) -> Option<String> {
    bindings
        .iter()
        .find(|b| b.mods == ["Mod4"] && targets(&b.command, p))
        .map(|b| b.key.clone())
}

/// Does this binding's command land on this workspace?
///
/// Two shapes, because this repo binds task workspaces through a script and
/// generic ones natively: `workspace number 24` and
/// `exec /nix/store/…-task-switch 9:t3a`. Matching on the trailing token
/// rather than on a substring, so `workspace number 2` does not claim `24`.
fn targets(command: &str, p: &Place) -> bool {
    let last = command.split_whitespace().next_back().unwrap_or("");
    if last == p.name {
        return true;
    }
    if p.num >= 0 && command.contains("workspace number") {
        return last == p.num.to_string();
    }
    false
}

/// Distinct app names, in order, at most three plus an overflow count.
fn detail_for(apps: &[String]) -> String {
    if apps.is_empty() {
        return "empty".to_string();
    }
    let mut seen: Vec<&str> = Vec::new();
    for a in apps {
        let pretty = pretty_app(a);
        if !seen.contains(&pretty) {
            seen.push(pretty);
        }
    }
    let shown: Vec<&str> = seen.iter().take(3).copied().collect();
    let rest = seen.len().saturating_sub(shown.len());
    if rest > 0 {
        format!("{} +{rest}", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

/// A readable name for an app_id.
///
/// Chrome PWAs arrive as `chrome-<32 hex>-Profile_1`, which is neither
/// readable nor distinguishable from the next one; they all become "Chrome"
/// rather than each becoming its own unreadable column.
fn pretty_app(app_id: &str) -> &str {
    if app_id.starts_with("chrome-") || app_id == "google-chrome" {
        return "Chrome";
    }
    match app_id {
        "Alacritty" => "Terminal",
        "" => "window",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(num: i32, name: &str, output: &str) -> Place {
        Place {
            num,
            name: name.into(),
            output: output.into(),
        }
    }

    fn ring() -> Vec<Place> {
        vec![
            place(9, "9:t3a", "eDP-1"),
            place(5, "5:t2a", "eDP-1"),
            place(24, "24:wg", "eDP-1"),
            place(30, "30:wm", "DP-2"),
        ]
    }

    fn binds() -> Vec<Binding> {
        vec![
            Binding {
                mods: vec!["Mod4".into()],
                key: "q".into(),
                command: "exec /nix/store/abc-task-switch 5:t2a".into(),
            },
            Binding {
                mods: vec!["Mod4".into()],
                key: "g".into(),
                command: "workspace number 24".into(),
            },
            Binding {
                mods: vec!["Mod4".into(), "Shift".into()],
                key: "m".into(),
                command: "move container to workspace number 30".into(),
            },
        ]
    }

    fn no_apps(_: &str) -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn the_list_is_capped_and_the_head_is_never_a_row() {
        assert_eq!(row_count(1), 0, "one place is a silent no-op");
        assert_eq!(row_count(2), 1);
        assert_eq!(row_count(9), 8);
        assert_eq!(row_count(40), MAX_ROWS);
    }

    // ── captions ────────────────────────────────────────────────────────

    fn row(label: &str, chord: Option<&str>) -> Row {
        Row {
            chord: chord.map(str::to_string),
            label: label.to_string(),
            detail: String::new(),
            windows: 0,
            other_output: false,
            command: String::new(),
        }
    }

    #[test]
    fn the_caption_does_not_repeat_the_chord() {
        assert_eq!(caption_label(&row("\u{f05c3} y", Some("y"))), "\u{f05c3}");
        assert_eq!(caption_label(&row("l", Some("l"))), "");
    }

    #[test]
    fn a_task_label_keeps_its_screen_mark() {
        assert_eq!(caption_label(&row("1\u{00b9}", Some("1"))), "1\u{00b9}");
    }

    #[test]
    fn a_label_that_only_contains_the_chord_elsewhere_stays_whole() {
        assert_eq!(caption_label(&row("key", Some("y"))), "key");
        assert_eq!(caption_label(&row("scratch", None)), "scratch");
    }

    // ── rows ────────────────────────────────────────────────────────────

    #[test]
    fn the_first_row_is_where_you_were() {
        let r = rows(&ring(), &binds(), &no_apps, "eDP-1");
        assert_eq!(r[0].label, "2\u{00b9}");
        assert_eq!(r[0].chord.as_deref(), Some("q"));
    }

    #[test]
    fn the_chord_column_prints_the_key_that_makes_this_surface_unnecessary() {
        let r = rows(&ring(), &binds(), &no_apps, "eDP-1");
        assert_eq!(
            r[1].chord.as_deref(),
            Some("g"),
            "generic workspace binding"
        );
        assert_eq!(r[2].chord, None, "no plain-Super binding targets 30:wm");
    }

    #[test]
    fn a_move_container_binding_is_not_offered_as_a_chord() {
        // Mod4+Shift+m targets 30:wm but moves a window there. Printing it
        // would teach a chord that does something else entirely.
        let r = rows(&ring(), &binds(), &no_apps, "eDP-1");
        assert_eq!(r[2].chord, None);
    }

    #[test]
    fn workspace_number_matching_does_not_confuse_a_prefix() {
        let b = vec![Binding {
            mods: vec!["Mod4".into()],
            key: "2".into(),
            command: "workspace number 2".into(),
        }];
        let p = place(24, "24:wg", "eDP-1");
        assert_eq!(chord_for(&b, &p), None, "`number 2` must not claim 24");
    }

    #[test]
    fn a_place_on_another_screen_is_marked() {
        let r = rows(&ring(), &binds(), &no_apps, "eDP-1");
        assert!(!r[0].other_output);
        assert!(r[2].other_output, "30:wm lives on DP-2");
    }

    #[test]
    fn the_detail_names_what_is_there() {
        let apps = |ws: &str| match ws {
            "5:t2a" => vec!["Alacritty".into(), "Alacritty".into()],
            "24:wg" => vec!["google-chrome".into(), "slack".into()],
            _ => vec![],
        };
        let r = rows(&ring(), &binds(), &apps, "eDP-1");
        assert_eq!(r[0].detail, "Terminal", "duplicates collapse");
        assert_eq!(r[1].detail, "Chrome, slack");
        assert_eq!(r[2].detail, "empty");
    }

    #[test]
    fn chrome_pwas_do_not_each_become_their_own_unreadable_column() {
        let apps = |_: &str| {
            vec![
                "chrome-cifhbcnohmdccbgoicgdjpfamggdegmo-Profile_1".to_string(),
                "google-chrome".to_string(),
            ]
        };
        let r = rows(&ring(), &binds(), &apps, "eDP-1");
        assert_eq!(r[0].detail, "Chrome");
    }

    #[test]
    fn the_detail_never_runs_away() {
        let apps = |_: &str| (0..30).map(|i| format!("app{i}")).collect();
        let r = rows(&ring(), &binds(), &apps, "eDP-1");
        assert!(r[0].detail.ends_with("+27"), "got {}", r[0].detail);
    }

    #[test]
    fn commit_targets_the_workspace_by_number_when_it_has_one() {
        let r = rows(&ring(), &binds(), &no_apps, "eDP-1");
        assert_eq!(r[0].command, "workspace number 5");
    }

    #[test]
    fn a_ring_of_one_produces_no_rows() {
        let one = vec![place(9, "9:t3a", "eDP-1")];
        assert!(rows(&one, &binds(), &no_apps, "eDP-1").is_empty());
    }
}
