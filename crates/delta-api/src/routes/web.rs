//! Server-rendered HTML pages for the web UI.

use askama::Template;
use axum::{
    Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use delta_core::db;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{owner}/{repo}/-/pipelines", get(pipeline_list))
        .route(
            "/{owner}/{repo}/-/pipelines/{pipeline_id}",
            get(pipeline_detail),
        )
}

/// Render an Askama template into an HTML response.
fn render_template(tmpl: impl Template) -> Response {
    match tmpl.render() {
        Ok(html) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            html,
        )
            .into_response(),
        Err(e) => {
            tracing::error!("template render error: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
        }
    }
}

async fn pipeline_list(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Response, (StatusCode, String)> {
    // Resolve repo
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "user not found".into()))?;
    let owner_id = owner_user.id.to_string();
    let repo_record = db::repo::get_by_owner_and_name(&state.db, &owner_id, &repo)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    let pipelines =
        db::pipeline::list_pipelines(&state.db, &repo_record.id.to_string(), None, 50)
            .await
            .map_err(|e| {
                tracing::error!("failed to list pipelines: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".into(),
                )
            })?;

    let page = delta_web::pipelines::PipelineListPage {
        owner,
        repo,
        pipelines,
    };

    Ok(render_template(page))
}

async fn pipeline_detail(
    State(state): State<AppState>,
    Path((owner, repo, pipeline_id)): Path<(String, String, String)>,
) -> Result<Response, (StatusCode, String)> {
    // Resolve repo
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "user not found".into()))?;
    let owner_id = owner_user.id.to_string();
    let repo_record = db::repo::get_by_owner_and_name(&state.db, &owner_id, &repo)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    let pipeline = db::pipeline::get_pipeline(&state.db, &pipeline_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    if pipeline.repo_id != repo_record.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "pipeline not found".into()));
    }

    let job_runs = db::pipeline::list_jobs(&state.db, &pipeline_id)
        .await
        .unwrap_or_default();

    let mut jobs = Vec::new();
    for job in job_runs {
        let steps = db::pipeline::get_step_logs(&state.db, &job.id)
            .await
            .unwrap_or_default();
        jobs.push(delta_web::pipelines::JobWithSteps { job, steps });
    }

    let page = delta_web::pipelines::PipelineDetailPage {
        owner,
        repo,
        pipeline,
        jobs,
    };

    Ok(render_template(page))
}
