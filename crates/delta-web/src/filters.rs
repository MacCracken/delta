//! Custom Askama template filters.

use delta_core::db::pipeline::PipelineRun;

/// Truncate a SHA to 8 characters.
pub fn truncate_sha(s: &str) -> askama::Result<String> {
    Ok(s.chars().take(8).collect())
}

/// Compute a human-readable duration string for a pipeline.
pub fn pipeline_duration(p: &PipelineRun) -> askama::Result<String> {
    let Some(started) = &p.started_at else {
        return Ok("--".to_string());
    };
    let end = p.finished_at.as_deref().unwrap_or(started.as_str());

    let Ok(start_dt) = chrono::DateTime::parse_from_rfc3339(started) else {
        return Ok("--".to_string());
    };
    let Ok(end_dt) = chrono::DateTime::parse_from_rfc3339(end) else {
        return Ok("--".to_string());
    };

    let dur = end_dt.signed_duration_since(start_dt);
    let secs = dur.num_seconds().unsigned_abs();

    Ok(if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    })
}
