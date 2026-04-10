// Progress tracking — skill levels, XP, badges, spaced repetition.
// See CLAUDE.md "Progress Tracking" and "Spaced Repetition" sections.

pub mod spaced;
pub mod tracker;

pub use spaced::{
    build_skill_health_map, classify_skill_health, plan_session_mix, update_spaced_repetition,
    ReviewCandidate, SkillHealth, SkillHealthEntry, MIN_EASE_FACTOR,
};
pub use tracker::*;

use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

const EXPECTED_SCHEMA_VERSION: u32 = 1;

/// Errors that can occur during progress operations.
#[derive(Debug, Error)]
pub enum ProgressError {
    #[error("Progress not found for learner: {0}")]
    NotFound(Uuid),

    #[error("Schema version mismatch: expected {expected}, got {actual}")]
    InvalidSchemaVersion { expected: u32, actual: u32 },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Returns the path to a learner's progress file:
/// `{data_dir}/learners/{learner_id}/progress.json`
fn progress_path(data_dir: &Path, learner_id: Uuid) -> std::path::PathBuf {
    data_dir
        .join("learners")
        .join(learner_id.to_string())
        .join("progress.json")
}

/// Initialize default progress for a new learner.
///
/// Returns a [`LearnerProgress`] with zero stats, empty skills, and no badges.
/// Call [`write_progress`] to persist it.
pub fn init_progress(learner_id: Uuid) -> LearnerProgress {
    LearnerProgress {
        schema_version: 1,
        learner_id,
        skills: std::collections::HashMap::new(),
        badges: Vec::new(),
        streaks: Streaks::default(),
        total_sessions: 0,
        total_time_minutes: 0,
        total_assignments: 0,
        metacognition: Metacognition::default(),
        challenge_flags: std::collections::HashMap::new(),
    }
}

/// Read a learner's progress from disk, validating the schema version.
///
/// Returns [`ProgressError::NotFound`] if no progress file exists yet —
/// a new learner won't have one until after their first session.
pub async fn read_progress(
    data_dir: &Path,
    learner_id: Uuid,
) -> Result<LearnerProgress, ProgressError> {
    let path = progress_path(data_dir, learner_id);

    let bytes = tokio::fs::read(&path).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ProgressError::NotFound(learner_id)
        } else {
            ProgressError::Io(e)
        }
    })?;

    let progress: LearnerProgress = serde_json::from_slice(&bytes)?;

    if progress.schema_version != EXPECTED_SCHEMA_VERSION {
        return Err(ProgressError::InvalidSchemaVersion {
            expected: EXPECTED_SCHEMA_VERSION,
            actual: progress.schema_version,
        });
    }

    Ok(progress)
}

/// Write (create or overwrite) a learner's `progress.json`.
///
/// Enforces the ring buffer constraint: all `recentAccuracy` slices are
/// truncated to [`RECENT_ACCURACY_MAX`] entries before writing.
pub async fn write_progress(
    data_dir: &Path,
    progress: &LearnerProgress,
) -> Result<(), ProgressError> {
    // Enforce ring buffer on every write.
    let mut p = progress.clone();
    for skill in p.skills.values_mut() {
        if skill.recent_accuracy.len() > RECENT_ACCURACY_MAX {
            let excess = skill.recent_accuracy.len() - RECENT_ACCURACY_MAX;
            skill.recent_accuracy.drain(..excess);
        }
    }

    let json = serde_json::to_string_pretty(&p)?;
    let path = progress_path(data_dir, p.learner_id);

    // Ensure the parent directory exists (it should already be created by the
    // learner profile module, but guard against standalone calls in tests).
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    tokio::fs::write(&path, json).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Badge eligibility
// ---------------------------------------------------------------------------

/// A badge definition loaded from `skill-tree.json`.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct BadgeDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub condition: String,
}

/// Optional per-session context required to evaluate session-specific badge
/// conditions (e.g. `sessionAccuracy == 1.0`).
#[derive(Clone, Debug, Default)]
pub struct BadgeContext {
    /// Accuracy of the session that just ended (0.0–1.0), if applicable.
    pub session_accuracy: Option<f32>,
}

/// Internal: groups of badge definitions loaded from the skill tree.
#[derive(Debug, serde::Deserialize)]
struct SkillTreeBadges {
    milestone: Vec<BadgeDefinition>,
    explorer: Vec<BadgeDefinition>,
    skill: Vec<BadgeDefinition>,
    streak: Vec<BadgeDefinition>,
    challenge: Vec<BadgeDefinition>,
}

/// Internal: the top-level skill-tree.json structure (we only need `badges`).
#[derive(Debug, serde::Deserialize)]
struct SkillTree {
    badges: SkillTreeBadges,
}

impl SkillTreeBadges {
    /// Iterate over all badge definitions paired with their category label.
    fn all(&self) -> impl Iterator<Item = (&BadgeDefinition, &'static str)> {
        let m = self.milestone.iter().map(|b| (b, "milestone"));
        let e = self.explorer.iter().map(|b| (b, "explorer"));
        let s = self.skill.iter().map(|b| (b, "skill"));
        let st = self.streak.iter().map(|b| (b, "streak"));
        let c = self.challenge.iter().map(|b| (b, "challenge"));
        m.chain(e).chain(s).chain(st).chain(c)
    }
}

/// Evaluate a single badge condition string against the learner's progress.
///
/// Supported patterns:
/// - `field >= N` / `<= N` / `== N` — numeric comparisons where `field` is one of
///   `totalSessions`, `totalAssignments`, `streakDays`, `anySkillLevel`, `sessionAccuracy`
/// - `firstAttempt:<skill-id>` — true if the skill exists in `progress.skills`
/// - bare identifier (e.g. `onboardingComplete`) — looks up `challenge_flags`
fn evaluate_condition(condition: &str, progress: &LearnerProgress, ctx: &BadgeContext) -> bool {
    // "firstAttempt:<skill-id>"
    if let Some(skill_id) = condition.strip_prefix("firstAttempt:") {
        return progress.skills.contains_key(skill_id);
    }

    // Bare boolean flag (no spaces, no colon — e.g. "onboardingComplete")
    if !condition.contains(' ') {
        return *progress.challenge_flags.get(condition).unwrap_or(&false);
    }

    // Comparison expression: "<field> <op> <value>"
    let parts: Vec<&str> = condition.splitn(3, ' ').collect();
    if parts.len() != 3 {
        return false;
    }
    let (field, op, value_str) = (parts[0], parts[1], parts[2]);

    let lhs: f64 = match field {
        "totalSessions" => progress.total_sessions as f64,
        "totalAssignments" => progress.total_assignments as f64,
        "streakDays" => progress.streaks.current_days as f64,
        "anySkillLevel" => progress.skills.values().map(|s| s.level).max().unwrap_or(0) as f64,
        "sessionAccuracy" => ctx.session_accuracy.unwrap_or(0.0) as f64,
        _ => return false,
    };

    let rhs: f64 = match value_str.parse() {
        Ok(v) => v,
        Err(_) => return false,
    };

    match op {
        ">=" => lhs >= rhs,
        "<=" => lhs <= rhs,
        "==" => (lhs - rhs).abs() < f64::EPSILON,
        ">" => lhs > rhs,
        "<" => lhs < rhs,
        _ => false,
    }
}

/// Check which badges a learner has newly earned given their current progress.
///
/// Loads badge definitions from `skill_tree_path`, evaluates every badge
/// condition, and returns definitions for badges that:
/// 1. Pass their condition, **and**
/// 2. Are **not** already in `progress.badges`.
///
/// Each returned tuple contains `(BadgeDefinition, category)`.
pub async fn check_new_badges(
    progress: &LearnerProgress,
    skill_tree_path: &Path,
    ctx: &BadgeContext,
) -> Result<Vec<(BadgeDefinition, String)>, ProgressError> {
    let bytes = tokio::fs::read(skill_tree_path).await?;
    let tree: SkillTree = serde_json::from_slice(&bytes)?;

    let earned_ids: std::collections::HashSet<&str> =
        progress.badges.iter().map(|b| b.id.as_str()).collect();

    let mut new_badges = Vec::new();
    for (def, category) in tree.badges.all() {
        if earned_ids.contains(def.id.as_str()) {
            continue;
        }
        if evaluate_condition(&def.condition, progress, ctx) {
            new_badges.push((def.clone(), category.to_string()));
        }
    }

    Ok(new_badges)
}
