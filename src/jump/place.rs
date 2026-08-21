//! Where you have been, most recent first.
//!
//! sway keeps this already and does not advertise it. Every output node in
//! `get_tree` carries a `focus` array, and for an output that array is its
//! workspaces in most-recently-focused order; the root's is its outputs in the
//! same order. So the whole recency model this surface needs is two fields
//! that are already on the wire, and swaypplet keeps no ring, no event feed
//! and no state file.
//!
//! That is only safe because stepping never switches anything. A switcher that
//! previewed by focusing would reorder the very list it is walking, which is
//! why the obvious design needs a frozen copy and a thaw on commit. This one
//! reads the stack once when the gesture starts and the compositor cannot move
//! it until the gesture ends. See `gesture.rs`.
//!
//! The order is focused-output-first: that output's stack in full, then each
//! other output's. True cross-output interleaving is not recoverable from the
//! tree - two outputs' stacks carry no relative timestamps - and it is not
//! worth an event feed to get, because "back to what I was doing" almost
//! always means on the screen you are looking at.

use swayipc::Node;

/// One entry in the ring: a workspace, and nothing more.
///
/// Deliberately not a task. The four-tasks-by-two-screens grouping exists in
/// this repo's workspace table, but the second screen slot is unused in
/// practice, and keying the ring on task identity buys a collapse rule and a
/// slot-memory field to serve it. The workspace is the thing you left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// sway's workspace number, or -1 for a workspace with no numeric prefix.
    pub num: i32,
    /// The full name, `"9:t3a"`. Identity, and what a fallback switch quotes.
    pub name: String,
    /// Which output it currently lives on.
    pub output: String,
}

/// Every occupied workspace, most recently focused first.
///
/// `places[0]` is where you are now. It is never a row: the gesture's first
/// step selects `places[1]`, which is the definition of "back".
pub fn mru(tree: &Node) -> Vec<Place> {
    let outputs = outputs_in_focus_order(tree);
    let mut places = Vec::new();
    for output in outputs {
        let name = output.name.clone().unwrap_or_default();
        // `focus` holds ids, and a workspace of this output is a child of it.
        // Walking the children per id rather than building a map: an output
        // has a handful of workspaces, and a map would have to be keyed over
        // the whole tree to be worth it.
        for id in &output.focus {
            if let Some(ws) = output.nodes.iter().find(|n| n.id == *id) {
                places.push(Place {
                    num: ws.num.unwrap_or(-1),
                    name: ws.name.clone().unwrap_or_default(),
                    output: name.clone(),
                });
            }
        }
    }
    places
}

/// The real outputs, focused first.
///
/// `__i3` is skipped: it is the scratchpad's holding pen, not a screen, and
/// its workspace would otherwise appear in the ring as somewhere you could go.
fn outputs_in_focus_order(tree: &Node) -> Vec<&Node> {
    let mut ordered: Vec<&Node> = Vec::new();
    for id in &tree.focus {
        if let Some(output) = tree.nodes.iter().find(|n| n.id == *id) {
            ordered.push(output);
        }
    }
    // Anything the root's focus list does not mention still exists and still
    // has workspaces on it - an output connected but never focused this
    // session. It goes last rather than missing.
    for output in &tree.nodes {
        if !ordered.iter().any(|o| o.id == output.id) {
            ordered.push(output);
        }
    }
    ordered
        .into_iter()
        .filter(|o| o.name.as_deref() != Some("__i3"))
        .collect()
}

#[cfg(test)]
pub(crate) mod fixture {
    //! Trees for tests, from JSON, because `swayipc::Node` is
    //! `#[non_exhaustive]` and serde is the only way to build one. Same trick
    //! `sway_ipc.rs`'s tests use.

    use serde_json::{Value, json};
    use swayipc::Node;

    pub fn rect() -> Value {
        json!({"x": 0, "y": 0, "width": 1440, "height": 900})
    }

    /// A node with every field `swayipc` requires, overridden by `extra`.
    pub fn node(extra: Value) -> Value {
        let mut base = json!({
            "id": 0, "type": "con", "focus": [], "nodes": [], "floating_nodes": [],
            "border": "none", "current_border_width": 0, "layout": "none",
            "orientation": "none", "percent": null, "rect": rect(),
            "window_rect": rect(), "deco_rect": rect(), "geometry": rect(),
            "urgent": false, "focused": false, "marks": [], "sticky": false,
            "fullscreen_mode": 0,
        });
        if let (Value::Object(b), Value::Object(e)) = (&mut base, extra) {
            for (k, v) in e {
                b.insert(k, v);
            }
        }
        base
    }

    pub fn workspace(id: i64, num: i32, name: &str) -> Value {
        node(json!({"id": id, "type": "workspace", "num": num, "name": name}))
    }

    /// One output, with its workspaces and its MRU order over them.
    pub fn output(id: i64, name: &str, focus: Vec<i64>, workspaces: Vec<Value>) -> Value {
        node(json!({
            "id": id, "type": "output", "name": name,
            "focus": focus, "nodes": workspaces,
        }))
    }

    pub fn tree(focus: Vec<i64>, outputs: Vec<Value>) -> Node {
        let root = node(json!({
            "id": 1, "type": "root", "name": "root",
            "focus": focus, "nodes": outputs,
        }));
        serde_json::from_value(root).expect("fixture tree must deserialize")
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;

    /// The recorded shape of the reference session: one output, nine
    /// workspaces, the focus stack sway actually reported.
    fn one_output() -> Node {
        tree(
            vec![3],
            vec![output(
                3,
                "eDP-1",
                vec![19, 10, 6, 8, 16],
                vec![
                    workspace(19, 9, "9:t3a"),
                    workspace(10, 5, "5:t2a"),
                    workspace(6, 1, "1:t1a"),
                    workspace(8, 6, "6:t2b"),
                    workspace(16, 24, "24:wg"),
                ],
            )],
        )
    }

    #[test]
    fn the_ring_is_swayss_own_focus_stack() {
        let names: Vec<String> = mru(&one_output()).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["9:t3a", "5:t2a", "1:t1a", "6:t2b", "24:wg"]);
    }

    #[test]
    fn the_head_is_where_you_are_and_the_first_row_is_where_you_were() {
        let places = mru(&one_output());
        assert_eq!(places[0].name, "9:t3a", "head is the current place");
        assert_eq!(places[1].name, "5:t2a", "one tap goes here");
    }

    #[test]
    fn the_focused_output_comes_first_and_whole() {
        // Two outputs, the second one focused. Its whole stack precedes the
        // other's, because "back" means back on the screen you are looking at.
        let t = tree(
            vec![7, 3],
            vec![
                output(
                    3,
                    "eDP-1",
                    vec![19, 10],
                    vec![workspace(19, 9, "9:t3a"), workspace(10, 5, "5:t2a")],
                ),
                output(
                    7,
                    "DP-2",
                    vec![40, 41],
                    vec![workspace(40, 30, "30:wm"), workspace(41, 31, "31:wn")],
                ),
            ],
        );
        let got: Vec<(String, String)> = mru(&t).into_iter().map(|p| (p.output, p.name)).collect();
        assert_eq!(
            got,
            [
                ("DP-2".into(), "30:wm".into()),
                ("DP-2".into(), "31:wn".into()),
                ("eDP-1".into(), "9:t3a".into()),
                ("eDP-1".into(), "5:t2a".into()),
            ]
        );
    }

    #[test]
    fn the_scratchpad_is_not_somewhere_you_can_go() {
        let t = tree(
            vec![3],
            vec![
                output(3, "eDP-1", vec![19], vec![workspace(19, 9, "9:t3a")]),
                output(2, "__i3", vec![5], vec![workspace(5, -1, "__i3_scratch")]),
            ],
        );
        let names: Vec<String> = mru(&t).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["9:t3a"]);
    }

    #[test]
    fn an_output_the_root_never_focused_still_contributes() {
        // A monitor plugged in but never switched to has no entry in the
        // root's focus list. Its workspaces exist and must still be reachable.
        let t = tree(
            vec![3],
            vec![
                output(3, "eDP-1", vec![19], vec![workspace(19, 9, "9:t3a")]),
                output(7, "DP-2", vec![40], vec![workspace(40, 30, "30:wm")]),
            ],
        );
        let names: Vec<String> = mru(&t).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["9:t3a", "30:wm"]);
    }

    #[test]
    fn a_workspace_with_no_number_keeps_its_name() {
        let t = tree(
            vec![3],
            vec![output(
                3,
                "eDP-1",
                vec![9],
                vec![workspace(9, -1, "scratch")],
            )],
        );
        let p = &mru(&t)[0];
        assert_eq!(p.num, -1);
        assert_eq!(p.name, "scratch");
    }

    #[test]
    fn an_empty_session_yields_nothing_rather_than_panicking() {
        assert!(mru(&tree(vec![], vec![])).is_empty());
    }

    /// The premise, against a real session rather than against a fixture I
    /// wrote to agree with me.
    ///
    /// `tests/fixtures/sessions/typical` is `swaymsg -t get_tree` from a live
    /// desktop, titles scrubbed. If sway's `focus` array ever stops being a
    /// workspace MRU stack - a different version, a different compositor -
    /// this is the test that says so, rather than the ring quietly ordering
    /// itself by workspace number and nobody noticing for a week.
    #[test]
    fn the_recorded_session_reproduces_its_own_focus_order() {
        let raw = std::fs::read_to_string("tests/fixtures/sessions/typical/tree.json")
            .expect("recorded fixture must exist; run dev/record-session.sh");
        let tree: Node = serde_json::from_str(&raw).expect("recorded tree must deserialize");

        let places = mru(&tree);
        assert!(!places.is_empty(), "the recording had workspaces");

        // Rebuild the expectation from the raw JSON rather than from `mru`,
        // so the test cannot pass by agreeing with the code under test.
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let outputs = value["nodes"].as_array().unwrap();
        let eDP = outputs
            .iter()
            .find(|o| o["name"] == "eDP-1")
            .expect("the recording is from the laptop");
        let expected: Vec<String> = eDP["focus"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| {
                eDP["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|w| w["id"] == *id)
                    .map(|w| w["name"].as_str().unwrap().to_string())
                    .unwrap()
            })
            .collect();

        let got: Vec<String> = places
            .iter()
            .filter(|p| p.output == "eDP-1")
            .map(|p| p.name.clone())
            .collect();
        assert_eq!(got, expected);
    }

    /// The recording is the shape the layout has to survive, so its numbers
    /// are worth asserting rather than assuming: if a future recording drops
    /// to two workspaces, the fixture stops exercising what it was taken for.
    #[test]
    fn the_recorded_session_is_still_representative() {
        let raw = std::fs::read_to_string("tests/fixtures/sessions/typical/tree.json").unwrap();
        let tree: Node = serde_json::from_str(&raw).unwrap();
        let n = mru(&tree).len();
        assert!(
            (5..=30).contains(&n),
            "recorded session has {n} places, which is not a realistic desktop"
        );
    }
}
