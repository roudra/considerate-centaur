// Offline assignment buffer and graceful degradation.
//
// See CLAUDE.md "Offline & Resilience Architecture" and CONSTITUTION.md §8.
//
// Architecture:
//   Tier detection: probe Claude API with a 5-second timeout.
//   Buffer path:    {data_dir}/buffer/{learner_id}-buffer.json
//   Staleness:      buffer entries older than 7 days are discarded on read.
//   Sync:           sessions missing behavioral observations are synced
//                   retroactively when Claude becomes available.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

use crate::assignments::{
    run_pipeline, select_skill, PipelineRequest, VerificationStatus,
    VerifiedAssignment,
};
use crate::claude::ClaudeClient;
use crate::progress::tracker::LearnerProgress;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum age of a buffer entry before it is considered stale and discarded.
///
/// A learner's ZPD may have changed significantly over this period, making old
/// assignments inappropriate.
pub const BUFFER_STALE_DAYS: i64 = 7;

/// Number of assignments to generate during a buffer replenishment.
pub const BUFFER_TARGET_SIZE: usize = 7;

/// Timeout in seconds for the Claude API availability probe.
pub const CLAUDE_PROBE_TIMEOUT_SECS: u64 = 5;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during offline buffer operations.
#[derive(Debug, Error)]
pub enum OfflineError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Degradation tier
// ---------------------------------------------------------------------------

/// The four-tier degradation model from CLAUDE.md § "Graceful Degradation Tiers".
///
/// The system automatically detects which tier it is operating in and selects
/// the appropriate assignment source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DegradationTier {
    /// Claude API is responsive (< 5 s) — fresh assignments, real-time evaluation.
    Full,
    /// Claude API is slow or down, but the buffer has valid entries.
    Buffered,
    /// Buffer is empty or stale, Claude is down — deterministic template generation.
    Template,
    /// No network connectivity — template assignments, queue data for sync.
    Offline,
}

// ---------------------------------------------------------------------------
// Buffer data types
// ---------------------------------------------------------------------------

/// A single pre-generated and pre-verified assignment stored in the buffer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BufferedAssignment {
    /// When this assignment was generated and verified.
    pub generated_at: DateTime<Utc>,
    /// The fully verified assignment (correct answer confirmed by backend).
    pub assignment: VerifiedAssignment,
}

impl BufferedAssignment {
    /// Returns `true` if this entry is older than [`BUFFER_STALE_DAYS`].
    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        let age = now.signed_duration_since(self.generated_at);
        age.num_days() >= BUFFER_STALE_DAYS
    }
}

/// The assignment buffer file for a single learner.
///
/// Stored at `{data_dir}/buffer/{learner_id}-buffer.json`.
/// All assignments here have been pre-verified by the backend; none are served
/// unless their [`VerificationStatus`] is `Verified` or `PartiallyVerified`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentBuffer {
    /// The learner this buffer belongs to (stored for integrity checks).
    pub learner_id: Uuid,
    /// When the most recent replenishment was performed.
    pub generated_at: DateTime<Utc>,
    /// Pre-verified assignments ready to serve.
    pub assignments: Vec<BufferedAssignment>,
}

impl AssignmentBuffer {
    /// Create an empty buffer for a learner.
    pub fn empty(learner_id: Uuid) -> Self {
        AssignmentBuffer {
            learner_id,
            generated_at: Utc::now(),
            assignments: Vec::new(),
        }
    }

    /// Remove and return fresh (non-stale) assignments from the buffer.
    ///
    /// Stale assignments are discarded rather than returned.
    /// Returns `None` if no fresh assignments remain.
    pub fn draw(&mut self) -> Option<VerifiedAssignment> {
        let now = Utc::now();
        // Drain stale entries first.
        self.assignments.retain(|e| !e.is_stale(now));

        if self.assignments.is_empty() {
            return None;
        }

        // Pop from the front so assignments are served in FIFO order.
        Some(self.assignments.remove(0).assignment)
    }

    /// How many fresh (non-stale) assignments remain.
    pub fn fresh_count(&self) -> usize {
        let now = Utc::now();
        self.assignments
            .iter()
            .filter(|e| !e.is_stale(now))
            .count()
    }

    /// Whether the buffer is empty (or all entries are stale).
    pub fn is_empty(&self) -> bool {
        self.fresh_count() == 0
    }
}

// ---------------------------------------------------------------------------
// Buffer status (dashboard / API response)
// ---------------------------------------------------------------------------

/// Status information returned by `GET /api/v1/learners/:id/buffer`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BufferStatus {
    /// Number of fresh (non-stale) assignments in the buffer.
    pub count: usize,
    /// When the buffer was last replenished. `null` if never replenished.
    pub generated_at: Option<DateTime<Utc>>,
    /// Whether the buffer has any stale entries that were discarded on read.
    pub has_stale_entries: bool,
    /// Number of fresh assignments broken down by skill ID.
    pub per_skill: HashMap<String, usize>,
    /// Current degradation tier the system detected.
    pub current_tier: DegradationTier,
}

// ---------------------------------------------------------------------------
// File path helpers
// ---------------------------------------------------------------------------

/// Returns the path to the buffer directory: `{data_dir}/buffer/`.
pub fn buffer_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("buffer")
}

/// Returns the path to a learner's buffer file:
/// `{data_dir}/buffer/{learner_id}-buffer.json`.
pub fn buffer_path(data_dir: &Path, learner_id: Uuid) -> PathBuf {
    buffer_dir(data_dir).join(format!("{}-buffer.json", learner_id))
}

// ---------------------------------------------------------------------------
// Buffer I/O
// ---------------------------------------------------------------------------

/// Read a learner's assignment buffer from disk.
///
/// - Returns `None` if no buffer file exists (first-ever replenishment not yet done).
/// - Stale entries (older than [`BUFFER_STALE_DAYS`]) are silently discarded on read.
/// - If the file is corrupt / unparseable, logs a warning, deletes it, and returns `None`.
pub async fn read_buffer(
    data_dir: &Path,
    learner_id: Uuid,
) -> Option<AssignmentBuffer> {
    let path = buffer_path(data_dir, learner_id);

    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(
                learner_id = %learner_id,
                error = %e,
                "I/O error reading buffer — treating as empty"
            );
            return None;
        }
    };

    let mut buffer: AssignmentBuffer = match serde_json::from_slice(&bytes) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                learner_id = %learner_id,
                error = %e,
                "Buffer file is corrupt — deleting and falling through to next tier"
            );
            if let Err(del_err) = tokio::fs::remove_file(&path).await {
                tracing::warn!(
                    learner_id = %learner_id,
                    error = %del_err,
                    "Failed to delete corrupt buffer file"
                );
            }
            return None;
        }
    };

    // Discard stale entries.
    let now = Utc::now();
    let before = buffer.assignments.len();
    buffer.assignments.retain(|e| !e.is_stale(now));
    let after = buffer.assignments.len();

    if before != after {
        tracing::info!(
            learner_id = %learner_id,
            discarded = before - after,
            remaining = after,
            "Discarded stale buffer entries"
        );
    }

    Some(buffer)
}

/// Write an assignment buffer to disk.
///
/// Creates the buffer directory if it does not exist.
pub async fn write_buffer(
    data_dir: &Path,
    buffer: &AssignmentBuffer,
) -> Result<(), OfflineError> {
    let dir = buffer_dir(data_dir);
    tokio::fs::create_dir_all(&dir).await?;

    let path = buffer_path(data_dir, buffer.learner_id);
    let bytes = serde_json::to_vec_pretty(buffer)?;
    tokio::fs::write(&path, bytes).await?;

    Ok(())
}

/// Draw one assignment from the buffer (consuming it).
///
/// Reads the buffer, removes the first fresh entry, writes back the modified
/// buffer, and returns the assignment.
///
/// Returns `None` if the buffer is empty or all entries are stale.
///
/// **Callers must hold a write lock on the learner's data before calling this.**
pub async fn draw_from_buffer(
    data_dir: &Path,
    learner_id: Uuid,
) -> Option<VerifiedAssignment> {
    let mut buffer = read_buffer(data_dir, learner_id).await?;

    let assignment = buffer.draw()?;

    // Write back the modified buffer (without the drawn entry).
    if let Err(e) = write_buffer(data_dir, &buffer).await {
        tracing::warn!(
            learner_id = %learner_id,
            error = %e,
            "Failed to write buffer after drawing — assignment still served but buffer state not persisted"
        );
    }

    Some(assignment)
}

/// Build a [`BufferStatus`] from an optional buffer and the current tier.
pub fn build_buffer_status(
    buffer: Option<&AssignmentBuffer>,
    tier: DegradationTier,
) -> BufferStatus {
    match buffer {
        None => BufferStatus {
            count: 0,
            generated_at: None,
            has_stale_entries: false,
            per_skill: HashMap::new(),
            current_tier: tier,
        },
        Some(buf) => {
            let now = Utc::now();
            let has_stale_entries = buf
                .assignments
                .iter()
                .any(|e| e.is_stale(now));
            let fresh: Vec<&BufferedAssignment> = buf
                .assignments
                .iter()
                .filter(|e| !e.is_stale(now))
                .collect();
            let count = fresh.len();
            let mut per_skill: HashMap<String, usize> = HashMap::new();
            for entry in &fresh {
                *per_skill
                    .entry(entry.assignment.assignment.skill.clone())
                    .or_insert(0) += 1;
            }
            BufferStatus {
                count,
                generated_at: Some(buf.generated_at),
                has_stale_entries,
                per_skill,
                current_tier: tier,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tier detection
// ---------------------------------------------------------------------------

/// Probe the Claude API availability with a [`CLAUDE_PROBE_TIMEOUT_SECS`]-second timeout.
///
/// We send a minimal 1-token request. Any successful response means "Full".
/// Timeout or transport error → "Buffered" or "Offline" (caller distinguishes
/// by whether the buffer has fresh entries).
pub async fn probe_claude_available(client: &ClaudeClient) -> bool {
    use tokio::time::{timeout, Duration};
    // Use a 1-token max response just to test reachability — it will fail with
    // an API error (max_tokens too low) but the HTTP round-trip succeeds,
    // which is enough to know the API is reachable.
    let probe = timeout(
        Duration::from_secs(CLAUDE_PROBE_TIMEOUT_SECS),
        client.probe_availability(),
    )
    .await;

    match probe {
        Ok(result) => result,
        Err(_elapsed) => {
            tracing::warn!("Claude API probe timed out after {CLAUDE_PROBE_TIMEOUT_SECS}s");
            false
        }
    }
}

/// Detect the current degradation tier.
///
/// Decision logic:
/// 1. If no Claude client is configured → always **Template** (no API key, no connectivity needed)
/// 2. If Claude responds within the timeout → **Full**
/// 3. If Claude timed out / unreachable and it looks like a network error → **Offline**
/// 4. If Claude timed out / unreachable but buffer has entries → **Buffered**
/// 5. Otherwise → **Template**
pub async fn detect_tier(
    claude_client: Option<&ClaudeClient>,
    buffer: Option<&AssignmentBuffer>,
) -> DegradationTier {
    let Some(client) = claude_client else {
        // No API key configured — never try Claude.
        return DegradationTier::Template;
    };

    if probe_claude_available(client).await {
        return DegradationTier::Full;
    }

    // Claude is not available. Check if the network itself is down.
    let network_down = is_network_unreachable(client).await;

    if network_down {
        return DegradationTier::Offline;
    }

    // Claude is down but network is up (API might be having issues).
    if buffer.map(|b| !b.is_empty()).unwrap_or(false) {
        DegradationTier::Buffered
    } else {
        DegradationTier::Template
    }
}

/// Quick check whether the network is completely unreachable.
///
/// Tries to connect to a reliable endpoint (Anthropic's domain) with a 2-second
/// timeout. A connection-refused or DNS-failure error suggests no network.
async fn is_network_unreachable(client: &ClaudeClient) -> bool {
    use tokio::time::{timeout, Duration};
    let check = timeout(
        Duration::from_secs(2),
        client.check_network_reachability(),
    )
    .await;
    match check {
        Ok(reachable) => !reachable,
        Err(_) => true, // timeout → assume unreachable
    }
}

// ---------------------------------------------------------------------------
// Buffer replenishment
// ---------------------------------------------------------------------------

/// Replenish the assignment buffer with freshly generated, verified assignments.
///
/// Generates [`BUFFER_TARGET_SIZE`] assignments using the same pipeline as the
/// live session endpoint:
/// - Skills are selected based on the spaced-repetition schedule and ZPD targets.
/// - All assignments are pre-verified by the backend (`Verified` or `PartiallyVerified`).
/// - `Unverifiable` assignments (only possible from Claude-generated content that
///   the backend cannot confirm) are **not** stored — the fallback uses a
///   deterministic generation which is always `Verified`.
///
/// If Claude is unavailable, deterministic fallbacks are used (still verified).
///
/// **Callers must hold a write lock on the learner's data before calling this.**
pub async fn replenish_buffer(
    data_dir: &Path,
    learner_id: Uuid,
    progress: &LearnerProgress,
    templates: &[crate::assignments::AssignmentTemplate],
    claude_client: Option<&ClaudeClient>,
) -> Result<(), OfflineError> {
    let today = chrono::Local::now().date_naive();
    let now = Utc::now();

    // Read existing buffer so we only top it up.
    let mut buffer = read_buffer(data_dir, learner_id)
        .await
        .unwrap_or_else(|| AssignmentBuffer::empty(learner_id));

    let already_fresh = buffer.fresh_count();
    if already_fresh >= BUFFER_TARGET_SIZE {
        tracing::debug!(
            learner_id = %learner_id,
            count = already_fresh,
            "Buffer already at or above target — skipping replenishment"
        );
        return Ok(());
    }

    let to_generate = BUFFER_TARGET_SIZE.saturating_sub(already_fresh);
    let mut generated = 0usize;

    for _ in 0..to_generate {
        // Select the next skill to cover (cycling through spaced-repetition priorities).
        let target = select_skill(progress, today);
        let (skill, difficulty) = match target {
            Some(t) => (t.skill_id, t.difficulty),
            None => ("pattern-recognition".to_string(), 3),
        };

        let req = PipelineRequest {
            skill,
            difficulty,
            preferred_type: None,
        };

        let verified = if let Some(client) = claude_client {
            // Clone the Arc-wrapped client for the closure.
            let client_ref = client.clone();
            let skill_clone = req.skill.clone();
            let difficulty_val = req.difficulty;

            run_pipeline(
                move || {
                    let c = client_ref.clone();
                    let s = skill_clone.clone();
                    let d = difficulty_val;
                    async move {
                        use crate::claude::prompts::{GenerationContext, ProgressSnapshot};
                        let ctx = GenerationContext {
                            profile: crate::claude::prompts::SanitizedProfile {
                                name: "learner".to_string(),
                                age: 8,
                                interests: vec![],
                                initial_preferences: crate::learner::profile::InitialPreferences {
                                    session_length_minutes: 25,
                                    challenge_preference: crate::learner::profile::ChallengePreference::Guided,
                                },
                                observed_behavior: crate::learner::profile::ObservedBehavior::default(),
                            },
                            progress: ProgressSnapshot {
                                skills: std::collections::HashMap::new(),
                                total_sessions: 0,
                                total_assignments: 0,
                            },
                            recent_session_summaries: vec![],
                            target_skill: s,
                            target_difficulty: d,
                        };
                        c.generate_assignment(&ctx).await.ok()
                    }
                },
                templates,
                &req,
                2,
            )
            .await
        } else {
            run_pipeline(
                || async { None::<crate::claude::schemas::GeneratedAssignment> },
                templates,
                &req,
                0,
            )
            .await
        };

        // Only store assignments the backend can actually verify.
        // Unverifiable assignments from Claude are rejected; the deterministic
        // fallback is always correct by construction (its sequence/pattern is
        // computed, not hallucinated) so it is always accepted.
        if verified.verification_status == VerificationStatus::Unverifiable && !verified.used_fallback {
            tracing::warn!(
                skill = %req.skill,
                "Buffer replenishment: skipping unverifiable Claude-generated assignment"
            );
            continue;
        }

        buffer.assignments.push(BufferedAssignment {
            generated_at: now,
            assignment: verified,
        });
        generated += 1;
    }

    buffer.generated_at = now;

    write_buffer(data_dir, &buffer).await?;

    tracing::info!(
        learner_id = %learner_id,
        generated,
        total = buffer.assignments.len(),
        "Buffer replenishment complete"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Offline session sync
// ---------------------------------------------------------------------------

/// Placeholder text used in session markdown files when Claude was unavailable.
///
/// Sessions containing this string in their behavioral observations section are
/// flagged for sync.
pub const OFFLINE_PLACEHOLDER: &str =
    "*(Behavioral observations unavailable — Claude API was not available during this session. Will be updated on next sync.)*";

/// Placeholder text in the continuity notes section.
pub const OFFLINE_NOTES_PLACEHOLDER: &str =
    "*(Continuity notes unavailable — Claude API was not available during this session. Will be updated on next sync.)*";

/// Marker added after a successful retroactive sync so the parent dashboard can
/// identify synced sessions.
pub const SYNCED_MARKER: &str = "*(Behavioral observations generated retroactively after sync.)*";

/// Find session markdown files that are missing behavioral observations.
///
/// Returns a list of session IDs (filename stems without `.md`) for sessions
/// that contain the offline placeholder text.
pub async fn find_sessions_needing_sync(
    data_dir: &Path,
    learner_id: Uuid,
) -> Vec<String> {
    let dir = crate::session::sessions_dir(data_dir, learner_id);

    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut needs_sync = Vec::new();

    while let Ok(Some(entry)) = entries.next_entry().await {
        let filename = entry.file_name().to_string_lossy().to_string();
        if !filename.ends_with(".md") || !filename.starts_with("session-") {
            continue;
        }

        let path = dir.join(&filename);
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            if content.contains(OFFLINE_PLACEHOLDER) {
                let stem = filename.trim_end_matches(".md").to_string();
                needs_sync.push(stem);
            }
        }
    }

    needs_sync.sort(); // chronological order
    needs_sync
}

/// Attempt to retroactively generate and inject behavioral observations into a
/// session markdown that was written without Claude.
///
/// The content of the existing markdown (assignments, responses, summary) is
/// used as context for the Claude call.  Only the **placeholder lines** are
/// replaced — all session data written during the session is preserved.
///
/// **Callers must hold a write lock on the learner's data before calling this.**
pub async fn sync_session(
    data_dir: &Path,
    learner_id: Uuid,
    session_id: &str,
    client: &ClaudeClient,
) -> Result<(), OfflineError> {
    let path = crate::session::session_markdown_path(data_dir, learner_id, session_id);

    let content = tokio::fs::read_to_string(&path).await?;

    // Skip if this session has already been synced or never needed syncing.
    if !content.contains(OFFLINE_PLACEHOLDER) {
        return Ok(());
    }

    // Build a minimal prompt from the existing markdown content.
    let system = "You are an educational observation assistant. \
        Given a session log of a child's learning session, generate a short, \
        warm behavioral observation paragraph and a brief continuity note. \
        Respond with JSON: \
        {\"behavioralObservations\": \"...\", \"continuityNotes\": \"...\"}. \
        Keep observations positive, growth-mindset focused, and factual. \
        Never use discouraging language. Never compare to other children.";

    let user = format!(
        "Here is the session log. Generate behavioral observations and continuity notes.\n\n{}",
        content
    );

    // Call Claude with a timeout.
    use tokio::time::{timeout, Duration};
    let call_result = timeout(
        Duration::from_secs(30),
        call_claude_raw(client, system, &user),
    )
    .await;

    let raw = match call_result {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => {
            tracing::warn!(
                session_id,
                error = %e,
                "Claude sync call failed — session will remain unsynced"
            );
            return Ok(());
        }
        Err(_) => {
            tracing::warn!(
                session_id,
                "Claude sync call timed out — session will remain unsynced"
            );
            return Ok(());
        }
    };

    // Parse the JSON response.
    let observations_json: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                session_id,
                error = %e,
                "Claude sync response was not valid JSON — session will remain unsynced"
            );
            return Ok(());
        }
    };

    let behavioral_obs = observations_json
        .get("behavioralObservations")
        .and_then(|v| v.as_str())
        .unwrap_or("*(Sync generated: no behavioral observations available.)*");

    let continuity = observations_json
        .get("continuityNotes")
        .and_then(|v| v.as_str())
        .unwrap_or("*(Sync generated: no continuity notes available.)*");

    // Build the replacement text (preserves all existing session data).
    let synced_obs = format!("{}\n\n_{}_", behavioral_obs, SYNCED_MARKER);
    let synced_notes = format!("{}\n\n_{}_", continuity, SYNCED_MARKER);

    let updated = content
        .replace(OFFLINE_PLACEHOLDER, &synced_obs)
        .replace(OFFLINE_NOTES_PLACEHOLDER, &synced_notes);

    tokio::fs::write(&path, updated).await?;

    tracing::info!(
        learner_id = %learner_id,
        session_id,
        "Session synced with retroactive behavioral observations"
    );

    Ok(())
}

/// Call the Claude API directly with a system prompt and user message.
///
/// This is a thin wrapper used by the sync path which needs a raw prompt rather
/// than a typed context struct.
async fn call_claude_raw(
    client: &ClaudeClient,
    system: &str,
    user: &str,
) -> Result<String, crate::claude::ClaudeError> {
    client.call_raw(system, user).await
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_verified_assignment() -> VerifiedAssignment {
        use crate::assignments::VerificationStatus;
        use crate::claude::schemas::GeneratedAssignment;

        VerifiedAssignment {
            assignment: GeneratedAssignment {
                assignment_type: "sequence-puzzle".to_string(),
                skill: "pattern-recognition".to_string(),
                difficulty: 3,
                theme: "math".to_string(),
                prompt: "What comes next: 1, 2, 3, ?".to_string(),
                correct_answer: serde_json::json!(4),
                acceptable_answers: vec![serde_json::json!(4)],
                hints: vec!["Count up".to_string()],
                explanation: "Count by 1".to_string(),
                modality: None,
                verification_data: None,
            },
            verification_status: VerificationStatus::Verified,
            needs_parent_review: false,
            used_fallback: true,
        }
    }

    #[test]
    fn test_buffered_assignment_fresh() {
        let entry = BufferedAssignment {
            generated_at: Utc::now(),
            assignment: make_verified_assignment(),
        };
        assert!(!entry.is_stale(Utc::now()));
    }

    #[test]
    fn test_buffered_assignment_stale() {
        let old = Utc::now() - Duration::days(BUFFER_STALE_DAYS + 1);
        let entry = BufferedAssignment {
            generated_at: old,
            assignment: make_verified_assignment(),
        };
        assert!(entry.is_stale(Utc::now()));
    }

    #[test]
    fn test_buffer_draw_removes_entry() {
        let mut buffer = AssignmentBuffer {
            learner_id: Uuid::new_v4(),
            generated_at: Utc::now(),
            assignments: vec![
                BufferedAssignment {
                    generated_at: Utc::now(),
                    assignment: make_verified_assignment(),
                },
                BufferedAssignment {
                    generated_at: Utc::now(),
                    assignment: make_verified_assignment(),
                },
            ],
        };

        let drawn = buffer.draw();
        assert!(drawn.is_some());
        assert_eq!(buffer.assignments.len(), 1);
    }

    #[test]
    fn test_buffer_draw_from_empty() {
        let mut buffer = AssignmentBuffer::empty(Uuid::new_v4());
        assert!(buffer.draw().is_none());
    }

    #[test]
    fn test_buffer_draw_discards_stale() {
        let old = Utc::now() - Duration::days(BUFFER_STALE_DAYS + 1);
        let mut buffer = AssignmentBuffer {
            learner_id: Uuid::new_v4(),
            generated_at: Utc::now(),
            assignments: vec![BufferedAssignment {
                generated_at: old,
                assignment: make_verified_assignment(),
            }],
        };

        let drawn = buffer.draw();
        assert!(drawn.is_none(), "stale entry should not be drawn");
        assert!(buffer.assignments.is_empty(), "stale entry should be removed");
    }

    #[test]
    fn test_fresh_count_excludes_stale() {
        let old = Utc::now() - Duration::days(BUFFER_STALE_DAYS + 1);
        let buffer = AssignmentBuffer {
            learner_id: Uuid::new_v4(),
            generated_at: Utc::now(),
            assignments: vec![
                BufferedAssignment {
                    generated_at: old,
                    assignment: make_verified_assignment(),
                },
                BufferedAssignment {
                    generated_at: Utc::now(),
                    assignment: make_verified_assignment(),
                },
            ],
        };
        assert_eq!(buffer.fresh_count(), 1);
    }

    #[test]
    fn test_build_buffer_status_empty() {
        let status = build_buffer_status(None, DegradationTier::Template);
        assert_eq!(status.count, 0);
        assert!(status.generated_at.is_none());
        assert!(!status.has_stale_entries);
        assert_eq!(status.current_tier, DegradationTier::Template);
    }

    #[test]
    fn test_build_buffer_status_with_entries() {
        let buffer = AssignmentBuffer {
            learner_id: Uuid::new_v4(),
            generated_at: Utc::now(),
            assignments: vec![BufferedAssignment {
                generated_at: Utc::now(),
                assignment: make_verified_assignment(),
            }],
        };
        let status = build_buffer_status(Some(&buffer), DegradationTier::Buffered);
        assert_eq!(status.count, 1);
        assert!(!status.has_stale_entries);
        assert_eq!(
            status.per_skill.get("pattern-recognition"),
            Some(&1)
        );
    }

    #[tokio::test]
    async fn test_read_buffer_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let result = read_buffer(dir.path(), Uuid::new_v4()).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_write_and_read_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let learner_id = Uuid::new_v4();
        let buffer = AssignmentBuffer {
            learner_id,
            generated_at: Utc::now(),
            assignments: vec![BufferedAssignment {
                generated_at: Utc::now(),
                assignment: make_verified_assignment(),
            }],
        };
        write_buffer(dir.path(), &buffer).await.unwrap();
        let read_back = read_buffer(dir.path(), learner_id).await.unwrap();
        assert_eq!(read_back.assignments.len(), 1);
    }

    #[tokio::test]
    async fn test_read_buffer_corrupt_file_deletes_and_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let learner_id = Uuid::new_v4();
        let path = buffer_path(dir.path(), learner_id);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, b"not valid json!!!").await.unwrap();

        let result = read_buffer(dir.path(), learner_id).await;
        assert!(result.is_none());
        // Corrupt file should be deleted.
        assert!(
            !path.exists(),
            "corrupt buffer file should have been deleted"
        );
    }

    #[tokio::test]
    async fn test_draw_from_buffer_removes_entry() {
        let dir = tempfile::tempdir().unwrap();
        let learner_id = Uuid::new_v4();
        let buffer = AssignmentBuffer {
            learner_id,
            generated_at: Utc::now(),
            assignments: vec![
                BufferedAssignment {
                    generated_at: Utc::now(),
                    assignment: make_verified_assignment(),
                },
                BufferedAssignment {
                    generated_at: Utc::now(),
                    assignment: make_verified_assignment(),
                },
            ],
        };
        write_buffer(dir.path(), &buffer).await.unwrap();

        let drawn = draw_from_buffer(dir.path(), learner_id).await;
        assert!(drawn.is_some());

        // Read the buffer back to confirm one was removed.
        let remaining = read_buffer(dir.path(), learner_id).await.unwrap();
        assert_eq!(remaining.assignments.len(), 1);
    }

    #[tokio::test]
    async fn test_find_sessions_needing_sync_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let learner_id = Uuid::new_v4();
        let result = find_sessions_needing_sync(dir.path(), learner_id).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_find_sessions_needing_sync_detects_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let learner_id = Uuid::new_v4();
        let sessions_dir =
            dir.path().join("learners").join(learner_id.to_string()).join("sessions");
        tokio::fs::create_dir_all(&sessions_dir).await.unwrap();

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

        let synced_content =
            "# Session: 2026-04-06 10:00\n## Behavioral Observations\nGreat focus today!\n";
        tokio::fs::write(
            sessions_dir.join("session-2026-04-06-1000.md"),
            synced_content,
        )
        .await
        .unwrap();

        let needs_sync = find_sessions_needing_sync(dir.path(), learner_id).await;
        assert_eq!(needs_sync.len(), 1);
        assert_eq!(needs_sync[0], "session-2026-04-07-1530");
    }
}
