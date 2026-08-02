//! Workspaces module — one button per sway workspace, driven by the
//! [`SwayService`] observer. Every bar shows all workspaces regardless of
//! output (waybar's `all-outputs = true`).

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::sway_ipc::{self, SwayService, WorkspaceInfo};

// Label tables — mirror users/modules/workspace-config.nix (nixos repo):
// nums 1–16 are 4 tasks × 4 screens rendered "1¹".."4⁴" behind a
// task-colored dot (`.bar-ws-dot.taskN`; colors live in data/style.css
// beside the accent-ripple rules), nums 17–29 the generic keyed
// workspaces. Keep in lockstep with that file.
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
    // also fires for pid-map-only snapshots (window churn), which must not
    // rebuild widgets mid-hover.
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
        let btn = gtk4::Button::builder()
            .css_classes(["bar-ws"])
            .child(&label_widget(ws.num, &ws.name))
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

/// Button content: task workspaces get a task-colored dot beside the
/// label, everything else a plain label.
fn label_widget(num: i32, name: &str) -> gtk4::Widget {
    let Some((task, text)) = task_label(num) else {
        return gtk4::Label::new(Some(generic_label(num, name))).upcast();
    };
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(4)
        .build();
    let dot = gtk4::Label::new(Some("●"));
    dot.add_css_class("bar-ws-dot");
    dot.add_css_class(&format!("task{task}"));
    row.append(&dot);
    row.append(&gtk4::Label::new(Some(&text)));
    row.upcast()
}

/// Task strip membership: `Some((task 1–4, "1¹".."4⁴"))` for nums 1–16.
fn task_label(num: i32) -> Option<(usize, String)> {
    if !(1..=16).contains(&num) {
        return None;
    }
    let task = ((num - 1) / 4) as usize + 1;
    let screen = ((num - 1) % 4) as usize;
    Some((task, format!("{task}{}", TASK_SUPERSCRIPTS[screen])))
}

/// Label for non-task workspaces.
fn generic_label(num: i32, name: &str) -> &str {
    match GENERIC_LABELS.iter().find(|(n, _)| *n == num) {
        Some((_, label)) => label,
        // Waybar's default icon was blank; the name keeps ad-hoc
        // workspaces visible instead of rendering an empty button.
        None => name,
    }
}

/// Numbered workspaces switch by number, so "5:t2a" and a bare "5" resolve
/// to the same target (matches waybar and the sway keybindings);
/// named-only ones by quoted name. Also the popover session rows' focus
/// path (bar/popover.rs).
pub(crate) fn switch_command(num: i32, name: &str) -> String {
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
        assert_eq!(generic_label(17, "17:wb"), "󰖟 b");
        assert_eq!(generic_label(29, "29:wy"), "󰗃 y");
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
