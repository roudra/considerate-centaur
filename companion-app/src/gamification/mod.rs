// Gamification — skill tree, challenges, teach-back, progression events.
// See CLAUDE.md "Deeper Gamification" section for full specification.

use chrono::{DateTime, Datelike, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

use crate::assignments::VerifiedAssignment;
use crate::progress::tracker::LearnerProgress;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during gamification operations.
#[derive(Debug, Error)]
pub enum GamificationError {
    #[error("Not eligible: {0}")]
    NotEligible(String),

    #[error("Challenge not found: {0}")]
    ChallengeNotFound(Uuid),

    #[error("Boss not found: {0}")]
    BossNotFound(String),

    #[error("Already completed today")]
    AlreadyCompletedToday,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Skill tree
// ---------------------------------------------------------------------------

/// Full skill definition loaded from `skill-tree.json`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDefinition {
    pub name: String,
    pub description: String,
    pub category: String,
    pub prerequisites: Vec<String>,
    pub max_level: u32,
    pub xp_per_level: u32,
}

/// A node in the skill tree returned to callers, enriched with unlock status.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillTreeNode {
    pub skill_id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub prerequisites: Vec<String>,
    pub max_level: u32,
    /// Whether the learner has met the prerequisites to use this skill.
    pub unlocked: bool,
    /// Learner's current level in this skill (0 if not started).
    pub current_level: u32,
    /// Learner's current XP in this skill (0 if not started).
    pub current_xp: u32,
}

/// Internal: the full skill-tree.json structure.
#[derive(Debug, Deserialize)]
struct FullSkillTree {
    skills: HashMap<String, SkillDefinition>,
}

/// Build the skill tree with unlock status for a given learner's progress.
///
/// A skill is unlocked when **all** its prerequisites have reached level 2+.
/// Pattern Recognition has no prerequisites and is always unlocked.
pub async fn build_skill_tree(
    data_dir: &Path,
    progress: &LearnerProgress,
) -> Result<Vec<SkillTreeNode>, GamificationError> {
    let skill_tree_path = data_dir.join("curriculum").join("skill-tree.json");
    let bytes = tokio::fs::read(&skill_tree_path).await?;
    let tree: FullSkillTree = serde_json::from_slice(&bytes)?;

    let mut nodes: Vec<SkillTreeNode> = tree
        .skills
        .into_iter()
        .map(|(skill_id, def)| {
            let unlocked = def.prerequisites.is_empty()
                || def.prerequisites.iter().all(|prereq| {
                    progress
                        .skills
                        .get(prereq)
                        .map(|s| s.level >= 2)
                        .unwrap_or(false)
                });

            let (current_level, current_xp) =
                if let Some(skill_prog) = progress.skills.get(&skill_id) {
                    (skill_prog.level, skill_prog.xp)
                } else {
                    (0, 0)
                };

            SkillTreeNode {
                skill_id,
                name: def.name,
                description: def.description,
                category: def.category,
                prerequisites: def.prerequisites,
                max_level: def.max_level,
                unlocked,
                current_level,
                current_xp,
            }
        })
        .collect();

    // Stable ordering: by skill_id for deterministic output.
    nodes.sort_by(|a, b| a.skill_id.cmp(&b.skill_id));

    Ok(nodes)
}

// ---------------------------------------------------------------------------
// Streak shield
// ---------------------------------------------------------------------------

/// Apply the streak shield automatically if a day was missed and a shield is
/// available (not used in the last 7 days).
///
/// Called from `update_streak_for_session` when a gap > 1 is detected. If a
/// shield is available, the streak is maintained (not reset) and the shield's
/// last-used date is updated. A shield can only be used once per 7-day window.
///
/// Returns `true` if the shield was activated.
pub fn apply_streak_shield_if_available(progress: &mut LearnerProgress, today: NaiveDate) -> bool {
    let shield_available = match progress.streaks.shield_last_used {
        None => true,
        Some(last_used) => today.signed_duration_since(last_used).num_days() >= 7,
    };

    if shield_available && progress.streaks.current_days > 0 {
        progress.streaks.shield_last_used = Some(today);
        true
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Timed challenge
// ---------------------------------------------------------------------------

/// The state of an active timed challenge stored server-side.
#[derive(Clone, Debug)]
pub struct TimedChallenge {
    pub id: Uuid,
    pub learner_id: Uuid,
    /// Wall-clock time when the challenge started (server-side enforcement).
    pub started_at: DateTime<Local>,
    /// The target skill for this challenge.
    pub skill: String,
    /// The 5 problems to solve.
    pub problems: Vec<VerifiedAssignment>,
}

/// Result of a single response within a timed challenge.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedChallengeResponse {
    /// Server-assigned assignment ID (from the challenge's problem list).
    pub assignment_id: String,
    /// The child's answer.
    pub child_response: String,
    /// Server-side timestamp when this response arrived.
    pub received_at_seconds: f64,
}

/// Result of a completed timed challenge.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedChallengeResult {
    pub challenge_id: String,
    pub total_problems: usize,
    pub correct_count: usize,
    pub accuracy: f32,
    /// Total elapsed time in seconds (server-side measured).
    pub elapsed_seconds: f64,
    /// Whether the Lightning badge was earned (80%+ accuracy).
    pub lightning_badge_earned: bool,
    /// Per-problem results (without exposing correct answers).
    pub problem_results: Vec<TimedProblemResult>,
}

/// Result for a single problem within a timed challenge.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedProblemResult {
    pub assignment_id: String,
    pub correct: bool,
    pub child_response: String,
}

// ---------------------------------------------------------------------------
// Boss battles
// ---------------------------------------------------------------------------

/// A boss battle definition loaded from `bosses.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BossDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Skills and minimum levels required to start this boss battle.
    pub required_skills: HashMap<String, u32>,
    /// The badge awarded for completing this boss.
    pub badge_id: String,
    pub badge_name: String,
}

/// The state of an active boss battle stored server-side.
#[derive(Clone, Debug)]
pub struct BossChallenge {
    pub id: Uuid,
    pub learner_id: Uuid,
    pub boss_id: String,
    pub started_at: DateTime<Local>,
    /// The multi-step problem to solve.
    pub problem: VerifiedAssignment,
}

/// Load all boss definitions from `{data_dir}/curriculum/bosses.json`.
pub async fn load_bosses(data_dir: &Path) -> Result<Vec<BossDefinition>, GamificationError> {
    let path = data_dir.join("curriculum").join("bosses.json");
    let bytes = tokio::fs::read(&path).await?;
    let bosses: Vec<BossDefinition> = serde_json::from_slice(&bytes)?;
    Ok(bosses)
}

/// Check whether a learner is eligible for a given boss battle.
///
/// Eligibility is server-side only — clients cannot claim eligibility.
pub fn is_boss_eligible(boss: &BossDefinition, progress: &LearnerProgress) -> bool {
    boss.required_skills.iter().all(|(skill_id, &min_level)| {
        progress
            .skills
            .get(skill_id)
            .map(|s| s.level >= min_level)
            .unwrap_or(false)
    })
}

// ---------------------------------------------------------------------------
// Daily puzzle
// ---------------------------------------------------------------------------

/// Per-learner daily puzzle state, stored in `{data_dir}/learners/{id}/daily-puzzles.json`.
///
/// Stored separately from `progress.json` to avoid confusion — daily puzzles
/// do not affect skill levels, only a separate XP and streak counter.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyPuzzleState {
    pub schema_version: u32,
    pub learner_id: Uuid,
    /// ISO dates of all days on which a puzzle was completed.
    pub completed_dates: Vec<NaiveDate>,
    /// Current daily puzzle streak (consecutive days).
    pub current_streak: u32,
    /// Longest daily puzzle streak ever achieved.
    pub longest_streak: u32,
    /// Date of the last completed puzzle (may differ from completed_dates.last()
    /// when completed_dates is empty but defaults exist).
    pub last_puzzle_date: Option<NaiveDate>,
    /// Total XP earned from daily puzzles (does not affect skill levels).
    pub total_xp: u32,
}

impl DailyPuzzleState {
    /// Create default state for a new learner.
    pub fn new(learner_id: Uuid) -> Self {
        Self {
            schema_version: 1,
            learner_id,
            completed_dates: Vec::new(),
            current_streak: 0,
            longest_streak: 0,
            last_puzzle_date: None,
            total_xp: 0,
        }
    }
}

/// Path to a learner's daily-puzzles.json file.
pub fn daily_puzzle_path(data_dir: &Path, learner_id: Uuid) -> PathBuf {
    data_dir
        .join("learners")
        .join(learner_id.to_string())
        .join("daily-puzzles.json")
}

/// Read daily puzzle state; returns a fresh default if the file does not exist.
pub async fn read_daily_puzzle_state(
    data_dir: &Path,
    learner_id: Uuid,
) -> Result<DailyPuzzleState, GamificationError> {
    let path = daily_puzzle_path(data_dir, learner_id);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let state: DailyPuzzleState = serde_json::from_slice(&bytes)?;
            Ok(state)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DailyPuzzleState::new(learner_id)),
        Err(e) => Err(GamificationError::Io(e)),
    }
}

/// Write daily puzzle state to disk.
pub async fn write_daily_puzzle_state(
    data_dir: &Path,
    state: &DailyPuzzleState,
) -> Result<(), GamificationError> {
    let path = daily_puzzle_path(data_dir, state.learner_id);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_string_pretty(state)?;
    tokio::fs::write(&path, json).await?;
    Ok(())
}

/// Determine which skill should be featured in today's daily puzzle.
///
/// Rotates across skills using the day-of-year modulo the number of skills in
/// the learner's progress (falling back to `"pattern-recognition"` if empty).
pub fn daily_puzzle_skill(progress: &LearnerProgress, today: NaiveDate) -> String {
    let skills: Vec<&String> = {
        let mut s: Vec<&String> = progress.skills.keys().collect();
        s.sort();
        s
    };

    if skills.is_empty() {
        return "pattern-recognition".to_string();
    }

    let day_of_year = today.ordinal0() as usize;
    skills[day_of_year % skills.len()].clone()
}

/// Record a completed daily puzzle and update the streak counter.
///
/// Returns the XP awarded (20 XP per daily puzzle, no skill level impact).
pub fn record_daily_puzzle_completion(
    state: &mut DailyPuzzleState,
    today: NaiveDate,
) -> Result<u32, GamificationError> {
    if state.completed_dates.last() == Some(&today) {
        return Err(GamificationError::AlreadyCompletedToday);
    }

    state.completed_dates.push(today);
    state.last_puzzle_date = Some(today);

    // Update streak.
    let prev_date = state.completed_dates.iter().rev().nth(1).copied();

    let gap = match prev_date {
        None => 1, // first completion
        Some(prev) => today.signed_duration_since(prev).num_days(),
    };

    if gap == 1 {
        state.current_streak += 1;
    } else {
        state.current_streak = 1;
    }

    if state.current_streak > state.longest_streak {
        state.longest_streak = state.current_streak;
    }

    let xp = 20_u32;
    state.total_xp += xp;

    Ok(xp)
}

// ---------------------------------------------------------------------------
// Teach-back
// ---------------------------------------------------------------------------

/// Pending teach-back responses stored when Claude is unavailable.
///
/// Stored in `{data_dir}/learners/{id}/pending-teach-backs.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingTeachBack {
    pub id: Uuid,
    pub skill: String,
    pub level: u32,
    pub child_response: String,
    pub submitted_at: String,
}

/// Teach-back evaluation result from Claude (structured output).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeachBackEvaluation {
    /// Score for accuracy of the explanation (0.0–1.0).
    pub accuracy_score: f32,
    /// Score for completeness of the explanation (0.0–1.0).
    pub completeness_score: f32,
    /// Score for clarity of the explanation (0.0–1.0).
    pub clarity_score: f32,
    /// Overall pass/fail (true if average score >= 0.6).
    pub passed: bool,
    /// Encouraging feedback for the child.
    pub feedback: String,
}

impl TeachBackEvaluation {
    /// Whether this teach-back earns the Teacher badge (passed with high quality).
    pub fn earns_teacher_badge(&self) -> bool {
        self.passed
    }
}

/// Check if a teach-back should be triggered for a given skill.
///
/// Returns `true` when the last 3 entries in `recentAccuracy` are all 1
/// (3 consecutive correct answers), signalling readiness to explain the concept.
pub fn should_trigger_teach_back(progress: &LearnerProgress, skill_id: &str) -> bool {
    if let Some(skill) = progress.skills.get(skill_id) {
        let acc = &skill.recent_accuracy;
        if acc.len() >= 3 {
            let last_three = &acc[acc.len() - 3..];
            return last_three.iter().all(|&v| v == 1);
        }
    }
    false
}

/// Path to the pending teach-backs file for a learner.
pub fn pending_teach_backs_path(data_dir: &Path, learner_id: Uuid) -> PathBuf {
    data_dir
        .join("learners")
        .join(learner_id.to_string())
        .join("pending-teach-backs.json")
}

/// Store a teach-back response for deferred evaluation (used when Claude is unavailable).
pub async fn store_pending_teach_back(
    data_dir: &Path,
    learner_id: Uuid,
    pending: &PendingTeachBack,
) -> Result<(), GamificationError> {
    let path = pending_teach_backs_path(data_dir, learner_id);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut existing: Vec<PendingTeachBack> = match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(GamificationError::Io(e)),
    };

    existing.push(pending.clone());
    let json = serde_json::to_string_pretty(&existing)?;
    tokio::fs::write(&path, json).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Progression events
// ---------------------------------------------------------------------------

/// A single progression event for a learner (used for the progression feed).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressionEvent {
    pub event_type: ProgressionEventType,
    pub description: String,
}

/// The kind of progression event.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProgressionEventType {
    XpGained,
    LevelUp,
    BadgeEarned,
    SkillUnlocked,
    StreakUpdated,
    DailyPuzzleCompleted,
}

/// Build a snapshot of progression events from the current progress state.
///
/// Computes which skills are newly unlocked (level >= 1 and prerequisites met),
/// current streak, badges, and total XP per skill.
pub fn build_progression_snapshot(
    progress: &LearnerProgress,
    skill_tree: &[SkillTreeNode],
) -> Vec<ProgressionEvent> {
    let mut events = Vec::new();

    // XP per skill
    for (skill_id, skill) in &progress.skills {
        if skill.xp > 0 {
            events.push(ProgressionEvent {
                event_type: ProgressionEventType::XpGained,
                description: format!("{}: {} XP (Level {})", skill_id, skill.xp, skill.level),
            });
        }
    }

    // Unlocked skills
    for node in skill_tree {
        if node.unlocked && node.current_level > 0 {
            events.push(ProgressionEvent {
                event_type: ProgressionEventType::SkillUnlocked,
                description: format!("{} is unlocked", node.name),
            });
        }
    }

    // Badges earned
    for badge in &progress.badges {
        events.push(ProgressionEvent {
            event_type: ProgressionEventType::BadgeEarned,
            description: format!("Badge: {} ({})", badge.name, badge.earned_date),
        });
    }

    // Streak
    if progress.streaks.current_days > 0 {
        events.push(ProgressionEvent {
            event_type: ProgressionEventType::StreakUpdated,
            description: format!(
                "{}-day learning streak (best: {})",
                progress.streaks.current_days, progress.streaks.longest_days
            ),
        });
    }

    events
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::tracker::{SkillProgress, Streaks};
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make_progress(learner_id: Uuid) -> LearnerProgress {
        LearnerProgress {
            schema_version: 1,
            learner_id,
            skills: HashMap::new(),
            badges: Vec::new(),
            streaks: Streaks::default(),
            total_sessions: 0,
            total_time_minutes: 0,
            total_assignments: 0,
            metacognition: crate::progress::tracker::Metacognition::default(),
            challenge_flags: HashMap::new(),
        }
    }

    #[test]
    fn test_teach_back_trigger_requires_3_consecutive() {
        let id = Uuid::new_v4();
        let mut progress = make_progress(id);
        let mut skill = SkillProgress::default();
        skill.recent_accuracy = vec![1, 0, 1, 1, 1]; // last 3 = [1,1,1]
        progress
            .skills
            .insert("pattern-recognition".to_string(), skill);
        assert!(should_trigger_teach_back(&progress, "pattern-recognition"));
    }

    #[test]
    fn test_teach_back_trigger_not_3_consecutive() {
        let id = Uuid::new_v4();
        let mut progress = make_progress(id);
        let mut skill = SkillProgress::default();
        skill.recent_accuracy = vec![1, 1, 0]; // last 3 ends with 0
        progress
            .skills
            .insert("pattern-recognition".to_string(), skill);
        assert!(!should_trigger_teach_back(&progress, "pattern-recognition"));
    }

    #[test]
    fn test_teach_back_trigger_fewer_than_3_entries() {
        let id = Uuid::new_v4();
        let mut progress = make_progress(id);
        let mut skill = SkillProgress::default();
        skill.recent_accuracy = vec![1, 1];
        progress
            .skills
            .insert("pattern-recognition".to_string(), skill);
        assert!(!should_trigger_teach_back(&progress, "pattern-recognition"));
    }

    #[test]
    fn test_is_boss_eligible_passes() {
        let id = Uuid::new_v4();
        let mut progress = make_progress(id);
        let mut skill = SkillProgress::default();
        skill.level = 5;
        progress
            .skills
            .insert("pattern-recognition".to_string(), skill);

        let mut required = HashMap::new();
        required.insert("pattern-recognition".to_string(), 3u32);

        let boss = BossDefinition {
            id: "test-boss".to_string(),
            name: "Test Boss".to_string(),
            description: "desc".to_string(),
            required_skills: required,
            badge_id: "boss-badge".to_string(),
            badge_name: "Boss Badge".to_string(),
        };

        assert!(is_boss_eligible(&boss, &progress));
    }

    #[test]
    fn test_is_boss_eligible_fails_missing_skill() {
        let id = Uuid::new_v4();
        let progress = make_progress(id);

        let mut required = HashMap::new();
        required.insert("pattern-recognition".to_string(), 3u32);

        let boss = BossDefinition {
            id: "test-boss".to_string(),
            name: "Test Boss".to_string(),
            description: "desc".to_string(),
            required_skills: required,
            badge_id: "boss-badge".to_string(),
            badge_name: "Boss Badge".to_string(),
        };

        assert!(!is_boss_eligible(&boss, &progress));
    }

    #[test]
    fn test_is_boss_eligible_fails_low_level() {
        let id = Uuid::new_v4();
        let mut progress = make_progress(id);
        let mut skill = SkillProgress::default();
        skill.level = 2;
        progress
            .skills
            .insert("pattern-recognition".to_string(), skill);

        let mut required = HashMap::new();
        required.insert("pattern-recognition".to_string(), 3u32);

        let boss = BossDefinition {
            id: "test-boss".to_string(),
            name: "Test Boss".to_string(),
            description: "desc".to_string(),
            required_skills: required,
            badge_id: "boss-badge".to_string(),
            badge_name: "Boss Badge".to_string(),
        };

        assert!(!is_boss_eligible(&boss, &progress));
    }

    #[test]
    fn test_daily_puzzle_skill_rotation() {
        let id = Uuid::new_v4();
        let mut progress = make_progress(id);
        progress
            .skills
            .insert("pattern-recognition".to_string(), SkillProgress::default());
        progress
            .skills
            .insert("sequential-logic".to_string(), SkillProgress::default());

        let today = NaiveDate::from_ymd_opt(2026, 4, 11).unwrap();
        let skill = daily_puzzle_skill(&progress, today);
        // Should be one of the two skills, not panic
        assert!(skill == "pattern-recognition" || skill == "sequential-logic");
    }

    #[test]
    fn test_daily_puzzle_skill_fallback_no_skills() {
        let id = Uuid::new_v4();
        let progress = make_progress(id);
        let today = NaiveDate::from_ymd_opt(2026, 4, 11).unwrap();
        let skill = daily_puzzle_skill(&progress, today);
        assert_eq!(skill, "pattern-recognition");
    }

    #[test]
    fn test_record_daily_puzzle_first_completion() {
        let id = Uuid::new_v4();
        let mut state = DailyPuzzleState::new(id);
        let today = NaiveDate::from_ymd_opt(2026, 4, 11).unwrap();
        let xp = record_daily_puzzle_completion(&mut state, today).unwrap();
        assert_eq!(xp, 20);
        assert_eq!(state.current_streak, 1);
        assert_eq!(state.total_xp, 20);
    }

    #[test]
    fn test_record_daily_puzzle_consecutive_days() {
        let id = Uuid::new_v4();
        let mut state = DailyPuzzleState::new(id);
        let day1 = NaiveDate::from_ymd_opt(2026, 4, 10).unwrap();
        let day2 = NaiveDate::from_ymd_opt(2026, 4, 11).unwrap();
        record_daily_puzzle_completion(&mut state, day1).unwrap();
        record_daily_puzzle_completion(&mut state, day2).unwrap();
        assert_eq!(state.current_streak, 2);
    }

    #[test]
    fn test_record_daily_puzzle_duplicate_day_returns_error() {
        let id = Uuid::new_v4();
        let mut state = DailyPuzzleState::new(id);
        let today = NaiveDate::from_ymd_opt(2026, 4, 11).unwrap();
        record_daily_puzzle_completion(&mut state, today).unwrap();
        let result = record_daily_puzzle_completion(&mut state, today);
        assert!(matches!(
            result,
            Err(GamificationError::AlreadyCompletedToday)
        ));
    }

    #[test]
    fn test_streak_shield_no_previous_use() {
        let id = Uuid::new_v4();
        let mut progress = make_progress(id);
        progress.streaks.current_days = 5;
        let today = NaiveDate::from_ymd_opt(2026, 4, 11).unwrap();
        let activated = apply_streak_shield_if_available(&mut progress, today);
        assert!(activated);
        assert_eq!(progress.streaks.shield_last_used, Some(today));
    }

    #[test]
    fn test_streak_shield_used_within_7_days_not_available() {
        let id = Uuid::new_v4();
        let mut progress = make_progress(id);
        progress.streaks.current_days = 5;
        let last_used = NaiveDate::from_ymd_opt(2026, 4, 8).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 4, 11).unwrap(); // 3 days later
        progress.streaks.shield_last_used = Some(last_used);
        let activated = apply_streak_shield_if_available(&mut progress, today);
        assert!(!activated);
    }

    #[test]
    fn test_streak_shield_used_7_days_ago_is_available() {
        let id = Uuid::new_v4();
        let mut progress = make_progress(id);
        progress.streaks.current_days = 5;
        let last_used = NaiveDate::from_ymd_opt(2026, 4, 4).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 4, 11).unwrap(); // exactly 7 days
        progress.streaks.shield_last_used = Some(last_used);
        let activated = apply_streak_shield_if_available(&mut progress, today);
        assert!(activated);
    }

    #[test]
    fn test_teach_back_evaluation_badge_earned_when_passed() {
        let eval = TeachBackEvaluation {
            accuracy_score: 0.8,
            completeness_score: 0.7,
            clarity_score: 0.9,
            passed: true,
            feedback: "Great explanation!".to_string(),
        };
        assert!(eval.earns_teacher_badge());
    }

    #[test]
    fn test_teach_back_evaluation_no_badge_when_failed() {
        let eval = TeachBackEvaluation {
            accuracy_score: 0.3,
            completeness_score: 0.4,
            clarity_score: 0.5,
            passed: false,
            feedback: "Keep practicing!".to_string(),
        };
        assert!(!eval.earns_teacher_badge());
    }
}
