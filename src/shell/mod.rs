//! What every swaypplet surface is made of: its layer-shell namespace
//! ([`Namespace`]) and its layer-shell window ([`layer`]).

pub mod layer;
mod namespace;

pub use namespace::Namespace;
