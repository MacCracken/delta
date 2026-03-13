//! AI/LLM integration for code review, PR summaries, and natural language queries.

use crate::config::{AiConfig, AiProvider};
use crate::{DeltaError, Result};
use serde::{Deserialize, Serialize};

/// A message in a conversation with the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String, // "user", "assistant", "system"
    pub content: String,
}

/// Response from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiResponse {
    pub content: String,
    pub model: String,
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// LLM client for making AI requests.
#[derive(Clone)]
pub struct AiClient {
    config: AiConfig,
    http: reqwest::Client,
}

impl AiClient {
    pub fn new(config: &AiConfig) -> Result<Self> {
        if !config.enabled {
            return Err(DeltaError::Storage("AI features are not enabled".into()));
        }
        // Hoosh (local gateway) does not require an API key
        if config.api_key.is_none() && !matches!(config.provider, AiProvider::Hoosh) {
            return Err(DeltaError::Storage("AI API key is not configured".into()));
        }
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| DeltaError::Storage(format!("failed to create HTTP client: {}", e)))?;
        Ok(Self {
            config: config.clone(),
            http,
        })
    }

    /// Check if AI features are available.
    pub fn is_available(config: &AiConfig) -> bool {
        config.enabled && (config.api_key.is_some() || matches!(config.provider, AiProvider::Hoosh))
    }

    /// Send messages to the LLM and get a response.
    pub async fn complete(&self, system: &str, messages: &[Message]) -> Result<AiResponse> {
        match self.config.provider {
            AiProvider::Anthropic => self.complete_anthropic(system, messages).await,
            AiProvider::OpenAI => self.complete_openai(system, messages).await,
            AiProvider::Hoosh => self.complete_hoosh(system, messages).await,
        }
    }

    async fn complete_anthropic(&self, system: &str, messages: &[Message]) -> Result<AiResponse> {
        let api_key = self.config.api_key.as_deref().unwrap_or_default();

        // Build Anthropic Messages API request
        let body = serde_json::json!({
            "model": self.config.model,
            "max_tokens": self.config.max_tokens,
            "system": system,
            "messages": messages.iter().map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            }).collect::<Vec<_>>(),
        });

        let resp = self
            .http
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| DeltaError::Storage(format!("Anthropic API request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(DeltaError::Storage(format!(
                "Anthropic API error {}: {}",
                status, body
            )));
        }

        let json: serde_json::Value = resp.json().await.map_err(|e| {
            DeltaError::Storage(format!("failed to parse Anthropic response: {}", e))
        })?;

        let content = json["content"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|block| block["text"].as_str())
            .unwrap_or("")
            .to_string();

        let model = json["model"].as_str().unwrap_or("").to_string();
        let input_tokens = json["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = json["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;

        Ok(AiResponse {
            content,
            model,
            usage: Usage {
                input_tokens,
                output_tokens,
            },
        })
    }

    async fn complete_openai(&self, system: &str, messages: &[Message]) -> Result<AiResponse> {
        let api_key = self.config.api_key.as_deref().unwrap_or_default();

        let mut all_messages = vec![serde_json::json!({
            "role": "system",
            "content": system,
        })];
        for m in messages {
            all_messages.push(serde_json::json!({
                "role": m.role,
                "content": m.content,
            }));
        }

        let body = serde_json::json!({
            "model": self.config.model,
            "max_tokens": self.config.max_tokens,
            "messages": all_messages,
        });

        let resp = self
            .http
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| DeltaError::Storage(format!("OpenAI API request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(DeltaError::Storage(format!(
                "OpenAI API error {}: {}",
                status, body
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| DeltaError::Storage(format!("failed to parse OpenAI response: {}", e)))?;

        let content = json["choices"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|choice| choice["message"]["content"].as_str())
            .unwrap_or("")
            .to_string();

        let model = json["model"].as_str().unwrap_or("").to_string();
        let input_tokens = json["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = json["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;

        Ok(AiResponse {
            content,
            model,
            usage: Usage {
                input_tokens,
                output_tokens,
            },
        })
    }

    async fn complete_hoosh(&self, system: &str, messages: &[Message]) -> Result<AiResponse> {
        let endpoint = self
            .config
            .endpoint
            .as_deref()
            .unwrap_or("http://localhost:8088");

        let mut all_messages = vec![serde_json::json!({
            "role": "system",
            "content": system,
        })];
        for m in messages {
            all_messages.push(serde_json::json!({
                "role": m.role,
                "content": m.content,
            }));
        }

        let body = serde_json::json!({
            "model": self.config.model,
            "max_tokens": self.config.max_tokens,
            "messages": all_messages,
        });

        let url = format!("{}/v1/chat/completions", endpoint.trim_end_matches('/'));
        let mut req = self
            .http
            .post(&url)
            .header("content-type", "application/json");

        if let Some(ref api_key) = self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }

        let resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| DeltaError::Storage(format!("Hoosh API request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(DeltaError::Storage(format!(
                "Hoosh API error {}: {}",
                status, body
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| DeltaError::Storage(format!("failed to parse Hoosh response: {}", e)))?;

        let content = json["choices"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|choice| choice["message"]["content"].as_str())
            .unwrap_or("")
            .to_string();

        let model = json["model"].as_str().unwrap_or("").to_string();
        let input_tokens = json["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = json["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;

        Ok(AiResponse {
            content,
            model,
            usage: Usage {
                input_tokens,
                output_tokens,
            },
        })
    }

    /// Generate a code review summary for a diff.
    pub async fn review_diff(&self, diff: &str, context: &str) -> Result<ReviewSummary> {
        let system = "You are a senior code reviewer. Analyze the provided diff and give a structured review. \
            Be concise, constructive, and focus on: bugs, security issues, performance concerns, and code quality. \
            Respond in JSON format with fields: summary (string), risk_level (low/medium/high), \
            issues (array of {file, line, severity, message}), and suggestions (array of {file, line, old_code, new_code, explanation}).";

        let prompt = format!(
            "Review this code change:\n\nContext: {}\n\nDiff:\n```\n{}\n```\n\nRespond with JSON only.",
            context,
            truncate_for_context(diff, 12000)
        );

        let messages = vec![Message {
            role: "user".into(),
            content: prompt,
        }];

        let response = self.complete(system, &messages).await?;
        parse_review_summary(&response.content)
    }

    /// Generate a PR description from a diff and commit messages.
    pub async fn generate_pr_description(
        &self,
        diff: &str,
        commits: &[String],
    ) -> Result<PrDescription> {
        let system = "You are an expert at writing clear, concise pull request descriptions. \
            Given a diff and commit messages, write a PR title and body. \
            The body should have a Summary section with bullet points and a Test Plan section. \
            Respond in JSON format with fields: title (string), body (string).";

        let commit_list = commits.join("\n- ");
        let prompt = format!(
            "Generate a PR description for these changes:\n\nCommits:\n- {}\n\nDiff:\n```\n{}\n```\n\nRespond with JSON only.",
            commit_list,
            truncate_for_context(diff, 12000)
        );

        let messages = vec![Message {
            role: "user".into(),
            content: prompt,
        }];

        let response = self.complete(system, &messages).await?;
        parse_pr_description(&response.content)
    }

    /// Generate a commit summary from a diff.
    pub async fn generate_commit_summary(&self, diff: &str) -> Result<String> {
        let system = "You are an expert at writing concise git commit messages. \
            Given a diff, write a one-line commit summary (50-72 chars) that describes what changed and why. \
            Respond with just the commit message, no JSON wrapping.";

        let prompt = format!(
            "Write a commit message for this diff:\n```\n{}\n```",
            truncate_for_context(diff, 8000)
        );

        let messages = vec![Message {
            role: "user".into(),
            content: prompt,
        }];

        let response = self.complete(system, &messages).await?;
        Ok(response.content.trim().to_string())
    }

    /// Answer a natural language question about repository contents.
    pub async fn query_repo(&self, question: &str, context: &str) -> Result<String> {
        let system = "You are a knowledgeable assistant helping developers understand a code repository. \
            Answer questions based on the provided repository context. Be concise and accurate. \
            If you're not sure about something, say so.";

        let prompt = format!(
            "Repository context:\n{}\n\nQuestion: {}",
            truncate_for_context(context, 12000),
            question
        );

        let messages = vec![Message {
            role: "user".into(),
            content: prompt,
        }];

        let response = self.complete(system, &messages).await?;
        Ok(response.content.trim().to_string())
    }
}

/// Truncate text to fit within a character limit, keeping the beginning.
fn truncate_for_context(text: &str, max_chars: usize) -> &str {
    if text.len() <= max_chars {
        text
    } else {
        // Just truncate — the LLM can work with partial diffs
        &text[..max_chars]
    }
}

// ---- AI response types ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSummary {
    pub summary: String,
    pub risk_level: String,
    pub issues: Vec<ReviewIssue>,
    pub suggestions: Vec<CodeSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewIssue {
    pub file: String,
    pub line: Option<u32>,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSuggestion {
    pub file: String,
    pub line: Option<u32>,
    pub old_code: String,
    pub new_code: String,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrDescription {
    pub title: String,
    pub body: String,
}

fn parse_review_summary(content: &str) -> Result<ReviewSummary> {
    // Try to extract JSON from the response (might be wrapped in markdown code blocks)
    let json_str = extract_json(content);
    serde_json::from_str(json_str)
        .map_err(|e| DeltaError::Storage(format!("failed to parse AI review response: {}", e)))
}

fn parse_pr_description(content: &str) -> Result<PrDescription> {
    let json_str = extract_json(content);
    serde_json::from_str(json_str).map_err(|e| {
        DeltaError::Storage(format!("failed to parse AI PR description response: {}", e))
    })
}

/// Extract JSON content, stripping markdown code fences if present.
fn extract_json(content: &str) -> &str {
    let trimmed = content.trim();
    if let Some(rest) = trimmed.strip_prefix("```json")
        && let Some(json) = rest.strip_suffix("```")
    {
        return json.trim();
    }
    if let Some(rest) = trimmed.strip_prefix("```")
        && let Some(json) = rest.strip_suffix("```")
    {
        return json.trim();
    }
    trimmed
}
