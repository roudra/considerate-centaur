/// Prompt construction for all three Claude operations.
///
/// Each function builds a complete prompt (system + user messages) with the
/// appropriate context tier as specified in CLAUDE.md → "Session Context Window
/// Management". Feedback guardrails are embedded in every evaluation prompt.
use crate::claude::schemas::GeneratedAssignment;
use crate::learner::profile::{InitialPreferences, ObservedBehavior};
use crate::progress::tracker::{LearnerProgress, SkillProgress};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Sanitized profile — what gets sent to Claude
// ---------------------------------------------------------------------------

/// A sanitized view of the learner profile safe to include in Claude API calls.
///
/// Per the privacy rules in CLAUDE.md → "NEVER SENT TO CLAUDE":
/// - `id` / `learnerId` / UUIDs are **excluded**
/// - `schemaVersion` is excluded (system-internal metadata)
/// - Only display name, age, interests, preferences, and observed behavior are included
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SanitizedProfile {
    /// Child-chosen display name — safe to pass to Claude (not a real name).
    pub name: String,
    pub age: u8,
    pub interests: Vec<String>,
    pub initial_preferences: InitialPreferences,
    pub observed_behavior: ObservedBehavior,
}

// ---------------------------------------------------------------------------
// Progress snapshot — what gets sent to Claude
// ---------------------------------------------------------------------------

/// A sanitized view of the learner's current progress safe to include in Claude
/// API calls. `learnerId` is excluded.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressSnapshot {
    pub skills: HashMap<String, SkillProgress>,
    pub total_sessions: u32,
    pub total_assignments: u32,
}

// ---------------------------------------------------------------------------
// Context structs for each operation
// ---------------------------------------------------------------------------

/// Context for an assignment **generation** request.
///
/// Follows CLAUDE.md "Session Context Window Management":
/// - Always: sanitized profile + progress snapshot
/// - Summarized: last 2–3 session summaries (behavioral obs + continuity notes)
/// - Focus skill and current session difficulty are also included
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationContext {
    pub profile: SanitizedProfile,
    pub progress: ProgressSnapshot,
    /// Extracted behavioral observations and continuity notes from the last 2–3 sessions.
    pub recent_session_summaries: Vec<SessionSummary>,
    /// Skill ID to target for this assignment.
    pub target_skill: String,
    /// Desired difficulty level.
    pub target_difficulty: u32,
}

/// Context for a response **evaluation** request.
///
/// Follows CLAUDE.md "Session Context Window Management":
/// - Always: sanitized profile + progress snapshot
/// - Full current session: all assignments + responses so far
/// - Evaluation target: the specific assignment and the child's response
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationContext {
    pub profile: SanitizedProfile,
    pub progress: ProgressSnapshot,
    /// All assignments and responses in the current session so far.
    pub session_history: Vec<SessionHistoryItem>,
    /// The assignment being evaluated.
    pub assignment: GeneratedAssignment,
    /// The verified correct answer (backend-computed, provided to Claude so it
    /// cannot hallucinate whether the child is right).
    pub verified_correct_answer: serde_json::Value,
    /// The child's raw response.
    pub child_response: String,
    /// Whether the backend determined the answer is correct.
    pub backend_verified_correct: bool,
}

/// Context for a **session narrative** generation request.
///
/// Follows CLAUDE.md "Session Context Window Management":
/// - Always: sanitized profile + progress snapshot
/// - Summarized: last 2–3 session summaries
/// - Full current session: complete assignment logs
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeContext {
    pub profile: SanitizedProfile,
    pub progress: ProgressSnapshot,
    pub recent_session_summaries: Vec<SessionSummary>,
    /// All assignments and responses for the session that just ended.
    pub session_history: Vec<SessionHistoryItem>,
    /// Total session duration in minutes.
    pub session_duration_minutes: u32,
}

// ---------------------------------------------------------------------------
// Supporting types for context structs
// ---------------------------------------------------------------------------

/// Extracted summary from a past session (behavioral observations + continuity notes only).
///
/// Per CLAUDE.md: include *only* these sections — not full assignment logs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    /// ISO date string (YYYY-MM-DD).
    pub date: String,
    pub behavioral_observations: String,
    pub continuity_notes: String,
}

/// A single assignment and the child's response, for within-session context.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryItem {
    pub assignment: GeneratedAssignment,
    pub child_response: String,
    pub correct: bool,
    pub time_seconds: u32,
}

// ---------------------------------------------------------------------------
// Prompt builders
// ---------------------------------------------------------------------------

/// Non-negotiable feedback guardrails embedded in every evaluation prompt.
///
/// See CLAUDE.md → "Feedback Guardrails".
pub const FEEDBACK_GUARDRAILS: &str = r#"FEEDBACK RULES — NON-NEGOTIABLE:
- NEVER say "correct" or "incorrect" as a standalone judgment — always phrase feedback as encouragement or guidance
- NEVER invent facts — only explain using concepts present in the assignment
- NEVER use discouraging language (no "Wrong", "No", "Bad answer", etc.)
- NEVER compare the child to other learners or to their past performance in a negative way
- Frame all incorrect answers as learning opportunities: "Not quite — but you're thinking in the right direction!"
- If uncertain about an evaluation, respond with curiosity: "That's an interesting approach! Let's look at it together..."
- Celebrate effort, strategy, and persistence — not raw ability ("You tried three different approaches!" not "You're so smart!")
- If backend_verified_correct is true, the answer IS correct — write positive, celebratory, growth-mindset feedback
- If backend_verified_correct is false, the answer is NOT correct — write encouraging, scaffolded, growth-mindset feedback
- Return ONLY valid JSON matching the EvaluationResult schema — no preamble, no extra text"#;

/// System prompt for assignment generation.
pub const GENERATION_SYSTEM_PROMPT: &str = r#"You are an adaptive educational assistant generating personalized learning assignments for children.

Your role:
- Generate age-appropriate, engaging assignments tailored to the child's interests and ZPD (Zone of Proximal Development)
- Theme problems around the child's interests to increase engagement
- Use growth-mindset language in hints and explanations
- Calibrate difficulty precisely to the target level
- Use ONLY skill IDs that exist in the learner's skill tree

Output requirements:
- Return ONLY valid JSON matching the GeneratedAssignment schema — no preamble, no extra text
- Include exactly 3 hints, from vague (hint[0]) to specific (hint[2])
- The correctAnswer must be mathematically/logically sound
- acceptableAnswers must include all reasonable representations of the correct answer"#;

/// System prompt for session narrative generation.
pub const NARRATIVE_SYSTEM_PROMPT: &str = r#"You are an adaptive educational assistant generating a session narrative for a parent dashboard.

Your role:
- Summarize behavioral observations from this learning session
- Write continuity notes to inform the next session (what to build on, what to revisit)
- Recommend focus areas for the next session based on performance and ZPD
- Recommend a difficulty adjustment based on session accuracy

Output requirements:
- Return ONLY valid JSON matching the SessionNarrative schema — no preamble, no extra text
- behavioralObservations: describe what you observed about how the child engaged, struggled, or succeeded
- continuityNotes: specific, actionable notes for the next session
- recommendedFocusAreas: list of skill IDs (e.g. "sequential-logic")
- difficultyAdjustment: "increase" / "maintain" / "decrease"
- flagForParentReview: true only if a low-confidence evaluation was made or something unusual occurred"#;

/// Build the system prompt for an evaluation request.
///
/// The feedback guardrails are always embedded — they are not optional.
pub fn evaluation_system_prompt() -> String {
    format!(
        r#"You are an adaptive educational assistant evaluating a child's response to a learning assignment.

Your role:
- Evaluate the child's response given the assignment and the backend-verified correct answer
- Provide warm, growth-mindset feedback appropriate to the child's age
- Identify any behavioral signals (self-correction, struggle, confidence)
- Report your confidence in the evaluation

{}

Output requirements:
- Return ONLY valid JSON matching the EvaluationResult schema — no preamble, no extra text"#,
        FEEDBACK_GUARDRAILS
    )
}

/// Build the user message for an assignment generation request.
pub fn build_generation_prompt(ctx: &GenerationContext) -> String {
    let profile_json = serde_json::to_string_pretty(&ctx.profile)
        .unwrap_or_else(|_| "{}".to_string());
    let progress_json = serde_json::to_string_pretty(&ctx.progress)
        .unwrap_or_else(|_| "{}".to_string());
    let summaries_json = serde_json::to_string_pretty(&ctx.recent_session_summaries)
        .unwrap_or_else(|_| "[]".to_string());

    format!(
        r#"Generate a single assignment for the following learner.

## Learner Profile
{profile_json}

## Current Progress Snapshot
{progress_json}

## Recent Session Summaries (last 2-3 sessions, behavioral observations + continuity notes only)
{summaries_json}

## Assignment Request
- Target skill: {skill}
- Target difficulty: {difficulty}/10
- Theme: Use the learner's interests listed in their profile

Generate one assignment that:
1. Targets the specified skill at the specified difficulty
2. Is themed around one of the learner's interests
3. Includes exactly 3 scaffolded hints (vague → specific)
4. Has a correct answer that can be verified programmatically
5. Is appropriate for age {age}

Return ONLY the JSON object."#,
        profile_json = profile_json,
        progress_json = progress_json,
        summaries_json = summaries_json,
        skill = ctx.target_skill,
        difficulty = ctx.target_difficulty,
        age = ctx.profile.age,
    )
}

/// Build the user message for a response evaluation request.
pub fn build_evaluation_prompt(ctx: &EvaluationContext) -> String {
    let profile_json = serde_json::to_string_pretty(&ctx.profile)
        .unwrap_or_else(|_| "{}".to_string());
    let assignment_json = serde_json::to_string_pretty(&ctx.assignment)
        .unwrap_or_else(|_| "{}".to_string());
    let history_json = serde_json::to_string_pretty(&ctx.session_history)
        .unwrap_or_else(|_| "[]".to_string());

    format!(
        r#"Evaluate the child's response to the following assignment.

## Learner Profile
{profile_json}

## Session History (current session, all assignments so far)
{history_json}

## Assignment Being Evaluated
{assignment_json}

## Child's Response
{child_response}

## Backend Verification
- Verified correct answer: {verified_answer}
- Backend determined correct: {backend_correct}

Evaluate the child's response. Remember: backend_verified_correct is authoritative —
use it to frame your feedback, not to second-guess it.

Return ONLY the JSON object."#,
        profile_json = profile_json,
        history_json = history_json,
        assignment_json = assignment_json,
        child_response = ctx.child_response,
        verified_answer = ctx.verified_correct_answer,
        backend_correct = ctx.backend_verified_correct,
    )
}

/// Build the user message for a session narrative request.
pub fn build_narrative_prompt(ctx: &NarrativeContext) -> String {
    let profile_json = serde_json::to_string_pretty(&ctx.profile)
        .unwrap_or_else(|_| "{}".to_string());
    let progress_json = serde_json::to_string_pretty(&ctx.progress)
        .unwrap_or_else(|_| "{}".to_string());
    let summaries_json = serde_json::to_string_pretty(&ctx.recent_session_summaries)
        .unwrap_or_else(|_| "[]".to_string());
    let history_json = serde_json::to_string_pretty(&ctx.session_history)
        .unwrap_or_else(|_| "[]".to_string());

    format!(
        r#"Generate a session narrative for the session that just ended.

## Learner Profile
{profile_json}

## Current Progress Snapshot
{progress_json}

## Recent Session Summaries (last 2-3 sessions)
{summaries_json}

## This Session — Complete Assignment Log
{history_json}

## Session Duration
{duration} minutes

Based on this data, generate:
1. Behavioral observations (what you noticed about how the child engaged)
2. Continuity notes (what to remember for the next session)
3. Recommended focus areas for the next session
4. A difficulty adjustment recommendation

Return ONLY the JSON object."#,
        profile_json = profile_json,
        progress_json = progress_json,
        summaries_json = summaries_json,
        history_json = history_json,
        duration = ctx.session_duration_minutes,
    )
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

use crate::learner::profile::LearnerProfile;

impl SanitizedProfile {
    /// Create a `SanitizedProfile` from a full `LearnerProfile`, stripping all
    /// system-internal identifiers (`id`, schema version, etc.).
    pub fn from_profile(profile: &LearnerProfile) -> Self {
        SanitizedProfile {
            name: profile.name.clone(),
            age: profile.age,
            interests: profile.interests.clone(),
            initial_preferences: profile.initial_preferences.clone(),
            observed_behavior: profile.observed_behavior.clone(),
        }
    }
}

impl ProgressSnapshot {
    /// Create a `ProgressSnapshot` from a full `LearnerProgress`, stripping
    /// `learnerId` and other system-internal fields.
    pub fn from_progress(progress: &LearnerProgress) -> Self {
        ProgressSnapshot {
            skills: progress.skills.clone(),
            total_sessions: progress.total_sessions,
            total_assignments: progress.total_assignments,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learner::profile::{
        AttentionPattern, ChallengePreference, InitialPreferences, LearnerProfile, ObservedBehavior,
    };
    use uuid::Uuid;

    fn sample_profile() -> LearnerProfile {
        LearnerProfile {
            schema_version: 1,
            id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            name: "StarExplorer42".to_string(),
            age: 8,
            interests: vec!["dinosaurs".to_string(), "space".to_string()],
            initial_preferences: InitialPreferences {
                session_length_minutes: 25,
                challenge_preference: ChallengePreference::Guided,
            },
            observed_behavior: ObservedBehavior {
                frustration_response: crate::learner::profile::FrustrationResponse::Unknown,
                effort_attribution: crate::learner::profile::EffortAttribution::Unknown,
                hint_usage: crate::learner::profile::HintUsage::Unknown,
                attention_pattern: AttentionPattern {
                    optimal_session_minutes: None,
                    accuracy_decay_onset: None,
                },
            },
        }
    }

    #[test]
    fn test_sanitized_profile_excludes_id() {
        let profile = sample_profile();
        let sanitized = SanitizedProfile::from_profile(&profile);
        let json = serde_json::to_string(&sanitized).expect("serialize");

        // Must NOT contain the UUID.
        assert!(
            !json.contains("550e8400"),
            "sanitized profile must not contain UUID"
        );
        // Must NOT contain schema version.
        assert!(
            !json.contains("schemaVersion"),
            "sanitized profile must not contain schemaVersion"
        );
        // Must contain the display name and age.
        assert!(json.contains("StarExplorer42"));
        assert!(json.contains("\"age\":8") || json.contains("\"age\": 8"));
    }

    #[test]
    fn test_sanitized_profile_contains_display_name() {
        let profile = sample_profile();
        let sanitized = SanitizedProfile::from_profile(&profile);
        assert_eq!(sanitized.name, "StarExplorer42");
        assert_eq!(sanitized.age, 8);
        assert_eq!(sanitized.interests, vec!["dinosaurs", "space"]);
    }

    #[test]
    fn test_sanitized_profile_no_learner_id_field() {
        let profile = sample_profile();
        let sanitized = SanitizedProfile::from_profile(&profile);
        let json = serde_json::to_string(&sanitized).expect("serialize");

        // These fields must not appear.
        assert!(!json.contains("\"id\""), "id field must be absent");
        assert!(!json.contains("\"learnerId\""), "learnerId must be absent");
    }

    #[test]
    fn test_evaluation_system_prompt_contains_guardrails() {
        let prompt = evaluation_system_prompt();
        // The guardrails must be present in every evaluation system prompt.
        assert!(
            prompt.contains("FEEDBACK RULES — NON-NEGOTIABLE"),
            "guardrails header must be present"
        );
        assert!(
            prompt.contains("NEVER use discouraging language"),
            "discouragement guardrail must be present"
        );
        assert!(
            prompt.contains("NEVER compare the child"),
            "comparison guardrail must be present"
        );
        assert!(
            prompt.contains("growth-mindset"),
            "growth-mindset instruction must be present"
        );
    }

    #[test]
    fn test_feedback_guardrails_constant_is_nonempty() {
        assert!(!FEEDBACK_GUARDRAILS.is_empty());
        assert!(FEEDBACK_GUARDRAILS.contains("NEVER"));
    }

    #[test]
    fn test_progress_snapshot_excludes_learner_id() {
        use crate::progress::tracker::{LearnerProgress, Metacognition, Streaks};
        use std::collections::HashMap;

        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let progress = LearnerProgress {
            schema_version: 1,
            learner_id: id,
            skills: HashMap::new(),
            badges: vec![],
            streaks: Streaks::default(),
            total_sessions: 5,
            total_time_minutes: 120,
            total_assignments: 25,
            metacognition: Metacognition::default(),
            challenge_flags: HashMap::new(),
        };

        let snapshot = ProgressSnapshot::from_progress(&progress);
        let json = serde_json::to_string(&snapshot).expect("serialize");

        assert!(
            !json.contains("550e8400"),
            "progress snapshot must not contain learner UUID"
        );
        assert!(
            !json.contains("\"learnerId\""),
            "learnerId must not appear in progress snapshot"
        );
        assert!(
            !json.contains("schemaVersion"),
            "schemaVersion must not appear in progress snapshot"
        );
    }
}
