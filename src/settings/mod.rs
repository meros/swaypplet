//! The settings pane: a deck page in the Helm card (`panel.rs`), one tab per
//! thing that can be configured.
//!
//! Five tabs. Look, Idle & Lock, Bar and Alerts edit `store::Settings`, one
//! file with one or two sections each; Glass edits the compositor material
//! and keeps its own file (`glass.rs`, for why). Every tab applies live and
//! saves after the fact, and every tab has one Reset that puts the defaults
//! back and removes its sections from the file, so there is always a way
//! out of a setting that turned out to be wrong.
//!
//! What is deliberately not here: the bar's position and height (a layout
//! the whole stylesheet is built around), the night light's temperature
//! (gammastep's config, owned by Nix), and anything the panel already has a
//! section for. A setting earns a row when it is a matter of taste that a
//! rebuild is too slow a loop for.

mod alerts_pane;
mod bar_pane;
pub mod cli;
pub mod glass;
mod glass_pane;
mod idle_pane;
mod look_pane;
pub mod preset;
pub mod schema;
pub mod store;
mod ui;
pub mod wallpaper;

use gtk4::prelude::*;

/// A tab: its stack name, the omnibox prefixes that open it, and its title.
struct Tab {
    name: &'static str,
    title: &'static str,
    prefixes: &'static [&'static str],
}

const TABS: [Tab; 5] = [
    Tab {
        name: "look",
        title: "Look",
        prefixes: &[":look", ":wall", ":bg", ":paper", ":motion"],
    },
    Tab {
        name: "idle",
        title: "Idle & Lock",
        prefixes: &[":idle", ":lock", ":timeout", ":sleep"],
    },
    Tab {
        name: "bar",
        title: "Bar",
        prefixes: &[":bar", ":clock", ":osd", ":keys"],
    },
    Tab {
        name: "alerts",
        title: "Alerts",
        prefixes: &[":alerts", ":quiet", ":shot", ":capture"],
    },
    Tab {
        name: "glass",
        title: "Glass",
        prefixes: &[":glass", ":material"],
    },
];

/// Every prefix and the tab it opens, for the omnibox's help page.
pub fn prefixes() -> impl Iterator<Item = (&'static [&'static str], String)> {
    TABS.iter()
        .map(|t| (t.prefixes, format!("Settings · {}", t.title)))
}

/// The tab an omnibox prefix opens, if it names one. `:set` and `:pref`
/// open the pane on whatever tab it was last on and are not in this table.
pub fn tab_for_prefix(prefix: &str) -> Option<&'static str> {
    TABS.iter()
        .find(|t| t.prefixes.iter().any(|p| prefix.starts_with(p)))
        .map(|t| t.name)
}

pub struct SettingsSection {
    root: gtk4::Box,
    tabs: Vec<(&'static str, gtk4::ToggleButton)>,
    look: look_pane::LookPane,
    idle: idle_pane::IdlePane,
    bar: bar_pane::BarPane,
    alerts: alerts_pane::AlertsPane,
    glass: glass_pane::GlassPane,
}

impl SettingsSection {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(10)
            .build();
        root.add_css_class("settings-pane");

        let look = look_pane::LookPane::new();
        let idle = idle_pane::IdlePane::new();
        let bar = bar_pane::BarPane::new();
        let alerts = alerts_pane::AlertsPane::new();
        let glass = glass_pane::GlassPane::new();

        let stack = gtk4::Stack::builder()
            .transition_type(gtk4::StackTransitionType::Crossfade)
            .transition_duration(120)
            .vhomogeneous(false)
            .build();
        stack.add_named(look.widget(), Some("look"));
        stack.add_named(idle.widget(), Some("idle"));
        stack.add_named(bar.widget(), Some("bar"));
        stack.add_named(alerts.widget(), Some("alerts"));
        stack.add_named(glass.widget(), Some("glass"));

        // Toggle buttons in one group rather than a StackSwitcher, so the
        // strip takes the pane's own chrome instead of the theme's tab bar.
        let strip = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .build();
        strip.add_css_class("settings-tabs");
        let mut tabs = Vec::new();
        let mut first: Option<gtk4::ToggleButton> = None;
        for tab in &TABS {
            let button = gtk4::ToggleButton::with_label(tab.title);
            button.add_css_class("settings-tab");
            if let Some(first) = &first {
                button.set_group(Some(first));
            } else {
                button.set_active(true);
                first = Some(button.clone());
            }
            {
                let stack = stack.clone();
                let name = tab.name;
                button.connect_toggled(move |b| {
                    if b.is_active() {
                        stack.set_visible_child_name(name);
                    }
                });
            }
            strip.append(&button);
            tabs.push((tab.name, button));
        }

        root.append(&strip);
        root.append(&stack);

        SettingsSection {
            root,
            tabs,
            look,
            idle,
            bar,
            alerts,
            glass,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Switch to a tab by its stack name. Unknown names are ignored.
    pub fn show(&self, name: &str) {
        if let Some((_, button)) = self.tabs.iter().find(|(n, _)| *n == name) {
            // Setting the toggle drives the stack through its handler, so
            // the strip and the page cannot disagree.
            button.set_active(true);
        } else {
            log::warn!("settings: no tab named {name}");
        }
    }

    /// Re-read every tab from what it edits. The panel refreshes every
    /// section when it opens.
    pub fn refresh(&self) {
        self.look.refresh();
        self.idle.refresh();
        self.bar.refresh();
        self.alerts.refresh();
        self.glass.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_prefix_opens_exactly_one_tab() {
        let mut seen = std::collections::HashSet::new();
        for tab in &TABS {
            for prefix in tab.prefixes {
                assert_eq!(tab_for_prefix(prefix), Some(tab.name), "{prefix}");
                assert!(seen.insert(*prefix), "{prefix} is claimed twice");
            }
        }
        assert_eq!(tab_for_prefix(":set"), None);
        assert_eq!(tab_for_prefix(":wallpaper"), Some("look"));
        assert_eq!(prefixes().count(), TABS.len());
    }
}
