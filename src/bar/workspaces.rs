//! Workspaces module — one button per sway workspace, driven by the
//! [`SwayService`] observer.
//!
//! # The strip's vocabulary
//!
//! Two levels, and every bar draws them identically — the strip states
//! the world, not the bar's own point of view, so both screens agree on
//! what they show.
//!
//! **Groups are screens.** [`group_by_output`] splits the strip into one
//! fused pill per output, ordered the way the monitors stand on the desk.
//! Each pill is a segmented control in the classic sense (Apple HIG:
//! mutually exclusive segments, exactly one selected), and the selected
//! segment is what that screen is showing. The pill for the screen
//! *without* input focus dims as a whole — the container-level cue iTerm2
//! uses for unfocused split panes and VS Code for unfocused editors. It
//! costs no new shape and scales to any number of screens. One screen
//! means one pill, always undimmed: exactly the pre-grouping look.
//!
//! **Buttons are workspaces** ([`WsState`]): idle, `current` (this
//! screen's selected segment), or `focused` (`current`, and its screen
//! holds input). Focus is a step on the `current` mark, never a mark of
//! its own — the inactive-selection convention (VS Code's
//! list.activeSelection vs list.inactiveSelection). It is redundant with
//! the group dimming on purpose: two channels for the single most-read
//! state in the bar.
//!
//! Everything here is achromatic. Hue belongs to task identity — the dot
//! and the ribbon — and a cursor that also carried hue would put two
//! meanings on one channel (vision: position and numeral first, hue as
//! reinforcement only).
//!
//! Task ribbons (docs/BAR_VISION.md, increment 10): a 2 px bottom lane
//! under each task's workspace group from [`TaskStateService`] — off = no
//! session, dim solid = working, task hue = waiting. The buttons are
//! fused segments, so per-button borders read as one ribbon across the
//! group. Selection uses the top lane, so the two never collide.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::sway_ipc::{self, OutputInfo, SwayService, WorkspaceInfo};
use crate::task_state::{Activity, TaskSnapshot, TaskStateService};

/// Gap between per-screen group pills. Wide enough to read as separate
/// pills, narrow enough that the strip stays one cluster.
const GROUP_GAP_PX: i32 = 8;

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

pub fn build(sway: &Rc<SwayService>, tasks: &Rc<TaskStateService>) -> gtk4::Box {
    // Holder for the per-screen group pills; the fused-segment styling
    // lives on the groups inside it.
    let container = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(GROUP_GAP_PX)
        .css_classes(["bar-ws-groups"])
        .build();

    // Buttons are rebuilt only when the workspace rows, the output
    // placement or the task ribbons change: the service also fires for
    // pid-map-only snapshots (window churn), which must not rebuild
    // widgets mid-hover. Ribbon state joins the key safely — task
    // transitions are a few per hour (vision). Occupancy ticks stay
    // backlog: window-event data in this key would defeat the
    // anti-shimmer gate it exists for.
    type CacheKey = (Vec<WorkspaceInfo>, Vec<OutputInfo>, [Ribbon; 4]);
    let cache: Rc<RefCell<CacheKey>> = Rc::new(RefCell::new(<_>::default()));
    let weak = container.downgrade();
    let sway_cb = sway.clone();
    let tasks_cb = tasks.clone();
    let sync = Rc::new(move || {
        // The observer outlives the widget when its output is unplugged
        // (the service has no disconnect); a dead weak ref makes the
        // leftover callback a no-op.
        let Some(container) = weak.upgrade() else {
            return;
        };
        let snap = sway_cb.snapshot();
        let mut workspaces = snap.workspaces;
        sort_workspaces(&mut workspaces);
        let outputs = snap.outputs;
        let ribbons = ribbon_states(&tasks_cb.snapshot());
        {
            let cache = cache.borrow();
            if cache.0 == workspaces && cache.1 == outputs && cache.2 == ribbons {
                return;
            }
        }
        let groups = group_by_output(&workspaces, &outputs);
        rebuild(&container, &groups, &ribbons);
        *cache.borrow_mut() = (workspaces, outputs, ribbons);
    });
    sync();
    let sync_cb = sync.clone();
    sway.connect_change(move || sync_cb());
    let sync_cb = sync.clone();
    tasks.connect_change(move || sync_cb());

    container
}

fn rebuild(container: &gtk4::Box, groups: &[Group], ribbons: &[Ribbon; 4]) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    for group in groups {
        let pill = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .css_classes(["bar-workspaces"])
            .build();
        if group.active {
            pill.add_css_class("active-screen");
        }
        for ws in &group.workspaces {
            let btn = gtk4::Button::builder()
                .css_classes(["bar-ws"])
                .child(&label_widget(ws.num, &ws.name))
                .build();
            match ws_state(ws) {
                WsState::Idle => {}
                WsState::Current => btn.add_css_class("current"),
                WsState::Focused => {
                    btn.add_css_class("current");
                    btn.add_css_class("focused");
                }
            }
            if ws.urgent {
                btn.add_css_class("urgent");
            }
            if let Some((task, _)) = task_label(ws.num) {
                apply_ribbon(&btn, task, ribbons[task - 1]);
            }
            let cmd = switch_command(ws.num, &ws.name);
            btn.connect_clicked(move |_| sway_ipc::run_command(&cmd));
            pill.append(&btn);
        }
        container.append(&pill);
    }
}

fn apply_ribbon(btn: &gtk4::Button, task: usize, ribbon: Ribbon) {
    match ribbon {
        Ribbon::Off => {}
        Ribbon::Working => btn.add_css_class("ribbon-working"),
        Ribbon::Waiting => {
            btn.add_css_class("ribbon-waiting");
            btn.add_css_class(&format!("task{task}"));
        }
    }
}

// ── Pure helpers (unit-tested below) ────────────────────────────────────

/// One screen's workspaces, rendered as one fused pill.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Group {
    /// sway output name; kept for ordering and for reading tests.
    output: String,
    /// This screen holds input focus, so its pill stays undimmed. Exactly
    /// one group unless sway reports no focused workspace at all (only
    /// seen mid-restart, before the first real snapshot).
    active: bool,
    workspaces: Vec<WorkspaceInfo>,
}

/// Split the strip into one group per screen, ordered the way the screens
/// stand on the desk: left to right, then top to bottom, by the output's
/// layout origin. Workspaces migrate between outputs in sway (nothing is
/// pinned here), so both the grouping and the order follow whatever the
/// current layout says.
///
/// `workspaces` must already be [`sort_workspaces`]-ordered; grouping is
/// stable, so each group keeps that order. Outputs sway did not report
/// (an unplug racing this snapshot) trail the known ones by name rather
/// than dropping their workspaces off the bar, and no output information
/// at all collapses to the single pill of the one-screen case.
fn group_by_output(workspaces: &[WorkspaceInfo], outputs: &[OutputInfo]) -> Vec<Group> {
    if outputs.is_empty() {
        return vec![Group {
            output: String::new(),
            active: true,
            workspaces: workspaces.to_vec(),
        }];
    }
    let mut placed: Vec<&OutputInfo> = outputs.iter().collect();
    placed.sort_by_key(|o| (o.x, o.y, o.name.clone()));

    let mut groups: Vec<Group> = Vec::new();
    let mut push = |out: &str, ws: &WorkspaceInfo| match groups.iter_mut().find(|g| g.output == out)
    {
        Some(g) => g.workspaces.push(ws.clone()),
        None => groups.push(Group {
            output: out.to_string(),
            active: false,
            workspaces: vec![ws.clone()],
        }),
    };
    for out in &placed {
        for ws in workspaces.iter().filter(|w| w.output == out.name) {
            push(&out.name, ws);
        }
    }
    // Unknown outputs last, grouped by name so the order is at least
    // stable while the layout settles.
    let mut orphans: Vec<&WorkspaceInfo> = workspaces
        .iter()
        .filter(|w| !placed.iter().any(|o| o.name == w.output))
        .collect();
    orphans.sort_by(|a, b| a.output.cmp(&b.output));
    for ws in orphans {
        push(&ws.output.clone(), ws);
    }
    for group in &mut groups {
        group.active = group.workspaces.iter().any(|w| w.focused);
    }
    groups
}

/// What a workspace is, stated globally — the same on every bar, because
/// which screen it belongs to is already said by the group it sits in.
/// Focus is a step on the `Current` mark rather than a mark of its own
/// (the inactive-selection convention: VS Code's list.activeSelection vs
/// list.inactiveSelection), so it reinforces the group dimming instead of
/// competing with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WsState {
    /// Not on screen anywhere.
    Idle,
    /// The selected segment of its screen's pill — what that screen shows.
    /// Exactly one per group.
    Current,
    /// `Current`, on the screen holding input. Exactly one overall.
    Focused,
}

fn ws_state(ws: &WorkspaceInfo) -> WsState {
    match (ws.visible, ws.focused) {
        (_, true) => WsState::Focused,
        (true, false) => WsState::Current,
        (false, false) => WsState::Idle,
    }
}

/// One task's ribbon — the coarse cross-room glance; the board carries
/// the detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Ribbon {
    #[default]
    Off,
    Working,
    Waiting,
}

/// Waiting outranks working. Stopped and stale sessions read Off: three
/// encodings is the whole ribbon vocabulary, and stale must not
/// impersonate a live state (P9) — the board's OFF-flag carries it.
fn ribbon_states(snap: &TaskSnapshot) -> [Ribbon; 4] {
    std::array::from_fn(|i| {
        let sessions = &snap.tasks[i].sessions;
        let any = |a: Activity| sessions.iter().any(|s| s.activity == a);
        if any(Activity::Waiting) {
            Ribbon::Waiting
        } else if any(Activity::Working) {
            Ribbon::Working
        } else {
            Ribbon::Off
        }
    })
}

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

    fn on(num: i32, name: &str, output: &str) -> WorkspaceInfo {
        WorkspaceInfo {
            output: output.into(),
            ..ws(num, name)
        }
    }

    fn out(name: &str, x: i32, y: i32) -> OutputInfo {
        OutputInfo {
            name: name.into(),
            x,
            y,
        }
    }

    fn shown(num: i32, name: &str, output: &str, focused: bool) -> WorkspaceInfo {
        WorkspaceInfo {
            visible: true,
            focused,
            ..on(num, name, output)
        }
    }

    fn shape(groups: &[Group]) -> Vec<(&str, Vec<i32>)> {
        groups
            .iter()
            .map(|g| {
                (
                    g.output.as_str(),
                    g.workspaces.iter().map(|w| w.num).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn one_screen_is_one_pill() {
        let list = [on(1, "1:t1a", "eDP-1"), on(17, "17:wb", "eDP-1")];
        let groups = group_by_output(&list, &[out("eDP-1", 0, 0)]);
        assert_eq!(shape(&groups), [("eDP-1", vec![1, 17])]);
    }

    #[test]
    fn groups_follow_the_screens_left_to_right_then_top_to_bottom() {
        let list = [
            on(1, "1:t1a", "right"),
            on(5, "5:t2a", "left"),
            on(9, "9:t3a", "below"),
            on(17, "17:wb", "left"),
        ];
        let outputs = [
            // Declared in a deliberately unhelpful order: placement
            // decides, not the order sway happened to report them.
            out("below", 0, 1440),
            out("right", 2560, 0),
            out("left", 0, 0),
        ];
        assert_eq!(
            shape(&group_by_output(&list, &outputs)),
            [
                ("left", vec![5, 17]),
                ("below", vec![9]),
                ("right", vec![1]),
            ]
        );
    }

    #[test]
    fn no_output_info_collapses_to_a_single_pill() {
        let list = [on(1, "1:t1a", "DP-3"), on(5, "5:t2a", "eDP-1")];
        let groups = group_by_output(&list, &[]);
        assert_eq!(groups.len(), 1);
        assert_eq!(shape(&groups), [("", vec![1, 5])]);
    }

    #[test]
    fn workspaces_on_an_unreported_output_trail_instead_of_vanishing() {
        let list = [on(1, "1:t1a", "DP-3"), on(5, "5:t2a", "ghost")];
        assert_eq!(
            shape(&group_by_output(&list, &[out("DP-3", 0, 0)])),
            [("DP-3", vec![1]), ("ghost", vec![5])]
        );
    }

    #[test]
    fn exactly_the_screen_holding_input_stays_undimmed() {
        let list = [
            shown(1, "1:t1a", "left", false),
            on(2, "2:t1b", "left"),
            shown(5, "5:t2a", "right", true),
        ];
        let groups = group_by_output(&list, &[out("left", 0, 0), out("right", 2560, 0)]);
        let active: Vec<(&str, bool)> = groups
            .iter()
            .map(|g| (g.output.as_str(), g.active))
            .collect();
        assert_eq!(active, [("left", false), ("right", true)]);
    }

    #[test]
    fn one_current_segment_per_screen_and_one_focused_overall() {
        let left = shown(1, "1:t1a", "left", false);
        let right = shown(5, "5:t2a", "right", true);
        // Each screen's shown workspace is its selected segment; only the
        // one on the focused screen steps up. Both read the same on every
        // bar — the group says which screen they belong to.
        assert_eq!(ws_state(&left), WsState::Current);
        assert_eq!(ws_state(&right), WsState::Focused);
        assert_eq!(ws_state(&ws(9, "9:t3a")), WsState::Idle);
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
    fn ribbons_rank_waiting_over_working_and_quiet_the_rest() {
        use crate::task_state::SessionState;
        let session = |activity| SessionState {
            pid: 1,
            desc: "d".into(),
            activity,
            progress: None,
            workspace: "5:t2a".into(),
            status_mtime: None,
            acked: false,
        };
        let mut snap = TaskSnapshot::default();
        snap.tasks[0].sessions = vec![session(Activity::Working), session(Activity::Waiting)];
        snap.tasks[1].sessions = vec![session(Activity::Working)];
        // Stopped/stale-only tasks stay Off — the board carries those.
        snap.tasks[2].sessions = vec![session(Activity::Stopped), session(Activity::Stale)];
        assert_eq!(
            ribbon_states(&snap),
            [Ribbon::Waiting, Ribbon::Working, Ribbon::Off, Ribbon::Off]
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
