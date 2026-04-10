/// Integration tests for learner progress file persistence and badge eligibility.
use chrono::NaiveDate;
use educational_companion::progress::{
    self, BadgeContext, EarnedBadge, LearnerProgress, Metacognition, MetacognitionTrend,
    SkillProgress, Streaks, WorkingMemorySignal, ZpdLevels,
};
use std::collections::HashMap;
use tempfile::TempDir;
use uuid::Uuid;

fn make_progress(learner_id: Uuid) -> LearnerProgress {
    let mut skills = HashMap::new();
    skills.insert(
        "pattern-recognition".to_string(),
        SkillProgress {
            level: 4,
            xp: 340,
            last_practiced: Some(NaiveDate::from_ymd_opt(2026, 4, 7).unwrap()),
            zpd: ZpdLevels {
                independent_level: 3,
                scaffolded_level: 5,
            },
            recent_accuracy: vec![1, 1, 0, 1, 1],
            working_memory_signal: WorkingMemorySignal::Stable,
            spaced_repetition: educational_companion::progress::SpacedRepetition {
                interval_days: 7,
                ease_factor: 2.5,
                next_review_date: NaiveDate::from_ymd_opt(2026, 4, 14).unwrap(),
                consecutive_correct: 4,
            },
        },
    );
    LearnerProgress {
        schema_version: 1,
        learner_id,
        skills,
        badges: vec![EarnedBadge {
            id: "first-puzzle".to_string(),
            name: "Puzzle Pioneer".to_string(),
            earned_date: NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
            category: "milestone".to_string(),
        }],
        streaks: Streaks {
            current_days: 5,
            longest_days: 12,
        },
        total_sessions: 28,
        total_time_minutes: 680,
        total_assignments: 140,
        metacognition: Metacognition {
            self_correction_rate: 0.3,
            hint_request_rate: 0.15,
            trend: MetacognitionTrend::Improving,
        },
        challenge_flags: HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Persistence tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_init_progress_valid_json() {
    let id = Uuid::new_v4();
    let progress = progress::init_progress(id);
    assert_eq!(progress.schema_version, 1);
    assert_eq!(progress.learner_id, id);
    assert!(progress.skills.is_empty());
    assert!(progress.badges.is_empty());
    assert_eq!(progress.total_sessions, 0);
    assert_eq!(progress.total_assignments, 0);

    // Must serialize to valid JSON with all required fields.
    let json = serde_json::to_string_pretty(&progress).expect("serialize init_progress");
    let val: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(val["schemaVersion"], 1);
    assert!(val["skills"].is_object());
    assert!(val["badges"].is_array());
}

#[tokio::test]
async fn test_write_and_read_round_trip() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    let progress = make_progress(id);

    progress::write_progress(tmp.path(), &progress)
        .await
        .unwrap();
    let restored = progress::read_progress(tmp.path(), id).await.unwrap();
    assert_eq!(progress, restored);
}

#[tokio::test]
async fn test_read_missing_returns_not_found() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();

    let result = progress::read_progress(tmp.path(), id).await;
    assert!(
        matches!(result, Err(progress::ProgressError::NotFound(got)) if got == id),
        "expected NotFound, got: {:?}",
        result
    );
}

#[tokio::test]
async fn test_read_wrong_schema_version_returns_error() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    let mut p = make_progress(id);
    p.schema_version = 2;

    progress::write_progress(tmp.path(), &p).await.unwrap();
    let result = progress::read_progress(tmp.path(), id).await;
    assert!(
        matches!(
            result,
            Err(progress::ProgressError::InvalidSchemaVersion {
                expected: 1,
                actual: 2
            })
        ),
        "expected InvalidSchemaVersion, got: {:?}",
        result
    );
}

#[tokio::test]
async fn test_no_gap_field_in_written_json() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    let progress = make_progress(id);

    progress::write_progress(tmp.path(), &progress)
        .await
        .unwrap();

    let path = tmp
        .path()
        .join("learners")
        .join(id.to_string())
        .join("progress.json");
    let json = tokio::fs::read_to_string(&path).await.unwrap();
    assert!(!json.contains("\"gap\""), "gap must not be stored in JSON");
}

#[tokio::test]
async fn test_write_enforces_ring_buffer() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    let mut progress = make_progress(id);

    // Stuff the ring buffer with 10 entries — write should trim to 5.
    let skill = progress
        .skills
        .entry("pattern-recognition".to_string())
        .or_default();
    skill.recent_accuracy = vec![1, 0, 1, 0, 1, 0, 1, 0, 1, 0];

    progress::write_progress(tmp.path(), &progress)
        .await
        .unwrap();
    let restored = progress::read_progress(tmp.path(), id).await.unwrap();

    let ra = &restored.skills["pattern-recognition"].recent_accuracy;
    assert_eq!(ra.len(), 5, "ring buffer must be truncated to 5 on write");
    // The 5 most recent entries are the last 5 of the original.
    assert_eq!(ra, &[0, 1, 0, 1, 0]);
}

#[tokio::test]
async fn test_write_creates_parent_dir() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    let progress = progress::init_progress(id);

    // The learner directory does not exist yet.
    progress::write_progress(tmp.path(), &progress)
        .await
        .unwrap();

    let path = tmp
        .path()
        .join("learners")
        .join(id.to_string())
        .join("progress.json");
    assert!(path.exists(), "progress.json should have been created");
}

// ---------------------------------------------------------------------------
// Badge eligibility tests
// ---------------------------------------------------------------------------

fn skill_tree_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("curriculum")
        .join("skill-tree.json")
}

#[tokio::test]
async fn test_check_new_badges_none_when_all_earned() {
    let id = Uuid::new_v4();
    // A learner who already has "first-puzzle" and meets the condition.
    let mut progress = progress::init_progress(id);
    progress.total_assignments = 1;
    progress.badges.push(EarnedBadge {
        id: "first-puzzle".to_string(),
        name: "Puzzle Pioneer".to_string(),
        earned_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        category: "milestone".to_string(),
    });

    let new_badges =
        progress::check_new_badges(&progress, &skill_tree_path(), &BadgeContext::default())
            .await
            .unwrap();

    assert!(
        !new_badges.iter().any(|(b, _)| b.id == "first-puzzle"),
        "first-puzzle is already earned; must not be returned"
    );
}

#[tokio::test]
async fn test_check_new_badges_first_puzzle() {
    let id = Uuid::new_v4();
    let mut progress = progress::init_progress(id);
    progress.total_assignments = 1; // condition: totalAssignments >= 1

    let new_badges =
        progress::check_new_badges(&progress, &skill_tree_path(), &BadgeContext::default())
            .await
            .unwrap();

    assert!(
        new_badges.iter().any(|(b, _)| b.id == "first-puzzle"),
        "first-puzzle should be earned when totalAssignments >= 1"
    );
}

#[tokio::test]
async fn test_check_new_badges_ten_sessions() {
    let id = Uuid::new_v4();
    let mut progress = progress::init_progress(id);
    progress.total_sessions = 10; // condition: totalSessions >= 10

    let new_badges =
        progress::check_new_badges(&progress, &skill_tree_path(), &BadgeContext::default())
            .await
            .unwrap();

    assert!(
        new_badges.iter().any(|(b, _)| b.id == "ten-sessions"),
        "ten-sessions badge should be earned when totalSessions >= 10"
    );
}

#[tokio::test]
async fn test_check_new_badges_streak() {
    let id = Uuid::new_v4();
    let mut progress = progress::init_progress(id);
    progress.streaks.current_days = 7;

    let new_badges =
        progress::check_new_badges(&progress, &skill_tree_path(), &BadgeContext::default())
            .await
            .unwrap();

    let ids: Vec<&str> = new_badges.iter().map(|(b, _)| b.id.as_str()).collect();
    assert!(
        ids.contains(&"streak-3"),
        "streak-3 should be earned at 7 days"
    );
    assert!(
        ids.contains(&"streak-7"),
        "streak-7 should be earned at 7 days"
    );
    assert!(
        !ids.contains(&"streak-30"),
        "streak-30 should not be earned at 7 days"
    );
}

#[tokio::test]
async fn test_check_new_badges_any_skill_level() {
    let id = Uuid::new_v4();
    let mut progress = progress::init_progress(id);
    let mut skill = SkillProgress::default();
    skill.level = 5;
    progress
        .skills
        .insert("pattern-recognition".to_string(), skill);

    let new_badges =
        progress::check_new_badges(&progress, &skill_tree_path(), &BadgeContext::default())
            .await
            .unwrap();

    let ids: Vec<&str> = new_badges.iter().map(|(b, _)| b.id.as_str()).collect();
    assert!(
        ids.contains(&"skill-level-3"),
        "skill-level-3 should be earned at level 5"
    );
    assert!(
        ids.contains(&"skill-level-5"),
        "skill-level-5 should be earned at level 5"
    );
    assert!(
        !ids.contains(&"skill-level-7"),
        "skill-level-7 should not be earned at level 5"
    );
}

#[tokio::test]
async fn test_check_new_badges_explorer() {
    let id = Uuid::new_v4();
    let mut progress = progress::init_progress(id);
    progress
        .skills
        .insert("pattern-recognition".to_string(), SkillProgress::default());
    progress
        .skills
        .insert("sequential-logic".to_string(), SkillProgress::default());

    let new_badges =
        progress::check_new_badges(&progress, &skill_tree_path(), &BadgeContext::default())
            .await
            .unwrap();

    let ids: Vec<&str> = new_badges.iter().map(|(b, _)| b.id.as_str()).collect();
    assert!(
        ids.contains(&"explore-pattern"),
        "explore-pattern badge should fire"
    );
    assert!(
        ids.contains(&"explore-sequential"),
        "explore-sequential badge should fire"
    );
    assert!(
        !ids.contains(&"explore-spatial"),
        "explore-spatial should not fire — skill not in progress"
    );
}

#[tokio::test]
async fn test_check_new_badges_challenge_flags() {
    let id = Uuid::new_v4();
    let mut progress = progress::init_progress(id);
    progress
        .challenge_flags
        .insert("timedChallenge80".to_string(), true);
    progress
        .challenge_flags
        .insert("teachBackSuccess".to_string(), true);

    let new_badges =
        progress::check_new_badges(&progress, &skill_tree_path(), &BadgeContext::default())
            .await
            .unwrap();

    let ids: Vec<&str> = new_badges.iter().map(|(b, _)| b.id.as_str()).collect();
    assert!(
        ids.contains(&"speed-round"),
        "speed-round should be earned when timedChallenge80 is set"
    );
    assert!(
        ids.contains(&"teacher"),
        "teacher should be earned when teachBackSuccess is set"
    );
    assert!(
        !ids.contains(&"boss-battle"),
        "boss-battle should not fire — bossComplete not set"
    );
}

#[tokio::test]
async fn test_check_new_badges_perfect_session() {
    let id = Uuid::new_v4();
    let progress = progress::init_progress(id);
    let ctx = BadgeContext {
        session_accuracy: Some(1.0),
    };

    let new_badges = progress::check_new_badges(&progress, &skill_tree_path(), &ctx)
        .await
        .unwrap();

    assert!(
        new_badges.iter().any(|(b, _)| b.id == "perfect-session"),
        "perfect-session should be earned when sessionAccuracy == 1.0"
    );
}

#[tokio::test]
async fn test_check_new_badges_categories_are_correct() {
    let id = Uuid::new_v4();
    let mut progress = progress::init_progress(id);
    progress.total_assignments = 5;
    progress.total_sessions = 10;
    progress.streaks.current_days = 3;

    let new_badges =
        progress::check_new_badges(&progress, &skill_tree_path(), &BadgeContext::default())
            .await
            .unwrap();

    for (badge, cat) in &new_badges {
        match badge.id.as_str() {
            "first-puzzle" | "ten-sessions" => {
                assert_eq!(
                    cat, "milestone",
                    "{} should be in milestone category",
                    badge.id
                )
            }
            "streak-3" => assert_eq!(cat, "streak", "streak-3 should be in streak category"),
            _ => {}
        }
    }
}
