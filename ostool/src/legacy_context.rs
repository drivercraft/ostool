//! Temporary compatibility bridge between `InvocationState` and legacy `AppContext`.
//!
//! `InvocationState` is the source of truth. `AppContext` is kept as a legacy
//! mirror for existing `Tool::ctx`, `Tool::ctx_mut`, and `Tool::into_context`
//! callers. Remove this module once those compatibility APIs are retired.
//! Runner and config compatibility methods stay on `Tool`; this file only
//! bridges old and new state storage.

use std::path::PathBuf;

use object::Architecture;

use crate::{
    artifact::{runtime::PreparedRuntimeArtifacts, state::OutputArtifacts},
    ctx::AppContext,
    invocation::{ActiveBuildContext, InvocationState},
};

/// Compatibility bridge for legacy `AppContext` state.
/// Remove this helper when callers no longer read runtime artifacts through `Tool`.
pub(crate) fn runtime_artifacts<'a>(
    state: &'a InvocationState,
    ctx: &'a AppContext,
) -> &'a OutputArtifacts {
    if state.artifacts().is_empty() {
        &ctx.artifacts
    } else {
        state.artifacts()
    }
}

/// Compatibility bridge for legacy `AppContext` state.
/// Remove this helper when callers no longer read runtime architecture through `Tool`.
pub(crate) fn runtime_arch(state: &InvocationState, ctx: &AppContext) -> Option<Architecture> {
    state.arch().or(ctx.arch)
}

/// Compatibility bridge for legacy `AppContext` state.
/// Remove this helper when callers no longer mirror build config paths into `Tool`.
pub(crate) fn set_build_config_path(
    state: &mut InvocationState,
    ctx: &mut AppContext,
    path: Option<PathBuf>,
) {
    state.set_build_config_path(path.clone());
    ctx.build_config_path = path;
}

/// Compatibility bridge for legacy `AppContext` state.
/// Remove this helper when callers no longer mirror active builds into `Tool`.
pub(crate) fn set_active_build(
    state: &mut InvocationState,
    ctx: &mut AppContext,
    active_build: &ActiveBuildContext,
) {
    state.set_active_build(active_build.clone());
    sync_build_context_from_state(ctx, state);
}

/// Compatibility bridge for legacy `AppContext` state.
/// Remove this helper when callers no longer mirror runtime artifacts into `Tool`.
pub(crate) fn apply_prepared_runtime_artifacts(
    state: &mut InvocationState,
    ctx: &mut AppContext,
    prepared: &PreparedRuntimeArtifacts,
) {
    state.apply_prepared_runtime_artifacts(prepared);
    sync_runtime_context_from_state(ctx, state);
}

/// Compatibility bridge for legacy `AppContext` state.
/// Remove this helper when callers no longer sync `Tool` contexts from invocation state.
pub(crate) fn sync_context_from_state(ctx: &mut AppContext, state: &InvocationState) {
    sync_build_context_from_state(ctx, state);
    sync_runtime_context_from_state(ctx, state);
}

// Compatibility bridge for legacy `AppContext` state; remove it with this module.
fn sync_build_context_from_state(ctx: &mut AppContext, state: &InvocationState) {
    ctx.build_config_path = state.build_config_path().map(PathBuf::from);
    ctx.build_config = state.active_build().map(ActiveBuildContext::build_config);
}

// Compatibility bridge for legacy `AppContext` state; remove it with this module.
fn sync_runtime_context_from_state(ctx: &mut AppContext, state: &InvocationState) {
    ctx.arch = state.arch();
    ctx.artifacts = state.artifacts().clone();
}
