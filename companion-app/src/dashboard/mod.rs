// Parent dashboard API — data types, review queue persistence, and response types.
// See CLAUDE.md "Parent Dashboard" section.

use chrono::Local;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

const EXPECTED_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during dashboard operations.
#[derive(Debug, Error)]
pub enum DashboardError {
    #[error("Review queue item not found: {0}")]
    ItemNotFound(String),

    #[error("Schema version mismatch: expected {expected}, got {actual}")]
    InvalidSchemaVersion { expected: u32, actual: u32 },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Review queue
// ---------------------------------------------------------------------------

/// The parent's decision on a flagged review item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewStatus {
    /// Awaiting parent decision.
    Pending,
    /// Parent confirmed Claude's assessment was correct.
    Confirmed,
    /// Parent overrode Claude's assessment.
    Overridden,
    /// Parent marked for discussion with the child.
    Discuss,
}

/// A single item in the parent review queue.
///
/// Created when an assignment cannot be fully verified programmatically
/// (e.g. free-form reasoning). The parent confirms, overrides, or flags
/// for discussion.
///
/// **Privacy**: no `learnerId` UUID is stored in items (Constitution §6).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewQueueItem {
    /// Unique ID for this review item (UUID as string).
    pub id: String,
    /// File-based session ID (e.g. `"session-2026-04-07-1530"`).
    pub session_id: String,
    /// Assignment type (e.g. `"sequence-puzzle"`).
    pub assignment_type: String,
    /// The assignment prompt shown to the child.
    pub prompt: String,
    /// The child's raw response.
    pub child_response: String,
    /// Claude's assessment text, if available. Empty string if Claude was unavailable.
    pub claude_assessment: String,
    /// Confidence tier reported by the system (`"high"`, `"medium"`, `"low"`).
    pub confidence: String,
    /// Current review status.
    pub status: ReviewStatus,
    /// ISO date when the item was created (e.g. `"2026-04-07"`).
    pub created_at: String,
    /// Optional notes added by the parent when confirming or overriding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_notes: Option<String>,
}

/// The serialized review queue stored in `review-queue.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewQueue {
    pub schema_version: u32,
    pub items: Vec<ReviewQueueItem>,
}

impl ReviewQueue {
    fn new() -> Self {
        Self {
            schema_version: EXPECTED_SCHEMA_VERSION,
            items: Vec::new(),
        }
    }
}

fn review_queue_path(data_dir: &Path, learner_id: Uuid) -> std::path::PathBuf {
    data_dir
        .join("learners")
        .join(learner_id.to_string())
        .join("review-queue.json")
}

/// Read the review queue from disk.
///
/// Returns an empty queue if the file does not exist (new learner or no flagged items yet).
pub async fn read_review_queue(
    data_dir: &Path,
    learner_id: Uuid,
) -> Result<ReviewQueue, DashboardError> {
    let path = review_queue_path(data_dir, learner_id);

    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ReviewQueue::new());
        }
        Err(e) => return Err(DashboardError::Io(e)),
    };

    let queue: ReviewQueue = serde_json::from_slice(&bytes)?;

    if queue.schema_version != EXPECTED_SCHEMA_VERSION {
        return Err(DashboardError::InvalidSchemaVersion {
            expected: EXPECTED_SCHEMA_VERSION,
            actual: queue.schema_version,
        });
    }

    Ok(queue)
}

/// Write the review queue to disk.
pub async fn write_review_queue(
    data_dir: &Path,
    learner_id: Uuid,
    queue: &ReviewQueue,
) -> Result<(), DashboardError> {
    let path = review_queue_path(data_dir, learner_id);

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let json = serde_json::to_vec_pretty(queue)?;
    tokio::fs::write(&path, json).await?;

    Ok(())
}

/// Append a new item to the learner's review queue.
///
/// Reads the current queue, appends the item, and writes back atomically.
pub async fn add_review_item(
    data_dir: &Path,
    learner_id: Uuid,
    item: ReviewQueueItem,
) -> Result<(), DashboardError> {
    let mut queue = read_review_queue(data_dir, learner_id).await?;
    queue.items.push(item);
    write_review_queue(data_dir, learner_id, &queue).await
}

/// Construct a new [`ReviewQueueItem`] with `status = Pending` and a fresh UUID.
pub fn new_review_item(
    session_id: &str,
    assignment_type: &str,
    prompt: &str,
    child_response: &str,
    claude_assessment: &str,
    confidence: &str,
) -> ReviewQueueItem {
    ReviewQueueItem {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        assignment_type: assignment_type.to_string(),
        prompt: prompt.to_string(),
        child_response: child_response.to_string(),
        claude_assessment: claude_assessment.to_string(),
        confidence: confidence.to_string(),
        status: ReviewStatus::Pending,
        created_at: Local::now().date_naive().to_string(),
        parent_notes: None,
    }
}

// ---------------------------------------------------------------------------
// Dashboard overview response types
// ---------------------------------------------------------------------------

/// One entry in the skill-level radar chart — skill ID and current level.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRadarEntry {
    pub skill_id: String,
    pub level: u32,
    pub xp: u32,
}

/// One entry in the ZPD visualization — independent vs scaffolded per skill.
///
/// The `gap` field is **always computed at runtime** and never stored
/// (Constitution §7 / CLAUDE.md invariant).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZpdVisualizationEntry {
    pub skill_id: String,
    pub independent_level: u32,
    pub scaffolded_level: u32,
    /// Computed at runtime: `scaffoldedLevel − independentLevel`.
    pub gap: u32,
}

/// The combined dashboard overview.
///
/// Aggregates learner info, streaks, badges, skill radar, ZPD, and session totals.
/// **Never includes `learnerId` or UUIDs** (Constitution §4, §6).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardOverview {
    /// Child-chosen display name (safe to include — no real name stored).
    pub name: String,
    pub age: u8,
    pub interests: Vec<String>,
    pub current_streak_days: u32,
    pub longest_streak_days: u32,
    /// Most recently earned badges (up to 5).
    pub recent_badges: Vec<crate::progress::EarnedBadge>,
    /// Per-skill level and XP for the radar chart.
    pub skill_radar: Vec<SkillRadarEntry>,
    /// Per-skill ZPD visualization (gap computed at runtime).
    pub zpd_visualization: Vec<ZpdVisualizationEntry>,
    pub total_sessions: u32,
    pub total_time_minutes: u32,
    pub total_assignments: u32,
}

// ---------------------------------------------------------------------------
// Skill detail response type
// ---------------------------------------------------------------------------

/// Detailed view of a single skill for the parent dashboard drill-down.
///
/// The `zpdGap` field is always computed at response time — never stored.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetailView {
    pub skill_id: String,
    pub level: u32,
    pub xp: u32,
    /// XP needed to reach the next level (0 at max level 10).
    pub xp_to_next_level: u32,
    pub independent_level: u32,
    pub scaffolded_level: u32,
    /// Computed at runtime: `scaffoldedLevel − independentLevel`.
    pub zpd_gap: u32,
    /// Ring buffer of last 5 attempts (1 = correct, 0 = incorrect).
    pub recent_accuracy: Vec<u8>,
    /// Fraction in `0.0..=1.0`.
    pub recent_accuracy_fraction: f32,
    pub working_memory_signal: crate::progress::WorkingMemorySignal,
    pub spaced_repetition_health: crate::progress::SkillHealth,
    /// ISO date of last practice, or `null` if never practiced.
    pub last_practiced: Option<String>,
}

// ---------------------------------------------------------------------------
// Behavioral insights response type
// ---------------------------------------------------------------------------

/// Behavioral observation text extracted from one session markdown file.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionObservation {
    pub session_date: String,
    pub observations: String,
    pub continuity_notes: String,
}

/// Behavioral insights view — the "soul" data for parents.
///
/// Combines observed behavioral dimensions (from `profile.json`) with
/// metacognition metrics (from `progress.json`) and recent session observations.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BehavioralInsightsView {
    // --- Profile: observed behavioral dimensions ---
    pub frustration_response: crate::learner::FrustrationResponse,
    pub effort_attribution: crate::learner::EffortAttribution,
    pub hint_usage: crate::learner::HintUsage,
    pub optimal_session_minutes: Option<u32>,
    pub accuracy_decay_onset: Option<u32>,

    // --- Progress: metacognition signals ---
    pub self_correction_rate: f32,
    pub hint_request_rate: f32,
    pub metacognition_trend: crate::progress::MetacognitionTrend,

    // --- Recent session observations (up to 3) ---
    pub recent_observations: Vec<SessionObservation>,
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_learner_dir(tmp: &TempDir, id: Uuid) -> std::path::PathBuf {
        let dir = tmp.path().join("learners").join(id.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn test_empty_review_queue_when_no_file() {
        let tmp = TempDir::new().unwrap();
        let id = Uuid::new_v4();
        make_learner_dir(&tmp, id);

        let queue = read_review_queue(tmp.path(), id).await.unwrap();
        assert!(queue.items.is_empty());
        assert_eq!(queue.schema_version, 1);
    }

    #[tokio::test]
    async fn test_add_and_read_review_item() {
        let tmp = TempDir::new().unwrap();
        let id = Uuid::new_v4();
        make_learner_dir(&tmp, id);

        let item = new_review_item(
            "session-2026-04-07-1530",
            "sequence-puzzle",
            "What comes next: 2, 4, 8?",
            "16",
            "Creative reasoning — correct",
            "medium",
        );

        add_review_item(tmp.path(), id, item.clone()).await.unwrap();

        let queue = read_review_queue(tmp.path(), id).await.unwrap();
        assert_eq!(queue.items.len(), 1);
        assert_eq!(queue.items[0].session_id, "session-2026-04-07-1530");
        assert_eq!(queue.items[0].status, ReviewStatus::Pending);
        assert_eq!(queue.items[0].child_response, "16");
    }

    #[tokio::test]
    async fn test_write_and_read_queue_round_trip() {
        let tmp = TempDir::new().unwrap();
        let id = Uuid::new_v4();
        make_learner_dir(&tmp, id);

        let item1 = new_review_item("session-a", "deductive-reasoning", "p1", "r1", "ok", "high");
        let item2 = new_review_item("session-b", "sequence-puzzle", "p2", "r2", "partial", "low");

        add_review_item(tmp.path(), id, item1).await.unwrap();
        add_review_item(tmp.path(), id, item2).await.unwrap();

        let queue = read_review_queue(tmp.path(), id).await.unwrap();
        assert_eq!(queue.items.len(), 2);
        assert_eq!(queue.items[0].session_id, "session-a");
        assert_eq!(queue.items[1].session_id, "session-b");
    }

    #[tokio::test]
    async fn test_pending_items_have_no_parent_notes() {
        let tmp = TempDir::new().unwrap();
        let id = Uuid::new_v4();
        make_learner_dir(&tmp, id);

        let item = new_review_item("session-x", "free-form", "q", "a", "unclear", "low");
        add_review_item(tmp.path(), id, item).await.unwrap();

        let queue = read_review_queue(tmp.path(), id).await.unwrap();
        assert!(queue.items[0].parent_notes.is_none());
        assert_eq!(queue.items[0].status, ReviewStatus::Pending);
    }

    #[test]
    fn test_review_status_kebab_case() {
        assert_eq!(
            serde_json::to_string(&ReviewStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&ReviewStatus::Confirmed).unwrap(),
            "\"confirmed\""
        );
        assert_eq!(
            serde_json::to_string(&ReviewStatus::Overridden).unwrap(),
            "\"overridden\""
        );
        assert_eq!(
            serde_json::to_string(&ReviewStatus::Discuss).unwrap(),
            "\"discuss\""
        );
    }

    #[test]
    fn test_review_item_no_learner_id_in_serialized_json() {
        let item = new_review_item("session-z", "t", "p", "r", "a", "medium");
        let json = serde_json::to_string(&item).unwrap();
        assert!(!json.contains("learnerId"));
        assert!(!json.contains("learner_id"));
    }
}
