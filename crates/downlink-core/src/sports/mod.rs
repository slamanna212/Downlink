//! Sports tracking (docs/SPORTS.md): the embedded catalog, the pure matcher, and the
//! `sports_match` store the commands read. Local only: guide rows and channel names, never a
//! stream endpoint or an outside schedule API.

pub mod catalog;
pub mod group;
pub mod matcher;
pub mod store;

pub use catalog::{Catalog, catalog};
