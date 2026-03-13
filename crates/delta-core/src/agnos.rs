//! AGNOS ecosystem integration — Daimon capability registration.

use serde::{Deserialize, Serialize};
use crate::config::AgnosConfig;

/// A capability that Delta provides to the AGNOS agent runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
}

/// Registration request sent to the Daimon agent runtime.
#[derive(Debug, Serialize)]
struct RegisterRequest {
    agent_id: String,
    capabilities: Vec<Capability>,
}

/// Build the list of capabilities Delta provides.
pub fn delta_capabilities(version: &str) -> Vec<Capability> {
    vec![
        Capability {
            name: "code-hosting".into(),
            version: version.into(),
            description: "Git repository hosting with smart HTTP and SSH transport".into(),
            input_schema: None,
            output_schema: None,
        },
        Capability {
            name: "pull-requests".into(),
            version: version.into(),
            description: "Code review with pull requests, inline comments, and reviews".into(),
            input_schema: None,
            output_schema: None,
        },
        Capability {
            name: "ci-cd".into(),
            version: version.into(),
            description: "CI/CD pipeline execution with sandboxed runners".into(),
            input_schema: None,
            output_schema: None,
        },
        Capability {
            name: "artifact-registry".into(),
            version: version.into(),
            description: "Artifact storage with OCI images, .ark packages, and signed releases".into(),
            input_schema: None,
            output_schema: None,
        },
        Capability {
            name: "code-search".into(),
            version: version.into(),
            description: "Full-text code search with semantic indexing".into(),
            input_schema: None,
            output_schema: None,
        },
        Capability {
            name: "ai-code-review".into(),
            version: version.into(),
            description: "AI-powered code review, PR descriptions, and natural language queries".into(),
            input_schema: None,
            output_schema: None,
        },
    ]
}

/// Register Delta's capabilities with the Daimon agent runtime.
/// Returns Ok(()) on success, or an error message on failure.
/// Failures are non-fatal — Delta works without Daimon.
pub async fn register_with_daimon(config: &AgnosConfig, version: &str) -> Result<(), String> {
    let capabilities = delta_capabilities(version);
    let body = RegisterRequest {
        agent_id: "delta".into(),
        capabilities,
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("failed to create HTTP client: {}", e))?;

    let url = format!("{}/v1/agents/register", config.daimon_url);
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("daimon registration failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("daimon registration returned {}: {}", status, text));
    }

    Ok(())
}

/// Deregister Delta from the Daimon agent runtime.
pub async fn deregister_from_daimon(config: &AgnosConfig) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("failed to create HTTP client: {}", e))?;

    let url = format!("{}/v1/agents/delta", config.daimon_url);
    let _resp = client
        .delete(&url)
        .send()
        .await
        .map_err(|e| format!("daimon deregistration failed: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delta_capabilities() {
        let caps = delta_capabilities("2026.3.13");
        assert!(!caps.is_empty());
        assert!(caps.iter().any(|c| c.name == "code-hosting"));
        assert!(caps.iter().any(|c| c.name == "ci-cd"));
        assert!(caps.iter().any(|c| c.name == "artifact-registry"));
        assert!(caps.iter().all(|c| c.version == "2026.3.13"));
    }

    #[test]
    fn test_register_request_serializes() {
        let caps = delta_capabilities("2026.3.13");
        let req = RegisterRequest {
            agent_id: "delta".into(),
            capabilities: caps,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("delta"));
        assert!(json.contains("code-hosting"));
    }
}
