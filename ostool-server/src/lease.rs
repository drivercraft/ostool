use chrono::{Duration, Utc};

use crate::{
    api::dto::CreateLeaseRequest,
    state::AppState,
    storage::{Lease, LeaseState, NewLease},
};

pub async fn create_user_lease(
    state: &AppState,
    user_id: &str,
    username: &str,
    request: CreateLeaseRequest,
) -> anyhow::Result<Lease> {
    let site = state.storage.get_site_settings().await?;
    if !site.self_service_enabled {
        anyhow::bail!("self-service rental is disabled");
    }
    let session = state
        .create_session(
            &request.board_type,
            &request.required_tags,
            Some(username.to_string()),
        )
        .await
        .map_err(|status| anyhow::anyhow!("failed to allocate board: {status:?}"))?;
    let lease = state
        .storage
        .create_lease(NewLease {
            user_id: user_id.to_string(),
            session_id: session.id,
            board_id: session.board_id,
            board_type: request.board_type,
            required_tags: request.required_tags,
            expires_at: Utc::now() + Duration::minutes(site.default_lease_minutes),
        })
        .await?;
    Ok(lease)
}

pub async fn release_lease(
    state: &AppState,
    lease: Lease,
    failure_message: Option<String>,
) -> anyhow::Result<()> {
    state
        .storage
        .mark_lease_state(
            &lease.id,
            LeaseState::Releasing,
            None,
            failure_message.clone(),
        )
        .await?;
    let result = state.remove_session(&lease.session_id).await;
    match result {
        Ok(_) => {
            state
                .storage
                .mark_lease_state(&lease.id, LeaseState::Released, Some(Utc::now()), None)
                .await?;
            Ok(())
        }
        Err(err) => {
            state
                .storage
                .mark_lease_state(
                    &lease.id,
                    LeaseState::Failed,
                    Some(Utc::now()),
                    Some(err.to_string()),
                )
                .await?;
            Err(err)
        }
    }
}
