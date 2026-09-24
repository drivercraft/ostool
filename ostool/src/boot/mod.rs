//! Boot artifact helpers.

pub(crate) mod artifacts;
pub(crate) mod fit;
// The configuration entry point is internal until boot preparation consumes it.
#[allow(dead_code)]
pub(crate) mod fit_config;
