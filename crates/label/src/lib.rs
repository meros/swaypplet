//! Workspace names. The bar shows and edits them; this crate keeps them
//! (`store`) and draws the toast that says one when you arrive.
//!
//! **In the bar.** A workspace with a name shows it in its segment while a
//! screen is on it, and a right-click on any segment opens the editor
//! (`swaypplet-bar`, `name_editor.rs`). Nothing here opens by itself under
//! the pointer.
//!
//! **The toast.** When you settle on a workspace that has a name, a card at
//! the top middle of that output says the workspace's mark and its name,
//! large, for [`TOAST_MS`]. It is a confirmation of where the keys landed,
//! so it is built never to be in the way:
//!
//! - It waits [`SETTLE_MS`] after the switch. Stepping through five
//!   workspaces with the keys shows the last one once, not five cards.
//! - It takes no input at all. The pointer and the keyboard go through it to
//!   the window behind, so it cannot catch a click or hold a key.
//! - It never shows over a fullscreen window, and the mode `hidden` turns it
//!   off for good (`swaypplet label mode hidden`, or the toggle in the
//!   editor).
//!
//! The CLI is a request to this process: `swaypplet label` opens the editor
//! for the focused workspace, `swaypplet label mode <hidden|auto|next>`,
//! `swaypplet label clear`.

mod store;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use swaypplet_base::spawn::Timeout;
use swaypplet_base::sway::ipc::SwayService;
use swaypplet_base::sway::workspace::label_for_name;
use swaypplet_ui::shell::{Namespace, PerMonitor, Surface};
use swaypplet_ui::ui;

pub use store::{Mode, connect_changed, connect_edit, mode, name_of, set_mode, set_name};

/// How long the toast stays.
const TOAST_MS: u64 = 1800;
/// How long a workspace must stay put before the toast is shown for it.
const SETTLE_MS: u64 = 160;

/// The toast on one output.
struct Toast {
    connector: String,
    surface: Surface,
    mark: gtk4::Label,
    name: gtk4::Label,
}

impl Toast {
    fn new(app: &gtk4::Application, monitor: &gdk::Monitor) -> Toast {
        let connector = monitor
            .connector()
            .map(|c| c.to_string())
            .unwrap_or_default();
        let surface = Surface::builder(app, Namespace::Label)
            .monitor(Some(monitor))
            .anchor(&[Edge::Top])
            .card(ui::Card::Thin)
            .apron(gtk4::Align::Start, "label-wrapper")
            .build();

        let outer = surface.card();
        outer.set_halign(gtk4::Align::Center);
        outer.add_css_class("label-toast");

        // The workspace's own mark, as the bar draws it, in a chip: it says
        // where, and the name says what.
        let mark = gtk4::Label::new(None);
        ui::set_text_style(&mark, ui::Text::DisplaySm, ui::Tone::Accent);
        mark.set_hexpand(true);
        mark.set_vexpand(true);
        mark.set_halign(gtk4::Align::Center);
        mark.set_valign(gtk4::Align::Center);
        let tile = ui::card::well();
        tile.add_css_class("label-toast-mark");
        tile.set_valign(gtk4::Align::Center);
        tile.append(&mark);

        let name = gtk4::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(32)
            .build();
        ui::set_text_style(&name, ui::Text::DisplaySm, ui::Tone::Fg);
        name.set_valign(gtk4::Align::Center);

        let content = ui::hbox(4);
        content.append(&tile);
        content.append(&name);
        outer.append(&content);
        surface.set_content(&content);

        // Nothing on it takes input: the region is empty, set on every map
        // because it lives on the surface the compositor re-creates.
        surface.window().connect_map(|window| {
            if let Some(surface) = window.surface() {
                surface.set_input_region(Some(&gdk::cairo::Region::create()));
            }
        });

        Toast {
            connector,
            surface,
            mark,
            name,
        }
    }
}

struct Inner {
    sway: Rc<SwayService>,
    toasts: Rc<PerMonitor<Toast>>,
    /// Output → the workspace visible on it at the last sway event.
    visible: RefCell<HashMap<String, String>>,
    /// Output → the countdown before its toast shows, and the one that ends it.
    settle: RefCell<HashMap<String, Rc<Timeout>>>,
    hide: RefCell<HashMap<String, Rc<Timeout>>>,
}

/// Cloning shares the service.
#[derive(Clone)]
pub struct Labels {
    inner: Rc<Inner>,
}

impl Labels {
    pub fn new(app: &gtk4::Application, sway: Rc<SwayService>) -> Labels {
        let inner = Rc::new(Inner {
            sway: sway.clone(),
            toasts: PerMonitor::new(),
            visible: RefCell::new(HashMap::new()),
            settle: RefCell::new(HashMap::new()),
            hide: RefCell::new(HashMap::new()),
        });
        let labels = Labels { inner };
        // Not on an extra screen unless switched on (`shell::extra`).
        swaypplet_ui::shell::extra::follow(
            &labels.inner.toasts,
            swaypplet_ui::shell::extra::Kind::WorkspaceNames,
        );
        {
            let app = app.clone();
            labels
                .inner
                .toasts
                .watch(move |monitor| Toast::new(&app, monitor));
        }
        {
            let weak = Rc::downgrade(&labels.inner);
            sway.connect_change(move || {
                if let Some(inner) = weak.upgrade() {
                    Labels { inner }.on_sway();
                }
            });
        }
        labels.on_sway();
        labels
    }

    /// What the CLI asks for: `edit` (the default), `mode <hidden|auto|next>`,
    /// `clear`.
    pub fn command(&self, args: &[&str]) {
        match args {
            [] | ["edit"] | ["toggle"] => {
                if let Some(output) = self.inner.sway.focused_output()
                    && let Some(ws) = self.workspace_on(&output)
                {
                    store::request_edit(&output, &ws);
                }
            }
            ["mode", name] => {
                let mode = if *name == "next" {
                    Some(store::mode().next())
                } else {
                    Mode::parse(name)
                };
                match mode {
                    Some(mode) => set_mode(mode),
                    None => log::warn!("label: unknown mode `{name}`"),
                }
            }
            ["clear"] => {
                if let Some(ws) = self
                    .inner
                    .sway
                    .focused_output()
                    .and_then(|output| self.workspace_on(&output))
                {
                    set_name(&ws, "");
                }
            }
            other => log::warn!("label: unknown command {other:?}"),
        }
    }

    fn workspace_on(&self, output: &str) -> Option<String> {
        self.inner.visible.borrow().get(output).cloned()
    }

    fn timer(map: &RefCell<HashMap<String, Rc<Timeout>>>, output: &str) -> Rc<Timeout> {
        map.borrow_mut()
            .entry(output.to_string())
            .or_insert_with(|| Rc::new(Timeout::new()))
            .clone()
    }

    fn on_sway(&self) {
        let snapshot = self.inner.sway.snapshot();
        let mut switched = Vec::new();
        {
            let mut visible = self.inner.visible.borrow_mut();
            let now: HashMap<String, String> = snapshot
                .workspaces
                .iter()
                .filter(|w| w.visible)
                .map(|w| (w.output.clone(), w.name.clone()))
                .collect();
            for (output, name) in &now {
                // The first sight of an output is not a switch.
                if visible.get(output).is_some_and(|old| old != name) {
                    switched.push(output.clone());
                }
            }
            *visible = now;
        }

        // A toast for the workspace you left goes at once.
        for output in &switched {
            Self::timer(&self.inner.settle, output).cancel();
            self.hide_toast(output);
        }

        let fullscreen = snapshot.fullscreen_on_focused_output;
        if store::mode() != Mode::Auto || fullscreen {
            return;
        }
        for output in switched {
            let weak: Weak<Inner> = Rc::downgrade(&self.inner);
            let out = output.clone();
            Self::timer(&self.inner.settle, &output).arm(
                Duration::from_millis(SETTLE_MS),
                move || {
                    if let Some(inner) = weak.upgrade() {
                        Labels { inner }.show_toast(&out);
                    }
                },
            );
        }
    }

    fn show_toast(&self, output: &str) {
        let Some(ws) = self.workspace_on(output) else {
            return;
        };
        let Some(text) = name_of(&ws) else { return };
        if store::mode() != Mode::Auto || self.inner.sway.snapshot().fullscreen_on_focused_output {
            return;
        }
        self.inner.toasts.for_each(|toast| {
            if toast.connector == output {
                toast.mark.set_label(&label_for_name(&ws));
                toast.name.set_label(&text);
                toast.surface.show();
            }
        });
        let weak = Rc::downgrade(&self.inner);
        let out = output.to_string();
        Self::timer(&self.inner.hide, output).arm(Duration::from_millis(TOAST_MS), move || {
            if let Some(inner) = weak.upgrade() {
                Labels { inner }.hide_toast(&out);
            }
        });
    }

    fn hide_toast(&self, output: &str) {
        Self::timer(&self.inner.hide, output).cancel();
        self.inner.toasts.for_each(|toast| {
            if toast.connector == output {
                toast.surface.hide();
            }
        });
    }
}
