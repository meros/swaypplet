//! The open windows the launcher offers: "Go to" rows above an app that is
//! already running, and every window of an app on Tab.
//!
//! Both read sway's tree over IPC, so both run on a worker (`run_search`,
//! `show_windows_of`), never on a keystroke's frame.

use crate::services::elephant::SearchResult;

use super::{MAX_WINDOW_ROWS, WINDOW_PROVIDER};

/// The windows already open for the best application match, as rows. Their
/// identifier is `<con_id> <foreign toplevel identifier>`: the first to go
/// there, the second to show it.
pub(super) fn running_windows(query: &str, results: &[SearchResult]) -> Vec<SearchResult> {
    let app = results.iter().find(|r| r.provider == "desktopapplications");
    let names = app.map(app_names).unwrap_or_default();
    let words = title_words(query);
    if names.is_empty() && words.is_empty() {
        return Vec::new();
    }
    let Some(tree) = crate::sway::ipc::connect()
        .ok()
        .and_then(|mut c| c.get_tree().ok())
    else {
        return Vec::new();
    };
    let windows: Vec<_> = crate::jump::scene::all_windows(&tree)
        .into_iter()
        .filter(|(w, _, _)| w.id.is_some())
        .collect();
    // A window whose title has the query in it comes first: that is the one
    // being asked for by name. Then the windows of the app that matched.
    let by_title = windows
        .iter()
        .filter(|(_, _, title)| title_matches(&words, title));
    let by_app = windows.iter().filter(|(w, _, title)| {
        !title_matches(&words, title) && names.iter().any(|n| same_app(n, &w.app))
    });
    by_title
        .chain(by_app)
        .take(MAX_WINDOW_ROWS)
        .map(|(w, ws, title)| {
            let name = app
                .filter(|a| app_names(a).iter().any(|n| same_app(n, &w.app)))
                .map_or(w.app.as_str(), |a| a.text.as_str());
            window_result(w, ws, title, name)
        })
        .collect()
}

/// The words a window title has to contain, lowercased. None for a query
/// under two characters, which would match nearly every title.
fn title_words(query: &str) -> Vec<String> {
    let query = query.trim().to_lowercase();
    if query.chars().count() < 2 {
        return Vec::new();
    }
    query.split_whitespace().map(str::to_string).collect()
}

/// Whether every query word is in the title, in any case.
fn title_matches(words: &[String], title: &str) -> bool {
    if words.is_empty() {
        return false;
    }
    let title = title.to_lowercase();
    words.iter().all(|w| title.contains(w.as_str()))
}

/// What an application result could be called as a window's app_id: its
/// desktop entry and its icon name, lowercased, without `.desktop`.
fn app_names(result: &SearchResult) -> Vec<String> {
    let mut names = Vec::new();
    for raw in [&result.identifier, &result.icon] {
        let base = raw.rsplit('/').next().unwrap_or(raw);
        let name = base.trim_end_matches(".desktop").to_lowercase();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// A desktop-entry name and an app_id name the same app: exactly, or by the
/// last part of a reverse-DNS id (`org.mozilla.firefox` is `firefox`).
fn same_app(name: &str, app_id: &str) -> bool {
    let id = app_id.to_lowercase();
    let tail = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();
    id == name || tail(&id) == tail(name)
}

/// Every window of the app `app` is a result for, for Tab on its row. Empty
/// when none is open.
pub(super) fn windows_of_app(app: &SearchResult) -> Vec<SearchResult> {
    let names = app_names(app);
    if names.is_empty() {
        return Vec::new();
    }
    let Some(tree) = crate::sway::ipc::connect()
        .ok()
        .and_then(|mut c| c.get_tree().ok())
    else {
        return Vec::new();
    };
    crate::jump::scene::all_windows(&tree)
        .into_iter()
        .filter(|(w, _, _)| w.id.is_some() && names.iter().any(|n| same_app(n, &w.app)))
        .map(|(w, ws, title)| window_result(&w, &ws, &title, &app.text))
        .collect()
}

fn window_result(
    w: &crate::jump::scene::Window,
    ws: &str,
    title: &str,
    app_name: &str,
) -> SearchResult {
    SearchResult {
        identifier: format!("{} {}", w.con_id, w.id.clone().unwrap_or_default()),
        text: format!("Go to {}", if title.is_empty() { &w.app } else { title }),
        subtext: format!("{app_name} \u{00b7} open on {ws}"),
        icon: String::new(),
        provider: WINDOW_PROVIDER.to_string(),
        score: 0,
        actions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_matches_every_query_word_in_any_case() {
        let words = title_words("dreaded Board");
        assert!(title_matches(
            &words,
            "The dreaded board view · The project - Google Chrome"
        ));
        assert!(!title_matches(&words, "The dreaded list view"));
        assert!(title_matches(
            &title_words("youtube"),
            "(238) YouTube - Google Chrome"
        ));
    }

    #[test]
    fn a_one_letter_query_matches_no_title() {
        assert!(!title_matches(
            &title_words("y"),
            "(238) YouTube - Google Chrome"
        ));
        assert!(!title_matches(&title_words(" "), "anything"));
    }

    fn app(identifier: &str, icon: &str) -> SearchResult {
        SearchResult {
            identifier: identifier.into(),
            text: String::new(),
            subtext: String::new(),
            icon: icon.into(),
            provider: "desktopapplications".into(),
            score: 0,
            actions: Vec::new(),
        }
    }

    #[test]
    fn an_application_is_named_by_its_entry_and_its_icon() {
        let names = app_names(&app(
            "/usr/share/applications/org.mozilla.firefox.desktop",
            "firefox",
        ));
        assert_eq!(names, ["org.mozilla.firefox", "firefox"]);
        assert_eq!(
            app_names(&app("Alacritty.desktop", "Alacritty")),
            ["alacritty"]
        );
    }

    #[test]
    fn a_window_matches_by_app_id_or_its_reverse_dns_tail() {
        assert!(same_app("alacritty", "Alacritty"));
        assert!(same_app("org.mozilla.firefox", "firefox"));
        assert!(same_app("firefox", "org.mozilla.firefox"));
        assert!(!same_app("code", "claude"));
    }
}
