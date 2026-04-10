//! Integration tests for the assignment generation & verification pipeline.
//! These tests exercise the full assignment module against real data, using
//! temporary directories (no live server or Claude API required).

use std::path::PathBuf;
use tempfile::TempDir;

use educational_companion::assignments::{
    self, PipelineRequest, VerificationLevel, VerificationStatus, VerifiedAssignment,
};
use educational_companion::claude::schemas::GeneratedAssignment;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a temporary directory with all three template JSON files.
fn setup_templates_dir() -> (TempDir, PathBuf) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("assignment-templates");
    std::fs::create_dir_all(&path).unwrap();

    // sequence-puzzle.json
    std::fs::write(
        path.join("sequence-puzzle.json"),
        serde_json::json!({
            "type": "sequence-puzzle",
            "constraints": {
                "sequenceTypes": ["arithmetic", "geometric", "fibonacci-like"],
                "maxTerms": 6,
                "numberRange": [1, 100],
                "operations": ["add", "multiply"]
            },
            "verificationLevel": "full",
            "verificationMethod": "compute-sequence"
        })
        .to_string(),
    )
    .unwrap();

    // deductive-reasoning.json
    std::fs::write(
        path.join("deductive-reasoning.json"),
        serde_json::json!({
            "type": "deductive-reasoning",
            "constraints": {
                "maxPremises": 3,
                "logicTypes": ["if-then", "elimination", "syllogism"],
                "domainVocabulary": "age-appropriate"
            },
            "verificationLevel": "partial",
            "verificationMethod": "rule-check"
        })
        .to_string(),
    )
    .unwrap();

    // pattern-matching.json
    std::fs::write(
        path.join("pattern-matching.json"),
        serde_json::json!({
            "type": "pattern-matching",
            "constraints": {
                "patternTypes": ["color", "shape", "number", "symbol"],
                "maxPatternLength": 6,
                "choices": 4
            },
            "verificationLevel": "partial",
            "verificationMethod": "acceptability-check"
        })
        .to_string(),
    )
    .unwrap();

    (dir, path)
}

fn make_bad_sequence_assignment() -> GeneratedAssignment {
    GeneratedAssignment {
        assignment_type: "sequence-puzzle".to_string(),
        skill: "pattern-recognition".to_string(),
        difficulty: 3,
        theme: "test".to_string(),
        prompt: "2, 4, 8, ?".to_string(),
        correct_answer: serde_json::json!(999), // wrong!
        acceptable_answers: vec![serde_json::json!(999)],
        hints: vec!["h1".to_string(), "h2".to_string(), "h3".to_string()],
        explanation: "doubling".to_string(),
        modality: None,
        verification_data: Some(serde_json::json!({"terms": [2, 4, 8]})),
    }
}

// ---------------------------------------------------------------------------
// Template loading tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_load_templates_reads_all_three() {
    let (_dir, path) = setup_templates_dir();
    let templates = assignments::load_templates(&path).await.expect("load");
    assert_eq!(templates.len(), 3, "should have loaded 3 templates");

    let types: Vec<&str> = templates
        .iter()
        .map(|t| t.assignment_type.as_str())
        .collect();
    assert!(types.contains(&"sequence-puzzle"));
    assert!(types.contains(&"deductive-reasoning"));
    assert!(types.contains(&"pattern-matching"));
}

#[tokio::test]
async fn test_load_templates_sequence_is_full_verification() {
    let (_dir, path) = setup_templates_dir();
    let templates = assignments::load_templates(&path).await.expect("load");
    let seq = templates
        .iter()
        .find(|t| t.assignment_type == "sequence-puzzle")
        .expect("sequence-puzzle template");

    assert_eq!(seq.verification_level, VerificationLevel::Full);
    assert_eq!(seq.verification_method, "compute-sequence");
}

#[tokio::test]
async fn test_load_templates_deductive_is_partial_verification() {
    let (_dir, path) = setup_templates_dir();
    let templates = assignments::load_templates(&path).await.expect("load");
    let ded = templates
        .iter()
        .find(|t| t.assignment_type == "deductive-reasoning")
        .expect("deductive-reasoning template");

    assert_eq!(ded.verification_level, VerificationLevel::Partial);
    assert_eq!(ded.verification_method, "rule-check");
}

#[tokio::test]
async fn test_load_templates_returns_error_for_empty_dir() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("empty");
    std::fs::create_dir_all(&path).unwrap();

    let result = assignments::load_templates(&path).await;
    assert!(
        result.is_err(),
        "should return error for empty templates dir"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("No assignment templates found"));
}

#[tokio::test]
async fn test_load_templates_ignores_non_json_files() {
    let (_dir, path) = setup_templates_dir();

    // Add a non-JSON file.
    std::fs::write(path.join("README.md"), "# Templates").unwrap();

    let templates = assignments::load_templates(&path).await.expect("load");
    // Should still only load 3 JSON templates.
    assert_eq!(templates.len(), 3);
}

// ---------------------------------------------------------------------------
// Verification integration tests
// ---------------------------------------------------------------------------

#[test]
fn test_sequence_verification_rejects_wrong_claude_answer() {
    let assignment = make_bad_sequence_assignment();
    let status =
        assignments::verify_assignment(&assignment, &VerificationLevel::Full, "compute-sequence");
    assert_eq!(status, VerificationStatus::Unverifiable);
    assert!(assignments::needs_parent_review(&status));
}

#[test]
fn test_sequence_verification_accepts_correct_claude_answer() {
    let assignment = GeneratedAssignment {
        assignment_type: "sequence-puzzle".to_string(),
        skill: "sequential-logic".to_string(),
        difficulty: 4,
        theme: "space".to_string(),
        prompt: "1, 3, 9, 27, ?".to_string(),
        correct_answer: serde_json::json!(81),
        acceptable_answers: vec![serde_json::json!(81)],
        hints: vec!["h1".to_string(), "h2".to_string(), "h3".to_string()],
        explanation: "geometric x3".to_string(),
        modality: None,
        verification_data: Some(serde_json::json!({"terms": [1, 3, 9, 27]})),
    };

    let status =
        assignments::verify_assignment(&assignment, &VerificationLevel::Full, "compute-sequence");
    assert_eq!(status, VerificationStatus::Verified);
    assert!(!assignments::needs_parent_review(&status));
}

// ---------------------------------------------------------------------------
// Pipeline integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_pipeline_fallback_produces_valid_sequence() {
    let (_dir, path) = setup_templates_dir();
    let templates = assignments::load_templates(&path).await.expect("load");

    let request = PipelineRequest {
        skill: "sequential-logic".to_string(),
        difficulty: 4,
        preferred_type: Some("sequence-puzzle".to_string()),
    };

    let result: VerifiedAssignment = assignments::run_pipeline(
        || async { None::<GeneratedAssignment> },
        &templates,
        &request,
        2,
    )
    .await;

    assert!(result.used_fallback);
    assert_eq!(result.assignment.assignment_type, "sequence-puzzle");
    assert!(!result.needs_parent_review);
    assert_eq!(result.verification_status, VerificationStatus::Verified);
}

#[tokio::test]
async fn test_pipeline_retries_then_falls_back() {
    let (_dir, path) = setup_templates_dir();
    let templates = assignments::load_templates(&path).await.expect("load");

    let bad = make_bad_sequence_assignment();
    let request = PipelineRequest {
        skill: "sequential-logic".to_string(),
        difficulty: 3,
        preferred_type: Some("sequence-puzzle".to_string()),
    };

    let result = assignments::run_pipeline(
        || {
            let b = bad.clone();
            async move { Some(b) }
        },
        &templates,
        &request,
        2,
    )
    .await;

    // After 3 attempts with always-wrong answers, should fall back.
    assert!(result.used_fallback);
    // The fallback must always be verifiable.
    assert_eq!(result.verification_status, VerificationStatus::Verified);
}

#[tokio::test]
async fn test_pipeline_accepts_good_claude_assignment() {
    let (_dir, path) = setup_templates_dir();
    let templates = assignments::load_templates(&path).await.expect("load");

    let good = GeneratedAssignment {
        assignment_type: "sequence-puzzle".to_string(),
        skill: "sequential-logic".to_string(),
        difficulty: 4,
        theme: "dinosaurs".to_string(),
        prompt: "2, 4, 6, 8, ?".to_string(),
        correct_answer: serde_json::json!(10),
        acceptable_answers: vec![serde_json::json!(10)],
        hints: vec!["h1".to_string(), "h2".to_string(), "h3".to_string()],
        explanation: "arithmetic +2".to_string(),
        modality: None,
        verification_data: Some(serde_json::json!({"terms": [2, 4, 6, 8]})),
    };

    let request = PipelineRequest {
        skill: "sequential-logic".to_string(),
        difficulty: 4,
        preferred_type: Some("sequence-puzzle".to_string()),
    };

    let result = assignments::run_pipeline(
        || {
            let g = good.clone();
            async move { Some(g) }
        },
        &templates,
        &request,
        2,
    )
    .await;

    assert!(!result.used_fallback);
    assert_eq!(result.verification_status, VerificationStatus::Verified);
    assert!(!result.needs_parent_review);
}

// ---------------------------------------------------------------------------
// Response checking integration tests
// ---------------------------------------------------------------------------

#[test]
fn test_check_response_evaluation_is_separate_from_generation() {
    // This test verifies the architecture: check_response_correct is always
    // called with the backend-verified assignment, not by Claude.
    let assignment = GeneratedAssignment {
        assignment_type: "sequence-puzzle".to_string(),
        skill: "sequential-logic".to_string(),
        difficulty: 3,
        theme: "test".to_string(),
        prompt: "2, 4, 8, ?".to_string(),
        correct_answer: serde_json::json!(16),
        acceptable_answers: vec![serde_json::json!(16), serde_json::json!("16")],
        hints: vec!["h1".to_string(), "h2".to_string(), "h3".to_string()],
        explanation: "doubling".to_string(),
        modality: None,
        verification_data: Some(serde_json::json!({"terms": [2, 4, 8]})),
    };

    // First, backend independently verifies the assignment.
    let verification =
        assignments::verify_assignment(&assignment, &VerificationLevel::Full, "compute-sequence");
    assert_eq!(verification, VerificationStatus::Verified);

    // Then, evaluate the child's response using the backend-verified correct answer.
    assert!(assignments::check_response_correct(&assignment, "16"));
    assert!(assignments::check_response_correct(&assignment, " 16 "));
    assert!(!assignments::check_response_correct(&assignment, "8"));
}

// ---------------------------------------------------------------------------
// Deterministic fallback integration tests
// ---------------------------------------------------------------------------

#[test]
fn test_deterministic_fallback_always_produces_correct_sequences() {
    // Run the deterministic generator multiple times and verify each output.
    for difficulty in 1..=8 {
        let assignment =
            assignments::generate_deterministic("sequence-puzzle", "sequential-logic", difficulty);

        let status = assignments::verify_assignment(
            &assignment,
            &VerificationLevel::Full,
            "compute-sequence",
        );
        assert_eq!(
            status,
            VerificationStatus::Verified,
            "deterministic sequence puzzle at difficulty {} must be verified",
            difficulty
        );
    }
}

#[test]
fn test_deterministic_fallback_for_all_types() {
    let types = [
        (
            "sequence-puzzle",
            VerificationLevel::Full,
            "compute-sequence",
        ),
        (
            "deductive-reasoning",
            VerificationLevel::Partial,
            "rule-check",
        ),
        (
            "pattern-matching",
            VerificationLevel::Partial,
            "acceptability-check",
        ),
    ];

    for (assignment_type, level, method) in &types {
        let assignment = assignments::generate_deterministic(assignment_type, "some-skill", 3);

        assert_eq!(&assignment.assignment_type, assignment_type);

        let status = assignments::verify_assignment(&assignment, level, method);
        assert_ne!(
            status,
            VerificationStatus::Unverifiable,
            "deterministic {} must not be unverifiable",
            assignment_type
        );
    }
}
