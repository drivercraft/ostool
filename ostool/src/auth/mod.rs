//! Authentication, credential storage, and token lifecycle management.
//!
//! This module communicates with the authentication gateway independently from
//! the board-service client. Board operations may consume an access token, but
//! they do not own login, refresh, revoke, or credential persistence.

mod client;
mod credential_store;

/// Access-token acquisition, refresh, logout, and local authentication state.
pub mod token_manager;
