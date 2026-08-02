//! Workspaces module — one button per sway workspace, driven by the
//! [`SwayService`] observer. Every bar shows all workspaces regardless of
//! output (waybar's `all-outputs = true`).

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::sway_ipc::{self, SwayService, WorkspaceInfo};

// Label tables — mirror users/modules/workspace-config.nix (nixos repo):
// nums 1–16 are 4 tasks × 4 screens rendered "1¹".."4⁴" behind a
// task-colored dot (waybar's taskColors, i.e. the style.css accents in
// task order), nums 17–29 the generic keyed workspaces. Keep in lockstep
// with that file.
const TASK_DOT_COLORS: [&str; 4] = ["#689d6a", "#d79921", "#458588", "#b16286"];
const TASK_SUPERSCRIPTS: [&str; 4] = ["¹", "²", "³", "⁴"];
const GENERIC_LABELS: &[(i32, &str)] = &[
    (17, "󰖟 b"),
    (18, "󰊤 g"),
    (19, "h"),
    (20, "i"),
    (21, "j"),
    (22, "k"),
    (23, "󰍡 m"),
    (24, "n"),
    (25, "📧 o"),
    (26, "󰓇 p"),
    (27, "󰓓 t"),
    (28, "󰑴 u"),
    (29, "󰗃 y"),
];

pub fn build(sway: &Rc<SwayService>) -> gtk4::Box {
    let container = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-workspaces"])
        .build();

    // Buttons are rebuilt only when the workspace rows change: the service
    // also fires for title-only snapshots (per keystroke in some
    // terminals), which must not churn widgets mid-hover.
    let cache: Rc<RefCell<Vec<WorkspaceInfo>>> = Rc::new(RefCell::new(Vec::new()));
    let weak = container.downgrade();
    let sway_cb = sway.clone();
    let sync = Rc::new(move || {
        // The observer outlives the widget when its output is unplugged
        // (the service has no disconnect); a dead weak ref makes the
        // leftover callback a no-op.
        let Some(container) = weak.upgrade() else {
            return;
        };
        let mut workspaces = sway_cb.workspaces();
        sort_workspaces(&mut workspaces);
        if *cache.borrow() == workspaces {
            return;
        }
        rebuild(&container, &workspaces);
        *cache.borrow_mut() = workspaces;
    });
    sync();
    let sync_cb = sync.clone();
    sway.connect_change(move || sync_cb());

    container
}

fn rebuild(container: &gtk4::Box, workspaces: &[WorkspaceInfo]) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    for ws in workspaces {
        let label = gtk4::Label::new(None);
        label.set_markup(&label_markup(ws.num, &ws.name));
        let btn = gtk4::Button::builder()
            .css_classes(["bar-ws"])
            .child(&label)
            .build();
        if ws.focused {
            btn.add_css_class("focused");
        }
        if ws.visible {
            btn.add_css_class("visible");
        }
        if ws.urgent {
            btn.add_css_class("urgent");
        }
        let cmd = switch_command(ws.num, &ws.name);
        btn.connect_clicked(move |_| sway_ipc::run_command(&cmd));
        container.append(&btn);
    }
}

// ── Pure helpers (unit-tested below) ────────────────────────────────────

/// Numbered workspaces in numeric order, named-only ones (num -1) last,
/// alphabetically — matches waybar's default sort.
fn sort_workspaces(list: &mut [WorkspaceInfo]) {
    list.sort_by(|a, b| (a.num < 0, a.num, &a.name).cmp(&(b.num < 0, b.num, &b.name)));
}

/// Pango markup for a workspace button label.
fn label_markup(num: i32, name: &str) -> String {
    if (1..=16).contains(&num) {
        let task = ((num - 1) / 4) as usize;
        let screen = ((num - 1) % 4) as usize;
        return format!(
            "<span foreground='{}'>●</span> {}{}",
            TASK_DOT_COLORS[task],
            task + 1,
            TASK_SUPERSCRIPTS[screen]
        );
    }
    match GENERIC_LABELS.iter().find(|(n, _)| *n == num) {
        Some((_, label)) => (*label).to_string(),
        // Waybar's default icon was blank; the name keeps ad-hoc
        // workspaces visible instead of rendering an empty button.
        None => glib::markup_escape_text(name).to_string(),
    }
}

/// Numbered workspaces switch by number, so "5:t2a" and a bare "5" resolve
/// to the same target (matches waybar and the sway keybindings);
/// named-only ones by quoted name.
fn switch_command(num: i32, name: &str) -> String {
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

    fn ws(num: i32, name: &str) -> WorkspaceInfo {
        WorkspaceInfo {
            num,
            name: name.into(),
            output: "eDP-1".into(),
            focused: false,
            urgent: false,
            visible: false,
        }
    }

    #[test]
    fn task_labels_carry_dot_and_superscript() {
        assert_eq!(
            label_markup(1, "1:t1a"),
            "<span foreground='#689d6a'>●</span> 1¹"
        );
        assert_eq!(
            label_markup(16, "16:t4d"),
            "<span foreground='#b16286'>●</span> 4⁴"
        );
        // Task boundary: num 5 is task 2, screen 1.
        assert_eq!(
            label_markup(5, "5:t2a"),
            "<span foreground='#d79921'>●</span> 2¹"
        );
    }

    #[test]
    fn generic_labels_come_from_the_table() {
        assert_eq!(label_markup(17, "17:wb"), "󰖟 b");
        assert_eq!(label_markup(29, "29:wy"), "󰗃 y");
    }

    #[test]
    fn unknown_workspaces_fall_back_to_the_escaped_name() {
        assert_eq!(label_markup(-1, "a<b&c"), "a&lt;b&amp;c");
        assert_eq!(label_markup(42, "42"), "42");
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
    fn sort_is_numeric_with_named_last() {
        let mut list = vec![
            ws(10, "10:t3b"),
            ws(-1, "mail"),
            ws(2, "2:t1b"),
            ws(-1, "chat"),
        ];
        sort_workspaces(&mut list);
        let order: Vec<&str> = list.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(order, ["2:t1b", "10:t3b", "chat", "mail"]);
    }
}
