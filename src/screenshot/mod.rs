//! Screenshots: freeze, select, keep.
//!
//! Three external programs used to cover this ground, badly. `slurp` drew the
//! selection on a live screen, `grim` captured a *different* moment through a
//! shell pipeline nobody watched, and `hyprpicker` — a Hyprland tool — was
//! carried for one colour-pick button. The panel spawned them and forgot them:
//! no shutter, no preview, no way to get a file and a clipboard entry from one
//! gesture, nothing to annotate with.
//!
//! One flow replaces all of it. Capture first (`capture`), select on the
//! frozen image (`select`), then save, copy, and post the card that offers
//! what to do next (`deliver`) — with the annotation editor (`annotate`) one
//! button away on that card.
//!
//! ```text
//!   take(Region) ─→ capture every output ─→ selector surfaces ─→ crop
//!                                                                 │
//!                            ┌────────────────────────────────────┘
//!                            ↓
//!             copy + save + card ──[Annotate]──→ editor ──→ copy + save + card
//!                                 ──[Open]────→ default PNG handler
//!                                 ──[Delete]──→ unlink
//! ```

pub mod annotate;
pub mod capture;
pub mod deliver;
pub mod record;
pub mod select;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::notifications::store::StoreRef;

/// What the owner asked for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Shot {
    /// Drag a rectangle; a click without a drag takes the whole output.
    Region,
    /// The focused output, no selector.
    Screen,
    /// Report the colour under the pointer instead of keeping an image.
    Pick,
    /// Record video of a selected region.
    Record,
}

impl Shot {
    pub fn parse(arg: Option<&str>) -> Shot {
        match arg {
            Some("screen") | Some("output") => Shot::Screen,
            Some("pick") | Some("color") | Some("colour") => Shot::Pick,
            Some("record") | Some("rec") | Some("video") => Shot::Record,
            _ => Shot::Region,
        }
    }
}

/// Notifications this module posted, so their buttons know which file they
/// are talking about.
///
/// Bounded, because a card that has scrolled out of history has no button
/// left to press and its entry is dead weight.
const REMEMBERED: usize = 8;

#[derive(Default)]
struct Cards {
    order: Vec<u32>,
    paths: HashMap<u32, PathBuf>,
}

impl Cards {
    fn remember(&mut self, id: u32, path: PathBuf) {
        self.order.push(id);
        self.paths.insert(id, path);
        while self.order.len() > REMEMBERED {
            let evicted = self.order.remove(0);
            self.paths.remove(&evicted);
        }
    }
}

thread_local! {
    static CARDS: RefCell<Cards> = RefCell::new(Cards::default());
}

/// Wire the card buttons up. Called once, at startup, alongside the store.
pub fn install(app: &gtk4::Application, store: &StoreRef) {
    let app = app.clone();
    let store_for_actions = store.clone();
    store.borrow_mut().connect_action(move |id, key| {
        let Some(path) = CARDS.with(|c| c.borrow().paths.get(&id).cloned()) else {
            return; // someone else's notification
        };
        match key {
            "open" => deliver::open(&path),
            "delete" => deliver::delete(&path),
            "annotate" => match deliver::load_png(&path) {
                Ok(image) => {
                    let store = store_for_actions.clone();
                    // The annotated version is a new shot, not an edit of
                    // the old one: the original file stays where it is,
                    // which is the only behaviour that cannot lose work.
                    annotate::open(&app, image, move |edited| {
                        keep(&store, &edited);
                    });
                }
                Err(e) => log::warn!("screenshot: {e}"),
            },
            _ => {}
        }
    });
}

/// Take a shot or start a recording.
pub fn take(app: &gtk4::Application, store: &StoreRef, shot: Shot) {
    if shot == Shot::Record {
        record::toggle(app, store);
        return;
    }

    let store = store.clone();
    let app_for_editor = app.clone();
    let mode = match shot {
        Shot::Pick => select::Mode::Pick,
        _ => select::Mode::Region,
    };

    // `Screen` still goes through the selector: it is what holds the frozen
    // image, and Enter on it is one keystroke. Skipping it would mean a
    // second capture path that could disagree with the first.
    select::region(app, mode, move |selection| {
        let Some(selection) = selection else {
            return;
        };
        match shot {
            Shot::Pick => pick(&store, &selection),
            _ => {
                keep(&store, &selection.image);
                // The Capture group's "annotate every shot": the editor
                // opens on the shot just kept, and what it produces is a
                // second shot. Kept first, so a closed editor loses nothing.
                if crate::settings::store::current().capture().annotate {
                    let store = store.clone();
                    annotate::open(&app_for_editor, selection.image.clone(), move |edited| {
                        keep(&store, &edited);
                    });
                }
            }
        }
    });
}

/// Save, copy, and post the card — remembering which file the card is for.
fn keep(store: &StoreRef, image: &capture::Image) {
    let (id, path) = deliver::finish(store, image);
    if let Some(path) = path {
        CARDS.with(|c| c.borrow_mut().remember(id, path));
    }
}

/// A picked colour goes to the clipboard as `#rrggbb` and says so.
///
/// Text, not an image: a colour is a value you paste into a stylesheet, and
/// hyprpicker's `-a` flag existed to say exactly that.
fn pick(store: &StoreRef, selection: &select::Selection) {
    let Some((r, g, b)) = selection.image.pixel(0, 0) else {
        return;
    };
    let hex = format!("#{r:02x}{g:02x}{b:02x}");

    if let Some(display) = gtk4::gdk::Display::default() {
        use gtk4::prelude::DisplayExt;
        display.clipboard().set_text(&hex);
    }

    crate::notifications::store::store_add(
        store,
        crate::notifications::Notification {
            app_name: "Colour picker".into(),
            summary: hex.clone(),
            body: format!("rgb({r}, {g}, {b}) — copied"),
            expire_timeout: 5000,
            ..Default::default()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_shot_is_a_region() {
        assert!(Shot::parse(None) == Shot::Region);
        assert!(Shot::parse(Some("region")) == Shot::Region);
        assert!(Shot::parse(Some("nonsense")) == Shot::Region);
    }

    #[test]
    fn screen_and_pick_are_named() {
        assert!(Shot::parse(Some("screen")) == Shot::Screen);
        assert!(Shot::parse(Some("output")) == Shot::Screen);
        assert!(Shot::parse(Some("pick")) == Shot::Pick);
        assert!(Shot::parse(Some("colour")) == Shot::Pick);
    }

    #[test]
    fn the_card_map_forgets_the_oldest_first() {
        let mut cards = Cards::default();
        for id in 0..(REMEMBERED as u32 + 2) {
            cards.remember(id, PathBuf::from(format!("/tmp/{id}.png")));
        }
        assert_eq!(cards.paths.len(), REMEMBERED);
        assert!(!cards.paths.contains_key(&0), "oldest evicted");
        assert!(
            cards.paths.contains_key(&(REMEMBERED as u32 + 1)),
            "newest kept"
        );
    }
}
