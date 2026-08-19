//! A moving backdrop, for exercising the live path.
//!
//! `SWAYPPLET_LOCK_LIVE=1` wraps the wallpaper in this instead of handing the
//! texture over directly, so the glass has something behind it that actually
//! changes. It is a dev hook in the same family as `SWAYPPLET_PREVIEW_CAPS`
//! and `SWAYPPLET_PREVIEW_AVATAR`: off unless asked for, and nothing in the
//! lock path depends on it.
//!
//! It exists because "does the glass support a live backdrop" is not a
//! question a still wallpaper can answer. A refraction that were secretly
//! sampling a cached frame would look identical on a still image and only
//! come apart on a moving one — so the pattern below deliberately moves a
//! hard-edged bar across the card, where any staleness shows immediately.
//!
//! Anything implementing `GdkPaintable` works the same way, a `GtkMediaFile`
//! playing a video included; this one is just the cheapest source that needs
//! no media on disk.

use gtk4::prelude::*;
use gtk4::{gdk, glib, graphene, subclass::prelude::*};

/// Frame interval. 16 ms rather than a frame clock because a paintable is not
/// a widget and has no clock of its own; the pane only does work when this
/// fires, so the rate is the cost.
const TICK_MS: u64 = 16;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct LiveBackdrop {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub start: Cell<i64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LiveBackdrop {
        const NAME: &'static str = "SwayppletLiveBackdrop";
        type Type = super::LiveBackdrop;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for LiveBackdrop {}

    impl PaintableImpl for LiveBackdrop {
        fn intrinsic_width(&self) -> i32 {
            self.texture.borrow().as_ref().map_or(0, |t| t.width())
        }

        fn intrinsic_height(&self) -> i32 {
            self.texture.borrow().as_ref().map_or(0, |t| t.height())
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            // Every GdkSnapshot GTK hands out is a GtkSnapshot, but this runs
            // on the lock screen's path and a wrong guess there is a panic
            // with the session held, so it declines rather than asserts.
            let Some(snapshot) = snapshot.downcast_ref::<gtk4::Snapshot>() else {
                return;
            };
            let (w, h) = (width as f32, height as f32);
            let full = graphene::Rect::new(0.0, 0.0, w, h);

            if let Some(texture) = self.texture.borrow().as_ref() {
                snapshot.append_texture(texture, &full);
            } else {
                snapshot.append_color(&gdk::RGBA::new(0.04, 0.05, 0.08, 1.0), &full);
            }

            let t = (glib::monotonic_time() - self.start.get()) as f32 / 1_000_000.0;
            // A hard-edged bar: soft gradients hide staleness, edges do not.
            let x = (t * 0.14).fract() * (w + 240.0) - 120.0;
            snapshot.append_color(
                &gdk::RGBA::new(1.0, 0.92, 0.72, 0.55),
                &graphene::Rect::new(x, 0.0, 40.0, h),
            );
            snapshot.append_color(
                &gdk::RGBA::new(0.35, 0.75, 1.0, 0.45),
                &graphene::Rect::new(x + 70.0, 0.0, 18.0, h),
            );
        }
    }
}

glib::wrapper! {
    pub struct LiveBackdrop(ObjectSubclass<imp::LiveBackdrop>) @implements gdk::Paintable;
}

impl LiveBackdrop {
    pub fn new(texture: Option<gdk::Texture>) -> Self {
        let obj: Self = glib::Object::new();
        *obj.imp().texture.borrow_mut() = texture;
        obj.imp().start.set(glib::monotonic_time());
        glib::timeout_add_local(std::time::Duration::from_millis(TICK_MS), {
            let weak = obj.downgrade();
            move || match weak.upgrade() {
                Some(obj) => {
                    obj.invalidate_contents();
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            }
        });
        obj
    }
}

/// Is the live-backdrop dev hook on?
pub fn wanted() -> bool {
    matches!(std::env::var("SWAYPPLET_LOCK_LIVE").as_deref(), Ok("1"))
}

/// A video file as the backdrop, looping and muted.
///
/// `GtkMediaFile` is a `GdkPaintable`, so the glass needs no special case for
/// it: `GlassPane::set_backdrop` takes any paintable and follows its
/// `invalidate-contents`, which for a video is one signal per decoded frame.
///
/// Returns `None` when GTK has no media backend compiled in, which is the
/// usual reason this fails rather than anything to do with the file: the
/// backend is a separate `libmedia-gstreamer.so` module, and a GTK built
/// without it hands back a `GtkNothingMediaFile` that reports an error and
/// paints nothing. The caller falls back to the still wallpaper.
pub fn video(path: &str) -> Option<gdk::Paintable> {
    let file = gtk4::MediaFile::for_filename(path);
    if let Some(err) = file.error() {
        log::warn!("lock video {path}: {err}");
        return None;
    }
    file.set_loop(true);
    file.set_muted(true);
    file.play();
    // The error often only arrives once the backend has looked at the file,
    // so re-check after the stream has had a chance to prepare, and drop back
    // to the still wallpaper if it never comes good.
    let weak = file.downgrade();
    glib::timeout_add_local_once(std::time::Duration::from_millis(700), move || {
        if let Some(f) = weak.upgrade()
            && let Some(err) = f.error()
        {
            log::warn!("lock video: {err}");
        }
    });
    Some(file.upcast())
}
