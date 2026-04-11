use educational_companion::claude::schemas::{
    DifficultyAdjustment, GeneratedAssignment, SessionNarrative,
};
use educational_companion::progress;
/// Integration tests for the session module.
///
/// Tests cover:
/// - Markdown file creation and content validation
/// - Session summary extraction from markdown
/// - Progress updates on session completion
/// - Listing and reading session files
use educational_companion::session::{
    self, ActiveSession, SessionAssignment, SessionMarkdownParams, SessionStatus, SharedSessionInfo,
};

use chrono::{Local, NaiveDate, TimeZone};
use std::collections::HashMap;
use tempfile::TempDir;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_session_assignment(skill: &str, difficulty: u32, correct: bool) -> SessionAssignment {
    SessionAssignment {
        assignment_id: Uuid::new_v4().to_string(),
        assignment: GeneratedAssignment {
            assignment_type: "sequence-puzzle".to_string(),
            skill: skill.to_string(),
            difficulty,
            theme: "space".to_string(),
            prompt: "What comes next: 2, 4, 8, ?".to_string(),
            correct_answer: serde_json::json!(16),
            acceptable_answers: vec![serde_json::json!(16)],
            hints: vec![
                "Look at the pattern...".to_string(),
                "It doubles each time.".to_string(),
                "8 × 2 = ?".to_string(),
            ],
            explanation: "Each term doubles.".to_string(),
            modality: None,
            verification_data: None,
        },
        child_response: if correct {
            "16".to_string()
        } else {
            "10".to_string()
        },
        correct,
        time_seconds: 30,
        hints_used: 0,
        self_corrected: false,
        notes: None,
        is_confidence_builder: false,
    }
}

fn make_active_session(learner_id: Uuid) -> ActiveSession {
    ActiveSession {
        id: Uuid::new_v4(),
        learner_id,
        started_at: Local.with_ymd_and_hms(2026, 4, 7, 15, 30, 0).unwrap(),
        focus_skill: Some("sequential-logic".to_string()),
        focus_level: Some(3),
        is_shared: false,
        assignments: vec![
            make_session_assignment("sequential-logic", 3, true),
            make_session_assignment("sequential-logic", 3, true),
            make_session_assignment("sequential-logic", 3, false),
            make_session_assignment("pattern-recognition", 2, true),
        ],
        status: SessionStatus::Completed,
    }
}

// ---------------------------------------------------------------------------
// Markdown file write and read
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_write_session_markdown_creates_file() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();
    let session = make_active_session(learner_id);

    let params = SessionMarkdownParams {
        narrative: None,
        badges_earned: &[],
        xp_by_skill: &HashMap::new(),
        difficulty_before: Some(3),
        difficulty_after: Some(3),
        shared_info: None,
    };
    let session_id = session::write_session_markdown_file(
        dir.path(),
        learner_id,
        &session,
        "TestChild",
        &params,
    )
    .await
    .expect("should write file");

    assert_eq!(session_id, "session-2026-04-07-1530");

    let path = session::session_markdown_path(dir.path(), learner_id, &session_id);
    assert!(path.exists(), "markdown file should exist on disk");
}

#[tokio::test]
async fn test_write_session_markdown_content_is_complete() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();
    let session = make_active_session(learner_id);

    let xp_by_skill: HashMap<String, u32> = [
        ("sequential-logic".to_string(), 90u32),
        ("pattern-recognition".to_string(), 20u32),
    ]
    .into();

    let params = SessionMarkdownParams {
        narrative: None,
        badges_earned: &[],
        xp_by_skill: &xp_by_skill,
        difficulty_before: Some(3),
        difficulty_after: Some(4),
        shared_info: None,
    };
    let session_id =
        session::write_session_markdown_file(dir.path(), learner_id, &session, "SpaceKid", &params)
            .await
            .unwrap();

    let path = session::session_markdown_path(dir.path(), learner_id, &session_id);
    let content = tokio::fs::read_to_string(&path).await.unwrap();

    // All required sections must be present.
    assert!(content.contains("# Session: 2026-04-07 15:30"));
    assert!(content.contains("## Learner: SpaceKid"));
    assert!(content.contains("## Focus: Sequential Logic — Level 3"));
    assert!(content.contains("### Assignment 1:"));
    assert!(content.contains("### Assignment 4:"));
    assert!(content.contains("## Session Summary"));
    assert!(content.contains("- Correct: 3/4"));
    assert!(content.contains("## Behavioral Observations"));
    assert!(content.contains("## Continuity Notes"));
    // Shared section should NOT appear.
    assert!(!content.contains("## Shared Session"));
}

#[tokio::test]
async fn test_write_session_markdown_with_narrative() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();
    let session = make_active_session(learner_id);

    let narrative = SessionNarrative {
        behavioral_observations: "The child stayed focused and asked good questions.".to_string(),
        continuity_notes: "Try introducing 3-step problems next session.".to_string(),
        recommended_focus_areas: vec!["sequential-logic".to_string()],
        difficulty_adjustment: DifficultyAdjustment::Increase,
        flag_for_parent_review: false,
    };

    let params = SessionMarkdownParams {
        narrative: Some(&narrative),
        badges_earned: &[],
        xp_by_skill: &HashMap::new(),
        difficulty_before: None,
        difficulty_after: None,
        shared_info: None,
    };
    let session_id =
        session::write_session_markdown_file(dir.path(), learner_id, &session, "SpaceKid", &params)
            .await
            .unwrap();

    let path = session::session_markdown_path(dir.path(), learner_id, &session_id);
    let content = tokio::fs::read_to_string(&path).await.unwrap();

    assert!(content.contains("The child stayed focused and asked good questions."));
    assert!(content.contains("Try introducing 3-step problems next session."));
    // Placeholder should NOT appear when narrative is available.
    assert!(!content.contains("unreachable"));
}

#[tokio::test]
async fn test_write_session_markdown_shared_session() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();
    let mut session = make_active_session(learner_id);
    session.is_shared = true;

    let shared_info = SharedSessionInfo {
        parent_role: "guide".to_string(),
        child_scaffolding_response: "positive — arrived at answer independently".to_string(),
        system_scaffolding_comparison: "child more willing to take risks with parent present"
            .to_string(),
    };

    let params = SessionMarkdownParams {
        narrative: None,
        badges_earned: &[],
        xp_by_skill: &HashMap::new(),
        difficulty_before: None,
        difficulty_after: None,
        shared_info: Some(&shared_info),
    };
    let session_id =
        session::write_session_markdown_file(dir.path(), learner_id, &session, "SpaceKid", &params)
            .await
            .unwrap();

    let path = session::session_markdown_path(dir.path(), learner_id, &session_id);
    let content = tokio::fs::read_to_string(&path).await.unwrap();

    assert!(content.contains("## Shared Session: Parent Co-Solve"));
    assert!(content.contains("- **Mode**: collaborative"));
    assert!(content.contains("guide"));
    assert!(content.contains("positive — arrived at answer independently"));
}

// ---------------------------------------------------------------------------
// Session summary extraction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_load_session_summaries_empty_when_no_sessions() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();

    let summaries = session::load_session_summaries(dir.path(), learner_id, 3).await;
    assert!(summaries.is_empty());
}

#[tokio::test]
async fn test_load_session_summaries_returns_only_summaries_not_full_logs() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();

    // Write two session files.
    for (date, obs, notes) in [
        (
            "session-2026-04-05-1500",
            "Child was engaged.",
            "Review patterns.",
        ),
        (
            "session-2026-04-06-1500",
            "Child struggled with step 3.",
            "Revisit 2-step logic.",
        ),
    ] {
        let sessions_dir = session::sessions_dir(dir.path(), learner_id);
        tokio::fs::create_dir_all(&sessions_dir).await.unwrap();
        let content = format!(
            "# Session: 2026-04-05 15:00\n## Learner: Kid\n\n### Assignment 1: Puzzle\n- **Type**: t\n- **Difficulty**: 2/10\n- **Prompt**: \"p\"\n- **Response**: r\n- **Result**: correct\n- **Time**: 10s\n\n## Session Summary\n- Correct: 1/1\n\n## Behavioral Observations\n{obs}\n\n## Continuity Notes\n{notes}\n",
        );
        tokio::fs::write(sessions_dir.join(format!("{date}.md")), content)
            .await
            .unwrap();
    }

    let summaries = session::load_session_summaries(dir.path(), learner_id, 3).await;
    assert_eq!(summaries.len(), 2);

    // Summaries should include behavioral observations and continuity notes.
    assert!(summaries[0].behavioral_observations.contains("engaged"));
    assert!(summaries[0].continuity_notes.contains("patterns"));
    assert!(summaries[1].behavioral_observations.contains("struggled"));

    // Summaries must NOT contain full assignment logs.
    for s in &summaries {
        assert!(!s.behavioral_observations.contains("### Assignment"));
        assert!(!s.continuity_notes.contains("### Assignment"));
    }
}

#[tokio::test]
async fn test_load_session_summaries_respects_max_sessions() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();
    let sessions_dir = session::sessions_dir(dir.path(), learner_id);
    tokio::fs::create_dir_all(&sessions_dir).await.unwrap();

    // Create 5 sessions.
    for day in 1..=5u32 {
        let name = format!("session-2026-04-{:02}-1500.md", day);
        let content = format!(
            "# Session\n## Behavioral Observations\nDay {day}.\n\n## Continuity Notes\nNotes {day}.\n"
        );
        tokio::fs::write(sessions_dir.join(&name), content)
            .await
            .unwrap();
    }

    // Request only the last 3.
    let summaries = session::load_session_summaries(dir.path(), learner_id, 3).await;
    assert_eq!(summaries.len(), 3);

    // Should be the 3 most recent (days 3, 4, 5 in chronological order).
    assert!(summaries[0].behavioral_observations.contains("Day 3"));
    assert!(summaries[1].behavioral_observations.contains("Day 4"));
    assert!(summaries[2].behavioral_observations.contains("Day 5"));
}

// ---------------------------------------------------------------------------
// Session listing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_list_sessions_empty_when_no_sessions() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();

    let sessions = session::list_sessions(dir.path(), learner_id).await;
    assert!(sessions.is_empty());
}

#[tokio::test]
async fn test_list_sessions_parses_metadata() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();
    let session = make_active_session(learner_id);

    let xp_by_skill = HashMap::new();
    let params = SessionMarkdownParams {
        narrative: None,
        badges_earned: &[],
        xp_by_skill: &xp_by_skill,
        difficulty_before: None,
        difficulty_after: None,
        shared_info: None,
    };
    session::write_session_markdown_file(dir.path(), learner_id, &session, "TestKid", &params)
        .await
        .unwrap();

    let sessions = session::list_sessions(dir.path(), learner_id).await;
    assert_eq!(sessions.len(), 1);

    let meta = &sessions[0];
    assert_eq!(meta.session_id, "session-2026-04-07-1530");
    assert_eq!(meta.date, "2026-04-07");
    assert_eq!(meta.total_assignments, 4);
    assert_eq!(meta.correct_assignments, 3);
    assert!((meta.accuracy - 0.75).abs() < 0.01);
}

// ---------------------------------------------------------------------------
// Progress update on session completion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_progress_updated_and_written() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();
    let session = make_active_session(learner_id);

    // Initialise and write a fresh progress record.
    let mut prog = progress::init_progress(learner_id);
    progress::write_progress(dir.path(), &prog).await.unwrap();

    let today = NaiveDate::from_ymd_opt(2026, 4, 7).unwrap();
    let xp_map = session::apply_session_to_progress(&mut prog, &session, today);

    // XP: sequential-logic: 2 correct × 3 × 10 + 1 incorrect × 3 × 5 = 75
    //     pattern-recognition: 1 correct × 2 × 10 = 20
    assert_eq!(*xp_map.get("sequential-logic").unwrap_or(&0), 75);
    assert_eq!(*xp_map.get("pattern-recognition").unwrap_or(&0), 20);

    // Total counters.
    assert_eq!(prog.total_sessions, 1);
    assert_eq!(prog.total_assignments, 4);

    // Streak — first session.
    assert_eq!(prog.streaks.current_days, 1);

    // Last practiced.
    assert_eq!(prog.skills["sequential-logic"].last_practiced, Some(today));

    // Write and read back.
    progress::write_progress(dir.path(), &prog).await.unwrap();
    let restored = progress::read_progress(dir.path(), learner_id)
        .await
        .unwrap();
    assert_eq!(restored.total_sessions, 1);
    assert_eq!(restored.total_assignments, 4);
}

#[tokio::test]
async fn test_abandoned_session_records_partial_data() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();

    // Session with only 1 assignment completed before abandonment.
    let mut session = make_active_session(learner_id);
    session.assignments = vec![make_session_assignment("sequential-logic", 3, true)];
    session.status = SessionStatus::Abandoned;

    let xp_by_skill = HashMap::new();
    let params = SessionMarkdownParams {
        narrative: None,
        badges_earned: &[],
        xp_by_skill: &xp_by_skill,
        difficulty_before: Some(3),
        difficulty_after: None,
        shared_info: None,
    };
    let session_id =
        session::write_session_markdown_file(dir.path(), learner_id, &session, "TestKid", &params)
            .await
            .unwrap();

    let path = session::session_markdown_path(dir.path(), learner_id, &session_id);
    let content = tokio::fs::read_to_string(&path).await.unwrap();

    // Partial data should still be present.
    assert!(content.contains("### Assignment 1:"));
    assert!(content.contains("- Correct: 1/1"));
    // Narrative placeholder for abandoned sessions.
    assert!(content.contains("unavailable"));
}
