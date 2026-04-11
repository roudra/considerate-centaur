// Session lifecycle management — session creation, markdown writing, offline buffer.
// See CLAUDE.md "Session Markdown" and "Offline & Resilience" sections.

use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

use crate::claude::prompts::SessionSummary;
use crate::claude::schemas::{GeneratedAssignment, SessionNarrative};
use crate::progress::tracker::{EarnedBadge, LearnerProgress, Metacognition, MetacognitionTrend};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during session operations.
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("Session not found: {0}")]
    NotFound(Uuid),

    #[error("Session is already finished (completed or abandoned)")]
    AlreadyFinished,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Session data types
// ---------------------------------------------------------------------------

/// The lifecycle state of a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionStatus {
    InProgress,
    Completed,
    Abandoned,
}

/// A single assignment that was completed (or attempted) within a session.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAssignment {
    /// The server-assigned assignment ID (key into the pending_assignments store).
    pub assignment_id: String,
    /// The full assignment details (stored server-side, looked up by ID).
    pub assignment: GeneratedAssignment,
    /// The child's raw text response.
    pub child_response: String,
    /// Whether the backend independently determined the answer was correct.
    pub correct: bool,
    /// Time taken to respond, in seconds.
    pub time_seconds: u32,
    /// Number of hints the child used before answering.
    pub hints_used: u32,
    /// Whether the child changed their answer before submitting.
    pub self_corrected: bool,
    /// Optional notes (e.g. from Claude's evaluation reasoning_note).
    pub notes: Option<String>,
}

/// An active session stored server-side in memory.
///
/// The client never holds session state — all session data lives here until
/// `complete_session` or `abandon_session` writes it to disk and removes it
/// from the active store.
#[derive(Clone, Debug)]
pub struct ActiveSession {
    /// Unique server-assigned session ID (UUID).
    pub id: Uuid,
    /// The learner this session belongs to.
    pub learner_id: Uuid,
    /// Wall-clock time when the session was started.
    pub started_at: DateTime<Local>,
    /// The primary skill being practiced (if specified at session start).
    pub focus_skill: Option<String>,
    /// The difficulty level at session start.
    pub focus_level: Option<u32>,
    /// Whether this is a parent co-solve (shared) session.
    pub is_shared: bool,
    /// All assignments attempted so far in this session (in order).
    pub assignments: Vec<SessionAssignment>,
    /// Current lifecycle state.
    pub status: SessionStatus,
}

/// Optional details for a parent co-solve session, included in the markdown.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedSessionInfo {
    /// Description of the parent's observed role (e.g. "guide", "observer").
    pub parent_role: String,
    /// How the child responded to the parent's scaffolding.
    pub child_scaffolding_response: String,
    /// How that compared to how the child responds to system scaffolding.
    pub system_scaffolding_comparison: String,
}

/// Session metadata returned when listing a learner's session history.
///
/// Contains only summary data — no full assignment logs.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadata {
    /// Filename-based ID (e.g. `"session-2026-04-07-1530"`).
    pub session_id: String,
    /// Date of the session in `YYYY-MM-DD` format.
    pub date: String,
    /// Primary focus skill (if specified in the session header).
    pub focus_skill: Option<String>,
    /// Accuracy as a fraction in `0.0..=1.0`.
    pub accuracy: f32,
    /// Total number of assignments in the session.
    pub total_assignments: usize,
    /// Number of assignments answered correctly.
    pub correct_assignments: usize,
}

// ---------------------------------------------------------------------------
// XP and level helpers
// ---------------------------------------------------------------------------

/// XP awarded per assignment based on difficulty and correctness.
///
/// - Correct answer: `difficulty × 10`
/// - Incorrect answer: `difficulty × 5` (partial credit for trying)
pub fn xp_for_assignment(difficulty: u32, correct: bool) -> u32 {
    if correct {
        difficulty * 10
    } else {
        difficulty * 5
    }
}

/// Compute a skill level from accumulated XP.
///
/// Formula: `floor(xp / 100) + 1`, capped at 10.
pub fn level_from_xp(xp: u32) -> u32 {
    (xp / 100 + 1).min(10)
}

// ---------------------------------------------------------------------------
// File path helpers
// ---------------------------------------------------------------------------

/// Build the filename-based session ID from a start timestamp.
///
/// Format: `session-YYYY-MM-DD-HHmm`
pub fn format_session_id(dt: &DateTime<Local>) -> String {
    dt.format("session-%Y-%m-%d-%H%M").to_string()
}

/// Returns the path to the `sessions/` directory for a learner.
///
/// `{data_dir}/learners/{learner_id}/sessions/`
pub fn sessions_dir(data_dir: &Path, learner_id: Uuid) -> PathBuf {
    data_dir
        .join("learners")
        .join(learner_id.to_string())
        .join("sessions")
}

/// Returns the path to a specific session markdown file.
///
/// `{data_dir}/learners/{learner_id}/sessions/{session_id}.md`
pub fn session_markdown_path(data_dir: &Path, learner_id: Uuid, session_id: &str) -> PathBuf {
    sessions_dir(data_dir, learner_id).join(format!("{}.md", session_id))
}

// ---------------------------------------------------------------------------
// Markdown builder
// ---------------------------------------------------------------------------

/// Parameters for building a session markdown file.
///
/// Groups the optional contextual fields that are computed at session-completion
/// time and threaded through to both the pure builder and the async writer.
pub struct SessionMarkdownParams<'a> {
    /// Claude's session narrative, if available.
    pub narrative: Option<&'a SessionNarrative>,
    /// Badges earned in this session.
    pub badges_earned: &'a [EarnedBadge],
    /// XP gained per skill in this session.
    pub xp_by_skill: &'a HashMap<String, u32>,
    /// Difficulty level at session start, if known.
    pub difficulty_before: Option<u32>,
    /// Difficulty level at session end (after adjustment), if known.
    pub difficulty_after: Option<u32>,
    /// Shared session details, if this was a parent co-solve.
    pub shared_info: Option<&'a SharedSessionInfo>,
}

/// Build the complete session markdown string from structured session data.
///
/// The backend assembles this from structured data — Claude provides narrative
/// content as structured output (in `params.narrative`), but the backend is the
/// file author (Constitution §5). This function is pure and side-effect-free.
pub fn build_session_markdown(
    session: &ActiveSession,
    learner_name: &str,
    params: &SessionMarkdownParams<'_>,
) -> String {
    let mut md = String::new();

    // --- Header ---
    let start_str = session.started_at.format("%Y-%m-%d %H:%M");
    md.push_str(&format!("# Session: {}\n", start_str));
    md.push_str(&format!("## Learner: {}\n", learner_name));

    if let Some(skill) = &session.focus_skill {
        let skill_display = title_case_skill(skill);
        if let Some(level) = session.focus_level {
            md.push_str(&format!("## Focus: {} — Level {}\n", skill_display, level));
        } else {
            md.push_str(&format!("## Focus: {}\n", skill_display));
        }
    }

    md.push('\n');

    // --- Individual assignments ---
    for (idx, sa) in session.assignments.iter().enumerate() {
        let assignment_title = title_case_skill(&sa.assignment.assignment_type);
        md.push_str(&format!(
            "### Assignment {}: {}\n",
            idx + 1,
            assignment_title
        ));
        md.push_str(&format!("- **Type**: {}\n", sa.assignment.assignment_type));
        md.push_str(&format!(
            "- **Difficulty**: {}/10\n",
            sa.assignment.difficulty
        ));
        md.push_str(&format!("- **Prompt**: \"{}\"\n", sa.assignment.prompt));
        md.push_str(&format!("- **Response**: {}\n", sa.child_response));
        let result_str = if sa.correct { "correct" } else { "incorrect" };
        md.push_str(&format!("- **Result**: {}\n", result_str));
        md.push_str(&format!("- **Time**: {}s\n", sa.time_seconds));
        if let Some(notes) = &sa.notes {
            md.push_str(&format!("- **Notes**: {}\n", notes));
        }
        md.push('\n');
    }

    // --- Session Summary ---
    let total = session.assignments.len();
    let correct_count = session.assignments.iter().filter(|a| a.correct).count();

    md.push_str("## Session Summary\n");
    md.push_str(&format!("- Correct: {}/{}\n", correct_count, total));

    match (params.difficulty_before, params.difficulty_after) {
        (Some(before), Some(after)) if before != after => {
            let direction = if after > before {
                "trending up"
            } else {
                "trending down"
            };
            md.push_str(&format!(
                "- Difficulty adjustment: {} → {} ({})\n",
                before, after, direction
            ));
        }
        (Some(before), _) => {
            md.push_str(&format!("- Difficulty: {} (maintained)\n", before));
        }
        _ => {}
    }

    if !params.xp_by_skill.is_empty() {
        let mut skill_entries: Vec<String> = params
            .xp_by_skill
            .iter()
            .map(|(skill, xp)| format!("{} (+{}xp)", skill, xp))
            .collect();
        skill_entries.sort(); // deterministic order
        md.push_str(&format!(
            "- Skills practiced: {}\n",
            skill_entries.join(", ")
        ));
    }

    for badge in params.badges_earned {
        md.push_str(&format!("- Badge earned: \"{}\"\n", badge.name));
    }

    md.push('\n');

    // --- Behavioral Observations ---
    md.push_str("## Behavioral Observations\n");
    if let Some(n) = params.narrative {
        md.push_str(&n.behavioral_observations);
        md.push('\n');
    } else {
        md.push_str("*(Behavioral observations unavailable — Claude API was not available during this session. Will be updated on next sync.)*\n");
    }
    md.push('\n');

    // --- Continuity Notes ---
    md.push_str("## Continuity Notes\n");
    if let Some(n) = params.narrative {
        md.push_str(&n.continuity_notes);
        md.push('\n');
    } else {
        md.push_str("*(Continuity notes unavailable — Claude API was not available during this session. Will be updated on next sync.)*\n");
    }
    md.push('\n');

    // --- Shared session section (if applicable) ---
    if session.is_shared {
        md.push_str("## Shared Session: Parent Co-Solve\n");
        md.push_str("- **Mode**: collaborative\n");
        if let Some(shared) = params.shared_info {
            md.push_str(&format!(
                "- **Parent role observed**: {}\n",
                shared.parent_role
            ));
            md.push_str(&format!(
                "- **Child response to parent scaffolding**: {}\n",
                shared.child_scaffolding_response
            ));
            md.push_str(&format!(
                "- **Comparison to system scaffolding**: {}\n",
                shared.system_scaffolding_comparison
            ));
        }
        md.push('\n');
    }

    md
}

/// Write a session markdown file to disk.
///
/// Creates the `sessions/` directory if it does not already exist.
/// Returns the filename-based session ID (e.g. `"session-2026-04-07-1530"`).
///
/// The markdown file is written **before** progress is updated so that session
/// data is never lost even if the progress write fails.
pub async fn write_session_markdown_file(
    data_dir: &Path,
    learner_id: Uuid,
    session: &ActiveSession,
    learner_name: &str,
    params: &SessionMarkdownParams<'_>,
) -> Result<String, SessionError> {
    let session_id = format_session_id(&session.started_at);
    let path = session_markdown_path(data_dir, learner_id, &session_id);

    // Ensure the sessions directory exists.
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let content = build_session_markdown(session, learner_name, params);

    tokio::fs::write(&path, content).await?;

    Ok(session_id)
}

// ---------------------------------------------------------------------------
// Session summary extraction (for Claude context)
// ---------------------------------------------------------------------------

/// Load the most recent session summaries for a learner.
///
/// Scans the `sessions/` directory, reads the last `max_sessions` markdown
/// files (sorted chronologically), and extracts only the **Session Summary**,
/// **Behavioral Observations**, and **Continuity Notes** sections.
///
/// Full assignment logs are intentionally excluded — this is the "summarised"
/// context tier used when building prompts for future Claude calls.
pub async fn load_session_summaries(
    data_dir: &Path,
    learner_id: Uuid,
    max_sessions: usize,
) -> Vec<SessionSummary> {
    let dir = sessions_dir(data_dir, learner_id);

    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut filenames: Vec<String> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".md") && name.starts_with("session-") {
            filenames.push(name);
        }
    }

    // Alphabetical sort == chronological order for the `session-YYYY-MM-DD-HHmm` format.
    filenames.sort();

    // Take the last `max_sessions` files (most recent).
    let recent: Vec<String> = filenames
        .into_iter()
        .rev()
        .take(max_sessions)
        .collect::<Vec<_>>()
        .into_iter()
        .rev() // restore chronological order for the returned summaries
        .collect();

    let mut summaries = Vec::new();
    for filename in &recent {
        let path = dir.join(filename);
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            if let Some(summary) = extract_session_summary(&content, filename) {
                summaries.push(summary);
            }
        }
    }

    summaries
}

/// Extract a [`SessionSummary`] from a session markdown file's content.
///
/// Returns `None` if the filename does not match the expected format or the
/// file lacks the required sections.
fn extract_session_summary(content: &str, filename: &str) -> Option<SessionSummary> {
    // Derive the ISO date from the filename: `session-YYYY-MM-DD-HHmm.md`
    let stem = filename.strip_suffix(".md")?;
    let date = stem.strip_prefix("session-")?.get(..10)?.to_string(); // `YYYY-MM-DD`

    let behavioral_observations = extract_section(content, "## Behavioral Observations")
        .unwrap_or_else(|| {
            "*(No behavioral observations available for this session.)*".to_string()
        });

    let continuity_notes = extract_section(content, "## Continuity Notes")
        .unwrap_or_else(|| "*(No continuity notes available for this session.)*".to_string());

    Some(SessionSummary {
        date,
        behavioral_observations,
        continuity_notes,
    })
}

/// Extract the text content of a `## Section` from a markdown string.
///
/// Returns everything between this header and the next `##`-level header (or
/// end of file), trimmed of leading/trailing whitespace.  Returns `None` if
/// the section header is not found or the extracted content is empty.
fn extract_section(content: &str, header: &str) -> Option<String> {
    let start = content.find(header)?;
    let after_header = &content[start + header.len()..];

    // Skip the newline immediately after the header.
    let body = after_header.trim_start_matches('\n');

    // Find the start of the next `##` section (same or higher level).
    let end = body.find("\n## ").unwrap_or(body.len());

    let section = body[..end].trim().to_string();

    if section.is_empty() {
        None
    } else {
        Some(section)
    }
}

// ---------------------------------------------------------------------------
// Session listing
// ---------------------------------------------------------------------------

/// List completed session files for a learner (most recent first).
///
/// Reads the `sessions/` directory, parses each markdown file's header and
/// summary, and returns a list of [`SessionMetadata`] for the parent dashboard.
pub async fn list_sessions(data_dir: &Path, learner_id: Uuid) -> Vec<SessionMetadata> {
    let dir = sessions_dir(data_dir, learner_id);

    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut filenames: Vec<String> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".md") && name.starts_with("session-") {
            filenames.push(name);
        }
    }

    filenames.sort();
    filenames.reverse(); // Most recent first.

    let mut metadata = Vec::new();
    for filename in &filenames {
        let path = dir.join(filename);
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            if let Some(meta) = parse_session_metadata(&content, filename) {
                metadata.push(meta);
            }
        }
    }

    metadata
}

/// Parse [`SessionMetadata`] from a session markdown file's content.
fn parse_session_metadata(content: &str, filename: &str) -> Option<SessionMetadata> {
    let stem = filename.strip_suffix(".md")?;
    let date = stem.strip_prefix("session-")?.get(..10)?.to_string();

    // Extract focus skill from `## Focus: Sequential Logic — Level 2`
    let focus_skill = content
        .lines()
        .find(|l| l.starts_with("## Focus:"))
        .map(|l| l.trim_start_matches("## Focus:").trim().to_string());

    // Extract accuracy from `- Correct: X/Y`
    let (total_assignments, correct_assignments) = content
        .lines()
        .find(|l| l.trim().starts_with("- Correct:"))
        .and_then(|l| {
            let rest = l.trim().trim_start_matches("- Correct:").trim();
            let parts: Vec<&str> = rest.split('/').collect();
            if parts.len() == 2 {
                let correct: usize = parts[0].trim().parse().ok()?;
                let total: usize = parts[1].trim().parse().ok()?;
                Some((total, correct))
            } else {
                None
            }
        })
        .unwrap_or((0, 0));

    let accuracy = if total_assignments > 0 {
        correct_assignments as f32 / total_assignments as f32
    } else {
        0.0
    };

    Some(SessionMetadata {
        session_id: stem.to_string(),
        date,
        focus_skill,
        accuracy,
        total_assignments,
        correct_assignments,
    })
}

// ---------------------------------------------------------------------------
// Progress update on session completion
// ---------------------------------------------------------------------------

/// Apply a completed session's results to a learner's progress record.
///
/// Updates in order:
/// 1. XP and level for each skill practiced
/// 2. `recentAccuracy` ring buffer for each skill
/// 3. `lastPracticed` dates
/// 4. Spaced-repetition fields via [`crate::progress::update_spaced_repetition`]
/// 5. `total_sessions` and `total_assignments` counters
/// 6. Metacognition signals (EMA of self-correction and hint rates)
/// 7. Streak counter
///
/// Returns a map of skill → XP gained (used to render the session markdown).
pub fn apply_session_to_progress(
    progress: &mut LearnerProgress,
    session: &ActiveSession,
    today: NaiveDate,
) -> HashMap<String, u32> {
    // Capture the previous "last practiced" date before any updates so the
    // streak logic can compare it to today without seeing the newly set dates.
    let previous_last_date = progress
        .skills
        .values()
        .filter_map(|s| s.last_practiced)
        .max();

    let mut xp_by_skill: HashMap<String, u32> = HashMap::new();

    // --- Per-skill updates ---
    for sa in &session.assignments {
        let skill_id = sa.assignment.skill.clone();
        let xp = xp_for_assignment(sa.assignment.difficulty, sa.correct);

        *xp_by_skill.entry(skill_id.clone()).or_insert(0) += xp;

        let skill = progress.skills.entry(skill_id).or_default();

        // XP and level
        skill.xp += xp;
        skill.level = level_from_xp(skill.xp);

        // Accuracy ring buffer and last-practiced date
        skill.record_accuracy(sa.correct);
        skill.last_practiced = Some(today);

        // Spaced repetition (update per skill using the session accuracy for that skill)
        let skill_accuracy = skill.recent_accuracy_fraction();
        crate::progress::update_spaced_repetition(
            &mut skill.spaced_repetition,
            skill_accuracy,
            today,
        );
    }

    // --- Totals ---
    progress.total_sessions += 1;
    progress.total_assignments += session.assignments.len() as u32;

    // --- Metacognition (exponential moving average) ---
    let total = session.assignments.len();
    if total > 0 {
        let self_corrections = session
            .assignments
            .iter()
            .filter(|a| a.self_corrected)
            .count();
        let hint_requests: u32 = session.assignments.iter().map(|a| a.hints_used).sum();

        let session_correction_rate = self_corrections as f32 / total as f32;
        let session_hint_rate = hint_requests as f32 / total as f32;

        // EMA: 70% historical, 30% new session.
        progress.metacognition.self_correction_rate =
            0.7 * progress.metacognition.self_correction_rate + 0.3 * session_correction_rate;
        progress.metacognition.hint_request_rate =
            0.7 * progress.metacognition.hint_request_rate + 0.3 * session_hint_rate;

        // Update metacognition trend heuristic.
        progress.metacognition.trend = infer_metacognition_trend(&progress.metacognition);
    }

    // --- Streak (uses the pre-update date so new lastPracticed = today doesn't
    //     confuse the consecutive-day detection) ---
    update_streak_for_session(progress, previous_last_date, today);

    xp_by_skill
}

/// Infer a metacognition trend from the current rates.
///
/// Heuristic: improving if self-correction rate > 0.2 or hint rate > 0.15,
/// stable otherwise.
fn infer_metacognition_trend(meta: &Metacognition) -> MetacognitionTrend {
    if meta.self_correction_rate >= 0.2 || meta.hint_request_rate >= 0.15 {
        MetacognitionTrend::Improving
    } else {
        MetacognitionTrend::Stable
    }
}

/// Update the streak counters for a completed session.
///
/// Uses `previous_last_date` (the most recent `lastPracticed` value from before
/// this session's updates) to determine whether today is consecutive.
///
/// - No previous date → first session; streak starts at 1
/// - Gap == 0 (same day as last session) → no change  
/// - Gap == 1 (consecutive day) → increment streak
/// - Gap > 1 → reset streak to 1
fn update_streak_for_session(
    progress: &mut LearnerProgress,
    previous_last_date: Option<NaiveDate>,
    today: NaiveDate,
) {
    match previous_last_date {
        None => {
            // First ever session — start streak at 1.
            progress.streaks.current_days = 1;
        }
        Some(last) => {
            let gap = today.signed_duration_since(last).num_days();
            if gap == 0 {
                // Multiple sessions today — streak already counted.
            } else if gap == 1 {
                // Consecutive day — increment streak.
                progress.streaks.current_days += 1;
            } else {
                // Gap in sessions — reset streak to 1.
                progress.streaks.current_days = 1;
            }
        }
    }

    if progress.streaks.current_days > progress.streaks.longest_days {
        progress.streaks.longest_days = progress.streaks.current_days;
    }
}

/// Update `observedBehavior` fields in the learner profile based on emerging
/// patterns from the metacognition data.
///
/// Only updates fields that remain `Unknown` — once the system has observed a
/// pattern it does not regress. Called by the session completion flow.
pub fn update_observed_behavior(
    behavior: &mut crate::learner::profile::ObservedBehavior,
    progress: &LearnerProgress,
    session: &ActiveSession,
) {
    use crate::learner::profile::HintUsage;

    // --- Hint usage pattern ---
    if behavior.hint_usage == crate::learner::profile::HintUsage::Unknown {
        let hint_rate = progress.metacognition.hint_request_rate;
        behavior.hint_usage = if hint_rate > 0.3 {
            HintUsage::Proactive
        } else if hint_rate < 0.05 {
            HintUsage::Avoidant
        } else {
            HintUsage::Reactive
        };
    }

    // --- Frustration response: detect "rushes" from rapid incorrect answers ---
    if behavior.frustration_response == crate::learner::profile::FrustrationResponse::Unknown
        && session.status == SessionStatus::Abandoned
    {
        // Abandoned sessions without attempting many assignments may indicate disengagement.
        if session.assignments.len() < 2 {
            behavior.frustration_response =
                crate::learner::profile::FrustrationResponse::Disengages;
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Convert a kebab-case skill or type ID to Title Case for display.
///
/// e.g. `"pattern-recognition"` → `"Pattern Recognition"`
fn title_case_skill(id: &str) -> String {
    id.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_assignment(skill: &str, difficulty: u32, correct: bool) -> SessionAssignment {
        SessionAssignment {
            assignment_id: Uuid::new_v4().to_string(),
            assignment: GeneratedAssignment {
                assignment_type: "sequence-puzzle".to_string(),
                skill: skill.to_string(),
                difficulty,
                theme: "dinosaurs".to_string(),
                prompt: "What comes next: 2, 4, 8, ?".to_string(),
                correct_answer: serde_json::json!(16),
                acceptable_answers: vec![serde_json::json!(16)],
                hints: vec![
                    "Look at how the number changes...".to_string(),
                    "Each time, the number doubles!".to_string(),
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
            time_seconds: 45,
            hints_used: 0,
            self_corrected: false,
            notes: None,
        }
    }

    fn sample_session(learner_id: Uuid) -> ActiveSession {
        ActiveSession {
            id: Uuid::new_v4(),
            learner_id,
            started_at: Local.with_ymd_and_hms(2026, 4, 7, 15, 30, 0).unwrap(),
            focus_skill: Some("sequential-logic".to_string()),
            focus_level: Some(2),
            is_shared: false,
            assignments: vec![
                sample_assignment("sequential-logic", 3, true),
                sample_assignment("sequential-logic", 3, true),
                sample_assignment("pattern-recognition", 2, false),
                sample_assignment("sequential-logic", 3, true),
                sample_assignment("sequential-logic", 3, true),
            ],
            status: SessionStatus::Completed,
        }
    }

    // --- Helpers ---

    #[test]
    fn test_format_session_id() {
        let dt = Local.with_ymd_and_hms(2026, 4, 7, 15, 30, 0).unwrap();
        assert_eq!(format_session_id(&dt), "session-2026-04-07-1530");
    }

    #[test]
    fn test_title_case_skill() {
        assert_eq!(
            title_case_skill("pattern-recognition"),
            "Pattern Recognition"
        );
        assert_eq!(title_case_skill("sequential-logic"), "Sequential Logic");
        assert_eq!(title_case_skill("spatial-reasoning"), "Spatial Reasoning");
    }

    #[test]
    fn test_xp_for_assignment() {
        assert_eq!(xp_for_assignment(3, true), 30);
        assert_eq!(xp_for_assignment(3, false), 15);
        assert_eq!(xp_for_assignment(5, true), 50);
    }

    #[test]
    fn test_level_from_xp() {
        assert_eq!(level_from_xp(0), 1);
        assert_eq!(level_from_xp(99), 1);
        assert_eq!(level_from_xp(100), 2);
        assert_eq!(level_from_xp(900), 10);
        assert_eq!(level_from_xp(1000), 10); // capped at 10
    }

    // --- Markdown builder ---

    #[test]
    fn test_build_session_markdown_all_sections_present() {
        let learner_id = Uuid::new_v4();
        let session = sample_session(learner_id);
        let badges: Vec<EarnedBadge> = vec![];
        let xp_by_skill: HashMap<String, u32> = [
            ("sequential-logic".to_string(), 120u32),
            ("pattern-recognition".to_string(), 10u32),
        ]
        .into();

        let params = SessionMarkdownParams {
            narrative: None,
            badges_earned: &badges,
            xp_by_skill: &xp_by_skill,
            difficulty_before: Some(3),
            difficulty_after: Some(4),
            shared_info: None,
        };
        let md = build_session_markdown(&session, "StarExplorer42", &params);

        assert!(md.contains("# Session: 2026-04-07 15:30"), "header missing");
        assert!(md.contains("## Learner: StarExplorer42"), "learner missing");
        assert!(
            md.contains("## Focus: Sequential Logic — Level 2"),
            "focus missing"
        );
        assert!(md.contains("### Assignment 1:"), "assignment 1 missing");
        assert!(md.contains("### Assignment 5:"), "assignment 5 missing");
        assert!(md.contains("## Session Summary"), "summary section missing");
        assert!(md.contains("- Correct: 4/5"), "accuracy missing");
        assert!(
            md.contains("## Behavioral Observations"),
            "behavioral obs missing"
        );
        assert!(
            md.contains("## Continuity Notes"),
            "continuity notes missing"
        );
        // No shared session section because is_shared = false
        assert!(
            !md.contains("## Shared Session"),
            "shared section should be absent"
        );
    }

    #[test]
    fn test_build_session_markdown_with_narrative() {
        let learner_id = Uuid::new_v4();
        let session = sample_session(learner_id);
        let narrative = SessionNarrative {
            behavioral_observations: "Child stayed engaged throughout.".to_string(),
            continuity_notes: "Ready for 3-step problems.".to_string(),
            recommended_focus_areas: vec!["sequential-logic".to_string()],
            difficulty_adjustment: crate::claude::schemas::DifficultyAdjustment::Increase,
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
        let md = build_session_markdown(&session, "StarExplorer42", &params);

        assert!(md.contains("Child stayed engaged throughout."));
        assert!(md.contains("Ready for 3-step problems."));
    }

    #[test]
    fn test_build_session_markdown_shared_session() {
        let learner_id = Uuid::new_v4();
        let mut session = sample_session(learner_id);
        session.is_shared = true;

        let shared_info = SharedSessionInfo {
            parent_role: "guide (let child lead)".to_string(),
            child_scaffolding_response: "positive".to_string(),
            system_scaffolding_comparison: "child more willing to take risks with parent"
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
        let md = build_session_markdown(&session, "StarExplorer42", &params);

        assert!(md.contains("## Shared Session: Parent Co-Solve"));
        assert!(md.contains("- **Mode**: collaborative"));
        assert!(md.contains("guide (let child lead)"));
    }

    #[test]
    fn test_build_session_markdown_missing_narrative_placeholder() {
        let learner_id = Uuid::new_v4();
        let session = sample_session(learner_id);

        let params = SessionMarkdownParams {
            narrative: None, // no narrative — Claude unavailable
            badges_earned: &[],
            xp_by_skill: &HashMap::new(),
            difficulty_before: None,
            difficulty_after: None,
            shared_info: None,
        };
        let md = build_session_markdown(&session, "StarExplorer42", &params);

        assert!(
            md.contains("unavailable"),
            "should note observations are unavailable"
        );
    }

    // --- Section extractor ---

    #[test]
    fn test_extract_section_found() {
        let content = "# Session\n\n## Behavioral Observations\nChild was engaged.\n\n## Continuity Notes\nReady for next level.\n";
        let obs = extract_section(content, "## Behavioral Observations");
        assert_eq!(obs, Some("Child was engaged.".to_string()));
    }

    #[test]
    fn test_extract_section_last_in_file() {
        let content = "# Session\n\n## Continuity Notes\nReady for 3-step.\n";
        let notes = extract_section(content, "## Continuity Notes");
        assert_eq!(notes, Some("Ready for 3-step.".to_string()));
    }

    #[test]
    fn test_extract_section_not_found() {
        let content = "# Session\n\n## Some Other Section\nContent.\n";
        let obs = extract_section(content, "## Behavioral Observations");
        assert_eq!(obs, None);
    }

    // --- Session summary extraction ---

    #[test]
    fn test_extract_session_summary_parses_date() {
        let content = "# Session: 2026-04-07 15:30\n\n## Behavioral Observations\nEngaged.\n\n## Continuity Notes\nNext level.\n";
        let summary = extract_session_summary(content, "session-2026-04-07-1530.md");
        assert!(summary.is_some());
        let s = summary.unwrap();
        assert_eq!(s.date, "2026-04-07");
        assert_eq!(s.behavioral_observations, "Engaged.");
        assert_eq!(s.continuity_notes, "Next level.");
    }

    // --- Progress update ---

    #[test]
    fn test_apply_session_to_progress_xp() {
        use crate::progress::tracker::LearnerProgress;

        let learner_id = Uuid::new_v4();
        let mut progress = LearnerProgress::default_for(learner_id);
        let session = sample_session(learner_id);
        let today = NaiveDate::from_ymd_opt(2026, 4, 7).unwrap();

        let xp_map = apply_session_to_progress(&mut progress, &session, today);

        // sequential-logic: 4 correct × diff 3 × 10 = 120 XP
        assert_eq!(*xp_map.get("sequential-logic").unwrap_or(&0), 120);
        // pattern-recognition: 1 incorrect × diff 2 × 5 = 10 XP
        assert_eq!(*xp_map.get("pattern-recognition").unwrap_or(&0), 10);

        assert_eq!(progress.total_sessions, 1);
        assert_eq!(progress.total_assignments, 5);
    }

    #[test]
    fn test_apply_session_to_progress_streak_first_session() {
        use crate::progress::tracker::LearnerProgress;

        let learner_id = Uuid::new_v4();
        let mut progress = LearnerProgress::default_for(learner_id);
        let session = sample_session(learner_id);
        let today = NaiveDate::from_ymd_opt(2026, 4, 7).unwrap();

        apply_session_to_progress(&mut progress, &session, today);

        assert_eq!(progress.streaks.current_days, 1);
        assert_eq!(progress.streaks.longest_days, 1);
    }

    #[test]
    fn test_apply_session_to_progress_streak_consecutive() {
        use crate::progress::tracker::LearnerProgress;

        let learner_id = Uuid::new_v4();
        let mut progress = LearnerProgress::default_for(learner_id);

        // First session yesterday.
        let yesterday = NaiveDate::from_ymd_opt(2026, 4, 6).unwrap();
        let mut session1 = sample_session(learner_id);
        session1.started_at = Local.with_ymd_and_hms(2026, 4, 6, 15, 0, 0).unwrap();
        apply_session_to_progress(&mut progress, &session1, yesterday);

        // Second session today.
        let today = NaiveDate::from_ymd_opt(2026, 4, 7).unwrap();
        let session2 = sample_session(learner_id);
        apply_session_to_progress(&mut progress, &session2, today);

        assert_eq!(progress.streaks.current_days, 2);
        assert_eq!(progress.streaks.longest_days, 2);
    }

    #[test]
    fn test_apply_session_to_progress_streak_gap_resets() {
        use crate::progress::tracker::LearnerProgress;

        let learner_id = Uuid::new_v4();
        let mut progress = LearnerProgress::default_for(learner_id);

        // First session 5 days ago.
        let five_days_ago = NaiveDate::from_ymd_opt(2026, 4, 2).unwrap();
        let mut session1 = sample_session(learner_id);
        session1.started_at = Local.with_ymd_and_hms(2026, 4, 2, 15, 0, 0).unwrap();
        apply_session_to_progress(&mut progress, &session1, five_days_ago);
        progress.streaks.current_days = 5; // simulate a streak built up

        // Session today (gap > 1 day → reset).
        let today = NaiveDate::from_ymd_opt(2026, 4, 7).unwrap();
        let session2 = sample_session(learner_id);
        apply_session_to_progress(&mut progress, &session2, today);

        assert_eq!(progress.streaks.current_days, 1);
    }

    #[test]
    fn test_parse_session_metadata() {
        let content = "# Session: 2026-04-07 15:30\n## Learner: StarExplorer42\n## Focus: Sequential Logic — Level 2\n\n### Assignment 1: Sequence Puzzle\n- **Type**: sequence-puzzle\n\n## Session Summary\n- Correct: 4/5\n\n## Behavioral Observations\n...\n## Continuity Notes\n...\n";
        let meta = parse_session_metadata(content, "session-2026-04-07-1530.md");
        assert!(meta.is_some());
        let m = meta.unwrap();
        assert_eq!(m.session_id, "session-2026-04-07-1530");
        assert_eq!(m.date, "2026-04-07");
        assert_eq!(m.total_assignments, 5);
        assert_eq!(m.correct_assignments, 4);
        assert!((m.accuracy - 0.8).abs() < f32::EPSILON);
    }
}
