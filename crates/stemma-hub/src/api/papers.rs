//! Working papers, authors, archived PDFs and source.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use stemma_core::api::{
    Contents, CreatePaper, Event, Paper, PaperFileText, PaperPage, PaperSource,
};

use super::{Json, Membership};
use crate::auth::Viewer;
use crate::error::{ApiError, ApiResult};
use crate::papers;
use crate::state::AppState;

/// The group's papers.
pub async fn list(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<Paper>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    Ok(Json(papers::list(&state, group.group_id).await?))
}

/// Creates a paper, from the template: an admin or a member.
pub async fn create(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(body): Json<CreatePaper>,
) -> ApiResult<(StatusCode, Json<Paper>)> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    let paper = papers::create(&state, &group, &viewer.user(), &body.slug, &body.title).await?;
    Ok((StatusCode::CREATED, Json(paper)))
}

/// A paper, with its newest build's log.
pub async fn get(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, paper)): Path<(String, String)>,
) -> ApiResult<Json<PaperPage>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = papers::find(&state, group.group_id, &paper).await?;
    Ok(Json(PaperPage {
        paper: row.to_api(&state).await?,
        log: papers::latest_log(&state, row.id).await?,
    }))
}

/// Builds the paper's failed newest build again; members and admins ask.
pub async fn build_again(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, paper, id)): Path<(String, String, i64)>,
    Json(body): Json<super::library::Again>,
) -> ApiResult<Json<PaperPage>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = papers::find(&state, group.group_id, &paper).await?;
    let work: Option<i64> = sqlx::query_scalar(
        "SELECT work_id
         FROM paper_builds
         WHERE paper_id = ? AND id = ?",
    )
    .bind(row.id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("Build"))?;
    let work = work.ok_or_else(|| ApiError::conflict("This build was already built again"))?;
    crate::builds::again_for(&state, &viewer, &group, work, body.builder).await?;
    Ok(Json(PaperPage {
        paper: row.to_api(&state).await?,
        log: papers::latest_log(&state, row.id).await?,
    }))
}

/// The paper, if the viewer may delete it: whoever created it, or a group
/// admin. Its other authors only leave it.
async fn deletable(
    state: &AppState,
    viewer: &Viewer,
    slug: &str,
    paper: &str,
) -> ApiResult<(Membership, papers::PaperRow)> {
    viewer.require_full()?;
    let group = Membership::of(state, viewer, slug).await?;
    let row = papers::find(state, group.group_id, paper).await?;
    if row.created_by != viewer.id {
        group.require_admin()?;
    }
    Ok((group, row))
}

/// What deleting a paper would take with it.
pub async fn contents(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, paper)): Path<(String, String)>,
) -> ApiResult<Json<Contents>> {
    let (_, row) = deletable(&state, &viewer, &slug, &paper).await?;
    Ok(Json(crate::removal::paper_contents(&state, &row).await?))
}

/// Deletes a paper, with its own threads, builds, PDFs and repository.
pub async fn delete(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, paper)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let (group, row) = deletable(&state, &viewer, &slug, &paper).await?;
    for thread in crate::removal::paper_thread_numbers(&state, &row).await? {
        state.publish(group.group_id, Event::ThreadDeleted { thread });
    }
    crate::removal::paper(&state, &row, &group.slug).await?;
    state.publish(
        group.group_id,
        Event::PaperDeleted {
            paper: row.slug.clone(),
        },
    );
    Ok(StatusCode::NO_CONTENT)
}

/// Adds an author: the paper's authors, or an admin.
pub async fn add_author(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, paper, username)): Path<(String, String, String)>,
) -> ApiResult<Json<Paper>> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    Ok(Json(
        papers::add_author(&state, &group, viewer.id, &paper, &username).await?,
    ))
}

/// Takes an author away: the paper's authors, or an admin.
pub async fn remove_author(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, paper, username)): Path<(String, String, String)>,
) -> ApiResult<Json<Paper>> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    Ok(Json(
        papers::remove_author(&state, &group, viewer.id, &paper, &username).await?,
    ))
}

/// The paper's PDF: that of its last build that made one. Browsers show
/// it in their own viewer.
pub async fn pdf(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, paper)): Path<(String, String)>,
) -> ApiResult<Response> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = papers::find(&state, group.group_id, &paper).await?;
    let (path, size) = papers::pdf_file(&state, &row).await?;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| ApiError::not_found("PDF"))?;
    let mut response = Body::from_stream(tokio_util::io::ReaderStream::new(file)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/pdf"),
    );
    headers.insert(header::CONTENT_LENGTH, size.into());
    if let Ok(disposition) =
        HeaderValue::from_str(&format!("inline; filename=\"{}.pdf\"", row.slug))
    {
        headers.insert(header::CONTENT_DISPOSITION, disposition);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    Ok(response)
}

/// The paper's source files, at its `main`.
pub async fn files(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, paper)): Path<(String, String)>,
) -> ApiResult<Json<PaperSource>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = papers::find(&state, group.group_id, &paper).await?;
    let (commit, files) = papers::source(&state, &row).await?;
    Ok(Json(PaperSource { commit, files }))
}

/// One of the paper's source files, at its `main`, as text.
pub async fn file(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, paper, path)): Path<(String, String, String)>,
) -> ApiResult<Json<PaperFileText>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = papers::find(&state, group.group_id, &paper).await?;
    let (commit, text) = papers::source_file(&state, &row, &path).await?;
    Ok(Json(PaperFileText { path, commit, text }))
}
