//! Types shared by every part of Stemma: the addresses of things, the
//! references to them written in Markdown, the bodies of the hub's API and
//! the protocol of its workers.

pub mod address;
pub mod api;
pub mod context;
pub mod diff;
pub mod locator;
pub mod mention;
pub mod paper;
pub mod protocol;
pub mod reference;

pub use address::{Address, AddressError, AddressKind};
pub use locator::{Located, locate};
pub use mention::mentions;
pub use reference::{Reference, references};
