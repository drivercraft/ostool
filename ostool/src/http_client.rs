//! Centralizes process-wide TLS provider selection and HTTP client construction.
//!
//! Although this module currently exposes only one small function, keeping it separate prevents
//! transport setup from being coupled to authentication, board, or build logic. All reqwest clients
//! should be created here so Ring is installed before Rustls is used and future transport changes
//! remain confined to one place.

pub(crate) fn builder() -> reqwest::ClientBuilder {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
}
