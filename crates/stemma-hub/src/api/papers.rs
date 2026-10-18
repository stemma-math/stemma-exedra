//! Working papers over the API: listing and creating them, their authors,
//! their builds and PDF, their source, and asking the workshop to edit one.

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use stemma_core::Address;
use stemma_core::api::{
    CreatePaper, CreateThread, EditPaper, Paper, PaperFileText, PaperPage, PaperSource, Task,
    ThreadKind,
};

use super::forum::{create_thread, find_thread};
use super::{Json, Membership};
use crate::auth::Viewer;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::{jobs, papers, workshop};

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

/// Builds the paper's `main` again.
pub async fn rebuild(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, paper)): Path<(String, String)>,
) -> ApiResult<(StatusCode, Json<Paper>)> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    let paper = papers::rebuild(&state, &group, viewer.id, &paper).await?;
    Ok((StatusCode::CREATED, Json(paper)))
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

/// Keeps the PDF of a build, as the machine building it uploads it, with
/// its job's token.
pub async fn upload_pdf(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, paper, build)): Path<(String, String, i64)>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let Some(job) = viewer.job() else {
        return Err(ApiError::forbidden(
            "Only the machine building it uploads a build's PDF",
        ));
    };
    let token_id = job.token_id;
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = papers::find(&state, group.group_id, &paper).await?;
    papers::upload(&state, token_id, &row, build, &body).await?;
    Ok(StatusCode::NO_CONTENT)
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

/// Asks the workshop to edit the paper: opens a thread about it with the
/// request, and hands the task over from its first message.
pub async fn edit(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, paper)): Path<(String, String)>,
    Json(body): Json<EditPaper>,
) -> ApiResult<(StatusCode, Json<Task>)> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    if !group.role.can_ask_for_work() {
        return Err(ApiError::forbidden(
            "Guests cannot ask the workshop for work",
        ));
    }
    let row = papers::find(&state, group.group_id, &paper).await?;
    let title = super::required_text(&body.title, "The title", 200)?;
    let instructions = super::required_text(&body.instructions, "The instructions", 20_000)?;
    let (_, Json(page)) = create_thread(
        State(state.clone()),
        viewer.clone(),
        Path(slug),
        Json(CreateThread {
            title: title.clone(),
            kind: ThreadKind::Discussion,
            resident_agent: None,
            anchor: Some(Address::Paper(row.slug.clone())),
            body: Some(instructions.clone()),
            attachments: Vec::new(),
        }),
    )
    .await?;
    let thread = find_thread(&state, group.group_id, page.thread.number).await?;
    let task = workshop::create(
        &state,
        &group,
        workshop::NewTask {
            title,
            instructions,
            architecture: None,
            paper_id: Some(row.id),
            thread_id: thread.id,
            origin_message: 1,
            requester_id: viewer.id,
            agent: None,
        },
    )
    .await?;
    jobs::dispatch(&state).await?;
    Ok((StatusCode::CREATED, Json(task)))
}
