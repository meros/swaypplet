//! Searches over sway's layout tree: the one walk every search shares, and
//! the search for the window a notification came from.
//!
//! The walk carries the nearest enclosing workspace down the tree, which is
//! the part each caller used to write again: the pid map in [`super::ipc`],
//! the Super+Tab scene's window lists (`jump::scene`), and the notification
//! jump-to-source below. The name is the raw one, sway's own `__i3_scratch`
//! included; each caller decides what a scratchpad view is to it.
//!
//! No GTK here, so the searches are unit tests.

use std::collections::HashMap;
use std::ops::ControlFlow;

use swayipc::{Node, NodeType};

/// Visit `node` and everything under it in tree order, tiled children before
/// floating ones, each with the name of the nearest enclosing workspace.
/// Stops at the first `Break`, and returns its value.
pub fn walk<'a, B>(
    node: &'a Node,
    visit: &mut impl FnMut(&'a Node, Option<&'a str>) -> ControlFlow<B>,
) -> Option<B> {
    fn go<'a, B>(
        node: &'a Node,
        workspace: Option<&'a str>,
        visit: &mut impl FnMut(&'a Node, Option<&'a str>) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        let workspace = if node.node_type == NodeType::Workspace {
            node.name.as_deref()
        } else {
            workspace
        };
        visit(node, workspace)?;
        for child in node.nodes.iter().chain(&node.floating_nodes) {
            go(child, workspace, visit)?;
        }
        ControlFlow::Continue(())
    }
    match go(node, None, visit) {
        ControlFlow::Break(b) => Some(b),
        ControlFlow::Continue(()) => None,
    }
}

/// [`walk`] over the whole tree, for a visitor that never stops early.
pub fn for_each<'a>(node: &'a Node, mut visit: impl FnMut(&'a Node, Option<&'a str>)) {
    walk::<()>(node, &mut |n, ws| {
        visit(n, ws);
        ControlFlow::Continue(())
    });
}

/// A workspace someone can switch to: sway's own `__i3_scratch` is not one,
/// so a scratchpad view is focused by id alone.
fn switchable(workspace: Option<&str>) -> Option<&str> {
    workspace.filter(|name| !name.starts_with("__"))
}

// ── The window a notification came from ─────────────────────────────────

/// One view in the tree, and where it lives.
pub struct Target {
    pub con_id: i64,
    /// The enclosing workspace, when it has a name worth switching to.
    pub workspace: Option<String>,
}

/// Focus the window a notification came from, and say whether there was
/// one. `done` runs on the GTK thread either way.
///
/// `names` are what the notification says about itself that a window could
/// be named after, best evidence first. A Claude session names its process
/// (`claude_pid`), and that is the only evidence used for it: the terminal
/// that owns the session is the ancestor that owns a window. Its app name,
/// "Claude Code", names no window, and on this desktop every session is in a
/// terminal indistinguishable by name from the others. A session that has
/// ended finds nothing, which falls through to the caller's fallback rather
/// than to a guess. `parent` walks the process tree (`/proc`), and is the
/// caller's so this module needs nothing from the services. `pick` says
/// whether a name several windows answer to equally still picks one of them.
///
/// The tree query is an IPC round trip, so it happens on a worker thread
/// (`spawn::spawn_work`) rather than under the pointer. A sway that is not
/// answering, or a name nothing in the tree matches, is a plain `false`: the
/// caller has somewhere else to go and the card must never be left waiting.
pub fn focus_source(
    names: Vec<String>,
    claude_pid: Option<i32>,
    parent: fn(i32) -> Option<i32>,
    pick: Pick,
    done: impl FnOnce(bool) + 'static,
) {
    if names.is_empty() && claude_pid.is_none() {
        done(false);
        return;
    }
    crate::spawn::spawn_work(
        move || {
            let tree = super::ipc::connect()
                .and_then(|mut c| c.get_tree())
                .map_err(|e| log::warn!("sway ipc: get_tree failed: {e}"))
                .ok()?;
            match claude_pid {
                Some(pid) => window_of_pid(&tree, pid, parent),
                None => find_named_window(&tree, &names, pick),
            }
        },
        move |found| match found {
            Some(target) => {
                let cmd = focus_command(&target);
                log::debug!("notification click: {cmd}");
                super::ipc::run_command(&cmd);
                done(true);
            }
            None => done(false),
        },
    );
}

/// The command that brings a view to the front.
///
/// The workspace switch comes first: `[con_id=N] focus` alone moves focus
/// without bringing that workspace onto the output, so the window you asked
/// for can stay out of sight. The name is quoted so a workspace someone
/// renamed cannot smuggle in further sway commands (`;` splits, quotes
/// group); a name carrying quote characters itself falls back to the bare
/// focus rather than trusting sway's escape handling.
fn focus_command(target: &Target) -> String {
    let con_id = target.con_id;
    match &target.workspace {
        Some(ws) if !ws.contains(['"', '\\']) => {
            format!("workspace \"{ws}\"; [con_id={con_id}] focus")
        }
        _ => format!("[con_id={con_id}] focus"),
    }
}

/// How well one of the notification's names matches one identity a view
/// carries. Higher is better; `None` is no match.
///
/// Exact beats the reverse-DNS tail (`org.mozilla.firefox` is `firefox`),
/// which beats a substring — and a substring counts only from three
/// characters up, because a two-letter token matches half the tree, and only
/// for a one-word name. An app name of several words is a phrase, not an
/// identifier: "Claude Code" contains "code", which is VS Code's `app_id`,
/// and a click on a Claude notification used to land there.
fn name_score(name: &str, id: &str) -> Option<u8> {
    let id = id.to_lowercase();
    if id == name {
        return Some(3);
    }
    let tail = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();
    if tail(&id) == tail(name) {
        return Some(2);
    }
    if name.len() >= 3
        && id.len() >= 3
        && !name.contains(char::is_whitespace)
        && (id.contains(name) || name.contains(&id))
    {
        return Some(1);
    }
    None
}

/// The window that `pid` runs under: the view owned by `pid` itself or by
/// its nearest ancestor that owns one. `parent` is injected so tests need no
/// /proc.
///
/// A Claude session sits a few processes below its terminal (shell,
/// wrappers), and each terminal window on this desktop is its own process,
/// so the first ancestor with a view is the one window it is in.
fn window_of_pid(root: &Node, pid: i32, parent: impl Fn(i32) -> Option<i32>) -> Option<Target> {
    // Every view that has a pid, keyed by it. The first view in tree order
    // wins for a process with several windows, which a terminal here never is.
    let mut views = HashMap::new();
    for_each(root, |node, workspace| {
        if node.nodes.is_empty()
            && let Some(pid) = node.pid
        {
            views.entry(pid).or_insert_with(|| Target {
                con_id: node.id,
                workspace: switchable(workspace).map(str::to_string),
            });
        }
    });
    let mut p = pid;
    while p > 1 {
        if let Some(target) = views.remove(&p) {
            return Some(target);
        }
        p = parent(p)?;
    }
    None
}

/// Which of several equally good windows a search may answer with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// The best one, ties going to the window the seat is on.
    Best,
    /// Only a window no other view matches as well; a tie is no answer.
    Unique,
}

/// The view a notification came from, or `None` when nothing in the tree
/// answers to any of its names — or, with [`Pick::Unique`], when more than
/// one view answers equally well.
fn find_named_window(root: &Node, names: &[String], pick: Pick) -> Option<Target> {
    let mut best: Option<((u8, usize, bool), Target)> = None;
    // How many views share the best match, the seat's focus left out: two
    // windows of one app match alike whichever of them was last focused.
    let mut tied = 0;
    for_each(root, |node, workspace| {
        let Some(rank) = view_rank(node, names) else {
            return;
        };
        let evidence = |r: (u8, usize, bool)| (r.0, r.1);
        match &best {
            Some((seen, _)) if evidence(rank) == evidence(*seen) => tied += 1,
            Some((seen, _)) if evidence(rank) < evidence(*seen) => {}
            _ => tied = 1,
        }
        if best.as_ref().is_none_or(|(seen, _)| rank > *seen) {
            best = Some((
                rank,
                Target {
                    con_id: node.id,
                    workspace: switchable(workspace).map(str::to_string),
                },
            ));
        }
    });
    if pick == Pick::Unique && tied > 1 {
        return None;
    }
    best.map(|(_, target)| target)
}

/// How strong a claim a view has on the notification: match quality first,
/// then which name matched (the `desktop-entry` hint outranks free text),
/// then whether the seat is already on it — among equally good windows of
/// one app, the one last focused is the one meant.
fn view_rank(node: &Node, names: &[String]) -> Option<(u8, usize, bool)> {
    let props = node.window_properties.as_ref();
    let ids = [
        node.app_id.as_deref(),
        props.and_then(|p| p.class.as_deref()),
        props.and_then(|p| p.instance.as_deref()),
    ];
    names
        .iter()
        .enumerate()
        .filter_map(|(i, name)| {
            let score = ids
                .iter()
                .flatten()
                .filter_map(|id| name_score(name, id))
                .max()?;
            Some((score, names.len() - i, node.focused))
        })
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid node JSON with `extra` merged over it — swayipc's `Node`
    /// is `#[non_exhaustive]`, so fixtures go through serde like the real
    /// replies do.
    fn node(extra: serde_json::Value) -> serde_json::Value {
        let rect = serde_json::json!({"x": 0, "y": 0, "width": 0, "height": 0});
        let mut base = serde_json::json!({
            "id": 1,
            "type": "con",
            "border": "none",
            "current_border_width": 0,
            "layout": "splith",
            "orientation": "none",
            "rect": rect,
            "window_rect": rect,
            "deco_rect": rect,
            "geometry": rect,
            "urgent": false,
            "focused": false,
            "focus": [],
            "floating_nodes": [],
            "sticky": false,
        });
        let serde_json::Value::Object(extra) = extra else {
            panic!("extra must be an object")
        };
        base.as_object_mut().unwrap().extend(extra);
        base
    }

    /// One output, two workspaces: "1" holds two firefox windows (the second
    /// focused) and an XWayland Slack, "2" holds a kitty.
    fn tree() -> Node {
        serde_json::from_value(node(serde_json::json!({
            "type": "root",
            "nodes": [node(serde_json::json!({
                "type": "output",
                "name": "eDP-1",
                "nodes": [
                    node(serde_json::json!({
                        "type": "workspace",
                        "name": "1",
                        "num": 1,
                        "nodes": [
                            node(serde_json::json!({"id": 10, "app_id": "firefox"})),
                            node(serde_json::json!({
                                "id": 11, "app_id": "firefox", "focused": true,
                            })),
                            node(serde_json::json!({
                                "id": 12,
                                "window": 4242,
                                "window_properties": {
                                    "class": "Slack", "instance": "slack",
                                },
                            })),
                        ],
                    })),
                    node(serde_json::json!({
                        "type": "workspace",
                        "name": "2",
                        "num": 2,
                        "nodes": [node(serde_json::json!({"id": 13, "app_id": "kitty"}))],
                    })),
                    // Two Claude sessions' terminals and a VS Code window,
                    // the case the pid exists for.
                    node(serde_json::json!({
                        "type": "workspace",
                        "name": "5:t2a",
                        "num": 5,
                        "nodes": [
                            node(serde_json::json!({"id": 20, "app_id": "Alacritty", "pid": 500})),
                            node(serde_json::json!({"id": 21, "app_id": "code", "pid": 510})),
                        ],
                    })),
                    node(serde_json::json!({
                        "type": "workspace",
                        "name": "9:t3a",
                        "num": 9,
                        "nodes": [node(serde_json::json!({
                            "id": 22, "app_id": "Alacritty", "pid": 600, "focused": true,
                        }))],
                    })),
                ],
            }))],
        })))
        .expect("valid node fixture")
    }

    fn found(names: &[&str]) -> Option<(i64, Option<String>)> {
        let names: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
        find_named_window(&tree(), &names, Pick::Best).map(|t| (t.con_id, t.workspace))
    }

    fn found_unique(names: &[&str]) -> Option<i64> {
        let names: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
        find_named_window(&tree(), &names, Pick::Unique).map(|t| t.con_id)
    }

    #[test]
    fn a_sender_with_several_windows_names_none_of_them_uniquely() {
        // Two firefox windows answer equally well: which one the
        // notification is about is the sender's to say (its default action),
        // and guessing the focused one did nothing at all when that was the
        // window already in front.
        assert_eq!(found_unique(&["firefox"]), None);
        // One window of the app is that window.
        assert_eq!(found_unique(&["kitty"]), Some(13));
        assert_eq!(found_unique(&["slack"]), Some(12));
        // A weaker match on another view does not make an exact one
        // ambiguous.
        assert_eq!(found_unique(&["kitty", "fire"]), Some(13));
    }

    #[test]
    fn the_walk_carries_the_workspace_and_stops_where_asked() {
        let t = tree();
        let mut seen = Vec::new();
        for_each(&t, |n, ws| {
            if n.nodes.is_empty() {
                seen.push((n.id, ws.map(str::to_string)));
            }
        });
        assert_eq!(seen[0], (10, Some("1".into())));
        assert_eq!(seen.last(), Some(&(22, Some("9:t3a".into()))));
        let first_kitty = walk(&t, &mut |n, ws| {
            if n.app_id.as_deref() == Some("kitty") {
                ControlFlow::Break((n.id, ws.map(str::to_string)))
            } else {
                ControlFlow::Continue(())
            }
        });
        assert_eq!(first_kitty, Some((13, Some("2".into()))));
    }

    #[test]
    fn the_window_a_notification_came_from_is_the_one_it_names() {
        // XWayland identity lives in window_properties, Wayland's in app_id.
        assert_eq!(found(&["slack"]), Some((12, Some("1".into()))));
        assert_eq!(found(&["kitty"]), Some((13, Some("2".into()))));
        // A sender naming its desktop entry in reverse-DNS still finds it.
        assert_eq!(
            found(&["org.mozilla.firefox"]),
            Some((11, Some("1".into())))
        );
        // Two windows of one app: the one the seat was last on.
        assert_eq!(found(&["firefox"]), Some((11, Some("1".into()))));
        // Nothing to jump to is not a crash, it is a fallback.
        assert_eq!(found(&["gimp"]), None);
        assert_eq!(found(&[]), None);
    }

    #[test]
    fn better_evidence_wins_over_a_loose_match() {
        // "kitty" is exact, so it beats the free-text app name that only
        // shares letters with another view.
        assert_eq!(found(&["kitty", "fire"]), Some((13, Some("2".into()))));
        // A short token must not match on being a substring of everything.
        assert_eq!(name_score("ki", "kitty"), None);
        assert_eq!(name_score("kitty", "kitty"), Some(3));
        assert_eq!(name_score("org.kde.konsole", "konsole"), Some(2));
        assert_eq!(name_score("firefox", "firefox-esr"), Some(1));
    }

    /// Process tree: claude 503 → shell 502 → alacritty 500, and claude 603
    /// → shell 602 → wrapper 601 → alacritty 600. Everything else is init's.
    fn parent(pid: i32) -> Option<i32> {
        match pid {
            503 => Some(502),
            502 => Some(500),
            603 => Some(602),
            602 => Some(601),
            601 => Some(600),
            _ => Some(1),
        }
    }

    #[test]
    fn a_claude_notification_goes_to_the_terminal_its_session_runs_in() {
        let t = tree();
        let at = |pid| window_of_pid(&t, pid, parent).map(|t| (t.con_id, t.workspace));
        assert_eq!(at(503), Some((20, Some("5:t2a".into()))));
        // Not the focused terminal, and not the other session's.
        assert_eq!(at(603), Some((22, Some("9:t3a".into()))));
        // A session that has ended owns nothing: no guess.
        assert_eq!(at(999), None);
    }

    #[test]
    fn a_phrase_app_name_does_not_match_a_word_inside_it() {
        // "Claude Code" once matched VS Code's app_id "code".
        assert_eq!(name_score("claude code", "code"), None);
        assert_eq!(found(&["claude code"]), None);
        // One-word names keep their substring match.
        assert_eq!(name_score("firefox", "firefox-esr"), Some(1));
    }

    #[test]
    fn a_renamed_workspace_cannot_smuggle_in_a_sway_command() {
        let cmd = |ws: Option<&str>| {
            focus_command(&Target {
                con_id: 7,
                workspace: ws.map(str::to_string),
            })
        };
        assert_eq!(cmd(Some("mail")), "workspace \"mail\"; [con_id=7] focus");
        // Focus alone would leave the window on an off-screen workspace, so
        // this is the fallback rather than the rule.
        assert_eq!(cmd(None), "[con_id=7] focus");
        assert_eq!(cmd(Some("a\"; exec rm -rf ~")), "[con_id=7] focus");
    }
}
