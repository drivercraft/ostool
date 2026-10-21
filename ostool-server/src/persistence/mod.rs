//! Persistent storage and explicit legacy-file adapters.
pub mod boards;
pub mod dtbs;
pub mod legacy;
pub mod postgres;
pub use boards::BoardStore;
pub use dtbs::{DtbFile, DtbStore};
pub use postgres::Database;
