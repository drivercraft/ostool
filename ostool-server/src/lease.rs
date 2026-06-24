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
    let now = Utc::now();
    let expires_at = now + Duration::minutes(site.default_lease_minutes);
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
            session_id: Some(session.id.clone()),
            board_id: session.board_id,
            board_type: request.board_type,
            required_tags: request.required_tags,
            starts_at: now,
            expires_at,
        })
        .await?;
    state.update_session_expiry(&session.id, expires_at).await;
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
    let Some(session_id) = lease.session_id.as_deref() else {
        state
            .storage
            .mark_lease_state(&lease.id, LeaseState::Released, Some(Utc::now()), None)
            .await?;
        return Ok(());
    };
    let result = state.remove_session(session_id).await;
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
