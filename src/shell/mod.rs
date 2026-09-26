//! What every swaypplet surface is made of (docs/design-system.md §6.1,
//! Surfaces).
//!
//! - [`Namespace`]: the layer-shell namespace, which is also the key the
//!   compositor's glass and the settings pane address a surface by.
//! - [`Surface`]: a layer surface with its root, its glass card and the
//!   transition that shows it, built by one builder and torn down in the one
//!   order the protocols allow.
//! - [`PerMonitor`]: one of something per output, following hotplug.
//! - [`fit`]: a card fitted to the output it opened on.
//! - [`layer`]: the layer-shell window underneath all of them. Only this
//!   module creates one (design lint `layer-window`).

pub mod fit;
pub mod layer;
mod namespace;
mod per_monitor;
mod surface;

pub use namespace::Namespace;
pub use per_monitor::PerMonitor;
pub use surface::Surface;
