// Integration tests for offline assignment buffer and graceful degradation.
//
// Tests cover:
// - Buffer CRUD round-trips
// - Staleness detection
// - Draw-from-buffer (consuming)
// - Corrupt buffer handling
// - Buffer status building
// - Session sync detection
// - Tier detection without Claude client
// - Buffer replenishment without Claude (deterministic fallback)

use educational_companion::assignments::{VerificationStatus, VerifiedAssignment};
use educational_companion::offline::{
    build_buffer_status, buffer_path, detect_tier, draw_from_buffer, find_sessions_needing_sync,
    read_buffer, replenish_buffer, write_buffer, AssignmentBuffer, BufferedAssignment,
    DegradationTier, BUFFER_STALE_DAYS, OFFLINE_PLACEHOLDER,
};
use educational_companion::claude::schemas::GeneratedAssignment;
use chrono::Utc;
use tempfile::TempDir;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn temp_data_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

fn make_verified(skill: &str) -> VerifiedAssignment {
    VerifiedAssignment {
        assignment: GeneratedAssignment {
            assignment_type: "sequence-puzzle".to_string(),
            skill: skill.to_string(),
            difficulty: 3,
            theme: "math".to_string(),
            prompt: "1, 2, 3, ?".to_string(),
            correct_answer: serde_json::json!(4),
            acceptable_answers: vec![serde_json::json!(4)],
            hints: vec!["Count up".to_string()],
            explanation: "Add 1 each time".to_string(),
            modality: None,
            verification_data: Some(serde_json::json!({
                "terms": [1, 2, 3],
                "sequenceType": "arithmetic",
            })),
        },
        verification_status: VerificationStatus::Verified,
        needs_parent_review: false,
        used_fallback: true,
    }
}

fn fresh_entry(skill: &str) -> BufferedAssignment {
    BufferedAssignment {
        generated_at: Utc::now(),
        assignment: make_verified(skill),
    }
}

fn stale_entry(skill: &str) -> BufferedAssignment {
    use chrono::Duration;
    BufferedAssignment {
        generated_at: Utc::now() - Duration::days(BUFFER_STALE_DAYS + 1),
        assignment: make_verified(skill),
    }
}

// ---------------------------------------------------------------------------
// Buffer I/O tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_read_buffer_returns_none_when_no_file() {
    let dir = temp_data_dir();
    let result = read_buffer(dir.path(), Uuid::new_v4()).await;
    assert!(result.is_none());
}

#[tokio::test]
async fn test_write_creates_buffer_dir_and_file() {
    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();
    let buffer = AssignmentBuffer {
        learner_id,
        generated_at: Utc::now(),
        assignments: vec![fresh_entry("pattern-recognition")],
    };

    write_buffer(dir.path(), &buffer).await.unwrap();

    let path = buffer_path(dir.path(), learner_id);
    assert!(path.exists(), "buffer file should exist after write");
}

#[tokio::test]
async fn test_write_and_read_round_trip() {
    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();
    let buffer = AssignmentBuffer {
        learner_id,
        generated_at: Utc::now(),
        assignments: vec![
            fresh_entry("pattern-recognition"),
            fresh_entry("sequential-logic"),
        ],
    };

    write_buffer(dir.path(), &buffer).await.unwrap();
    let read_back = read_buffer(dir.path(), learner_id).await.unwrap();

    assert_eq!(read_back.assignments.len(), 2);
    assert_eq!(read_back.learner_id, learner_id);
}

#[tokio::test]
async fn test_read_buffer_discards_stale_entries() {
    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();
    let buffer = AssignmentBuffer {
        learner_id,
        generated_at: Utc::now(),
        assignments: vec![
            stale_entry("pattern-recognition"),
            fresh_entry("sequential-logic"),
        ],
    };

    write_buffer(dir.path(), &buffer).await.unwrap();
    let read_back = read_buffer(dir.path(), learner_id).await.unwrap();

    // Only the fresh entry should remain.
    assert_eq!(read_back.assignments.len(), 1);
    assert_eq!(
        read_back.assignments[0].assignment.assignment.skill,
        "sequential-logic"
    );
}

#[tokio::test]
async fn test_read_corrupt_buffer_returns_none_and_deletes_file() {
    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();
    let path = buffer_path(dir.path(), learner_id);

    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&path, b"{this is not valid json!!")
        .await
        .unwrap();

    let result = read_buffer(dir.path(), learner_id).await;

    assert!(result.is_none(), "corrupt buffer should return None");
    assert!(!path.exists(), "corrupt buffer file should be deleted");
}

// ---------------------------------------------------------------------------
// draw_from_buffer tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_draw_from_buffer_returns_none_when_empty() {
    let dir = temp_data_dir();
    let result = draw_from_buffer(dir.path(), Uuid::new_v4()).await;
    assert!(result.is_none());
}

#[tokio::test]
async fn test_draw_from_buffer_removes_entry_from_file() {
    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();
    let buffer = AssignmentBuffer {
        learner_id,
        generated_at: Utc::now(),
        assignments: vec![
            fresh_entry("pattern-recognition"),
            fresh_entry("sequential-logic"),
        ],
    };
    write_buffer(dir.path(), &buffer).await.unwrap();

    let drawn = draw_from_buffer(dir.path(), learner_id).await;
    assert!(drawn.is_some(), "should draw an assignment");

    // Confirm one entry was removed from disk.
    let remaining = read_buffer(dir.path(), learner_id).await.unwrap();
    assert_eq!(
        remaining.assignments.len(),
        1,
        "one entry should remain after draw"
    );
}

#[tokio::test]
async fn test_draw_from_buffer_fifo_order() {
    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();
    let buffer = AssignmentBuffer {
        learner_id,
        generated_at: Utc::now(),
        assignments: vec![
            fresh_entry("pattern-recognition"),
            fresh_entry("sequential-logic"),
        ],
    };
    write_buffer(dir.path(), &buffer).await.unwrap();

    let drawn = draw_from_buffer(dir.path(), learner_id).await.unwrap();
    assert_eq!(
        drawn.assignment.skill, "pattern-recognition",
        "should draw the first entry (FIFO)"
    );
}

#[tokio::test]
async fn test_draw_from_buffer_skips_stale() {
    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();
    let buffer = AssignmentBuffer {
        learner_id,
        generated_at: Utc::now(),
        assignments: vec![
            stale_entry("pattern-recognition"), // stale — should be discarded
            fresh_entry("sequential-logic"),     // fresh — should be drawn
        ],
    };
    write_buffer(dir.path(), &buffer).await.unwrap();

    let drawn = draw_from_buffer(dir.path(), learner_id).await.unwrap();
    assert_eq!(
        drawn.assignment.skill, "sequential-logic",
        "should skip stale entry and draw fresh one"
    );
}

// ---------------------------------------------------------------------------
// Buffer status tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_build_buffer_status_no_buffer() {
    let status = build_buffer_status(None, DegradationTier::Template);
    assert_eq!(status.count, 0);
    assert!(status.generated_at.is_none());
    assert!(!status.has_stale_entries);
    assert_eq!(status.current_tier, DegradationTier::Template);
}

#[tokio::test]
async fn test_build_buffer_status_counts_fresh_per_skill() {
    let buffer = AssignmentBuffer {
        learner_id: Uuid::new_v4(),
        generated_at: Utc::now(),
        assignments: vec![
            fresh_entry("pattern-recognition"),
            fresh_entry("pattern-recognition"),
            fresh_entry("sequential-logic"),
        ],
    };
    let status = build_buffer_status(Some(&buffer), DegradationTier::Buffered);
    assert_eq!(status.count, 3);
    assert_eq!(status.per_skill.get("pattern-recognition"), Some(&2));
    assert_eq!(status.per_skill.get("sequential-logic"), Some(&1));
}

#[tokio::test]
async fn test_build_buffer_status_flags_stale() {
    let buffer = AssignmentBuffer {
        learner_id: Uuid::new_v4(),
        generated_at: Utc::now(),
        assignments: vec![
            stale_entry("pattern-recognition"),
            fresh_entry("sequential-logic"),
        ],
    };
    let status = build_buffer_status(Some(&buffer), DegradationTier::Buffered);
    assert!(status.has_stale_entries);
    assert_eq!(status.count, 1); // only fresh entries count
}

// ---------------------------------------------------------------------------
// Tier detection tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_detect_tier_no_claude_client_returns_template() {
    let tier = detect_tier(None, None).await;
    assert_eq!(tier, DegradationTier::Template);
}

#[tokio::test]
async fn test_detect_tier_no_claude_client_buffer_still_template() {
    let buffer = AssignmentBuffer {
        learner_id: Uuid::new_v4(),
        generated_at: Utc::now(),
        assignments: vec![fresh_entry("pattern-recognition")],
    };
    // Even with a buffer, no Claude client means Template (not Buffered).
    // (The buffer is only meaningful if Claude was previously available.)
    let tier = detect_tier(None, Some(&buffer)).await;
    assert_eq!(tier, DegradationTier::Template);
}

// ---------------------------------------------------------------------------
// Session sync tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_find_sessions_needing_sync_empty_dir() {
    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();
    let result = find_sessions_needing_sync(dir.path(), learner_id).await;
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_find_sessions_needing_sync_detects_offline_sessions() {
    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();
    let sessions_dir = dir
        .path()
        .join("learners")
        .join(learner_id.to_string())
        .join("sessions");
    tokio::fs::create_dir_all(&sessions_dir).await.unwrap();

    // Write a session that needs sync (has the placeholder).
    let offline_content = format!(
        "# Session: 2026-04-07 15:30\n## Behavioral Observations\n{}\n",
        OFFLINE_PLACEHOLDER
    );
    tokio::fs::write(
        sessions_dir.join("session-2026-04-07-1530.md"),
        &offline_content,
    )
    .await
    .unwrap();

    // Write a session that does NOT need sync.
    let ok_content = "# Session: 2026-04-06 10:00\n## Behavioral Observations\nGreat session!\n";
    tokio::fs::write(
        sessions_dir.join("session-2026-04-06-1000.md"),
        ok_content,
    )
    .await
    .unwrap();

    let needs_sync = find_sessions_needing_sync(dir.path(), learner_id).await;
    assert_eq!(needs_sync.len(), 1);
    assert_eq!(needs_sync[0], "session-2026-04-07-1530");
}

#[tokio::test]
async fn test_find_sessions_needing_sync_returns_sorted() {
    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();
    let sessions_dir = dir
        .path()
        .join("learners")
        .join(learner_id.to_string())
        .join("sessions");
    tokio::fs::create_dir_all(&sessions_dir).await.unwrap();

    let offline_content = |date: &str| {
        format!(
            "# Session: {date}\n## Behavioral Observations\n{}\n",
            OFFLINE_PLACEHOLDER
        )
    };

    tokio::fs::write(
        sessions_dir.join("session-2026-04-09-1000.md"),
        offline_content("2026-04-09 10:00"),
    )
    .await
    .unwrap();

    tokio::fs::write(
        sessions_dir.join("session-2026-04-07-0900.md"),
        offline_content("2026-04-07 09:00"),
    )
    .await
    .unwrap();

    let needs_sync = find_sessions_needing_sync(dir.path(), learner_id).await;
    assert_eq!(needs_sync.len(), 2);
    // Should be sorted chronologically (alphabetical == chronological for this format).
    assert!(needs_sync[0] < needs_sync[1]);
}

// ---------------------------------------------------------------------------
// Buffer replenishment tests (without Claude — deterministic fallback)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_replenish_buffer_without_claude_fills_with_deterministic() {
    use educational_companion::assignments::AssignmentTemplate;

    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();

    // Build a minimal progress record.
    let progress = educational_companion::progress::LearnerProgress::default_for(learner_id);

    // Load templates from the project's data directory if available, or use empty list.
    let templates: Vec<AssignmentTemplate> = Vec::new();

    replenish_buffer(dir.path(), learner_id, &progress, &templates, None)
        .await
        .unwrap();

    let buffer = read_buffer(dir.path(), learner_id).await.unwrap();
    assert!(
        !buffer.assignments.is_empty(),
        "buffer should have assignments after replenishment"
    );
    // All assignments in the buffer must be deterministic fallbacks (used_fallback=true)
    // since no Claude client was given. Deterministic fallbacks are always correct
    // by construction even if verification status is Unverifiable (no template to run
    // the checker against).
    for entry in &buffer.assignments {
        assert!(
            entry.assignment.used_fallback,
            "assignments without Claude should use the deterministic fallback"
        );
    }
}

#[tokio::test]
async fn test_replenish_buffer_does_not_exceed_target() {
    use educational_companion::offline::BUFFER_TARGET_SIZE;
    use educational_companion::assignments::AssignmentTemplate;

    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();
    let progress = educational_companion::progress::LearnerProgress::default_for(learner_id);
    let templates: Vec<AssignmentTemplate> = Vec::new();

    replenish_buffer(dir.path(), learner_id, &progress, &templates, None)
        .await
        .unwrap();

    let buffer = read_buffer(dir.path(), learner_id).await.unwrap();
    assert!(
        buffer.assignments.len() <= BUFFER_TARGET_SIZE,
        "buffer should not exceed target size"
    );
}

#[tokio::test]
async fn test_replenish_buffer_skips_if_already_full() {
    use educational_companion::offline::BUFFER_TARGET_SIZE;
    use educational_companion::assignments::AssignmentTemplate;

    let dir = temp_data_dir();
    let learner_id = Uuid::new_v4();

    // Pre-fill the buffer to target.
    let assignments: Vec<BufferedAssignment> = (0..BUFFER_TARGET_SIZE)
        .map(|_| fresh_entry("pattern-recognition"))
        .collect();
    let buffer = AssignmentBuffer {
        learner_id,
        generated_at: Utc::now(),
        assignments,
    };
    write_buffer(dir.path(), &buffer).await.unwrap();

    let progress = educational_companion::progress::LearnerProgress::default_for(learner_id);
    let templates: Vec<AssignmentTemplate> = Vec::new();

    replenish_buffer(dir.path(), learner_id, &progress, &templates, None)
        .await
        .unwrap();

    let read_back = read_buffer(dir.path(), learner_id).await.unwrap();
    // Should not have added more — already at target.
    assert_eq!(read_back.assignments.len(), BUFFER_TARGET_SIZE);
}
