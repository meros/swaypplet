//! An extra screen for a TV: a virtual sway output (`output`), sent
//! somewhere by a stream backend (`backend`), to a target found on the
//! network (`discovery`), in the panel process.
//!
//! One at a time. Starting makes the output and starts the backend on it,
//! on a worker; the output is removed again when the backend cannot start.
//! Stopping ends the stream and removes the output, and its workspaces move
//! to the screens. A stream that ends by itself (wayvnc exited) is noticed
//! within [`WATCH`] and stopped the same way, with the reason in the view.
//!
//! What runs at rest: nothing without a session; with one, a check of the
//! child process every [`WATCH`]. The device list is one avahi browse, when
//! the Displays page is shown ([`search_if_stale`]).

pub mod backend;
pub mod discovery;
pub mod output;

use std::cell::RefCell;
use std::time::{Duration, Instant};

use backend::{Stream, Target};
use discovery::Device;

use crate::service::Observed;
use crate::spawn::spawn_work;

/// How long a browse waits for avahi to list its cache.
const BROWSE_WAIT: Duration = Duration::from_secs(3);
/// A list younger than this is not searched again when the page is shown.
const FRESH: Duration = Duration::from_secs(30);
/// How often a running stream is checked.
const WATCH: Duration = Duration::from_secs(2);

/// What the section shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct View {
    /// The Cast devices with a screen, from the last search.
    pub devices: Vec<Device>,
    pub searching: bool,
    /// Starting or stopping: the buttons wait.
    pub busy: bool,
    pub session: Option<SessionView>,
    /// The last thing that went wrong, in words, until the next action.
    pub problem: Option<String>,
}

/// The extra screen that is on.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionView {
    /// Its sway output, HEADLESS-N.
    pub output: String,
    pub target: Target,
    /// The backend's name and where the stream can be seen.
    pub backend: &'static str,
    pub location: String,
}

struct Session {
    view: SessionView,
    stream: Box<dyn Stream>,
    watch: glib::SourceId,
}

thread_local! {
    static VIEW: Observed<View> = Observed::new(View::default());
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static SEARCHED: RefCell<Option<Instant>> = const { RefCell::new(None) };
}

/// Read the current view.
pub fn view() -> View {
    VIEW.with(|v| v.with(Clone::clone))
}

/// Run `cb` after the view changes.
pub fn observe(cb: impl Fn() + 'static) {
    VIEW.with(|v| v.connect_change(cb));
}

fn edit(f: impl FnOnce(&mut View)) {
    let mut v = view();
    f(&mut v);
    VIEW.with(|o| o.set_if_changed(v));
}

/// Call once, in the panel process: removes an extra screen the panel
/// before this one left (`output::sweep`).
pub fn start() {
    spawn_work(output::sweep, |()| {});
}

/// Search the network for Cast devices, unless the list is fresh or a
/// search runs.
pub fn search_if_stale() {
    let fresh = SEARCHED.with(|s| s.borrow().is_some_and(|t| t.elapsed() < FRESH));
    if !fresh {
        search();
    }
}

/// Search the network for Cast devices now.
pub fn search() {
    if view().searching {
        return;
    }
    SEARCHED.with(|s| *s.borrow_mut() = Some(Instant::now()));
    edit(|v| v.searching = true);
    spawn_work(
        || discovery::browse_blocking(BROWSE_WAIT),
        |found| {
            edit(|v| {
                v.searching = false;
                match found {
                    Ok(devices) => {
                        v.devices = devices.into_iter().filter(|d| d.video).collect();
                    }
                    Err(e) => {
                        log::info!("cast: {e}");
                        v.problem = Some(format!("Could not look for Cast devices: {e}"));
                    }
                }
            });
        },
    );
}

/// Make the extra screen and send it to `target`.
pub fn begin(target: Target) {
    let v = view();
    if v.busy || v.session.is_some() {
        return;
    }
    let Some(backend) = backend::for_target(&target) else {
        edit(|v| v.problem = Some("Nothing built here can send to that device yet.".into()));
        return;
    };
    edit(|v| {
        v.busy = true;
        v.problem = None;
    });
    let for_worker = target.clone();
    spawn_work(
        move || {
            let name = output::create()?;
            match backend.start(&name, &for_worker) {
                Ok(stream) => Ok((name, stream)),
                Err(e) => {
                    if let Err(r) = output::remove(&name) {
                        log::warn!("cast: {r}");
                    }
                    Err(e)
                }
            }
        },
        move |result: Result<(String, Box<dyn Stream>), String>| match result {
            Ok((name, stream)) => {
                let view = SessionView {
                    output: name,
                    target,
                    backend: backend.name(),
                    location: stream.describe(),
                };
                let watch = glib::timeout_add_local(WATCH, watch_stream);
                SESSION.with(|s| {
                    *s.borrow_mut() = Some(Session {
                        view: view.clone(),
                        stream,
                        watch,
                    })
                });
                edit(|v| {
                    v.busy = false;
                    v.session = Some(view);
                });
            }
            Err(e) => {
                log::warn!("cast: {e}");
                edit(|v| {
                    v.busy = false;
                    v.problem = Some(e);
                });
            }
        },
    );
}

/// End the stream and remove the extra screen.
pub fn end() {
    end_with(None);
}

fn end_with(problem: Option<String>) {
    let Some(mut session) = SESSION.with(|s| s.borrow_mut().take()) else {
        return;
    };
    crate::spawn::remove_source(session.watch);
    session.stream.stop();
    let name = session.view.output.clone();
    edit(|v| {
        v.busy = true;
        v.problem = problem;
    });
    spawn_work(
        move || output::remove(&name),
        |result| {
            edit(|v| {
                v.busy = false;
                v.session = None;
                if let Err(e) = result {
                    log::warn!("cast: {e}");
                    v.problem.get_or_insert(e);
                }
            });
        },
    );
}

/// `swaypplet cast list`: every Cast device on the network, speakers too,
/// one per line: name, model, address and port, and whether it has a
/// screen, tab separated. No GTK.
pub fn cli(mut args: impl Iterator<Item = String>) {
    match args.next().as_deref() {
        Some("list") => match discovery::browse_blocking(BROWSE_WAIT) {
            Ok(devices) => {
                for d in devices {
                    let kind = match (d.video, d.group) {
                        (true, _) => "screen",
                        (false, true) => "group",
                        (false, false) => "audio",
                    };
                    println!(
                        "{}\t{}\t{}:{}\t{kind}",
                        d.name,
                        d.model.as_deref().unwrap_or("-"),
                        d.address,
                        d.port
                    );
                }
            }
            Err(e) => {
                eprintln!("swaypplet cast: {e}");
                std::process::exit(1);
            }
        },
        _ => {
            eprintln!("usage: swaypplet cast list");
            std::process::exit(2);
        }
    }
}

fn watch_stream() -> glib::ControlFlow {
    let ended = SESSION.with(|s| {
        s.borrow_mut()
            .as_mut()
            .map(|s| !s.stream.running())
            .unwrap_or(true)
    });
    if ended {
        // `end_with` removes this source; it is still running, so break
        // instead and let the removal find it gone.
        glib::idle_add_local_once(|| end_with(Some("The stream ended by itself.".into())));
        return glib::ControlFlow::Break;
    }
    glib::ControlFlow::Continue
}
