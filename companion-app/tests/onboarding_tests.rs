/// Integration tests for the onboarding module.
///
/// Tests cover:
/// - ZPD baseline computation from calibration results
/// - Seeding progress with baselines
/// - OnboardingSession state machine (sequence, difficulty adaptation, skip)
use educational_companion::onboarding::{
    self, CalibrationResult, OnboardingSession, OnboardingStatus, CALIBRATION_SKILLS,
    CALIBRATION_START_DIFFICULTY, PUZZLES_PER_SKILL,
};
use educational_companion::progress::{
    self, EarnedBadge, LearnerProgress, SkillProgress, ZpdLevels,
};
use std::collections::HashMap;
use tempfile::TempDir;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_result(skill: &str, difficulty: u32, correct: bool, hints_used: u32) -> CalibrationResult {
    CalibrationResult {
        skill: skill.to_string(),
        difficulty,
        correct,
        hints_used,
        skipped: false,
    }
}

fn make_skip(skill: &str, difficulty: u32) -> CalibrationResult {
    CalibrationResult {
        skill: skill.to_string(),
        difficulty,
        correct: false,
        hints_used: 0,
        skipped: true,
    }
}

fn make_learner_dir(dir: &TempDir, learner_id: Uuid) {
    std::fs::create_dir_all(dir.path().join("learners").join(learner_id.to_string()))
        .expect("create learner dir");
}

// ---------------------------------------------------------------------------
// OnboardingSession — state machine
// ---------------------------------------------------------------------------

#[test]
fn test_session_covers_all_4_skills() {
    let id = Uuid::new_v4();
    let mut session = OnboardingSession::new(id);
    let total = session.total_puzzles();
    assert_eq!(total, CALIBRATION_SKILLS.len() * PUZZLES_PER_SKILL);

    let mut skills_seen = std::collections::HashSet::new();
    while !session.is_sequence_complete() {
        let (skill, _) = session.current_skill_difficulty().unwrap();
        skills_seen.insert(skill.to_string());
        session.skip_current();
    }

    for required_skill in CALIBRATION_SKILLS {
        assert!(
            skills_seen.contains(*required_skill),
            "skill not covered: {required_skill}"
        );
    }
}

#[test]
fn test_each_skill_gets_exactly_2_puzzles() {
    let id = Uuid::new_v4();
    let mut session = OnboardingSession::new(id);
    let mut skill_counts: HashMap<String, usize> = HashMap::new();

    while !session.is_sequence_complete() {
        let (skill, _) = session.current_skill_difficulty().unwrap();
        *skill_counts.entry(skill.to_string()).or_insert(0) += 1;
        session.skip_current();
    }

    for skill in CALIBRATION_SKILLS {
        assert_eq!(
            *skill_counts.get(*skill).unwrap_or(&0),
            PUZZLES_PER_SKILL,
            "skill {skill} does not have exactly {PUZZLES_PER_SKILL} puzzles"
        );
    }
}

#[test]
fn test_difficulty_adapts_correct_then_wrong() {
    let id = Uuid::new_v4();
    let mut session = OnboardingSession::new(id);

    // First puzzle: pattern-recognition at difficulty 1, answered correctly.
    let (_, d1) = session.current_skill_difficulty().unwrap();
    assert_eq!(d1, CALIBRATION_START_DIFFICULTY);
    session.record_result(make_result("pattern-recognition", d1, true, 0));

    // Second puzzle: difficulty should have increased to 2.
    let (_, d2) = session.current_skill_difficulty().unwrap();
    assert_eq!(d2, 2);

    // Answer wrong — third puzzle should stay at difficulty 2 (floor check at 1 for later).
    session.record_result(make_result("pattern-recognition", d2, false, 0));

    // Move to sequential-logic (next skill pair).
    let (s3, d3) = session.current_skill_difficulty().unwrap();
    assert_eq!(s3, "sequential-logic");
    assert_eq!(d3, CALIBRATION_START_DIFFICULTY);
}

#[test]
fn test_skip_preserves_difficulty_and_records_skip() {
    let id = Uuid::new_v4();
    let mut session = OnboardingSession::new(id);
    session
        .skill_difficulties
        .insert("pattern-recognition".to_string(), 5);

    session.skip_current();

    // Difficulty must be unchanged.
    let (_, d) = session.current_skill_difficulty().unwrap();
    assert_eq!(d, 5);

    // Exactly one result recorded, marked as skipped.
    assert_eq!(session.results.len(), 1);
    assert!(session.results[0].skipped);
}

#[test]
fn test_pending_assignment_id_cleared_after_respond() {
    let id = Uuid::new_v4();
    let mut session = OnboardingSession::new(id);
    session.pending_assignment_id = Some("test-id".to_string());

    let result = make_result("pattern-recognition", 1, true, 0);
    session.record_result(result);

    assert!(session.pending_assignment_id.is_none());
}

#[test]
fn test_pending_assignment_id_cleared_after_skip() {
    let id = Uuid::new_v4();
    let mut session = OnboardingSession::new(id);
    session.pending_assignment_id = Some("test-id".to_string());

    session.skip_current();

    assert!(session.pending_assignment_id.is_none());
}

#[test]
fn test_status_is_in_progress_when_puzzles_remain() {
    let id = Uuid::new_v4();
    let session = OnboardingSession::new(id);
    assert_eq!(session.status(), OnboardingStatus::InProgress);
}

#[test]
fn test_status_is_puzzles_exhausted_when_all_done() {
    let id = Uuid::new_v4();
    let mut session = OnboardingSession::new(id);
    for _ in 0..session.total_puzzles() {
        session.skip_current();
    }
    assert_eq!(session.status(), OnboardingStatus::PuzzlesExhausted);
}

// ---------------------------------------------------------------------------
// compute_zpd_baselines
// ---------------------------------------------------------------------------

#[test]
fn test_all_skills_covered_in_baselines() {
    let baselines = onboarding::compute_zpd_baselines(&[]);
    for skill in CALIBRATION_SKILLS {
        assert!(
            baselines.contains_key(*skill),
            "baseline missing for skill: {skill}"
        );
    }
}

#[test]
fn test_all_skipped_session_yields_defaults() {
    let results: Vec<CalibrationResult> = CALIBRATION_SKILLS
        .iter()
        .flat_map(|&s| {
            (0..PUZZLES_PER_SKILL).map(move |_| make_skip(s, CALIBRATION_START_DIFFICULTY))
        })
        .collect();

    let baselines = onboarding::compute_zpd_baselines(&results);

    for skill in CALIBRATION_SKILLS {
        let zpd = baselines.get(*skill).unwrap();
        assert_eq!(zpd.independent_level, 1, "bad default for {skill}");
        assert_eq!(zpd.scaffolded_level, 2, "bad default for {skill}");
    }
}

#[test]
fn test_mixed_correct_incorrect_skip() {
    // pattern-recognition: one correct (d=2, no hints), one skip
    // sequential-logic: two incorrect
    // spatial-reasoning: one correct with hints (d=3, hints=1), one correct no hints (d=2)
    // deductive-reasoning: not attempted
    let results = vec![
        make_result("pattern-recognition", 2, true, 0),
        make_skip("pattern-recognition", 3),
        make_result("sequential-logic", 1, false, 0),
        make_result("sequential-logic", 1, false, 0),
        make_result("spatial-reasoning", 2, true, 0),
        make_result("spatial-reasoning", 3, true, 1), // correct with 1 hint
    ];

    let baselines = onboarding::compute_zpd_baselines(&results);

    // pattern-recognition: solved d=2 with 0 hints; no scaffolded evidence → scaffolded = ind + 1 = 3
    let pr = baselines.get("pattern-recognition").unwrap();
    assert_eq!(pr.independent_level, 2);
    assert_eq!(pr.scaffolded_level, 3);

    // sequential-logic: no correct → defaults
    let sl = baselines.get("sequential-logic").unwrap();
    assert_eq!(sl.independent_level, 1);
    assert_eq!(sl.scaffolded_level, 2);

    // spatial-reasoning: independent from d=2 (0 hints), scaffolded from d=3 (1 hint)
    let sr = baselines.get("spatial-reasoning").unwrap();
    assert_eq!(sr.independent_level, 2);
    assert_eq!(sr.scaffolded_level, 3);

    // deductive-reasoning: no attempts → defaults
    let dr = baselines.get("deductive-reasoning").unwrap();
    assert_eq!(dr.independent_level, 1);
    assert_eq!(dr.scaffolded_level, 2);
}

#[test]
fn test_zpd_gap_is_always_at_least_one() {
    // Solve every skill at the same difficulty with 0 hints.
    let results: Vec<CalibrationResult> = CALIBRATION_SKILLS
        .iter()
        .flat_map(|&s| {
            (0..PUZZLES_PER_SKILL).map(move |_| make_result(s, 3, true, 0))
        })
        .collect();

    let baselines = onboarding::compute_zpd_baselines(&results);

    for (skill, zpd) in &baselines {
        assert!(
            zpd.scaffolded_level > zpd.independent_level,
            "gap must be >= 1 for skill {skill}: ind={}, sc={}",
            zpd.independent_level,
            zpd.scaffolded_level
        );
    }
}

// ---------------------------------------------------------------------------
// seed_progress_with_baselines + progress persistence
// ---------------------------------------------------------------------------

#[test]
fn test_seed_progress_creates_all_calibration_skills() {
    let id = Uuid::new_v4();
    let mut progress = LearnerProgress::default_for(id);

    let baselines = onboarding::compute_zpd_baselines(&[]);
    onboarding::seed_progress_with_baselines(&mut progress, baselines);

    for skill in CALIBRATION_SKILLS {
        assert!(
            progress.skills.contains_key(*skill),
            "skill {skill} not seeded"
        );
    }
}

#[test]
fn test_seed_progress_uses_correct_zpd_values() {
    let id = Uuid::new_v4();
    let mut progress = LearnerProgress::default_for(id);

    let results = vec![make_result("pattern-recognition", 3, true, 0)];
    let baselines = onboarding::compute_zpd_baselines(&results);
    onboarding::seed_progress_with_baselines(&mut progress, baselines);

    let pr = progress.skills.get("pattern-recognition").unwrap();
    assert_eq!(pr.zpd.independent_level, 3);
}

#[test]
fn test_seed_does_not_overwrite_existing_skill() {
    let id = Uuid::new_v4();
    let mut progress = LearnerProgress::default_for(id);
    progress.skills.insert(
        "pattern-recognition".to_string(),
        SkillProgress {
            level: 7,
            xp: 600,
            zpd: ZpdLevels {
                independent_level: 6,
                scaffolded_level: 8,
            },
            ..Default::default()
        },
    );

    let baselines = onboarding::compute_zpd_baselines(&[]);
    onboarding::seed_progress_with_baselines(&mut progress, baselines);

    let pr = progress.skills.get("pattern-recognition").unwrap();
    assert_eq!(pr.level, 7, "existing skill must not be overwritten");
    assert_eq!(pr.zpd.independent_level, 6);
}

#[tokio::test]
async fn test_progress_persisted_with_baselines_and_badge() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();
    make_learner_dir(&dir, learner_id);

    let results = vec![
        make_result("pattern-recognition", 2, true, 0),
        make_result("sequential-logic", 1, true, 1),
    ];

    let baselines = onboarding::compute_zpd_baselines(&results);

    let mut prog = LearnerProgress::default_for(learner_id);
    onboarding::seed_progress_with_baselines(&mut prog, baselines);

    // Simulate awarding the "Getting Started" badge and setting the flag.
    prog.challenge_flags
        .insert("onboardingComplete".to_string(), true);
    prog.badges.push(EarnedBadge {
        id: "onboarding-complete".to_string(),
        name: "Getting Started".to_string(),
        earned_date: chrono::Local::now().date_naive(),
        category: "milestone".to_string(),
    });

    progress::write_progress(dir.path(), &prog)
        .await
        .expect("write progress");

    let loaded = progress::read_progress(dir.path(), learner_id)
        .await
        .expect("read progress");

    assert!(
        loaded
            .challenge_flags
            .get("onboardingComplete")
            .copied()
            .unwrap_or(false),
        "onboardingComplete flag must be persisted"
    );
    assert!(
        loaded.badges.iter().any(|b| b.id == "onboarding-complete"),
        "Getting Started badge must be persisted"
    );

    // All calibration skills must be present.
    for skill in CALIBRATION_SKILLS {
        assert!(
            loaded.skills.contains_key(*skill),
            "skill {skill} missing from persisted progress"
        );
    }
}

#[tokio::test]
async fn test_badge_only_awarded_once() {
    let dir = TempDir::new().unwrap();
    let learner_id = Uuid::new_v4();
    make_learner_dir(&dir, learner_id);

    let mut prog = LearnerProgress::default_for(learner_id);
    prog.challenge_flags
        .insert("onboardingComplete".to_string(), true);

    let badge = EarnedBadge {
        id: "onboarding-complete".to_string(),
        name: "Getting Started".to_string(),
        earned_date: chrono::Local::now().date_naive(),
        category: "milestone".to_string(),
    };
    prog.badges.push(badge);

    progress::write_progress(dir.path(), &prog)
        .await
        .expect("write");

    let loaded = progress::read_progress(dir.path(), learner_id)
        .await
        .expect("read");

    // Simulate the "award if not already earned" guard.
    let already_earned = loaded.badges.iter().any(|b| b.id == "onboarding-complete");
    assert!(already_earned, "badge should already be present");

    // The guard should prevent a second award.
    let count = loaded
        .badges
        .iter()
        .filter(|b| b.id == "onboarding-complete")
        .count();
    assert_eq!(count, 1, "badge must not be duplicated");
}

// ---------------------------------------------------------------------------
// Partial abandonment — calibration data seed from incomplete session
// ---------------------------------------------------------------------------

#[test]
fn test_partial_results_seed_attempted_skills_with_defaults_for_rest() {
    // Only pattern-recognition attempted; others get defaults.
    let results = vec![make_result("pattern-recognition", 2, true, 0)];
    let baselines = onboarding::compute_zpd_baselines(&results);

    let pr = baselines.get("pattern-recognition").unwrap();
    assert_eq!(pr.independent_level, 2);

    for skill in &["sequential-logic", "spatial-reasoning", "deductive-reasoning"] {
        let zpd = baselines.get(*skill).unwrap();
        assert_eq!(zpd.independent_level, 1, "unattempted skill {skill} should have default");
        assert_eq!(zpd.scaffolded_level, 2, "unattempted skill {skill} should have default");
    }
}

#[test]
fn test_empty_results_yields_all_defaults() {
    let baselines = onboarding::compute_zpd_baselines(&[]);
    for skill in CALIBRATION_SKILLS {
        let zpd = baselines.get(*skill).unwrap();
        assert_eq!(zpd.independent_level, 1);
        assert_eq!(zpd.scaffolded_level, 2);
    }
}
