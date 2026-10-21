//! Filesystem adapters for one-time imports and isolated test fixtures.
//! Runtime repositories use PostgreSQL; connection failures never select these adapters.
pub mod boards;
pub mod dtbs;
