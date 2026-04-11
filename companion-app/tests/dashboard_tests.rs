/// Integration tests for the parent dashboard module.
///
/// Tests cover:
/// - Review queue CRUD (create, read, update status)
/// - Empty-queue defaults for new learners
/// - Schema validation for the review queue file
/// - Privacy: no learnerId in serialized output
use educational_companion::dashboard::{self, ReviewStatus, SkillDetailView};

use tempfile::TempDir;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_learner_dir(tmp: &TempDir, id: Uuid) {
    let dir = tmp.path().join("learners").join(id.to_string());
    std::fs::create_dir_all(dir).unwrap();
}

// ---------------------------------------------------------------------------
// Review queue: create and read
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_new_learner_has_empty_review_queue() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    make_learner_dir(&tmp, id);

    let queue = dashboard::read_review_queue(tmp.path(), id).await.unwrap();
    assert!(queue.items.is_empty());
    assert_eq!(queue.schema_version, 1);
}

#[tokio::test]
async fn test_add_review_item_and_retrieve() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    make_learner_dir(&tmp, id);

    let item = dashboard::new_review_item(
        "session-2026-04-07-1530",
        "sequence-puzzle",
        "What comes next: 2, 4, 8?",
        "16",
        "Correct reasoning",
        "high",
    );
    let item_id = item.id.clone();

    dashboard::add_review_item(tmp.path(), id, item)
        .await
        .unwrap();

    let queue = dashboard::read_review_queue(tmp.path(), id).await.unwrap();
    assert_eq!(queue.items.len(), 1);
    assert_eq!(queue.items[0].id, item_id);
    assert_eq!(queue.items[0].status, ReviewStatus::Pending);
    assert_eq!(queue.items[0].session_id, "session-2026-04-07-1530");
    assert_eq!(queue.items[0].assignment_type, "sequence-puzzle");
    assert!(queue.items[0].parent_notes.is_none());
}

#[tokio::test]
async fn test_multiple_items_preserved_in_order() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    make_learner_dir(&tmp, id);

    for i in 0..3 {
        let item = dashboard::new_review_item(
            &format!("session-{i}"),
            "deductive-reasoning",
            "q",
            "a",
            "ok",
            "medium",
        );
        dashboard::add_review_item(tmp.path(), id, item)
            .await
            .unwrap();
    }

    let queue = dashboard::read_review_queue(tmp.path(), id).await.unwrap();
    assert_eq!(queue.items.len(), 3);
    assert_eq!(queue.items[0].session_id, "session-0");
    assert_eq!(queue.items[1].session_id, "session-1");
    assert_eq!(queue.items[2].session_id, "session-2");
}

// ---------------------------------------------------------------------------
// Review queue: update status
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_update_status_to_confirmed() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    make_learner_dir(&tmp, id);

    let item = dashboard::new_review_item("session-x", "t", "p", "r", "a", "medium");
    let item_id = item.id.clone();
    dashboard::add_review_item(tmp.path(), id, item)
        .await
        .unwrap();

    // Simulate what the HTTP handler does: read, mutate, write.
    let mut queue = dashboard::read_review_queue(tmp.path(), id).await.unwrap();
    let idx = queue.items.iter().position(|i| i.id == item_id).unwrap();
    queue.items[idx].status = ReviewStatus::Confirmed;
    queue.items[idx].parent_notes = Some("Looks good".to_string());
    dashboard::write_review_queue(tmp.path(), id, &queue)
        .await
        .unwrap();

    let queue2 = dashboard::read_review_queue(tmp.path(), id).await.unwrap();
    assert_eq!(queue2.items[0].status, ReviewStatus::Confirmed);
    assert_eq!(queue2.items[0].parent_notes.as_deref(), Some("Looks good"));
}

#[tokio::test]
async fn test_pending_items_only_filter() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    make_learner_dir(&tmp, id);

    let pending = dashboard::new_review_item("s1", "t", "p", "r", "a", "medium");
    let mut confirmed = dashboard::new_review_item("s2", "t", "p", "r", "a", "medium");
    confirmed.status = ReviewStatus::Confirmed;

    dashboard::add_review_item(tmp.path(), id, pending)
        .await
        .unwrap();
    dashboard::add_review_item(tmp.path(), id, confirmed)
        .await
        .unwrap();

    let queue = dashboard::read_review_queue(tmp.path(), id).await.unwrap();
    let pending_items: Vec<_> = queue
        .items
        .iter()
        .filter(|i| i.status == ReviewStatus::Pending)
        .collect();

    assert_eq!(pending_items.len(), 1);
    assert_eq!(pending_items[0].session_id, "s1");
}

// ---------------------------------------------------------------------------
// Privacy: no learnerId in output
// ---------------------------------------------------------------------------

#[test]
fn test_review_item_serialization_contains_no_learner_id() {
    let item = dashboard::new_review_item("session-a", "t", "p", "r", "a", "low");
    let json = serde_json::to_string(&item).unwrap();
    assert!(!json.contains("learnerId"));
    assert!(!json.contains("learner_id"));
}

#[tokio::test]
async fn test_review_queue_file_contains_no_learner_id() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    make_learner_dir(&tmp, id);

    let item = dashboard::new_review_item("session-z", "t", "p", "r", "a", "medium");
    dashboard::add_review_item(tmp.path(), id, item)
        .await
        .unwrap();

    // Read the raw JSON bytes of the queue file.
    let queue_path = tmp
        .path()
        .join("learners")
        .join(id.to_string())
        .join("review-queue.json");
    let raw = std::fs::read_to_string(queue_path).unwrap();

    // The UUIDs in the file belong to items, not the learner — and they are
    // used as item IDs, not as learnerId fields.
    assert!(!raw.contains("\"learnerId\""));
    assert!(!raw.contains("\"learner_id\""));
}

// ---------------------------------------------------------------------------
// Schema version validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_invalid_schema_version_returns_error() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    make_learner_dir(&tmp, id);

    let bad_json = r#"{"schemaVersion":99,"items":[]}"#;
    let queue_path = tmp
        .path()
        .join("learners")
        .join(id.to_string())
        .join("review-queue.json");
    std::fs::write(queue_path, bad_json).unwrap();

    let result = dashboard::read_review_queue(tmp.path(), id).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Schema version mismatch"));
}

// ---------------------------------------------------------------------------
// Camel-case field names in response types
// ---------------------------------------------------------------------------

#[test]
fn test_skill_detail_view_camel_case() {
    use educational_companion::progress::{SkillHealth, WorkingMemorySignal};

    let view = SkillDetailView {
        skill_id: "pattern-recognition".to_string(),
        level: 4,
        xp: 340,
        xp_to_next_level: 60,
        independent_level: 3,
        scaffolded_level: 5,
        zpd_gap: 2,
        recent_accuracy: vec![1, 1, 0, 1, 1],
        recent_accuracy_fraction: 0.8,
        working_memory_signal: WorkingMemorySignal::Stable,
        spaced_repetition_health: SkillHealth::Fresh,
        last_practiced: Some("2026-04-07".to_string()),
    };

    let json = serde_json::to_string(&view).unwrap();
    assert!(json.contains("\"skillId\""));
    assert!(json.contains("\"xpToNextLevel\""));
    assert!(json.contains("\"independentLevel\""));
    assert!(json.contains("\"scaffoldedLevel\""));
    assert!(json.contains("\"zpdGap\""));
    assert!(json.contains("\"recentAccuracy\""));
    assert!(json.contains("\"recentAccuracyFraction\""));
    assert!(json.contains("\"workingMemorySignal\""));
    assert!(json.contains("\"spacedRepetitionHealth\""));
    assert!(json.contains("\"lastPracticed\""));
    // ZPD gap must be computed (== 2), never "stored" separately.
    assert!(!json.contains("\"gap\""));
}

#[test]
fn test_review_status_values() {
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
