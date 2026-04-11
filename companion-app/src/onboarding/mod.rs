// Onboarding and calibration session management.
// Implements the 5-step onboarding flow from CLAUDE.md:
//   1. Choose Your Name — handled by profile creation.
//   2. Interest Discovery — update profile interests (handled in start handler).
//   3. Calibration Puzzles — 8+ short puzzles, 2 per skill category.
//   4. Baseline Seeding — compute ZPD from calibration results.
//   5. Welcome Badge — award "Getting Started" on completion.

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::progress::tracker::{SkillProgress, SpacedRepetition, WorkingMemorySignal, ZpdLevels};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The 4 calibration skill categories covered during onboarding.
///
/// 2 puzzles are generated per skill, giving 8 calibration puzzles total.
pub const CALIBRATION_SKILLS: &[&str] = &[
    "pattern-recognition",
    "sequential-logic",
    "spatial-reasoning",
    "deductive-reasoning",
];

/// Number of calibration puzzles generated per skill category.
pub const PUZZLES_PER_SKILL: usize = 2;

/// Starting difficulty for all calibration puzzles.
pub const CALIBRATION_START_DIFFICULTY: u32 = 1;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// The result of a single calibration puzzle attempt.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationResult {
    /// Skill being calibrated.
    pub skill: String,
    /// Difficulty level of this puzzle.
    pub difficulty: u32,
    /// Whether the child answered correctly.
    pub correct: bool,
    /// Number of hints the child used before answering.
    pub hints_used: u32,
    /// Whether the child opted to skip this puzzle.
    pub skipped: bool,
}

/// Lifecycle status of an onboarding session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OnboardingStatus {
    /// Onboarding is running and calibration is in progress.
    InProgress,
    /// All puzzles have been presented but `complete` has not been called yet.
    PuzzlesExhausted,
    /// Onboarding was completed and ZPD baselines have been seeded.
    Complete,
}

/// In-memory state for an active onboarding session.
///
/// Stored in [`AppState::onboarding_sessions`], keyed by learner UUID.
/// Removed when `complete_onboarding` is called.
#[derive(Clone, Debug)]
pub struct OnboardingSession {
    /// The learner this session belongs to.
    pub learner_id: Uuid,
    /// Wall-clock start time.
    pub started_at: DateTime<Local>,
    /// Current per-skill difficulty (adapts as puzzles are answered).
    pub skill_difficulties: HashMap<String, u32>,
    /// All calibration results captured so far.
    pub results: Vec<CalibrationResult>,
    /// Position in the fixed calibration sequence.
    ///
    /// Ranges from 0 to `CALIBRATION_SKILLS.len() * PUZZLES_PER_SKILL`.
    pub current_index: usize,
    /// Assignment ID currently pending a response (set by `generate`, cleared on respond/skip).
    pub pending_assignment_id: Option<String>,
}

impl OnboardingSession {
    /// Create a new onboarding session for a learner.
    pub fn new(learner_id: Uuid) -> Self {
        let mut skill_difficulties = HashMap::new();
        for skill in CALIBRATION_SKILLS {
            skill_difficulties.insert(skill.to_string(), CALIBRATION_START_DIFFICULTY);
        }
        Self {
            learner_id,
            started_at: chrono::Local::now(),
            skill_difficulties,
            results: Vec::new(),
            current_index: 0,
            pending_assignment_id: None,
        }
    }

    /// Total number of puzzles in the calibration sequence.
    pub fn total_puzzles(&self) -> usize {
        CALIBRATION_SKILLS.len() * PUZZLES_PER_SKILL
    }

    /// Whether the calibration sequence is fully exhausted.
    pub fn is_sequence_complete(&self) -> bool {
        self.current_index >= self.total_puzzles()
    }

    /// Returns `(skill_id, difficulty)` for the current puzzle, or `None` if
    /// the sequence is exhausted.
    pub fn current_skill_difficulty(&self) -> Option<(&str, u32)> {
        if self.is_sequence_complete() {
            return None;
        }
        let skill_index = self.current_index / PUZZLES_PER_SKILL;
        let skill = CALIBRATION_SKILLS[skill_index];
        let difficulty = *self
            .skill_difficulties
            .get(skill)
            .unwrap_or(&CALIBRATION_START_DIFFICULTY);
        Some((skill, difficulty))
    }

    /// Record a calibration result and adapt the skill difficulty for the next puzzle.
    ///
    /// - Correct → difficulty + 1 (cap at 10)
    /// - Wrong → difficulty − 1 (floor at 1)
    /// - Skipped → difficulty unchanged
    ///
    /// Advances `current_index` and clears `pending_assignment_id`.
    pub fn record_result(&mut self, result: CalibrationResult) {
        let skill = result.skill.clone();
        let correct = result.correct;
        let skipped = result.skipped;

        self.results.push(result);

        if !skipped {
            let difficulty = self
                .skill_difficulties
                .entry(skill)
                .or_insert(CALIBRATION_START_DIFFICULTY);
            if correct {
                *difficulty = (*difficulty + 1).min(10);
            } else {
                *difficulty = difficulty.saturating_sub(1).max(1);
            }
        }

        self.current_index += 1;
        self.pending_assignment_id = None;
    }

    /// Skip the current puzzle without altering the skill difficulty.
    ///
    /// Records a skipped `CalibrationResult` and advances `current_index`.
    pub fn skip_current(&mut self) {
        if let Some((skill, difficulty)) = self.current_skill_difficulty() {
            let result = CalibrationResult {
                skill: skill.to_string(),
                difficulty,
                correct: false,
                hints_used: 0,
                skipped: true,
            };
            self.results.push(result);
        }
        self.current_index += 1;
        self.pending_assignment_id = None;
    }

    /// Current status of this onboarding session.
    pub fn status(&self) -> OnboardingStatus {
        if self.is_sequence_complete() {
            OnboardingStatus::PuzzlesExhausted
        } else {
            OnboardingStatus::InProgress
        }
    }
}

// ---------------------------------------------------------------------------
// ZPD baseline computation
// ---------------------------------------------------------------------------

/// Compute ZPD baselines for each calibration skill from recorded results.
///
/// - `independentLevel`: highest difficulty answered **correctly with 0 hints**.
/// - `scaffoldedLevel`: highest difficulty answered **correctly** (any hints), at least
///   `independentLevel + 1`.
/// - Skills with no correct answers: defaults (`independentLevel: 1, scaffoldedLevel: 2`).
/// - Skills not attempted at all: defaults.
///
/// Always returns an entry for every skill in [`CALIBRATION_SKILLS`].
pub fn compute_zpd_baselines(results: &[CalibrationResult]) -> HashMap<String, ZpdLevels> {
    let mut baselines: HashMap<String, ZpdLevels> = CALIBRATION_SKILLS
        .iter()
        .map(|&s| (s.to_string(), ZpdLevels::default()))
        .collect();

    let mut best_independent: HashMap<&str, u32> = HashMap::new();
    let mut best_scaffolded: HashMap<&str, u32> = HashMap::new();

    for r in results {
        if r.skipped || !r.correct {
            continue;
        }
        // Highest correct difficulty (with or without hints) → scaffoldedLevel candidate.
        let sc = best_scaffolded.entry(r.skill.as_str()).or_insert(0);
        if r.difficulty > *sc {
            *sc = r.difficulty;
        }
        // Highest correct difficulty with 0 hints → independentLevel candidate.
        if r.hints_used == 0 {
            let ind = best_independent.entry(r.skill.as_str()).or_insert(0);
            if r.difficulty > *ind {
                *ind = r.difficulty;
            }
        }
    }

    for (skill, zpd) in baselines.iter_mut() {
        let ind = best_independent.get(skill.as_str()).copied().unwrap_or(0);
        let sc = best_scaffolded.get(skill.as_str()).copied().unwrap_or(0);

        if ind > 0 {
            zpd.independent_level = ind;
        }
        if sc > 0 {
            // scaffoldedLevel must be at least independentLevel + 1 to preserve a ZPD gap.
            zpd.scaffolded_level = sc.max(zpd.independent_level + 1);
        }
    }

    baselines
}

/// Seed a learner's progress with ZPD baselines from calibration.
///
/// Inserts a default [`SkillProgress`] for each calibration skill that does not
/// already exist in `progress.skills`. Existing entries are left untouched.
pub fn seed_progress_with_baselines(
    progress: &mut crate::progress::tracker::LearnerProgress,
    baselines: HashMap<String, ZpdLevels>,
) {
    for (skill, zpd) in baselines {
        progress.skills.entry(skill).or_insert_with(|| SkillProgress {
            level: 0,
            xp: 0,
            last_practiced: None,
            zpd,
            recent_accuracy: Vec::new(),
            working_memory_signal: WorkingMemorySignal::Stable,
            spaced_repetition: SpacedRepetition::default(),
        });
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(
        skill: &str,
        difficulty: u32,
        correct: bool,
        hints_used: u32,
    ) -> CalibrationResult {
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

    // --- OnboardingSession ---

    #[test]
    fn test_new_session_has_correct_initial_state() {
        let id = Uuid::new_v4();
        let session = OnboardingSession::new(id);
        assert_eq!(session.learner_id, id);
        assert_eq!(session.current_index, 0);
        assert!(session.results.is_empty());
        assert!(!session.is_sequence_complete());
        assert_eq!(session.total_puzzles(), 8);
        assert_eq!(session.status(), OnboardingStatus::InProgress);
    }

    #[test]
    fn test_current_skill_difficulty_follows_sequence() {
        let id = Uuid::new_v4();
        let session = OnboardingSession::new(id);

        let (skill, diff) = session.current_skill_difficulty().unwrap();
        assert_eq!(skill, "pattern-recognition");
        assert_eq!(diff, CALIBRATION_START_DIFFICULTY);
    }

    #[test]
    fn test_record_correct_increases_difficulty() {
        let id = Uuid::new_v4();
        let mut session = OnboardingSession::new(id);

        let (skill, _) = session.current_skill_difficulty().unwrap();
        let result = make_result(skill, 1, true, 0);
        session.record_result(result);

        let (skill2, diff2) = session.current_skill_difficulty().unwrap();
        assert_eq!(skill2, "pattern-recognition");
        assert_eq!(diff2, 2);
    }

    #[test]
    fn test_record_wrong_keeps_difficulty_at_floor() {
        let id = Uuid::new_v4();
        let mut session = OnboardingSession::new(id);

        let (skill, _) = session.current_skill_difficulty().unwrap();
        let result = make_result(skill, 1, false, 0);
        session.record_result(result);

        let (_, diff) = session.current_skill_difficulty().unwrap();
        assert_eq!(diff, 1);
    }

    #[test]
    fn test_skip_does_not_change_difficulty() {
        let id = Uuid::new_v4();
        let mut session = OnboardingSession::new(id);
        session
            .skill_difficulties
            .insert("pattern-recognition".to_string(), 3);

        session.skip_current();

        let (_, diff) = session.current_skill_difficulty().unwrap();
        assert_eq!(diff, 3);
    }

    #[test]
    fn test_sequence_exhausted_after_all_puzzles() {
        let id = Uuid::new_v4();
        let mut session = OnboardingSession::new(id);

        for _ in 0..session.total_puzzles() {
            session.skip_current();
        }

        assert!(session.is_sequence_complete());
        assert!(session.current_skill_difficulty().is_none());
        assert_eq!(session.status(), OnboardingStatus::PuzzlesExhausted);
    }

    #[test]
    fn test_skill_transitions_after_puzzles_per_skill() {
        let id = Uuid::new_v4();
        let mut session = OnboardingSession::new(id);

        // First two puzzles: pattern-recognition
        let (s1, _) = session.current_skill_difficulty().unwrap();
        assert_eq!(s1, "pattern-recognition");
        session.skip_current();

        let (s2, _) = session.current_skill_difficulty().unwrap();
        assert_eq!(s2, "pattern-recognition");
        session.skip_current();

        // Third puzzle: sequential-logic
        let (s3, _) = session.current_skill_difficulty().unwrap();
        assert_eq!(s3, "sequential-logic");
    }

    // --- compute_zpd_baselines ---

    #[test]
    fn test_all_defaults_when_no_results() {
        let baselines = compute_zpd_baselines(&[]);
        for skill in CALIBRATION_SKILLS {
            let zpd = baselines.get(*skill).expect("baseline for all skills");
            assert_eq!(zpd.independent_level, 1);
            assert_eq!(zpd.scaffolded_level, 2);
        }
    }

    #[test]
    fn test_defaults_for_skills_not_attempted() {
        let results = vec![make_result("pattern-recognition", 3, true, 0)];
        let baselines = compute_zpd_baselines(&results);

        let zpd = baselines.get("sequential-logic").unwrap();
        assert_eq!(zpd.independent_level, 1);
        assert_eq!(zpd.scaffolded_level, 2);
    }

    #[test]
    fn test_independent_level_from_correct_no_hints() {
        let results = vec![
            make_result("pattern-recognition", 2, true, 0),
            make_result("pattern-recognition", 3, true, 0),
        ];
        let baselines = compute_zpd_baselines(&results);
        let zpd = baselines.get("pattern-recognition").unwrap();
        assert_eq!(zpd.independent_level, 3);
    }

    #[test]
    fn test_scaffolded_level_includes_hints() {
        let results = vec![
            make_result("sequential-logic", 2, true, 0),
            make_result("sequential-logic", 4, true, 2), // solved with hints
        ];
        let baselines = compute_zpd_baselines(&results);
        let zpd = baselines.get("sequential-logic").unwrap();
        assert_eq!(zpd.independent_level, 2);
        assert_eq!(zpd.scaffolded_level, 4);
    }

    #[test]
    fn test_scaffolded_level_at_least_independent_plus_one() {
        // Both solved at the same difficulty with no hints.
        let results = vec![
            make_result("deductive-reasoning", 3, true, 0),
            make_result("deductive-reasoning", 3, true, 0),
        ];
        let baselines = compute_zpd_baselines(&results);
        let zpd = baselines.get("deductive-reasoning").unwrap();
        assert_eq!(zpd.independent_level, 3);
        assert!(zpd.scaffolded_level >= zpd.independent_level + 1);
    }

    #[test]
    fn test_skipped_results_not_counted() {
        let results = vec![
            make_skip("pattern-recognition", 3),
            make_skip("pattern-recognition", 4),
        ];
        let baselines = compute_zpd_baselines(&results);
        let zpd = baselines.get("pattern-recognition").unwrap();
        assert_eq!(zpd.independent_level, 1);
        assert_eq!(zpd.scaffolded_level, 2);
    }

    #[test]
    fn test_incorrect_results_not_counted() {
        let results = vec![make_result("spatial-reasoning", 5, false, 0)];
        let baselines = compute_zpd_baselines(&results);
        let zpd = baselines.get("spatial-reasoning").unwrap();
        assert_eq!(zpd.independent_level, 1);
        assert_eq!(zpd.scaffolded_level, 2);
    }

    // --- seed_progress_with_baselines ---

    #[test]
    fn test_seed_creates_skill_entries() {
        use crate::progress::tracker::LearnerProgress;

        let id = Uuid::new_v4();
        let mut progress = LearnerProgress::default_for(id);
        assert!(progress.skills.is_empty());

        let baselines = compute_zpd_baselines(&[]);
        seed_progress_with_baselines(&mut progress, baselines);

        for skill in CALIBRATION_SKILLS {
            assert!(progress.skills.contains_key(*skill));
        }
    }

    #[test]
    fn test_seed_does_not_overwrite_existing_skills() {
        use crate::progress::tracker::LearnerProgress;

        let id = Uuid::new_v4();
        let mut progress = LearnerProgress::default_for(id);
        progress.skills.insert(
            "pattern-recognition".to_string(),
            SkillProgress {
                level: 5,
                xp: 400,
                zpd: ZpdLevels {
                    independent_level: 4,
                    scaffolded_level: 6,
                },
                ..Default::default()
            },
        );

        let baselines = compute_zpd_baselines(&[]);
        seed_progress_with_baselines(&mut progress, baselines);

        let skill = progress.skills.get("pattern-recognition").unwrap();
        assert_eq!(skill.level, 5);
        assert_eq!(skill.zpd.independent_level, 4);
    }
}
