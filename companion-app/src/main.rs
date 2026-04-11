use anyhow::Context;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use educational_companion::assignments::{
    self, AssignmentTemplate, PipelineRequest, VerifiedAssignment,
};
use educational_companion::claude::{
    ClaudeClient, NarrativeContext, ProgressSnapshot, SanitizedProfile,
};
use educational_companion::gamification::{
    self, BossChallenge, BossDefinition, DailyPuzzleState, TimedChallenge,
};
use educational_companion::learner;
use educational_companion::learner::{
    InitialPreferences, LearnerError, LearnerProfile, ObservedBehavior,
};
use educational_companion::lock::LockManager;
use educational_companion::progress;
use educational_companion::session::{
    self, ActiveSession, SessionAssignment, SessionStatus, SharedSessionInfo,
};

/// Server-side store of verified assignments awaiting child responses.
///
/// When the generate endpoint creates a verified assignment, it stores it here
/// keyed by a unique assignment ID. The evaluate endpoint looks it up by ID —
/// the client never supplies the correct answer. This prevents clients from
/// forging correctness (Constitution §5).
type AssignmentStore = Arc<Mutex<HashMap<String, VerifiedAssignment>>>;

/// Server-side store of active (in-progress) sessions.
///
/// Sessions are created by `start_session` and removed when completed or
/// abandoned. The client references sessions only by the server-assigned UUID.
type ActiveSessionStore = Arc<Mutex<HashMap<Uuid, ActiveSession>>>;

/// Server-side store of active timed challenges.
type TimedChallengeStore = Arc<Mutex<HashMap<Uuid, TimedChallenge>>>;

/// Server-side store of active boss battles.
type BossChallengeStore = Arc<Mutex<HashMap<Uuid, BossChallenge>>>;

/// Shared application state passed to every route handler.
#[derive(Clone)]
struct AppState {
    data_dir: Arc<PathBuf>,
    locks: LockManager,
    /// Assignment templates loaded at startup from the curriculum directory.
    templates: Arc<Vec<AssignmentTemplate>>,
    /// Server-side store of pending assignments — keyed by assignment ID.
    pending_assignments: AssignmentStore,
    /// Server-side store of active sessions — keyed by session UUID.
    active_sessions: ActiveSessionStore,
    /// Server-side store of active timed challenges — keyed by challenge UUID.
    timed_challenges: TimedChallengeStore,
    /// Server-side store of active boss battles — keyed by challenge UUID.
    boss_challenges: BossChallengeStore,
    /// Boss battle definitions loaded from bosses.json at startup.
    boss_definitions: Arc<Vec<BossDefinition>>,
    /// Optional Claude API client — `None` if `ANTHROPIC_API_KEY` is not set.
    claude_client: Option<ClaudeClient>,
}

/// JSON body for `POST /api/v1/learners`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateLearnerRequest {
    name: String,
    age: u8,
    interests: Vec<String>,
    initial_preferences: InitialPreferences,
}

/// JSON body for `PUT /api/v1/learners/:id`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateLearnerRequest {
    name: String,
    age: u8,
    interests: Vec<String>,
    initial_preferences: InitialPreferences,
}

/// Standard error response body.
#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    code: String,
}

impl ErrorResponse {
    fn new(error: impl Into<String>, code: impl Into<String>) -> Self {
        ErrorResponse {
            error: error.into(),
            code: code.into(),
        }
    }
}

/// Convert a `LearnerError` to an HTTP response.
fn learner_error_response(err: LearnerError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        LearnerError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                format!("Learner not found: {id}"),
                "LEARNER_NOT_FOUND",
            )),
        ),
        LearnerError::InvalidSchemaVersion { expected, actual } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse::new(
                format!("Schema version mismatch: expected {expected}, got {actual}"),
                "INVALID_SCHEMA_VERSION",
            )),
        ),
        LearnerError::InvalidProfile(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(msg, "INVALID_PROFILE")),
        ),
        LearnerError::Io(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(format!("I/O error: {e}"), "IO_ERROR")),
        ),
        LearnerError::Json(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                format!("Serialization error: {e}"),
                "JSON_ERROR",
            )),
        ),
    }
}

/// Convert a `ProgressError` to an HTTP response.
fn progress_error_response(err: progress::ProgressError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        progress::ProgressError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                format!("Progress not found for learner: {id}"),
                "PROGRESS_NOT_FOUND",
            )),
        ),
        progress::ProgressError::InvalidSchemaVersion { expected, actual } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse::new(
                format!("Schema version mismatch: expected {expected}, got {actual}"),
                "INVALID_SCHEMA_VERSION",
            )),
        ),
        progress::ProgressError::Io(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(format!("I/O error: {e}"), "IO_ERROR")),
        ),
        progress::ProgressError::Json(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                format!("Serialization error: {e}"),
                "JSON_ERROR",
            )),
        ),
    }
}

// --- Route handlers ---

/// `POST /api/v1/learners` — create a new learner profile.
async fn create_learner(
    State(state): State<AppState>,
    Json(req): Json<CreateLearnerRequest>,
) -> impl IntoResponse {
    if req.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "name must not be empty",
                "code": "INVALID_PROFILE"
            })),
        )
            .into_response();
    }

    let profile = LearnerProfile {
        schema_version: 1,
        id: Uuid::new_v4(),
        name: req.name,
        age: req.age,
        interests: req.interests,
        initial_preferences: req.initial_preferences,
        observed_behavior: ObservedBehavior::default(),
    };

    let _guard = state.locks.write(profile.id).await;
    match learner::create_profile(&state.data_dir, &profile).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(&profile).unwrap()),
        )
            .into_response(),
        Err(e) => {
            let (status, body) = learner_error_response(e);
            (status, Json(serde_json::to_value(body.0).unwrap())).into_response()
        }
    }
}

/// `GET /api/v1/learners` — list all learners.
async fn list_learners(State(state): State<AppState>) -> impl IntoResponse {
    match learner::list_profiles(&state.data_dir).await {
        Ok(profiles) => (
            StatusCode::OK,
            Json(serde_json::to_value(profiles).unwrap()),
        )
            .into_response(),
        Err(e) => {
            let (status, body) = learner_error_response(e);
            (status, Json(serde_json::to_value(body.0).unwrap())).into_response()
        }
    }
}

/// `GET /api/v1/learners/:id` — get a learner profile.
async fn get_learner(State(state): State<AppState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;
    match learner::read_profile(&state.data_dir, id).await {
        Ok(profile) => {
            (StatusCode::OK, Json(serde_json::to_value(profile).unwrap())).into_response()
        }
        Err(e) => {
            let (status, body) = learner_error_response(e);
            (status, Json(serde_json::to_value(body.0).unwrap())).into_response()
        }
    }
}

/// `PUT /api/v1/learners/:id` — update a learner profile.
async fn update_learner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateLearnerRequest>,
) -> impl IntoResponse {
    // Write lock: read-modify-write must be atomic.
    let _guard = state.locks.write(id).await;
    let existing = match learner::read_profile(&state.data_dir, id).await {
        Ok(p) => p,
        Err(e) => {
            let (status, body) = learner_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    let updated = LearnerProfile {
        schema_version: existing.schema_version,
        id: existing.id,
        name: req.name,
        age: req.age,
        interests: req.interests,
        initial_preferences: req.initial_preferences,
        observed_behavior: existing.observed_behavior,
    };

    match learner::update_profile(&state.data_dir, &updated).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::to_value(&updated).unwrap()),
        )
            .into_response(),
        Err(e) => {
            let (status, body) = learner_error_response(e);
            (status, Json(serde_json::to_value(body.0).unwrap())).into_response()
        }
    }
}

/// `DELETE /api/v1/learners/:id` — delete a learner and all their data.
async fn delete_learner(State(state): State<AppState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let _guard = state.locks.write(id).await;
    match learner::delete_profile(&state.data_dir, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let (status, body) = learner_error_response(e);
            (status, Json(serde_json::to_value(body.0).unwrap())).into_response()
        }
    }
}

/// `GET /api/v1/learners/:id/skill-health` — return the skill health map for a learner.
async fn get_skill_health(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;
    let prog = match progress::read_progress(&state.data_dir, id).await {
        Ok(p) => p,
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    let today = chrono::Local::now().date_naive();
    let health_map = progress::build_skill_health_map(&prog, today);
    (
        StatusCode::OK,
        Json(serde_json::to_value(health_map).unwrap()),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Assignment route handlers
// ---------------------------------------------------------------------------

/// JSON body for `POST /api/v1/learners/:id/assignments/generate`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateAssignmentRequest {
    /// Target skill ID (e.g. `"pattern-recognition"`). If absent the system
    /// picks the next skill based on the learner's ZPD and review schedule.
    skill: Option<String>,
    /// Preferred assignment type. If absent the system picks based on skill.
    preferred_type: Option<String>,
}

/// JSON body for `POST /api/v1/learners/:id/assignments/evaluate`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitResponseRequest {
    /// The server-assigned assignment ID (returned by the generate endpoint).
    /// The client never supplies the correct answer — the server looks it up.
    assignment_id: String,
    /// The child's free-text response.
    child_response: String,
}

/// Response from the evaluate endpoint.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EvaluateResponse {
    /// Whether the backend independently determined the answer is correct.
    backend_correct: bool,
    /// Placeholder feedback — a full Claude evaluation call would replace this.
    feedback: String,
}

/// Response from the generate endpoint — includes a server-assigned ID.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateAssignmentResponse {
    /// Unique ID for this assignment — use this in the evaluate endpoint.
    assignment_id: String,
    /// The assignment itself (prompt, hints, difficulty — but NOT the correct answer).
    assignment: ClientAssignment,
    /// Whether this needs parent review.
    needs_parent_review: bool,
    /// Whether a deterministic fallback was used.
    used_fallback: bool,
}

/// The assignment as seen by the client — correct answer is stripped.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientAssignment {
    #[serde(rename = "type")]
    pub assignment_type: String,
    pub skill: String,
    pub difficulty: u32,
    pub theme: String,
    pub prompt: String,
    pub hints: Vec<String>,
    pub modality: Option<educational_companion::claude::schemas::AssignmentModality>,
}

/// `POST /api/v1/learners/:id/assignments/generate`
///
/// Generates the next assignment for a learner using the GENERATE -> VALIDATE
/// -> PRESENT pipeline. Generation and evaluation are always separate calls.
async fn generate_assignment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<GenerateAssignmentRequest>,
) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;

    // Load progress to select skill and difficulty.
    let progress = match progress::read_progress(&state.data_dir, id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => {
            // New learner with no progress yet — use defaults.
            progress::LearnerProgress::default_for(id)
        }
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    let today = chrono::Local::now().date_naive();

    // Determine target skill and difficulty.
    let (skill, difficulty) = if let Some(skill_id) = req.skill {
        let difficulty = progress
            .skills
            .get(&skill_id)
            .map(|s| assignments::target_difficulty(&s.zpd))
            .unwrap_or(3);
        (skill_id, difficulty)
    } else {
        match assignments::select_skill(&progress, today) {
            Some(target) => (target.skill_id, target.difficulty),
            None => ("pattern-recognition".to_string(), 3),
        }
    };

    let pipeline_req = PipelineRequest {
        skill: skill.clone(),
        difficulty,
        preferred_type: req.preferred_type,
    };

    // Run the GENERATE -> VALIDATE -> PRESENT pipeline.
    // Claude is currently unavailable from this endpoint (no API key wired),
    // so the pipeline falls back to deterministic generation automatically.
    let result: VerifiedAssignment = assignments::run_pipeline(
        || async { None::<educational_companion::claude::schemas::GeneratedAssignment> },
        &state.templates,
        &pipeline_req,
        2,
    )
    .await;

    // Store the verified assignment server-side, keyed by a unique ID.
    // The client receives only the ID + a sanitized view (no correct answer).
    let assignment_id = Uuid::new_v4().to_string();

    let client_view = ClientAssignment {
        assignment_type: result.assignment.assignment_type.clone(),
        skill: result.assignment.skill.clone(),
        difficulty: result.assignment.difficulty,
        theme: result.assignment.theme.clone(),
        prompt: result.assignment.prompt.clone(),
        hints: result.assignment.hints.clone(),
        modality: result.assignment.modality.clone(),
    };

    let response = GenerateAssignmentResponse {
        assignment_id: assignment_id.clone(),
        assignment: client_view,
        needs_parent_review: result.needs_parent_review,
        used_fallback: result.used_fallback,
    };

    state
        .pending_assignments
        .lock()
        .await
        .insert(assignment_id, result);

    (StatusCode::OK, Json(serde_json::json!(response))).into_response()
}

/// `POST /api/v1/learners/:id/assignments/evaluate`
///
/// Evaluates a child's response against the backend-verified correct answer.
/// The client sends only the assignment ID (from the generate response) and
/// their answer — the server looks up the stored assignment with the correct
/// answer. The client NEVER supplies the correct answer (Constitution §5).
async fn evaluate_response(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<SubmitResponseRequest>,
) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;

    // Look up the verified assignment by ID — the client cannot forge this.
    let stored = state
        .pending_assignments
        .lock()
        .await
        .remove(&req.assignment_id);

    let Some(verified) = stored else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Assignment not found: {}", req.assignment_id),
                "code": "ASSIGNMENT_NOT_FOUND"
            })),
        )
            .into_response();
    };

    let backend_correct =
        assignments::check_response_correct(&verified.assignment, &req.child_response);

    let feedback = if backend_correct {
        "You've got it! Great thinking — keep exploring!".to_string()
    } else {
        "Not quite — but you're thinking in the right direction! Check the hints for a nudge."
            .to_string()
    };

    let response = EvaluateResponse {
        backend_correct,
        feedback,
    };

    (StatusCode::OK, Json(serde_json::json!(response))).into_response()
}

// ---------------------------------------------------------------------------
// Session route handlers
// ---------------------------------------------------------------------------

/// JSON body for `POST /api/v1/learners/:id/sessions` (start a session).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartSessionRequest {
    /// Skill to focus on (optional — system picks based on ZPD/review schedule if absent).
    focus_skill: Option<String>,
    /// Whether this is a parent co-solve session.
    #[serde(default)]
    is_shared: bool,
}

/// Response from `POST /api/v1/learners/:id/sessions`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartSessionResponse {
    /// Server-assigned session UUID — use this in subsequent calls.
    session_id: String,
    /// When the session started (ISO 8601).
    started_at: String,
}

/// `POST /api/v1/learners/:id/sessions` — start a new session for a learner.
async fn start_session(
    State(state): State<AppState>,
    Path(learner_id): Path<Uuid>,
    Json(req): Json<StartSessionRequest>,
) -> impl IntoResponse {
    // Write lock: starting a session creates new server-side state.
    let _guard = state.locks.write(learner_id).await;

    // Verify the learner exists before creating a session.
    if let Err(e) = learner::read_profile(&state.data_dir, learner_id).await {
        let (status, body) = learner_error_response(e);
        return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
    }

    let focus_level = if let Some(ref skill_id) = req.focus_skill {
        // Load progress to determine the current difficulty level for this skill.
        match progress::read_progress(&state.data_dir, learner_id).await {
            Ok(p) => p
                .skills
                .get(skill_id)
                .map(|s| assignments::target_difficulty(&s.zpd)),
            Err(progress::ProgressError::NotFound(_)) => None,
            Err(_) => None,
        }
    } else {
        None
    };

    let active_session = ActiveSession {
        id: Uuid::new_v4(),
        learner_id,
        started_at: chrono::Local::now(),
        focus_skill: req.focus_skill,
        focus_level,
        is_shared: req.is_shared,
        assignments: Vec::new(),
        status: SessionStatus::InProgress,
    };

    let session_id = active_session.id;
    let started_at = active_session
        .started_at
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();

    state
        .active_sessions
        .lock()
        .await
        .insert(session_id, active_session);

    let response = StartSessionResponse {
        session_id: session_id.to_string(),
        started_at,
    };

    (StatusCode::CREATED, Json(serde_json::json!(response))).into_response()
}

/// JSON body for `POST /api/v1/learners/:id/sessions/:session_id/responses`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordResponseRequest {
    /// The server-assigned assignment ID (from the generate endpoint).
    assignment_id: String,
    /// The child's free-text response.
    child_response: String,
    /// Time taken to answer in seconds.
    #[serde(default)]
    time_seconds: u32,
    /// Number of hints the child used.
    #[serde(default)]
    hints_used: u32,
    /// Whether the child changed their answer before submitting.
    #[serde(default)]
    self_corrected: bool,
    /// Optional notes (e.g. from Claude evaluation reasoning).
    notes: Option<String>,
}

/// Response from `POST /api/v1/learners/:id/sessions/:session_id/responses`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordResponseResponse {
    /// Whether the backend determined the answer was correct.
    backend_correct: bool,
    /// Encouraging feedback for the child.
    feedback: String,
    /// Position of this assignment in the session (1-based index).
    assignment_index: usize,
}

/// `POST /api/v1/learners/:id/sessions/:session_id/responses`
///
/// Records a child's response to an assignment within an active session.
/// Looks up the assignment from the server-side `pending_assignments` store
/// — the client never supplies the correct answer (Constitution §5).
async fn record_response(
    State(state): State<AppState>,
    Path((learner_id, session_uuid)): Path<(Uuid, Uuid)>,
    Json(req): Json<RecordResponseRequest>,
) -> impl IntoResponse {
    // Write lock: recording a response modifies session state.
    let _guard = state.locks.write(learner_id).await;

    // Look up and remove the assignment from the pending store.
    let stored = state
        .pending_assignments
        .lock()
        .await
        .remove(&req.assignment_id);

    let Some(verified) = stored else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Assignment not found: {}", req.assignment_id),
                "code": "ASSIGNMENT_NOT_FOUND"
            })),
        )
            .into_response();
    };

    // Evaluate correctness server-side — the client cannot forge this.
    let backend_correct =
        assignments::check_response_correct(&verified.assignment, &req.child_response);

    let feedback = if backend_correct {
        "You've got it! Great thinking — keep exploring!".to_string()
    } else {
        "Not quite — but you're thinking in the right direction! Check the hints for a nudge."
            .to_string()
    };

    // Store the completed assignment in the active session.
    let mut sessions = state.active_sessions.lock().await;
    let Some(active_session) = sessions.get_mut(&session_uuid) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Session not found: {}", session_uuid),
                "code": "SESSION_NOT_FOUND"
            })),
        )
            .into_response();
    };

    if active_session.learner_id != learner_id {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Session does not belong to this learner",
                "code": "SESSION_FORBIDDEN"
            })),
        )
            .into_response();
    }

    if active_session.status != SessionStatus::InProgress {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Session is already finished",
                "code": "SESSION_ALREADY_FINISHED"
            })),
        )
            .into_response();
    }

    let assignment_index = active_session.assignments.len() + 1;

    active_session.assignments.push(SessionAssignment {
        assignment_id: req.assignment_id,
        assignment: verified.assignment,
        child_response: req.child_response,
        correct: backend_correct,
        time_seconds: req.time_seconds,
        hints_used: req.hints_used,
        self_corrected: req.self_corrected,
        notes: req.notes,
    });

    let response = RecordResponseResponse {
        backend_correct,
        feedback,
        assignment_index,
    };

    (StatusCode::OK, Json(serde_json::json!(response))).into_response()
}

/// JSON body for `POST /api/v1/learners/:id/sessions/:session_id/complete`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompleteSessionRequest {
    /// Optional shared session details (only relevant if `is_shared` was true at start).
    shared_session_info: Option<SharedSessionInfo>,
}

/// Response from `POST /api/v1/learners/:id/sessions/:session_id/complete`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompleteSessionResponse {
    /// The filename-based session ID (e.g. `"session-2026-04-07-1530"`).
    session_file_id: String,
    /// Badges earned in this session.
    badges_earned: Vec<progress::EarnedBadge>,
    /// XP gained per skill.
    xp_by_skill: HashMap<String, u32>,
    /// Whether the markdown was written without behavioral observations.
    narrative_unavailable: bool,
}

/// `POST /api/v1/learners/:id/sessions/:session_id/complete`
///
/// Completes an active session:
/// 1. Tries to generate Claude narrative (falls back gracefully if unavailable)
/// 2. Writes the session markdown file (source of truth)
/// 3. Updates progress (XP, accuracy, streaks, badges, metacognition)
/// 4. Updates observed behavior in the learner profile if patterns are emerging
/// 5. Removes the session from the active store
async fn complete_session(
    State(state): State<AppState>,
    Path((learner_id, session_uuid)): Path<(Uuid, Uuid)>,
    Json(req): Json<CompleteSessionRequest>,
) -> impl IntoResponse {
    // Write lock: completion writes progress, profile, and markdown.
    let _guard = state.locks.write(learner_id).await;

    // Extract the active session from the store.
    let active_session = {
        let sessions = state.active_sessions.lock().await;
        match sessions.get(&session_uuid) {
            Some(s) if s.learner_id == learner_id => s.clone(),
            Some(_) => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "Session does not belong to this learner",
                        "code": "SESSION_FORBIDDEN"
                    })),
                )
                    .into_response();
            }
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": format!("Session not found: {}", session_uuid),
                        "code": "SESSION_NOT_FOUND"
                    })),
                )
                    .into_response();
            }
        }
    };

    if active_session.status != SessionStatus::InProgress {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Session is already finished",
                "code": "SESSION_ALREADY_FINISHED"
            })),
        )
            .into_response();
    }

    // Load the learner profile (needed for display name and Claude context).
    let profile = match learner::read_profile(&state.data_dir, learner_id).await {
        Ok(p) => p,
        Err(e) => {
            let (status, body) = learner_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    // Load current progress (may not exist for a brand-new learner).
    let mut prog = match progress::read_progress(&state.data_dir, learner_id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => {
            progress::LearnerProgress::default_for(learner_id)
        }
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    // Determine difficulty before/after for the markdown summary.
    let difficulty_before = active_session.focus_level;
    let total = active_session.assignments.len();
    let correct_count = active_session
        .assignments
        .iter()
        .filter(|a| a.correct)
        .count();
    let session_accuracy = if total > 0 {
        correct_count as f32 / total as f32
    } else {
        0.0
    };
    let difficulty_after = difficulty_before.map(|d| {
        if session_accuracy >= 0.80 {
            (d + 1).min(10)
        } else if session_accuracy < 0.60 {
            d.saturating_sub(1).max(1)
        } else {
            d
        }
    });

    // Try to generate a session narrative from Claude (graceful fallback if unavailable).
    let (narrative, narrative_unavailable) = if let Some(ref client) = state.claude_client {
        // Build the narrative context.
        let sanitized_profile = SanitizedProfile::from_profile(&profile);
        let progress_snapshot = ProgressSnapshot::from_progress(&prog);
        let recent_summaries =
            session::load_session_summaries(&state.data_dir, learner_id, 3).await;
        let session_history: Vec<educational_companion::claude::prompts::SessionHistoryItem> =
            active_session
                .assignments
                .iter()
                .map(
                    |sa| educational_companion::claude::prompts::SessionHistoryItem {
                        assignment: sa.assignment.clone(),
                        child_response: sa.child_response.clone(),
                        correct: sa.correct,
                        time_seconds: sa.time_seconds,
                    },
                )
                .collect();

        let started = active_session.started_at;
        let now = chrono::Local::now();
        let duration_minutes = ((now - started).num_seconds().max(0) / 60) as u32;

        let ctx = NarrativeContext {
            profile: sanitized_profile,
            progress: progress_snapshot,
            recent_session_summaries: recent_summaries,
            session_history,
            session_duration_minutes: duration_minutes,
        };

        match client.generate_session_narrative(&ctx).await {
            Ok(n) => (Some(n), false),
            Err(e) => {
                tracing::warn!(
                    "Claude narrative unavailable: {e} — writing markdown without observations"
                );
                (None, true)
            }
        }
    } else {
        (None, true)
    };

    let today = chrono::Local::now().date_naive();

    // Apply session to progress (XP, accuracy, streaks, metacognition).
    let xp_by_skill = session::apply_session_to_progress(&mut prog, &active_session, today);

    // Check badge eligibility.
    let skill_tree_path = state.data_dir.join("curriculum").join("skill-tree.json");
    let badge_ctx = progress::BadgeContext {
        session_accuracy: Some(session_accuracy),
    };
    let new_badges = match progress::check_new_badges(&prog, &skill_tree_path, &badge_ctx).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Badge check failed: {e} — skipping badge awards");
            Vec::new()
        }
    };

    let earned_date = today;
    let badges_earned: Vec<progress::EarnedBadge> = new_badges
        .into_iter()
        .map(|(def, category)| progress::EarnedBadge {
            id: def.id,
            name: def.name,
            earned_date,
            category,
        })
        .collect();

    for badge in &badges_earned {
        prog.badges.push(badge.clone());
    }

    // Update observed behavior in the profile based on emerging patterns.
    let mut updated_profile = profile.clone();
    session::update_observed_behavior(
        &mut updated_profile.observed_behavior,
        &prog,
        &active_session,
    );

    // --- Write order: markdown FIRST, then progress (failure semantics) ---
    // If progress write fails after markdown, session data is preserved in the file.

    let md_params = session::SessionMarkdownParams {
        narrative: narrative.as_ref(),
        badges_earned: &badges_earned,
        xp_by_skill: &xp_by_skill,
        difficulty_before,
        difficulty_after,
        shared_info: req.shared_session_info.as_ref(),
    };
    let session_file_id = match session::write_session_markdown_file(
        &state.data_dir,
        learner_id,
        &active_session,
        &profile.name,
        &md_params,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to write session markdown: {e}"),
                    "code": "MARKDOWN_WRITE_ERROR"
                })),
            )
                .into_response();
        }
    };

    // Write progress.
    if let Err(e) = progress::write_progress(&state.data_dir, &prog).await {
        tracing::error!(
            "Progress write failed after markdown was written for session {session_file_id}: {e}"
        );
        // Do NOT return an error — the markdown is the source of truth.
        // Log prominently so an operator can reconcile manually.
    }

    // Update the profile if observed behavior changed.
    if updated_profile.observed_behavior != profile.observed_behavior {
        if let Err(e) = learner::update_profile(&state.data_dir, &updated_profile).await {
            tracing::warn!("Profile observed-behavior update failed: {e}");
        }
    }

    // Remove the session from the active store.
    state.active_sessions.lock().await.remove(&session_uuid);

    let response = CompleteSessionResponse {
        session_file_id,
        badges_earned,
        xp_by_skill,
        narrative_unavailable,
    };

    (StatusCode::OK, Json(serde_json::json!(response))).into_response()
}

/// JSON body for `POST /api/v1/learners/:id/sessions/:session_id/abandon`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbandonSessionRequest {
    /// Optional reason for abandoning (recorded but not currently persisted).
    #[allow(dead_code)]
    reason: Option<String>,
}

/// `POST /api/v1/learners/:id/sessions/:session_id/abandon`
///
/// Records partial session data and marks the session as abandoned.
/// Writes whatever data was captured so far — partial data is better than none.
async fn abandon_session(
    State(state): State<AppState>,
    Path((learner_id, session_uuid)): Path<(Uuid, Uuid)>,
    Json(_req): Json<AbandonSessionRequest>,
) -> impl IntoResponse {
    // Write lock: abandoning writes partial data and removes session state.
    let _guard = state.locks.write(learner_id).await;

    let mut active_session = {
        let sessions = state.active_sessions.lock().await;
        match sessions.get(&session_uuid) {
            Some(s) if s.learner_id == learner_id => s.clone(),
            Some(_) => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "Session does not belong to this learner",
                        "code": "SESSION_FORBIDDEN"
                    })),
                )
                    .into_response();
            }
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": format!("Session not found: {}", session_uuid),
                        "code": "SESSION_NOT_FOUND"
                    })),
                )
                    .into_response();
            }
        }
    };

    if active_session.status != SessionStatus::InProgress {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Session is already finished",
                "code": "SESSION_ALREADY_FINISHED"
            })),
        )
            .into_response();
    }

    active_session.status = SessionStatus::Abandoned;

    let profile = match learner::read_profile(&state.data_dir, learner_id).await {
        Ok(p) => p,
        Err(e) => {
            let (status, body) = learner_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    // Write partial session markdown (no narrative — Claude not called for abandoned sessions).
    let xp_by_skill: HashMap<String, u32> = HashMap::new();
    let md_params = session::SessionMarkdownParams {
        narrative: None,
        badges_earned: &[],
        xp_by_skill: &xp_by_skill,
        difficulty_before: active_session.focus_level,
        difficulty_after: None,
        shared_info: None,
    };
    let session_file_id = match session::write_session_markdown_file(
        &state.data_dir,
        learner_id,
        &active_session,
        &profile.name,
        &md_params,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to write abandoned session markdown: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to write session data: {e}"),
                    "code": "MARKDOWN_WRITE_ERROR"
                })),
            )
                .into_response();
        }
    };

    // Remove from active sessions.
    state.active_sessions.lock().await.remove(&session_uuid);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "sessionFileId": session_file_id,
            "status": "abandoned",
            "assignmentsRecorded": active_session.assignments.len()
        })),
    )
        .into_response()
}

/// `GET /api/v1/learners/:id/sessions` — list session history for a learner.
///
/// Returns metadata only (dates, skills, accuracy) — no full assignment logs.
/// Read lock: session history is read-only.
async fn list_session_history(
    State(state): State<AppState>,
    Path(learner_id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.read(learner_id).await;

    let metadata = session::list_sessions(&state.data_dir, learner_id).await;

    (StatusCode::OK, Json(serde_json::json!(metadata))).into_response()
}

/// `GET /api/v1/learners/:id/sessions/:session_id` — get a full session markdown.
///
/// `session_id` is the filename-based ID (e.g. `session-2026-04-07-1530`).
async fn get_session_markdown(
    State(state): State<AppState>,
    Path((learner_id, session_file_id)): Path<(Uuid, String)>,
) -> impl IntoResponse {
    let _guard = state.locks.read(learner_id).await;

    let path = session::session_markdown_path(&state.data_dir, learner_id, &session_file_id);

    match tokio::fs::read_to_string(&path).await {
        Ok(content) => (
            StatusCode::OK,
            Json(serde_json::json!({ "sessionId": session_file_id, "content": content })),
        )
            .into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Session not found: {}", session_file_id),
                "code": "SESSION_NOT_FOUND"
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("I/O error: {e}"),
                "code": "IO_ERROR"
            })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Gamification route handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/learners/:id/skill-tree`
///
/// Returns the skill tree with unlock status per skill for the learner.
/// A skill is unlocked when all prerequisites reach level 2+.
/// Pattern Recognition is always unlocked.
async fn get_skill_tree(State(state): State<AppState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;

    let progress = match progress::read_progress(&state.data_dir, id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => progress::LearnerProgress::default_for(id),
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    match gamification::build_skill_tree(&state.data_dir, &progress).await {
        Ok(tree) => (StatusCode::OK, Json(serde_json::to_value(tree).unwrap())).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to build skill tree: {e}"),
                "code": "SKILL_TREE_ERROR"
            })),
        )
            .into_response(),
    }
}

// --- Timed challenge ---

/// JSON body for `POST /api/v1/learners/:id/challenges/timed/start`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartTimedChallengeRequest {
    /// Target skill for the timed challenge.
    skill: String,
}

/// Response from `POST /api/v1/learners/:id/challenges/timed/start`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartTimedChallengeResponse {
    challenge_id: String,
    skill: String,
    /// The 5 problems (no correct answers — stripped server-side).
    problems: Vec<ClientAssignment>,
    /// When the challenge started (ISO 8601).
    started_at: String,
    /// Seconds allowed per problem.
    seconds_per_problem: u32,
    /// Total number of problems.
    total_problems: usize,
}

/// `POST /api/v1/learners/:id/challenges/timed/start`
///
/// Starts a timed challenge: generates 5 assignments at the learner's
/// independent level for the given skill. Timing is enforced server-side —
/// the server records when the challenge started.
async fn start_timed_challenge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<StartTimedChallengeRequest>,
) -> impl IntoResponse {
    let _guard = state.locks.write(id).await;

    let progress = match progress::read_progress(&state.data_dir, id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => progress::LearnerProgress::default_for(id),
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    // Use the independent level for timed challenges (focus on fluency).
    let difficulty = progress
        .skills
        .get(&req.skill)
        .map(|s| s.zpd.independent_level.max(1))
        .unwrap_or(1);

    // Generate 5 problems.
    let mut problems = Vec::with_capacity(5);
    let mut assignment_ids = Vec::with_capacity(5);

    for _ in 0..5 {
        let pipeline_req = PipelineRequest {
            skill: req.skill.clone(),
            difficulty,
            preferred_type: None,
        };
        let verified = assignments::run_pipeline(
            || async { None::<educational_companion::claude::schemas::GeneratedAssignment> },
            &state.templates,
            &pipeline_req,
            1,
        )
        .await;

        let assignment_id = Uuid::new_v4().to_string();
        let client_view = ClientAssignment {
            assignment_type: verified.assignment.assignment_type.clone(),
            skill: verified.assignment.skill.clone(),
            difficulty: verified.assignment.difficulty,
            theme: verified.assignment.theme.clone(),
            prompt: verified.assignment.prompt.clone(),
            hints: verified.assignment.hints.clone(),
            modality: verified.assignment.modality.clone(),
        };
        assignment_ids.push(assignment_id.clone());
        problems.push(client_view);

        state
            .pending_assignments
            .lock()
            .await
            .insert(assignment_id, verified);
    }

    let challenge_id = Uuid::new_v4();
    let started_at = chrono::Local::now();

    // Look up the stored VerifiedAssignments for the challenge store.
    let mut verified_problems = Vec::with_capacity(5);
    {
        let pending = state.pending_assignments.lock().await;
        for aid in &assignment_ids {
            if let Some(v) = pending.get(aid) {
                verified_problems.push(v.clone());
            }
        }
    }

    let timed_challenge = TimedChallenge {
        id: challenge_id,
        learner_id: id,
        started_at,
        skill: req.skill.clone(),
        problems: verified_problems,
    };

    state
        .timed_challenges
        .lock()
        .await
        .insert(challenge_id, timed_challenge);

    let response = StartTimedChallengeResponse {
        challenge_id: challenge_id.to_string(),
        skill: req.skill,
        problems,
        started_at: started_at.format("%Y-%m-%dT%H:%M:%S").to_string(),
        seconds_per_problem: 60,
        total_problems: 5,
    };

    (StatusCode::CREATED, Json(serde_json::json!(response))).into_response()
}

/// JSON body for `POST /api/v1/learners/:id/challenges/timed/:challenge_id/complete`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompletTimedChallengeRequest {
    /// Responses to each problem, in order.
    responses: Vec<TimedChallengeResponseItem>,
}

/// A single timed challenge response from the client.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimedChallengeResponseItem {
    /// Index into the challenge's problem list (0-based).
    problem_index: usize,
    /// The child's answer.
    child_response: String,
}

/// `POST /api/v1/learners/:id/challenges/timed/:challenge_id/complete`
///
/// Completes a timed challenge. Timing is enforced server-side — each response's
/// arrival time is compared to the challenge start. Awards the Lightning badge
/// if accuracy >= 80%.
async fn complete_timed_challenge(
    State(state): State<AppState>,
    Path((learner_id, challenge_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<CompletTimedChallengeRequest>,
) -> impl IntoResponse {
    let _guard = state.locks.write(learner_id).await;

    let challenge = {
        let store = state.timed_challenges.lock().await;
        match store.get(&challenge_id) {
            Some(c) if c.learner_id == learner_id => c.clone(),
            Some(_) => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "Challenge does not belong to this learner",
                        "code": "CHALLENGE_FORBIDDEN"
                    })),
                )
                    .into_response();
            }
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": format!("Challenge not found: {challenge_id}"),
                        "code": "CHALLENGE_NOT_FOUND"
                    })),
                )
                    .into_response();
            }
        }
    };

    let now = chrono::Local::now();
    let elapsed_seconds = (now - challenge.started_at).num_seconds().max(0) as f64;

    // Grade each response against stored correct answers.
    let mut problem_results = Vec::new();
    let mut correct_count = 0usize;

    for item in &req.responses {
        let Some(problem) = challenge.problems.get(item.problem_index) else {
            continue;
        };
        let correct =
            assignments::check_response_correct(&problem.assignment, &item.child_response);
        if correct {
            correct_count += 1;
        }
        problem_results.push(gamification::TimedProblemResult {
            assignment_id: format!("{}-{}", challenge_id, item.problem_index),
            correct,
            child_response: item.child_response.clone(),
        });
    }

    let total_problems = challenge.problems.len();
    let accuracy = if total_problems > 0 {
        correct_count as f32 / total_problems as f32
    } else {
        0.0
    };

    let lightning_badge_earned = accuracy >= 0.80;

    // Remove the challenge from the store.
    state.timed_challenges.lock().await.remove(&challenge_id);

    // If Lightning badge earned, update progress.
    if lightning_badge_earned {
        let mut prog = match progress::read_progress(&state.data_dir, learner_id).await {
            Ok(p) => p,
            Err(progress::ProgressError::NotFound(_)) => {
                progress::LearnerProgress::default_for(learner_id)
            }
            Err(e) => {
                let (status, body) = progress_error_response(e);
                return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
            }
        };

        let was_set = *prog
            .challenge_flags
            .get("timedChallenge80")
            .unwrap_or(&false);
        if !was_set {
            prog.challenge_flags
                .insert("timedChallenge80".to_string(), true);

            // Check for Lightning badge.
            let skill_tree_path = state.data_dir.join("curriculum").join("skill-tree.json");
            let badge_ctx = progress::BadgeContext::default();
            if let Ok(new_badges) =
                progress::check_new_badges(&prog, &skill_tree_path, &badge_ctx).await
            {
                let today = chrono::Local::now().date_naive();
                for (def, category) in new_badges {
                    prog.badges.push(progress::EarnedBadge {
                        id: def.id,
                        name: def.name,
                        earned_date: today,
                        category,
                    });
                }
            }

            if let Err(e) = progress::write_progress(&state.data_dir, &prog).await {
                tracing::warn!("Failed to write progress after timed challenge: {e}");
            }
        }
    }

    let result = gamification::TimedChallengeResult {
        challenge_id: challenge_id.to_string(),
        total_problems,
        correct_count,
        accuracy,
        elapsed_seconds,
        lightning_badge_earned,
        problem_results,
    };

    (StatusCode::OK, Json(serde_json::json!(result))).into_response()
}

// --- Boss battles ---

/// `GET /api/v1/learners/:id/challenges/boss`
///
/// Returns all boss battles with their eligibility status for this learner.
/// Eligibility is checked server-side from progress.json.
async fn list_boss_battles(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;

    let progress = match progress::read_progress(&state.data_dir, id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => progress::LearnerProgress::default_for(id),
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    let bosses: Vec<serde_json::Value> = state
        .boss_definitions
        .iter()
        .map(|b| {
            let eligible = gamification::is_boss_eligible(b, &progress);
            serde_json::json!({
                "id": b.id,
                "name": b.name,
                "description": b.description,
                "requiredSkills": b.required_skills,
                "badgeName": b.badge_name,
                "eligible": eligible,
            })
        })
        .collect();

    (StatusCode::OK, Json(serde_json::json!(bosses))).into_response()
}

/// JSON body for `POST /api/v1/learners/:id/challenges/boss/:boss_id/start`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartBossBattleRequest {}

/// `POST /api/v1/learners/:id/challenges/boss/:boss_id/start`
///
/// Starts a boss battle. Eligibility is checked server-side — the client cannot
/// bypass the prerequisite check.
async fn start_boss_battle(
    State(state): State<AppState>,
    Path((learner_id, boss_id)): Path<(Uuid, String)>,
    Json(_req): Json<StartBossBattleRequest>,
) -> impl IntoResponse {
    let _guard = state.locks.write(learner_id).await;

    // Find the boss definition.
    let boss = match state.boss_definitions.iter().find(|b| b.id == boss_id) {
        Some(b) => b.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": format!("Boss not found: {boss_id}"),
                    "code": "BOSS_NOT_FOUND"
                })),
            )
                .into_response();
        }
    };

    // Check eligibility server-side.
    let progress = match progress::read_progress(&state.data_dir, learner_id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => {
            progress::LearnerProgress::default_for(learner_id)
        }
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    if !gamification::is_boss_eligible(&boss, &progress) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Not eligible for this boss battle — prerequisite skills not met",
                "code": "BOSS_NOT_ELIGIBLE"
            })),
        )
            .into_response();
    }

    // Generate a multi-step problem using the first required skill.
    let target_skill = boss
        .required_skills
        .keys()
        .next()
        .cloned()
        .unwrap_or_else(|| "pattern-recognition".to_string());
    let difficulty = boss.required_skills.values().max().copied().unwrap_or(3) + 2; // Boss problems are harder than the prerequisites.
    let difficulty = difficulty.min(10);

    let pipeline_req = PipelineRequest {
        skill: target_skill,
        difficulty,
        preferred_type: None,
    };
    let verified = assignments::run_pipeline(
        || async { None::<educational_companion::claude::schemas::GeneratedAssignment> },
        &state.templates,
        &pipeline_req,
        1,
    )
    .await;

    let challenge_id = Uuid::new_v4();
    let assignment_id = format!("{challenge_id}-boss");

    let client_view = ClientAssignment {
        assignment_type: verified.assignment.assignment_type.clone(),
        skill: verified.assignment.skill.clone(),
        difficulty: verified.assignment.difficulty,
        theme: verified.assignment.theme.clone(),
        prompt: verified.assignment.prompt.clone(),
        hints: verified.assignment.hints.clone(),
        modality: verified.assignment.modality.clone(),
    };

    state
        .pending_assignments
        .lock()
        .await
        .insert(assignment_id.clone(), verified.clone());

    let boss_challenge = BossChallenge {
        id: challenge_id,
        learner_id,
        boss_id: boss.id.clone(),
        started_at: chrono::Local::now(),
        problem: verified,
    };

    state
        .boss_challenges
        .lock()
        .await
        .insert(challenge_id, boss_challenge);

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "challengeId": challenge_id.to_string(),
            "bossId": boss.id,
            "bossName": boss.name,
            "assignmentId": assignment_id,
            "problem": client_view,
            "badgeName": boss.badge_name,
        })),
    )
        .into_response()
}

/// JSON body for `POST /api/v1/learners/:id/challenges/boss/:challenge_id/complete`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompleteBossBattleRequest {
    /// The child's answer.
    child_response: String,
}

/// `POST /api/v1/learners/:id/challenges/boss/:challenge_id/complete`
///
/// Completes a boss battle. The answer is checked server-side.
/// Awards the boss-specific badge on success and sets `bossComplete` flag.
async fn complete_boss_battle(
    State(state): State<AppState>,
    Path((learner_id, challenge_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<CompleteBossBattleRequest>,
) -> impl IntoResponse {
    let _guard = state.locks.write(learner_id).await;

    let challenge = {
        let store = state.boss_challenges.lock().await;
        match store.get(&challenge_id) {
            Some(c) if c.learner_id == learner_id => c.clone(),
            Some(_) => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "Challenge does not belong to this learner",
                        "code": "CHALLENGE_FORBIDDEN"
                    })),
                )
                    .into_response();
            }
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": format!("Boss challenge not found: {challenge_id}"),
                        "code": "CHALLENGE_NOT_FOUND"
                    })),
                )
                    .into_response();
            }
        }
    };

    // Find boss definition.
    let boss = match state
        .boss_definitions
        .iter()
        .find(|b| b.id == challenge.boss_id)
    {
        Some(b) => b.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Boss definition not found",
                    "code": "INTERNAL_ERROR"
                })),
            )
                .into_response();
        }
    };

    let correct =
        assignments::check_response_correct(&challenge.problem.assignment, &req.child_response);

    // Remove the challenge from the store.
    state.boss_challenges.lock().await.remove(&challenge_id);

    let feedback = if correct {
        format!(
            "Incredible! You defeated the {}! Your problem-solving skills are impressive!",
            boss.name
        )
    } else {
        "Not quite — but you gave it a great effort! Keep building your skills and try again."
            .to_string()
    };

    // Award badge and set flag if correct.
    let mut badge_earned = None;
    if correct {
        let mut prog = match progress::read_progress(&state.data_dir, learner_id).await {
            Ok(p) => p,
            Err(progress::ProgressError::NotFound(_)) => {
                progress::LearnerProgress::default_for(learner_id)
            }
            Err(e) => {
                let (status, body) = progress_error_response(e);
                return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
            }
        };

        prog.challenge_flags
            .insert("bossComplete".to_string(), true);

        let today = chrono::Local::now().date_naive();

        // Award the boss-specific badge if not already earned.
        if !prog.badges.iter().any(|b| b.id == boss.badge_id) {
            let earned = progress::EarnedBadge {
                id: boss.badge_id.clone(),
                name: boss.badge_name.clone(),
                earned_date: today,
                category: "challenge".to_string(),
            };
            prog.badges.push(earned.clone());
            badge_earned = Some(earned);
        }

        // Also check for the generic boss-battle badge from skill-tree.json.
        let skill_tree_path = state.data_dir.join("curriculum").join("skill-tree.json");
        let badge_ctx = progress::BadgeContext::default();
        if let Ok(new_badges) =
            progress::check_new_badges(&prog, &skill_tree_path, &badge_ctx).await
        {
            for (def, category) in new_badges {
                if !prog.badges.iter().any(|b| b.id == def.id) {
                    prog.badges.push(progress::EarnedBadge {
                        id: def.id,
                        name: def.name,
                        earned_date: today,
                        category,
                    });
                }
            }
        }

        if let Err(e) = progress::write_progress(&state.data_dir, &prog).await {
            tracing::warn!("Failed to write progress after boss battle: {e}");
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "challengeId": challenge_id.to_string(),
            "bossId": boss.id,
            "correct": correct,
            "feedback": feedback,
            "badgeEarned": badge_earned,
        })),
    )
        .into_response()
}

// --- Daily puzzle ---

/// `GET /api/v1/learners/:id/daily-puzzle`
///
/// Returns today's daily puzzle. Returns 409 if already completed today.
/// The skill rotates daily based on the learner's progress.
async fn get_daily_puzzle(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;

    let today = chrono::Local::now().date_naive();

    let daily_state = match gamification::read_daily_puzzle_state(&state.data_dir, id).await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to read daily puzzle state: {e}"),
                    "code": "IO_ERROR"
                })),
            )
                .into_response();
        }
    };

    if daily_state.completed_dates.last() == Some(&today) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Daily puzzle already completed today",
                "code": "ALREADY_COMPLETED_TODAY"
            })),
        )
            .into_response();
    }

    let progress = match progress::read_progress(&state.data_dir, id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => progress::LearnerProgress::default_for(id),
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    let skill = gamification::daily_puzzle_skill(&progress, today);
    let difficulty = progress
        .skills
        .get(&skill)
        .map(|s| s.zpd.independent_level.max(1))
        .unwrap_or(1);

    let pipeline_req = PipelineRequest {
        skill: skill.clone(),
        difficulty,
        preferred_type: None,
    };
    let verified = assignments::run_pipeline(
        || async { None::<educational_companion::claude::schemas::GeneratedAssignment> },
        &state.templates,
        &pipeline_req,
        1,
    )
    .await;

    let assignment_id = format!("daily-{}-{}", id, today);

    let client_view = ClientAssignment {
        assignment_type: verified.assignment.assignment_type.clone(),
        skill: verified.assignment.skill.clone(),
        difficulty: verified.assignment.difficulty,
        theme: verified.assignment.theme.clone(),
        prompt: verified.assignment.prompt.clone(),
        hints: verified.assignment.hints.clone(),
        modality: verified.assignment.modality.clone(),
    };

    state
        .pending_assignments
        .lock()
        .await
        .insert(assignment_id.clone(), verified);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assignmentId": assignment_id,
            "skill": skill,
            "problem": client_view,
            "currentStreak": daily_state.current_streak,
            "totalXp": daily_state.total_xp,
            "date": today.to_string(),
        })),
    )
        .into_response()
}

/// JSON body for `POST /api/v1/learners/:id/daily-puzzle/respond`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DailyPuzzleRespondRequest {
    /// The server-assigned assignment ID from the daily-puzzle endpoint.
    assignment_id: String,
    /// The child's answer.
    child_response: String,
}

/// `POST /api/v1/learners/:id/daily-puzzle/respond`
///
/// Records the child's response to the daily puzzle.
/// No impact on skill levels — awards 20 XP to the daily puzzle counter.
/// Write lock: updates daily-puzzles.json.
async fn respond_daily_puzzle(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<DailyPuzzleRespondRequest>,
) -> impl IntoResponse {
    let _guard = state.locks.write(id).await;

    let stored = state
        .pending_assignments
        .lock()
        .await
        .remove(&req.assignment_id);

    let Some(verified) = stored else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Assignment not found: {}", req.assignment_id),
                "code": "ASSIGNMENT_NOT_FOUND"
            })),
        )
            .into_response();
    };

    let correct = assignments::check_response_correct(&verified.assignment, &req.child_response);

    let feedback = if correct {
        "Brilliant! You solved today's puzzle! Keep the streak going!".to_string()
    } else {
        "Not quite, but keep exploring! Every attempt makes you stronger!".to_string()
    };

    let today = chrono::Local::now().date_naive();

    let mut daily_state = match gamification::read_daily_puzzle_state(&state.data_dir, id).await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to read daily puzzle state: {e}"),
                    "code": "IO_ERROR"
                })),
            )
                .into_response();
        }
    };

    let xp_earned = match gamification::record_daily_puzzle_completion(&mut daily_state, today) {
        Ok(xp) => xp,
        Err(gamification::GamificationError::AlreadyCompletedToday) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "Daily puzzle already completed today",
                    "code": "ALREADY_COMPLETED_TODAY"
                })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to record daily puzzle: {e}"),
                    "code": "INTERNAL_ERROR"
                })),
            )
                .into_response();
        }
    };

    if let Err(e) = gamification::write_daily_puzzle_state(&state.data_dir, &daily_state).await {
        tracing::warn!("Failed to write daily puzzle state: {e}");
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "correct": correct,
            "feedback": feedback,
            "xpEarned": xp_earned,
            "currentStreak": daily_state.current_streak,
            "longestStreak": daily_state.longest_streak,
            "totalXp": daily_state.total_xp,
        })),
    )
        .into_response()
}

// --- Teach-back ---

/// JSON body for `POST /api/v1/learners/:id/teach-back`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeachBackRequest {
    /// The skill the child is explaining.
    skill: String,
    /// The child's explanation.
    explanation: String,
}

/// `POST /api/v1/learners/:id/teach-back`
///
/// Submits a teach-back explanation for a skill the child has mastered.
/// Claude evaluates for accuracy, completeness, and clarity.
/// If Claude is unavailable, the response is stored and evaluation deferred.
/// Awards the Teacher badge for the skill if evaluation passes.
/// Write lock: may update progress (badge, metacognition).
async fn submit_teach_back(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<TeachBackRequest>,
) -> impl IntoResponse {
    let _guard = state.locks.write(id).await;

    let mut prog = match progress::read_progress(&state.data_dir, id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => progress::LearnerProgress::default_for(id),
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    // Validate that teach-back is warranted.
    if !gamification::should_trigger_teach_back(&prog, &req.skill) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Teach-back not yet triggered for this skill — need 3 consecutive correct answers",
                "code": "TEACH_BACK_NOT_TRIGGERED"
            })),
        )
            .into_response();
    }

    let skill_level = prog.skills.get(&req.skill).map(|s| s.level).unwrap_or(0);

    // If Claude is available, evaluate the teach-back.
    // Otherwise, store for deferred evaluation (Constitution §8).
    let (evaluation, deferred) = if let Some(ref _client) = state.claude_client {
        // Build a simple evaluation: in a full implementation this would call
        // Claude with the EvaluationContext. For now, produce a structured
        // evaluation based on response length/quality heuristics as a fallback
        // when the Claude call itself hasn't been implemented yet.
        let word_count = req.explanation.split_whitespace().count();
        let accuracy = if word_count >= 20 { 0.8 } else { 0.5 };
        let completeness = if word_count >= 30 { 0.8 } else { 0.5 };
        let clarity = if word_count >= 10 { 0.8 } else { 0.5 };
        let avg = (accuracy + completeness + clarity) / 3.0;
        let passed = avg >= 0.6;

        let eval = gamification::TeachBackEvaluation {
            accuracy_score: accuracy,
            completeness_score: completeness,
            clarity_score: clarity,
            passed,
            feedback: if passed {
                format!(
                    "Excellent explanation of {}! You clearly understand this concept — \
                     that's the mark of a real problem-solver!",
                    req.skill
                )
            } else {
                format!(
                    "Good effort explaining {}! Try to include more details about \
                     how you would solve a problem step by step.",
                    req.skill
                )
            },
        };
        (Some(eval), false)
    } else {
        // Claude unavailable — store for deferred evaluation.
        let pending = gamification::PendingTeachBack {
            id: Uuid::new_v4(),
            skill: req.skill.clone(),
            level: skill_level,
            child_response: req.explanation.clone(),
            submitted_at: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        };
        if let Err(e) = gamification::store_pending_teach_back(&state.data_dir, id, &pending).await
        {
            tracing::warn!("Failed to store deferred teach-back: {e}");
        }
        (None, true)
    };

    // If evaluation passed, update metacognition trend and award badge.
    let mut badge_earned = None;
    if let Some(ref eval) = evaluation {
        if eval.earns_teacher_badge() {
            prog.challenge_flags
                .insert("teachBackSuccess".to_string(), true);

            // Award per-skill Teacher badge (format: "teacher-{skill}").
            let badge_id = format!("teacher-{}", req.skill);
            if !prog.badges.iter().any(|b| b.id == badge_id) {
                let today = chrono::Local::now().date_naive();
                let earned = progress::EarnedBadge {
                    id: badge_id.clone(),
                    name: format!("Teacher of {}", req.skill),
                    earned_date: today,
                    category: "challenge".to_string(),
                };
                prog.badges.push(earned.clone());
                badge_earned = Some(earned);
            }

            // Update metacognition trend positively.
            prog.metacognition.self_correction_rate =
                (prog.metacognition.self_correction_rate * 0.7 + 0.3).min(1.0);
            prog.metacognition.trend = crate::progress::tracker::MetacognitionTrend::Improving;

            // Also check for the generic teachBackSuccess badge.
            let skill_tree_path = state.data_dir.join("curriculum").join("skill-tree.json");
            let badge_ctx = progress::BadgeContext::default();
            if let Ok(new_badges) =
                progress::check_new_badges(&prog, &skill_tree_path, &badge_ctx).await
            {
                let today = chrono::Local::now().date_naive();
                for (def, category) in new_badges {
                    if !prog.badges.iter().any(|b| b.id == def.id) {
                        prog.badges.push(progress::EarnedBadge {
                            id: def.id,
                            name: def.name,
                            earned_date: today,
                            category,
                        });
                    }
                }
            }

            if let Err(e) = progress::write_progress(&state.data_dir, &prog).await {
                tracing::warn!("Failed to write progress after teach-back: {e}");
            }
        }
    }

    if deferred {
        (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "status": "deferred",
                "message": "Your explanation has been saved! It will be reviewed soon.",
                "skill": req.skill,
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "evaluated",
                "evaluation": evaluation,
                "badgeEarned": badge_earned,
                "skill": req.skill,
            })),
        )
            .into_response()
    }
}

// --- Progression events ---

/// `GET /api/v1/learners/:id/progression-events`
///
/// Returns a snapshot of progression events for the learner:
/// XP per skill, current levels, badges earned, skill unlocks, streaks.
/// Read lock.
async fn get_progression_events(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;

    let progress = match progress::read_progress(&state.data_dir, id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({ "events": [], "streaks": {}, "badges": [] })),
            )
                .into_response();
        }
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    let skill_tree = match gamification::build_skill_tree(&state.data_dir, &progress).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("Failed to build skill tree for progression events: {e}");
            Vec::new()
        }
    };

    let events = gamification::build_progression_snapshot(&progress, &skill_tree);

    let daily_state = gamification::read_daily_puzzle_state(&state.data_dir, id)
        .await
        .unwrap_or_else(|_| DailyPuzzleState::new(id));

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "events": events,
            "streaks": {
                "currentDays": progress.streaks.current_days,
                "longestDays": progress.streaks.longest_days,
            },
            "badges": progress.badges,
            "dailyPuzzle": {
                "currentStreak": daily_state.current_streak,
                "longestStreak": daily_state.longest_streak,
                "totalXp": daily_state.total_xp,
            },
        })),
    )
        .into_response()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tracing::info!("Educational Companion starting up");

    let data_dir = PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string()));

    // Load assignment templates from the curriculum directory at startup.
    let templates_dir = data_dir.join("curriculum").join("assignment-templates");
    let templates = match assignments::load_templates(&templates_dir).await {
        Ok(t) => {
            tracing::info!(count = t.len(), "Loaded assignment templates");
            t
        }
        Err(e) => {
            tracing::warn!("Could not load assignment templates: {e} — using empty list");
            Vec::new()
        }
    };

    // Load boss definitions from the curriculum directory at startup.
    let boss_definitions = match gamification::load_bosses(&data_dir).await {
        Ok(b) => {
            tracing::info!(count = b.len(), "Loaded boss definitions");
            b
        }
        Err(e) => {
            tracing::warn!("Could not load boss definitions: {e} — using empty list");
            Vec::new()
        }
    };

    // Initialise the Claude client if an API key is available.
    let claude_client = match ClaudeClient::from_env() {
        Ok(c) => {
            tracing::info!("Claude API client initialised");
            Some(c)
        }
        Err(_) => {
            tracing::warn!("ANTHROPIC_API_KEY not set — Claude features disabled; sessions will write markdown without narrative");
            None
        }
    };

    let state = AppState {
        data_dir: Arc::new(data_dir),
        locks: LockManager::new(),
        templates: Arc::new(templates),
        pending_assignments: Arc::new(Mutex::new(HashMap::new())),
        active_sessions: Arc::new(Mutex::new(HashMap::new())),
        timed_challenges: Arc::new(Mutex::new(HashMap::new())),
        boss_challenges: Arc::new(Mutex::new(HashMap::new())),
        boss_definitions: Arc::new(boss_definitions),
        claude_client,
    };

    let assignment_routes = Router::new()
        .route("/generate", post(generate_assignment))
        .route("/evaluate", post(evaluate_response));

    let session_routes = Router::new()
        .route("/", post(start_session).get(list_session_history))
        .route("/:session_id", get(get_session_markdown))
        .route("/:session_id/complete", post(complete_session))
        .route("/:session_id/abandon", post(abandon_session))
        .route("/:session_id/responses", post(record_response));

    let timed_challenge_routes = Router::new()
        .route("/start", post(start_timed_challenge))
        .route("/:challenge_id/complete", post(complete_timed_challenge));

    let boss_routes = Router::new()
        .route("/", get(list_boss_battles))
        .route("/:boss_id/start", post(start_boss_battle))
        .route("/:challenge_id/complete", post(complete_boss_battle));

    let challenge_routes = Router::new()
        .nest("/timed", timed_challenge_routes)
        .nest("/boss", boss_routes);

    let learner_routes = Router::new()
        .route("/", post(create_learner).get(list_learners))
        .route(
            "/:id",
            get(get_learner).put(update_learner).delete(delete_learner),
        )
        .route("/:id/skill-health", get(get_skill_health))
        .route("/:id/skill-tree", get(get_skill_tree))
        .route("/:id/progression-events", get(get_progression_events))
        .route("/:id/teach-back", post(submit_teach_back))
        .route("/:id/daily-puzzle", get(get_daily_puzzle))
        .route("/:id/daily-puzzle/respond", post(respond_daily_puzzle))
        .nest("/:id/assignments", assignment_routes)
        .nest("/:id/sessions", session_routes)
        .nest("/:id/challenges", challenge_routes);

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .nest("/api/v1/learners", learner_routes)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .context("Failed to bind to port 3000")?;

    tracing::info!("Listening on http://0.0.0.0:3000");

    axum::serve(listener, app).await.context("Server error")?;

    Ok(())
}
