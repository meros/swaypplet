//! The shell's services: what the machine and the session are doing, read
//! and changed without any widget in sight. Each one owns its connection or
//! its reads (a D-Bus proxy, a sysfs file, a Wayland protocol, a socket),
//! and the GTK side learns of changes through [`crate::service::Observed`]
//! where there is a state to observe. Surfaces (`bar`, `widgets`, `panel`,
//! `notifications`) depend on these; nothing here depends on a surface.
//!
//! Sway is the exception to living here: its IPC, workspace naming and tree
//! searches are their own module (`crate::sway`), because the services read
//! it too.

pub mod audio;
pub mod backup;
pub mod bluez;
pub mod clipboard;
pub mod displays;
pub mod elephant;
pub mod gamma;
pub mod inhibit;
pub mod lid;
pub mod mpris;
pub mod network;
pub mod notifications;
pub mod power;
pub mod presence;
pub mod task_state;
pub mod tray;
