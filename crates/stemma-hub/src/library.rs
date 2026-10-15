//! Libraries: each group's Lean project, its builds and their reports.

use stemma_core::api::{Build, DeclarationState};
use stemma_core::protocol::{Built, LibraryBuild};

use crate::api::Membership;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// What a declaration's address resolves to.
pub struct Found {
    pub exists: bool,
    pub title: Option<String>,
    pub note: Option<String>,
    pub state: Option<DeclarationState>,
}

pub async fn resolve(_state: &AppState, _group_id: i64, _name: &str) -> ApiResult<Found> {
    Ok(Found {
        exists: false,
        title: None,
        note: Some("The library is not here yet".into()),
        state: None,
    })
}

pub async fn build_of_job(
    _state: &AppState,
    _job_id: i64,
    _note: Option<String>,
) -> ApiResult<Option<Build>> {
    Ok(None)
}

pub async fn work_for(
    _state: &AppState,
    _group: &Membership,
    _job_id: i64,
) -> ApiResult<LibraryBuild> {
    Err(ApiError::internal("no library builds yet"))
}

pub async fn build_finished(
    _state: &AppState,
    _group_id: i64,
    _job_id: i64,
    _built: Built,
) -> ApiResult<()> {
    Ok(())
}
