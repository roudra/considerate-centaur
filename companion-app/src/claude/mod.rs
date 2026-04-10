// Claude API integration layer — prompt construction, structured output, context injection.
// See CLAUDE.md "Claude Integration" and "Reliability Architecture" sections.

pub mod prompts;
pub mod schemas;

pub use prompts::{
    EvaluationContext, GenerationContext, NarrativeContext, ProgressSnapshot, SanitizedProfile,
    SessionHistoryItem, SessionSummary,
};
pub use schemas::{
    AssignmentModality, DifficultyAdjustment, EvaluationConfidence, EvaluationResult,
    GeneratedAssignment, ObservedBehavioralSignals, SessionNarrative,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during Claude API operations.
#[derive(Debug, Error)]
pub enum ClaudeError {
    /// The `ANTHROPIC_API_KEY` environment variable is not set.
    #[error("ANTHROPIC_API_KEY environment variable is not set")]
    MissingApiKey,

    /// The Claude API returned a non-success HTTP status.
    #[error("Claude API error (HTTP {status}): {body}")]
    ApiError { status: u16, body: String },

    /// A network or transport error occurred while contacting the Claude API.
    #[error("Network error contacting Claude API: {0}")]
    Network(#[from] reqwest::Error),

    /// The Claude API response could not be deserialized into the expected type.
    #[error("Failed to parse Claude response: {0}")]
    ParseError(#[from] serde_json::Error),

    /// Claude returned an empty content block with no text.
    #[error("Claude returned an empty response")]
    EmptyResponse,
}

// ---------------------------------------------------------------------------
// Anthropic API request / response types
// ---------------------------------------------------------------------------

/// A single message in the Anthropic Messages API format.
#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: &'static str,
    content: String,
}

/// Request body for the Anthropic Messages API.
#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<AnthropicMessage>,
}

/// A single content block in an Anthropic API response.
#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

/// The Anthropic Messages API response envelope.
#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

// ---------------------------------------------------------------------------
// Claude client
// ---------------------------------------------------------------------------

/// Default Claude model to use if none is specified.
pub const DEFAULT_MODEL: &str = "claude-3-5-haiku-20241022";

/// The Anthropic Messages API endpoint.
const API_URL: &str = "https://api.anthropic.com/v1/messages";

/// The Anthropic API version header value.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// HTTP client for the Anthropic Claude API.
///
/// - API key is read from the `ANTHROPIC_API_KEY` environment variable at
///   construction time — it is never hardcoded.
/// - All three operations (generate, evaluate, narrative) are **separate** API
///   calls, keeping generation and evaluation fully decoupled.
/// - All responses are parsed into strongly-typed structs — never raw strings.
#[derive(Clone, Debug)]
pub struct ClaudeClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
}

impl ClaudeClient {
    /// Create a new `ClaudeClient`, reading the API key from the
    /// `ANTHROPIC_API_KEY` environment variable.
    ///
    /// Returns [`ClaudeError::MissingApiKey`] if the variable is not set.
    pub fn from_env() -> Result<Self, ClaudeError> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| ClaudeError::MissingApiKey)?;
        Ok(Self::new(api_key, DEFAULT_MODEL.to_string()))
    }

    /// Create a new `ClaudeClient` with the given API key and model name.
    pub fn new(api_key: String, model: String) -> Self {
        ClaudeClient {
            http: reqwest::Client::new(),
            api_key,
            model,
        }
    }

    /// Send a request to the Anthropic Messages API and return the raw text
    /// from the first content block.
    async fn call(&self, system: &str, user_message: &str) -> Result<String, ClaudeError> {
        let request_body = AnthropicRequest {
            model: self.model.clone(),
            max_tokens: 4096,
            system: system.to_string(),
            messages: vec![AnthropicMessage {
                role: "user",
                content: user_message.to_string(),
            }],
        };

        let response = self
            .http
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ClaudeError::ApiError {
                status: status.as_u16(),
                body,
            });
        }

        let anthropic_response: AnthropicResponse = response.json().await?;

        let text = anthropic_response
            .content
            .into_iter()
            .find(|b| b.block_type == "text")
            .and_then(|b| b.text)
            .ok_or(ClaudeError::EmptyResponse)?;

        Ok(text)
    }

    /// Generate a new assignment for a learner.
    ///
    /// This is a **separate** API call from evaluation — generation and
    /// evaluation are always kept apart (CLAUDE.md § "Separate Generation
    /// from Evaluation").
    pub async fn generate_assignment(
        &self,
        ctx: &GenerationContext,
    ) -> Result<GeneratedAssignment, ClaudeError> {
        let system = prompts::GENERATION_SYSTEM_PROMPT;
        let user = prompts::build_generation_prompt(ctx);
        let raw = self.call(system, &user).await?;
        let assignment: GeneratedAssignment = serde_json::from_str(&raw)?;
        Ok(assignment)
    }

    /// Evaluate a child's response to an assignment.
    ///
    /// **Critical**: `ctx.verified_correct_answer` and `ctx.backend_verified_correct`
    /// are set by the backend **before** this call. Claude is given the correct
    /// answer so it cannot hallucinate correctness — it only supplies tone and
    /// explanation.
    ///
    /// This is always a **separate** API call from generation.
    pub async fn evaluate_response(
        &self,
        ctx: &EvaluationContext,
    ) -> Result<EvaluationResult, ClaudeError> {
        let system = prompts::evaluation_system_prompt();
        let user = prompts::build_evaluation_prompt(ctx);
        let raw = self.call(&system, &user).await?;
        let result: EvaluationResult = serde_json::from_str(&raw)?;
        Ok(result)
    }

    /// Generate a session narrative after a session ends.
    ///
    /// The narrative provides structured content (behavioral observations,
    /// continuity notes, recommendations) that the **backend** then assembles
    /// into the session markdown file. Claude provides the narrative; the
    /// backend is the file author.
    pub async fn generate_session_narrative(
        &self,
        ctx: &NarrativeContext,
    ) -> Result<SessionNarrative, ClaudeError> {
        let system = prompts::NARRATIVE_SYSTEM_PROMPT;
        let user = prompts::build_narrative_prompt(ctx);
        let raw = self.call(system, &user).await?;
        let narrative: SessionNarrative = serde_json::from_str(&raw)?;
        Ok(narrative)
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_env_missing_key_returns_error() {
        // Ensure the variable is not set for this test.
        std::env::remove_var("ANTHROPIC_API_KEY");
        let result = ClaudeClient::from_env();
        assert!(
            matches!(result, Err(ClaudeError::MissingApiKey)),
            "expected MissingApiKey, got: {:?}",
            result
        );
    }

    #[test]
    fn test_from_env_with_key_succeeds() {
        std::env::set_var("ANTHROPIC_API_KEY", "test-key-12345");
        let result = ClaudeClient::from_env();
        assert!(result.is_ok(), "should succeed when key is set");
        let client = result.unwrap();
        assert_eq!(client.model, DEFAULT_MODEL);
        // Clean up
        std::env::remove_var("ANTHROPIC_API_KEY");
    }

    #[test]
    fn test_new_client_uses_given_model() {
        let client = ClaudeClient::new("key".to_string(), "claude-3-opus-20240229".to_string());
        assert_eq!(client.model, "claude-3-opus-20240229");
    }

    #[test]
    fn test_claude_error_display_missing_key() {
        let e = ClaudeError::MissingApiKey;
        assert!(e.to_string().contains("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn test_claude_error_display_api_error() {
        let e = ClaudeError::ApiError {
            status: 401,
            body: "unauthorized".to_string(),
        };
        let msg = e.to_string();
        assert!(msg.contains("401"));
        assert!(msg.contains("unauthorized"));
    }

    #[test]
    fn test_claude_error_display_empty_response() {
        let e = ClaudeError::EmptyResponse;
        assert!(e.to_string().contains("empty"));
    }
}
