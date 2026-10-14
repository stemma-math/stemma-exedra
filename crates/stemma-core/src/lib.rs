//! Types shared by every part of Stemma: the addresses of things, the
//! references to them written in Markdown, and the bodies of the hub's API.

pub mod address;
pub mod api;
pub mod reference;

pub use address::{Address, AddressError, AddressKind};
pub use reference::{Reference, references};
