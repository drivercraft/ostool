//! Temporary compatibility bridge between `InvocationState` and legacy `AppContext`.
//!
//! `InvocationState` is the source of truth. `AppContext` is kept as a legacy
//! mirror for existing `Tool::ctx`, `Tool::ctx_mut`, and `Tool::into_context`
//! callers. Remove this module once those compatibility APIs are retired.

use std::path::PathBuf;

use object::Architecture;

use crate::{
    artifact::{runtime::PreparedRuntimeArtifacts, state::OutputArtifacts},
    ctx::AppContext,
    invocation::{ActiveBuildContext, InvocationState},
};

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

pub(crate) fn runtime_arch(state: &InvocationState, ctx: &AppContext) -> Option<Architecture> {
    state.arch().or(ctx.arch)
}

pub(crate) fn set_build_config_path(
    state: &mut InvocationState,
    ctx: &mut AppContext,
    path: Option<PathBuf>,
) {
    state.set_build_config_path(path.clone());
    ctx.build_config_path = path;
}

pub(crate) fn set_active_build(
    state: &mut InvocationState,
    ctx: &mut AppContext,
    active_build: &ActiveBuildContext,
) {
    state.set_active_build(active_build.clone());
    sync_build_context_from_state(ctx, state);
}

pub(crate) fn apply_prepared_runtime_artifacts(
    state: &mut InvocationState,
    ctx: &mut AppContext,
    prepared: &PreparedRuntimeArtifacts,
) {
    state.apply_prepared_runtime_artifacts(prepared);
    sync_runtime_context_from_state(ctx, state);
}

pub(crate) fn sync_context_from_state(ctx: &mut AppContext, state: &InvocationState) {
    sync_build_context_from_state(ctx, state);
    sync_runtime_context_from_state(ctx, state);
}

fn sync_build_context_from_state(ctx: &mut AppContext, state: &InvocationState) {
    ctx.build_config_path = state.build_config_path().map(PathBuf::from);
    ctx.build_config = state.active_build().map(ActiveBuildContext::build_config);
}

fn sync_runtime_context_from_state(ctx: &mut AppContext, state: &InvocationState) {
    ctx.arch = state.arch();
    ctx.artifacts = state.artifacts().clone();
}
