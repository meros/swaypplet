//! Claude task pill — middle segment of the bar's right instrument track.
//!
//! A TaskStateService consumer: session data comes from the shared
//! snapshot; what stays here is the per-output task selection (this
//! output's visible workspace → task number) and the render. Sway drives
//! the selection and the service drives the data, so both observers fire
//! the same refresh and the view cache drops the no-ops.
//!
//! Accent ripple: the pill stamps bar-task1..4 on its bar's root and
//! style.css colors the track band + focused workspace from there —
//! descendant selectors replace waybar's invisible emitter modules and
//! their general-sibling hack.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::spawn::spawn_work;
use crate::sway_ipc::SwayService;
use crate::task_state::{Activity, TaskStateService, task_of_name};

const TASK_CLASSES: [&str; 4] = ["bar-task1", "bar-task2", "bar-task3", "bar-task4"];
/// Waybar left descriptions uncapped; a runaway description would push the
/// clock off the card here, so each one ellipsizes instead.
const MAX_DESC_CHARS: i32 = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Session {
    desc: String,
    activity: Activity,
    progress: Option<String>,
}

/// Everything the pill shows, comparable so no-op refreshes (sway fires
/// per keystroke on title changes) skip the widget rebuild.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PillView {
    /// 1–4 when this output's visible workspace belongs to a task.
    task: Option<u8>,
    sessions: Vec<Session>,
    manual: Option<String>,
}

impl PillView {
    fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.manual.is_none()
    }
}

/// `output` is this bar's sway output name (gdk connector); `bar_root`
/// receives the bar-taskN accent class.
pub fn build(
    sway: &Rc<SwayService>,
    tasks: &Rc<TaskStateService>,
    output: Option<String>,
    bar_root: &impl IsA<gtk4::Widget>,
) -> gtk4::Button {
    let content = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(5)
        .build();
    let btn = gtk4::Button::builder()
        .child(&content)
        .css_classes(["bar-task", "bar-seg", "none"])
        .build();

    let refresh: Rc<dyn Fn()> = {
        let weak = btn.downgrade();
        let content = content.clone();
        let root = bar_root.upcast_ref::<gtk4::Widget>().downgrade();
        let sway = sway.clone();
        let tasks = tasks.clone();
        let cache: RefCell<Option<PillView>> = RefCell::new(None);
        Rc::new(move || {
            // Output unplugged → the leftover observer no-ops (same story
            // as workspaces.rs).
            let Some(btn) = weak.upgrade() else {
                return;
            };
            let view = read_view(output.as_deref(), &sway, &tasks);
            if cache.borrow().as_ref() == Some(&view) {
                return;
            }
            if let Some(root) = root.upgrade() {
                apply_accent(&root, view.task);
            }
            render(&btn, &content, &view);
            *cache.borrow_mut() = Some(view);
        })
    };

    refresh();
    {
        let refresh = refresh.clone();
        sway.connect_change(move || refresh());
    }
    {
        let refresh = refresh.clone();
        tasks.connect_change(move || refresh());
    }

    btn.connect_clicked(|_| run_task_command("task-find"));
    let right = gtk4::GestureClick::new();
    right.set_button(gtk4::gdk::BUTTON_SECONDARY);
    right.connect_pressed(|_, _, _, _| run_task_command("task-rename"));
    btn.add_controller(right);

    btn
}

/// task-find / task-rename come from the nixos config's PATH; a machine
/// without them just logs. `status()` runs on a worker so the picker the
/// script opens can't block the bar, and the child gets reaped.
fn run_task_command(cmd: &'static str) {
    spawn_work(
        move || {
            std::process::Command::new(cmd)
                .status()
                .map_err(|e| e.to_string())
        },
        move |result| {
            if let Err(e) = result {
                log::warn!("bar task: `{cmd}`: {e}");
            }
        },
    );
}

/// Project this output's slice of the shared snapshot into a comparable
/// view.
fn read_view(output: Option<&str>, sway: &SwayService, tasks: &TaskStateService) -> PillView {
    let task = output
        .and_then(|out| {
            sway.snapshot()
                .workspaces
                .into_iter()
                .find(|w| w.visible && w.output == out)
        })
        .and_then(|w| task_of_name(&w.name));
    let Some(task) = task else {
        return PillView::default();
    };

    let snapshot = tasks.snapshot();
    let state = snapshot.task(task);
    PillView {
        task: Some(task),
        sessions: state
            .sessions
            .iter()
            .map(|s| Session {
                desc: s.desc.clone(),
                activity: s.activity,
                progress: s.progress.as_ref().map(|p| p.raw.clone()),
            })
            .collect(),
        manual: state.manual.clone(),
    }
}

// ── Rendering ───────────────────────────────────────────────────────────

fn apply_accent(root: &gtk4::Widget, task: Option<u8>) {
    for (i, class) in TASK_CLASSES.iter().enumerate() {
        if task == Some(i as u8 + 1) {
            root.add_css_class(class);
        } else {
            root.remove_css_class(class);
        }
    }
}

fn render(btn: &gtk4::Button, content: &gtk4::Box, view: &PillView) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    for (i, session) in view.sessions.iter().enumerate() {
        if i > 0 {
            content.append(&dim_label("|"));
        }
        let dot = gtk4::Label::new(Some(session.activity.glyph()));
        dot.add_css_class("bar-task-dot");
        dot.add_css_class(session.activity.css_class());
        content.append(&dot);
        content.append(&desc_label(&session.desc));
        if let Some(progress) = &session.progress {
            content.append(&dim_label("·"));
            content.append(&desc_label(progress));
        }
    }
    if let Some(manual) = &view.manual {
        if !view.sessions.is_empty() {
            content.append(&dim_label("|"));
        }
        content.append(&desc_label(manual));
    }

    // No session and no manual name → no pill. An empty reserved slot read
    // as a hole in the track; battery|clock close ranks instead (the CSS
    // first/last-child radii still see the hidden widget, so the track's
    // rounded ends stay put).
    let empty = view.is_empty();
    btn.set_visible(!empty);
    if empty {
        btn.set_tooltip_text(None);
    } else if let Some(task) = view.task {
        btn.set_tooltip_text(Some(&format!("Task {task}")));
    }
}

fn desc_label(text: &str) -> gtk4::Label {
    gtk4::Label::builder()
        .label(text)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(MAX_DESC_CHARS)
        .build()
}

fn dim_label(text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.add_css_class("bar-task-dim");
    label
}
