//! Types shared by every part of Stemma: the addresses of things, the
//! references to them written in Markdown, the bodies of the hub's API and
//! the protocol of its machines.

pub mod address;
pub mod api;
pub mod context;
pub mod locator;
pub mod mention;
pub mod protocol;
pub mod reference;
pub mod secrets;
pub mod text;

pub use address::{Address, AddressError, AddressKind};
pub use locator::{Located, locate};
pub use mention::mentions;
pub use reference::{Reference, references};

pub mod recipe;
pub mod workshop;
