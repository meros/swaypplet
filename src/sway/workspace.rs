//! What a sway workspace is called, and how to go to it: the labels the bar
//! draws, the Super+Tab rows print and the keybinding sheet shows, and the
//! command every surface switches with. Shared domain data, so it lives with
//! sway rather than with any one surface that draws it.

// Label tables — mirror users/modules/workspace-config.nix (nixos repo):
// nums 1–16 are the task grid, rendered "1¹".."4⁴" behind a dot in the
// task's categorical tone (`ui::set_category`), nums 17–38 the generic
// keyed workspaces. Keep in lockstep with that file.
//
// The grid is 4 tasks × 2 screens as of 2026-08-10; the superscript table
// still runs to four because the `num` spacing (1,2 / 5,6 / 9,10 / 13,14)
// was kept when screens c and d were retired, and a task that grows a
// third screen should render rather than fall back to its raw name.
const TASK_SUPERSCRIPTS: [&str; 4] = ["¹", "²", "³", "⁴"];
const GENERIC_LABELS: &[(i32, &str)] = &[
    (17, "3"),
    (18, "4"),
    (19, "󰖟 b"),
    (20, "󰃭 c"),
    (21, "󰈙 d"),
    (22, "e"),
    (23, "󰉋 f"),
    (24, "󰊤 g"),
    (25, "h"),
    (26, "i"),
    (27, "j"),
    (28, "k"),
    (29, "l"),
    (30, "󰍡 m"),
    (31, "󱄅 n"),
    (32, "📧 o"),
    (33, "󰓇 p"),
    (34, "󰜎 r"),
    (35, "󰓓 t"),
    (36, "󰑴 u"),
    (37, "󰕧 v"),
    (38, "󰗃 y"),
];

/// Task strip membership: `Some((task 1–4, "1¹".."4⁴"))` for nums 1–16.
pub fn task_label(num: i32) -> Option<(usize, String)> {
    if !(1..=16).contains(&num) {
        return None;
    }
    let task = ((num - 1) / 4) as usize + 1;
    let screen = ((num - 1) % 4) as usize;
    Some((task, format!("{task}{}", TASK_SUPERSCRIPTS[screen])))
}

/// Label for non-task workspaces.
pub fn generic_label(num: i32, name: &str) -> &str {
    match GENERIC_LABELS.iter().find(|(n, _)| *n == num) {
        Some((_, label)) => label,
        // Waybar's default icon was blank; the name keeps ad-hoc
        // workspaces visible instead of rendering an empty button.
        None => name,
    }
}

/// The label the bar draws for a workspace, as text: a task workspace's
/// "1¹", anything else its generic label. The Super+Tab rows and the bar's
/// pins popover print it, so every surface names a workspace identically.
pub fn label_text(num: i32, name: &str) -> String {
    match task_label(num) {
        Some((_, text)) => text,
        None => generic_label(num, name).to_string(),
    }
}

/// [`label_text`] for a workspace known only by name: the number is the
/// leading field of the name ("24:wg"), and a name without one is a named-only
/// workspace. The pins, which remember workspaces by name, label them with it.
pub fn label_for_name(name: &str) -> String {
    let num = name
        .split(':')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(-1);
    label_text(num, name)
}

/// Numbered workspaces switch by number, so "5:t2a" and a bare "5" resolve
/// to the same target (matches waybar and the sway keybindings);
/// named-only ones by quoted name. The bar's buttons, the popover session
/// rows' focus path (bar/popover.rs) and the Super+Tab rows all use it.
pub fn switch_command(num: i32, name: &str) -> String {
    if num >= 0 {
        format!("workspace number {num}")
    } else {
        let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
        format!("workspace \"{escaped}\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_workspaces_carry_task_index_and_superscript() {
        assert_eq!(task_label(1), Some((1, "1¹".into())));
        assert_eq!(task_label(16), Some((4, "4⁴".into())));
        // Task boundary: num 5 is task 2, screen 1.
        assert_eq!(task_label(5), Some((2, "2¹".into())));
        assert_eq!(task_label(17), None);
        assert_eq!(task_label(-1), None);
    }

    #[test]
    fn generic_labels_come_from_the_table() {
        assert_eq!(generic_label(19, "19:wb"), "󰖟 b");
        assert_eq!(generic_label(38, "38:wy"), "󰗃 y");
        assert_eq!(generic_label(34, "34:wr"), "󰜎 r");
        assert_eq!(generic_label(29, "29:wl"), "l");
        assert_eq!(generic_label(17, "17:w3"), "3");
    }

    #[test]
    fn unknown_workspaces_fall_back_to_the_name() {
        assert_eq!(generic_label(-1, "mail"), "mail");
        assert_eq!(generic_label(42, "42"), "42");
    }

    #[test]
    fn switch_command_targets_number_or_quoted_name() {
        assert_eq!(switch_command(5, "5:t2a"), "workspace number 5");
        assert_eq!(switch_command(-1, "mail"), "workspace \"mail\"");
        assert_eq!(
            switch_command(-1, "we\"ird\\ws"),
            "workspace \"we\\\"ird\\\\ws\""
        );
    }

    #[test]
    fn label_text_is_the_task_mark_or_the_generic_label() {
        assert_eq!(label_text(2, "2:t1b"), "1\u{00b2}");
        assert_eq!(label_text(29, "29:wl"), "l");
        assert_eq!(label_text(-1, "mail"), "mail");
        assert_eq!(label_for_name("2:t1b"), "1\u{00b2}");
        assert_eq!(label_for_name("29:wl"), "l");
        assert_eq!(label_for_name("mail"), "mail");
    }
}
