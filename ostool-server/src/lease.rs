use chrono::Utc;

use crate::{
    api::dto::CreateLeaseRequest,
    config::BoardConfig,
    state::AppState,
    storage::{Lease, LeaseState, NewLease},
};

pub async fn create_user_lease(
    state: &AppState,
    user_id: &str,
    username: &str,
    request: CreateLeaseRequest,
    source_ip: Option<String>,
) -> anyhow::Result<Lease> {
    let site = state.storage.get_site_settings().await?;
    if !site.self_service_enabled {
        anyhow::bail!("self-service rental is disabled");
    }
    if request.expires_at <= request.starts_at {
        anyhow::bail!("expires_at must be after starts_at");
    }
    if request.expires_at <= Utc::now() {
        anyhow::bail!("expires_at must be in the future");
    }
    let lease_minutes = (request.expires_at - request.starts_at).num_minutes();
    if lease_minutes > site.max_lease_minutes {
        anyhow::bail!(
            "lease duration exceeds the maximum of {} minutes",
            site.max_lease_minutes
        );
    }

    let required_tags = request
        .required_tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    let board = find_available_board_for_window(
        state,
        &request.board_type,
        &required_tags,
        request.starts_at,
        request.expires_at,
    )
    .await?;
    let session_id = if request.starts_at <= Utc::now() {
        let session = state
            .create_session_for_board(&board.id, Some(username.to_string()), source_ip)
            .await
            .map_err(|status| anyhow::anyhow!("failed to allocate board: {status:?}"))?;
        state
            .update_session_expiry(&session.id, request.expires_at)
            .await;
        Some(session.id)
    } else {
        None
    };
    let lease = state
        .storage
        .create_lease(NewLease {
            user_id: user_id.to_string(),
            session_id,
            board_id: board.id,
            board_type: request.board_type,
            required_tags,
            starts_at: request.starts_at,
            expires_at: request.expires_at,
        })
        .await?;
    Ok(lease)
}

async fn find_available_board_for_window(
    state: &AppState,
    board_type: &str,
    required_tags: &[String],
    starts_at: chrono::DateTime<Utc>,
    expires_at: chrono::DateTime<Utc>,
) -> anyhow::Result<BoardConfig> {
    let boards = state.storage.list_board_configs().await?;
    let active_leases = state
        .storage
        .list_leases()
        .await?
        .into_iter()
        .filter(|lease| lease.state == LeaseState::Active)
        .collect::<Vec<_>>();

    let has_board_type = boards.iter().any(|board| board.board_type == board_type);
    if !has_board_type {
        anyhow::bail!("board type `{board_type}` not found");
    }

    boards
        .into_iter()
        .filter(|board| {
            !board.disabled
                && board.board_type == board_type
                && required_tags
                    .iter()
                    .all(|required| board.tags.iter().any(|tag| tag == required))
        })
        .find(|board| {
            active_leases.iter().all(|lease| {
                lease.board_id != board.id
                    || starts_at >= lease.expires_at
                    || expires_at <= lease.starts_at
            })
        })
        .ok_or_else(|| {
            anyhow::anyhow!("no available board for type `{board_type}` in that time window")
        })
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
