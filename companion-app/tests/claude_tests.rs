/// Integration tests for the Claude API integration layer.
///
/// These tests validate:
/// - Sanitized profile contains no `id`, `learnerId`, or UUID fields
/// - Structured output types deserialize correctly
/// - Evaluation system prompt always contains feedback guardrails
/// - Generation and evaluation contexts carry the right data tiers
use educational_companion::claude::{
    self, ClaudeClient, ClaudeError, DifficultyAdjustment, EvaluationConfidence, EvaluationContext,
    EvaluationResult, GeneratedAssignment, GenerationContext, NarrativeContext, ObservedBehavioralSignals,
    ProgressSnapshot, SanitizedProfile, SessionHistoryItem, SessionNarrative, SessionSummary,
};
use educational_companion::learner::{
    AttentionPattern, ChallengePreference, InitialPreferences, LearnerProfile, ObservedBehavior,
};
use educational_companion::progress::{
    LearnerProgress, Metacognition, MetacognitionTrend, SkillProgress, Streaks,
    WorkingMemorySignal, ZpdLevels,
};
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_profile() -> LearnerProfile {
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
            frustration_response: educational_companion::learner::FrustrationResponse::Unknown,
            effort_attribution: educational_companion::learner::EffortAttribution::Unknown,
            hint_usage: educational_companion::learner::HintUsage::Unknown,
            attention_pattern: AttentionPattern {
                optimal_session_minutes: None,
                accuracy_decay_onset: None,
            },
        },
    }
}

fn make_progress() -> LearnerProgress {
    let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let mut skills = HashMap::new();
    skills.insert(
        "pattern-recognition".to_string(),
        SkillProgress {
            level: 4,
            xp: 340,
            last_practiced: None,
            zpd: ZpdLevels {
                independent_level: 3,
                scaffolded_level: 5,
            },
            recent_accuracy: vec![1, 1, 0, 1, 1],
            working_memory_signal: WorkingMemorySignal::Stable,
            spaced_repetition: educational_companion::progress::SpacedRepetition {
                interval_days: 7,
                ease_factor: 2.5,
                next_review_date: chrono::NaiveDate::from_ymd_opt(2026, 4, 14).unwrap(),
                consecutive_correct: 4,
            },
        },
    );
    LearnerProgress {
        schema_version: 1,
        learner_id: id,
        skills,
        badges: vec![],
        streaks: Streaks::default(),
        total_sessions: 5,
        total_time_minutes: 120,
        total_assignments: 25,
        metacognition: Metacognition {
            self_correction_rate: 0.2,
            hint_request_rate: 0.1,
            trend: MetacognitionTrend::Improving,
        },
        challenge_flags: HashMap::new(),
    }
}

fn make_assignment() -> GeneratedAssignment {
    GeneratedAssignment {
        assignment_type: "sequence-puzzle".to_string(),
        skill: "pattern-recognition".to_string(),
        difficulty: 4,
        theme: "dinosaurs".to_string(),
        prompt: "A T-Rex takes 2 steps, then 4, then 8. How many steps next?".to_string(),
        correct_answer: serde_json::json!(16),
        acceptable_answers: vec![serde_json::json!(16), serde_json::json!("16")],
        hints: vec![
            "Look at how the number changes each time...".to_string(),
            "Each time, the number doubles!".to_string(),
            "8 × 2 = ?".to_string(),
        ],
        explanation: "Each step doubles: 2→4→8→16. This is a geometric sequence!".to_string(),
        modality: None,
    }
}

// ---------------------------------------------------------------------------
// Sanitized profile tests
// ---------------------------------------------------------------------------

#[test]
fn test_sanitized_profile_has_no_id_field() {
    let profile = make_profile();
    let sanitized = SanitizedProfile::from_profile(&profile);
    let json = serde_json::to_string(&sanitized).expect("serialize");

    // The UUID string must not appear.
    assert!(
        !json.contains("550e8400"),
        "UUID must not be present in sanitized profile: {}",
        json
    );
    // The field names `id` and `learnerId` must not appear.
    assert!(
        !json.contains("\"id\""),
        "field 'id' must not be present: {}",
        json
    );
    assert!(
        !json.contains("\"learnerId\""),
        "field 'learnerId' must not be present: {}",
        json
    );
}

#[test]
fn test_sanitized_profile_has_no_schema_version() {
    let profile = make_profile();
    let sanitized = SanitizedProfile::from_profile(&profile);
    let json = serde_json::to_string(&sanitized).expect("serialize");

    assert!(
        !json.contains("schemaVersion"),
        "schemaVersion must not be present in sanitized profile: {}",
        json
    );
}

#[test]
fn test_sanitized_profile_contains_required_fields() {
    let profile = make_profile();
    let sanitized = SanitizedProfile::from_profile(&profile);

    assert_eq!(sanitized.name, "StarExplorer42");
    assert_eq!(sanitized.age, 8);
    assert_eq!(
        sanitized.interests,
        vec!["dinosaurs".to_string(), "space".to_string()]
    );
}

#[test]
fn test_sanitized_profile_serializes_all_required_fields() {
    let profile = make_profile();
    let sanitized = SanitizedProfile::from_profile(&profile);
    let json = serde_json::to_string(&sanitized).expect("serialize");

    assert!(json.contains("\"name\""), "name field must be present");
    assert!(json.contains("\"age\""), "age field must be present");
    assert!(json.contains("\"interests\""), "interests must be present");
    assert!(
        json.contains("\"initialPreferences\""),
        "initialPreferences must be present"
    );
    assert!(
        json.contains("\"observedBehavior\""),
        "observedBehavior must be present"
    );
}

// ---------------------------------------------------------------------------
// Progress snapshot tests
// ---------------------------------------------------------------------------

#[test]
fn test_progress_snapshot_excludes_learner_id() {
    let progress = make_progress();
    let snapshot = ProgressSnapshot::from_progress(&progress);
    let json = serde_json::to_string(&snapshot).expect("serialize");

    assert!(
        !json.contains("550e8400"),
        "learner UUID must not be in progress snapshot: {}",
        json
    );
    assert!(
        !json.contains("\"learnerId\""),
        "learnerId field must not be in progress snapshot: {}",
        json
    );
    assert!(
        !json.contains("schemaVersion"),
        "schemaVersion must not be in progress snapshot: {}",
        json
    );
}

#[test]
fn test_progress_snapshot_contains_skills_and_counts() {
    let progress = make_progress();
    let snapshot = ProgressSnapshot::from_progress(&progress);

    assert!(snapshot.skills.contains_key("pattern-recognition"));
    assert_eq!(snapshot.total_sessions, 5);
    assert_eq!(snapshot.total_assignments, 25);
}

// ---------------------------------------------------------------------------
// Structured output schema tests
// ---------------------------------------------------------------------------

#[test]
fn test_generated_assignment_full_round_trip() {
    let a = make_assignment();
    let json = serde_json::to_string(&a).expect("serialize");
    let restored: GeneratedAssignment = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(a, restored);
}

#[test]
fn test_evaluation_result_round_trip() {
    let e = EvaluationResult {
        correct: true,
        confidence: EvaluationConfidence::High,
        feedback: "Great job! You found the doubling pattern!".to_string(),
        explanation: "2→4→8→16 — each number doubles the previous one.".to_string(),
        behavioral_signals: ObservedBehavioralSignals {
            self_correction_detected: false,
            hint_used: false,
            response_suggests_struggle: false,
            reasoning_note: None,
        },
    };
    let json = serde_json::to_string(&e).expect("serialize");
    let restored: EvaluationResult = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(e, restored);
}

#[test]
fn test_session_narrative_round_trip() {
    let n = SessionNarrative {
        behavioral_observations: "Child stayed engaged throughout the session.".to_string(),
        continuity_notes: "Ready for 3-step sequential problems next session.".to_string(),
        recommended_focus_areas: vec!["sequential-logic".to_string()],
        difficulty_adjustment: DifficultyAdjustment::Increase,
        flag_for_parent_review: false,
    };
    let json = serde_json::to_string(&n).expect("serialize");
    let restored: SessionNarrative = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(n, restored);
}

// ---------------------------------------------------------------------------
// Feedback guardrails tests
// ---------------------------------------------------------------------------

#[test]
fn test_evaluation_prompt_contains_guardrails() {
    let system_prompt = claude::prompts::evaluation_system_prompt();

    assert!(
        system_prompt.contains("FEEDBACK RULES — NON-NEGOTIABLE"),
        "guardrails header must be present in every evaluation prompt"
    );
    assert!(
        system_prompt.contains("NEVER use discouraging language"),
        "discouraging language guardrail must be present"
    );
    assert!(
        system_prompt.contains("NEVER compare the child"),
        "comparison guardrail must be present"
    );
    assert!(
        system_prompt.contains("growth-mindset"),
        "growth-mindset instruction must be in evaluation prompt"
    );
    assert!(
        system_prompt.contains("backend_verified_correct"),
        "backend verification instruction must be present"
    );
}

#[test]
fn test_guardrails_constant_present_in_evaluation_prompt() {
    let prompt = claude::prompts::evaluation_system_prompt();
    // The full guardrails constant must be embedded.
    assert!(prompt.contains(claude::prompts::FEEDBACK_GUARDRAILS));
}

// ---------------------------------------------------------------------------
// Context construction tests
// ---------------------------------------------------------------------------

#[test]
fn test_generation_context_has_correct_tiers() {
    let profile = make_profile();
    let progress = make_progress();

    let ctx = GenerationContext {
        profile: SanitizedProfile::from_profile(&profile),
        progress: ProgressSnapshot::from_progress(&progress),
        recent_session_summaries: vec![SessionSummary {
            date: "2026-04-07".to_string(),
            behavioral_observations: "Stayed engaged the whole time.".to_string(),
            continuity_notes: "Try multi-step problems next.".to_string(),
        }],
        target_skill: "pattern-recognition".to_string(),
        target_difficulty: 4,
    };

    // Profile must have no UUID.
    let profile_json = serde_json::to_string(&ctx.profile).expect("serialize");
    assert!(!profile_json.contains("550e8400"));

    // Context includes the right data.
    assert_eq!(ctx.target_skill, "pattern-recognition");
    assert_eq!(ctx.target_difficulty, 4);
    assert_eq!(ctx.recent_session_summaries.len(), 1);
}

#[test]
fn test_evaluation_context_includes_verified_answer() {
    let profile = make_profile();
    let progress = make_progress();
    let assignment = make_assignment();

    let ctx = EvaluationContext {
        profile: SanitizedProfile::from_profile(&profile),
        progress: ProgressSnapshot::from_progress(&progress),
        session_history: vec![],
        assignment,
        verified_correct_answer: serde_json::json!(16),
        child_response: "16".to_string(),
        backend_verified_correct: true,
    };

    assert_eq!(ctx.verified_correct_answer, serde_json::json!(16));
    assert!(ctx.backend_verified_correct);
    assert_eq!(ctx.child_response, "16");

    // Profile must have no UUID.
    let profile_json = serde_json::to_string(&ctx.profile).expect("serialize");
    assert!(!profile_json.contains("550e8400"));
}

#[test]
fn test_narrative_context_includes_full_session_history() {
    let profile = make_profile();
    let progress = make_progress();
    let assignment = make_assignment();

    let history_item = SessionHistoryItem {
        assignment: assignment.clone(),
        child_response: "16".to_string(),
        correct: true,
        time_seconds: 45,
    };

    let ctx = NarrativeContext {
        profile: SanitizedProfile::from_profile(&profile),
        progress: ProgressSnapshot::from_progress(&progress),
        recent_session_summaries: vec![],
        session_history: vec![history_item],
        session_duration_minutes: 25,
    };

    assert_eq!(ctx.session_history.len(), 1);
    assert_eq!(ctx.session_duration_minutes, 25);

    // Profile must have no UUID.
    let profile_json = serde_json::to_string(&ctx.profile).expect("serialize");
    assert!(!profile_json.contains("550e8400"));
}

// ---------------------------------------------------------------------------
// Client construction tests
// ---------------------------------------------------------------------------

#[test]
fn test_client_from_env_missing_key() {
    std::env::remove_var("ANTHROPIC_API_KEY");
    let result = ClaudeClient::from_env();
    assert!(
        matches!(result, Err(ClaudeError::MissingApiKey)),
        "expected MissingApiKey, got: {:?}",
        result
    );
}

#[test]
fn test_client_from_env_with_key() {
    std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-key");
    let result = ClaudeClient::from_env();
    assert!(result.is_ok(), "should succeed when key is set");
    std::env::remove_var("ANTHROPIC_API_KEY");
}

// ---------------------------------------------------------------------------
// Generation and evaluation are distinct operations (structural test)
// ---------------------------------------------------------------------------

#[test]
fn test_generate_and_evaluate_are_separate_methods() {
    // This test verifies at the type level that there are distinct methods for
    // generation and evaluation — confirming the separation of concerns.
    // The ClaudeClient has three separate async methods:
    //   generate_assignment(&self, ctx: &GenerationContext) -> Result<GeneratedAssignment, ClaudeError>
    //   evaluate_response(&self, ctx: &EvaluationContext) -> Result<EvaluationResult, ClaudeError>
    //   generate_session_narrative(&self, ctx: &NarrativeContext) -> Result<SessionNarrative, ClaudeError>
    //
    // GenerationContext and EvaluationContext are different types — they cannot
    // be accidentally swapped.
    let profile = make_profile();
    let progress = make_progress();

    let _gen_ctx = GenerationContext {
        profile: SanitizedProfile::from_profile(&profile),
        progress: ProgressSnapshot::from_progress(&progress),
        recent_session_summaries: vec![],
        target_skill: "pattern-recognition".to_string(),
        target_difficulty: 3,
    };

    let _eval_ctx = EvaluationContext {
        profile: SanitizedProfile::from_profile(&profile),
        progress: ProgressSnapshot::from_progress(&progress),
        session_history: vec![],
        assignment: make_assignment(),
        verified_correct_answer: serde_json::json!(16),
        child_response: "16".to_string(),
        backend_verified_correct: true,
    };

    // If this compiles, the types are distinct — generation and evaluation
    // contexts are separate structs, preventing accidental mixing.
}
