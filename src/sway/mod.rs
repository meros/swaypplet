//! Sway: the IPC connection and service (`ipc`), what its workspaces are
//! called (`workspace`), and searches over its layout tree (`tree`). No GTK
//! widgets here; every surface that reads sway reads it through this.

pub mod ipc;
pub mod tree;
pub mod workspace;
