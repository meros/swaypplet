//! The search index as the settings window installs it: every pane's
//! table, searched the way people type. Each pane checks its own table
//! against its own rows (`search::anchors`); these tests check what the
//! tables add up to.

use swaypplet_settings_kit::search;

fn top(query: &str) -> String {
    search::find(query)
        .first()
        .map(|h| h.path.clone())
        .unwrap_or_default()
}

#[test]
fn synonyms_find_their_row() {
    super::install_search();
    for (query, path) in [
        ("dark mode", "Appearance › Mode"),
        ("dark", "Appearance › Mode"),
        ("light theme", "Appearance › Mode"),
        ("wallpaper", "Appearance › Wallpaper"),
        ("background image", "Appearance › Wallpaper"),
        ("night light", "Appearance › Night light › Night light"),
        ("blue light", "Appearance › Night light › Night light"),
        (
            "colour temperature",
            "Appearance › Night light › Night warmth",
        ),
        (
            "screen timeout",
            "Idle & Lock › Idle timers › Screen off after",
        ),
        ("auto lock", "Idle & Lock › Idle timers › Lock after"),
        ("sleep", "Idle & Lock › Idle timers › Suspend after"),
        ("blur", "Glass › Material › Frost"),
        ("transparency", "Glass › Material › Clarity"),
        ("resolution", "Displays › Display › Resolution"),
        ("scale", "Displays › Display › Scale"),
        ("hidpi", "Displays › Display › Scale"),
        ("hz", "Displays › Display › Refresh rate"),
        ("do not disturb", "Alerts › Quiet hours › Quiet hours"),
        ("screenshot folder", "Alerts › Capture › Folder"),
        ("24h", "Bar › Clock › 24-hour clock"),
        (
            "sudo",
            "Idle & Lock › Administrator access › Card for terminal sudo",
        ),
        ("wireless", "Panel › Wi-Fi"),
        ("headphones", "Panel › Audio output"),
        ("pair", "Panel › Bluetooth"),
        ("accent colour", "Appearance › Accent"),
        // Words people type that named nothing before.
        ("font", "Displays › Display › Scale"),
        ("font size", "Displays › Display › Scale"),
        ("fingerprint", "Idle & Lock › Lock › Face unlock"),
        ("time zone", "Bar › Clock"),
        ("scroll direction", "Input › Touchpad › Natural scrolling"),
        ("screen saver", "Idle & Lock › Idle timers › Lock after"),
        ("lid", "Idle & Lock › Idle timers"),
        ("second monitor", "Displays › Arrangement"),
        ("nightlight", "Appearance › Night light › Night light"),
        // Rankings that went to the wrong pane.
        ("notifications", "Alerts › Notifications › Linger"),
        ("night", "Appearance › Night light › Night light"),
        ("clock", "Bar › Clock"),
        ("monitor", "Displays › Arrangement"),
        ("bar", "Bar › Segments"),
    ] {
        assert_eq!(top(query), path, "{query:?}");
    }
}

/// "reset" and "default" name no row: every pane has one. The empty
/// search says where it is instead of "try another word".
#[test]
fn reset_finds_no_row_but_says_where_it_is() {
    super::install_search();
    assert!(search::find("reset").is_empty() && search::find("default").is_empty());
    assert_eq!(
        search::no_match_hint("Reset"),
        "Every pane has Reset to system at its foot"
    );
    assert_eq!(
        search::no_match_hint("default"),
        "Every pane has Reset to system at its foot"
    );
    assert_eq!(search::no_match_hint("zzzq"), "Try another word");
}

#[test]
fn titles_and_subtitles_find_their_row_too() {
    super::install_search();
    assert_eq!(top("launch zoom"), "Appearance › Motion › Launch zoom");
    assert_eq!(top("refraction"), "Glass › Material › Refraction");
    assert_eq!(top("adaptive"), "Displays › Display › Adaptive sync");
    // A word only the subtitle has.
    assert!(
        search::find("backlight")
            .iter()
            .any(|h| h.path == "Idle & Lock › Idle timers › Dim after")
    );
}

#[test]
fn ranking_is_sane() {
    super::install_search();
    // A whole word of the title or a keyword names the setting; a
    // prefix of one only suggests it.
    assert!(search::find("dark")[0].clear);
    assert!(search::find("resolution")[0].clear);
    assert!(!search::find("reso")[0].clear);
    // Two letters are a query, one is not; "fi" alone is too short to
    // be clear.
    assert!(search::find("x").is_empty());
    assert!(search::find("fi").iter().all(|h| !h.clear));
    // An app's name finds no setting.
    assert!(search::find("firefox").is_empty());
    assert!(search::find("spotify").is_empty());
    // Every query word has to land.
    assert!(search::find("dark zebra").is_empty());
    // The title outranks a keyword, a keyword the tab's name.
    let lock = search::find("lock");
    assert!(lock[0].path.ends_with("Lock after"), "{:?}", lock[0].path);
    // Scores come out best first.
    let hits = search::find("screen");
    assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));
}

#[test]
fn every_key_a_row_names_is_a_setting() {
    super::install_search();
    let settings = swaypplet_model::store::Settings::default();
    for search::Tab {
        name: tab, entries, ..
    } in search::tabs()
    {
        for e in *entries {
            for key in e.keys {
                assert!(
                    settings.get(key).is_some(),
                    "{tab} › {} › {} names {key}, which is no setting",
                    e.group,
                    e.title
                );
            }
        }
    }
}

#[test]
fn an_id_goes_back_to_its_target() {
    super::install_search();
    for hit in search::find("mode").into_iter().chain(search::find("wifi")) {
        assert_eq!(search::Target::from_id(&hit.target.id()), Some(hit.target));
    }
    assert_eq!(search::Target::from_id("look\tNope\tMode"), None);
    assert_eq!(search::Target::from_id(":nothing"), None);
}
