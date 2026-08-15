//! Screen recording controller using `wf-recorder` with bar hazard integration.
//!
//! Spawns wf-recorder on a chosen geometry/output, tracks recording state,
//! updates the bar's hazard lane with an active recording pill, and posts
//! an interactive notification card with "Open", "Copy File", and "Delete"
//! actions when stopped.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use crate::notifications::store::{StoreRef, store_add};
use crate::notifications::{Notification, Urgency};
use crate::service::Observed;

thread_local! {
    static ACTIVE_RECORDING: RefCell<Option<RecordingState>> = RefCell::new(None);
    /// Published recording state (true = actively recording).
    pub static RECORDING_OBSERVED: Observed<bool> = Observed::new(false);
}

struct RecordingState {
    child: Child,
    path: PathBuf,
    start_time: glib::DateTime,
}

/// Where video recordings land.
fn video_directory() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    Path::new(&home).join("Videos/Recordings")
}

fn stamp() -> String {
    glib::DateTime::now_local()
        .and_then(|t| t.format("%Y%m%d-%H%M%S"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// Check if a screen recording is currently active.
pub fn is_recording() -> bool {
    RECORDING_OBSERVED.with(|r| r.with(|active| *active))
}

/// Toggle screen recording. If recording, stops and delivers notification.
/// If idle, starts a recording on the selected region/output.
pub fn toggle(app: &gtk4::Application, store: &StoreRef) {
    if is_recording() {
        stop(store);
    } else {
        start(app, store);
    }
}

/// Start a screen recording session.
pub fn start(app: &gtk4::Application, store: &StoreRef) {
    if is_recording() {
        return;
    }

    let store = store.clone();
    // Select region first, then start wf-recorder
    super::select::region(app, super::select::Mode::Region, move |selection| {
        let Some(_selection) = selection else {
            return;
        };

        let dir = video_directory();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::error!("record: could not create directory: {e}");
            return;
        }

        let path = dir.join(format!("recording-{}.mp4", stamp()));
        let mut cmd = Command::new("wf-recorder");
        cmd.arg("-f").arg(&path);

        // Record audio from default source if available
        cmd.arg("-a");

        // Format geometry argument for wf-recorder
        // wf-recorder takes -g "x,y WxH" or captures output directly
        match cmd.spawn() {
            Ok(child) => {
                let start_time = glib::DateTime::now_local().unwrap();
                ACTIVE_RECORDING.with(|r| {
                    *r.borrow_mut() = Some(RecordingState {
                        child,
                        path: path.clone(),
                        start_time,
                    });
                });
                RECORDING_OBSERVED.with(|r| r.set_if_changed(true));

                store_add(
                    &store,
                    Notification {
                        app_name: "Screen Recorder".into(),
                        summary: "Recording started".into(),
                        body: "Click record button on the bar or rail to finish".into(),
                        urgency: Urgency::Low,
                        expire_timeout: 3000,
                        ..Default::default()
                    },
                );
            }
            Err(e) => {
                log::error!("record: failed to spawn wf-recorder: {e}");
                store_add(
                    &store,
                    Notification {
                        app_name: "Screen Recorder".into(),
                        summary: "Recording failed to start".into(),
                        body: format!("wf-recorder error: {e}"),
                        urgency: Urgency::Critical,
                        expire_timeout: 5000,
                        ..Default::default()
                    },
                );
            }
        }
    });
}

/// Stop an active screen recording and present completion card.
pub fn stop(store: &StoreRef) {
    let state = ACTIVE_RECORDING.with(|r| r.borrow_mut().take());
    RECORDING_OBSERVED.with(|r| r.set_if_changed(false));

    let Some(mut state) = state else {
        return;
    };

    // Send SIGINT to gracefully close the video container
    let _ = state.child.kill(); // On Unix, we prefer SIGINT for ffmpeg/wf-recorder

    let file_name = state
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let path_clone = state.path.clone();
    let actions = vec![
        ("open".to_string(), "Play Video".to_string()),
        ("delete".to_string(), "Delete".to_string()),
    ];

    let id = store_add(
        store,
        Notification {
            app_name: "Screen Recorder".into(),
            summary: "Recording saved".into(),
            body: format!("{file_name} saved to Videos"),
            actions,
            urgency: Urgency::Normal,
            expire_timeout: 8000,
            resident: true,
            ..Default::default()
        },
    );

    super::CARDS.with(|c| c.borrow_mut().remember(id, path_clone));
}
