use anyhow::Context;
use axum::{
    extract::{Path, Query, State},
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
use educational_companion::dashboard;
use educational_companion::learner;
use educational_companion::learner::{
    InitialPreferences, LearnerError, LearnerProfile, ObservedBehavior,
};
use educational_companion::lock::LockManager;
use educational_companion::offline;
use educational_companion::progress;
use educational_companion::session::{
    self, ActiveSession, SessionAssignment, SessionListParams, SessionStatus, SharedSessionInfo,
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
        needs_parent_review: verified.needs_parent_review,
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

    // Add review queue items for any assignments flagged for parent review.
    // We do this after writing the markdown so we know the file-based session ID.
    for sa in active_session
        .assignments
        .iter()
        .filter(|a| a.needs_parent_review)
    {
        let item = dashboard::new_review_item(
            &session_file_id,
            &sa.assignment.assignment_type,
            &sa.assignment.prompt,
            &sa.child_response,
            sa.notes.as_deref().unwrap_or(""),
            "medium",
        );
        if let Err(e) = dashboard::add_review_item(&state.data_dir, learner_id, item).await {
            tracing::warn!("Failed to write review queue item for session {session_file_id}: {e}");
        }
    }

    // Remove the session from the active store.
    state.active_sessions.lock().await.remove(&session_uuid);

    // Spawn a background task to replenish the assignment buffer.
    // This runs after the write lock is released so it can re-acquire it.
    // The background task is fire-and-forget — failure is logged, not propagated.
    // A 10-second timeout is applied to the write lock acquisition to prevent
    // the task from blocking indefinitely if another operation holds the lock.
    {
        let data_dir = state.data_dir.clone();
        let templates = state.templates.clone();
        let claude_client = state.claude_client.clone();
        let locks = state.locks.clone();
        let prog_clone = prog.clone();

        tokio::spawn(async move {
            // Acquire write lock with a timeout — buffer replenishment is not
            // critical to session completion; skip rather than block indefinitely.
            let lock_result = tokio::time::timeout(
                tokio::time::Duration::from_secs(10),
                locks.write(learner_id),
            )
            .await;

            let _bg_guard = match lock_result {
                Ok(guard) => guard,
                Err(_) => {
                    tracing::warn!(
                        learner_id = %learner_id,
                        "Background buffer replenishment skipped — could not acquire write lock within 10s"
                    );
                    return;
                }
            };

            if let Err(e) = offline::replenish_buffer(
                &data_dir,
                learner_id,
                &prog_clone,
                &templates,
                claude_client.as_ref(),
            )
            .await
            {
                tracing::warn!(
                    learner_id = %learner_id,
                    error = %e,
                    "Background buffer replenishment failed — buffer unchanged"
                );
            }
        });
    }

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

// ---------------------------------------------------------------------------
// Buffer route handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/learners/:id/buffer` — get buffer status for a learner.
///
/// Returns the number of pre-verified assignments in the buffer, when they were
/// generated, a per-skill breakdown, and the current degradation tier.
/// Read lock: buffer status is read-only.
async fn get_buffer_status(
    State(state): State<AppState>,
    Path(learner_id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.read(learner_id).await;

    // Verify learner exists.
    if let Err(e) = learner::read_profile(&state.data_dir, learner_id).await {
        let (status, body) = learner_error_response(e);
        return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
    }

    let buffer = offline::read_buffer(&state.data_dir, learner_id).await;
    let tier = offline::detect_tier(state.claude_client.as_ref(), buffer.as_ref()).await;
    let status = offline::build_buffer_status(buffer.as_ref(), tier);

    (StatusCode::OK, Json(serde_json::json!(status))).into_response()
}

/// `POST /api/v1/learners/:id/buffer/replenish` — manually trigger buffer replenishment.
///
/// Generates fresh, pre-verified assignments and stores them in the buffer.
/// Write lock: replenishment writes the buffer file.
async fn replenish_buffer_handler(
    State(state): State<AppState>,
    Path(learner_id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.write(learner_id).await;

    // Verify learner exists.
    if let Err(e) = learner::read_profile(&state.data_dir, learner_id).await {
        let (status, body) = learner_error_response(e);
        return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
    }

    // Load progress to determine skill priorities.
    let prog = match progress::read_progress(&state.data_dir, learner_id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => {
            progress::LearnerProgress::default_for(learner_id)
        }
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    match offline::replenish_buffer(
        &state.data_dir,
        learner_id,
        &prog,
        &state.templates,
        state.claude_client.as_ref(),
    )
    .await
    {
        Ok(()) => {
            let buffer = offline::read_buffer(&state.data_dir, learner_id).await;
            let count = buffer.as_ref().map(|b| b.fresh_count()).unwrap_or(0);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "replenished",
                    "count": count,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Buffer replenishment failed: {e}"),
                "code": "BUFFER_REPLENISH_ERROR"
            })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Sync route handler
// ---------------------------------------------------------------------------

/// `POST /api/v1/learners/:id/sync` — retroactively sync offline sessions.
///
/// Finds sessions whose behavioral observations are missing (written when Claude
/// was unavailable) and generates them retroactively. Only adds missing content —
/// never overwrites session data recorded during the session.
/// Write lock: sync writes to session markdown files.
async fn sync_offline_sessions(
    State(state): State<AppState>,
    Path(learner_id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.write(learner_id).await;

    // Verify learner exists.
    if let Err(e) = learner::read_profile(&state.data_dir, learner_id).await {
        let (status, body) = learner_error_response(e);
        return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
    }

    let Some(ref client) = state.claude_client else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "Claude API not configured — cannot sync offline sessions",
                "code": "CLAUDE_UNAVAILABLE"
            })),
        )
            .into_response();
    };

    let sessions_to_sync =
        offline::find_sessions_needing_sync(&state.data_dir, learner_id).await;

    if sessions_to_sync.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "no_sync_needed",
                "synced": 0
            })),
        )
            .into_response();
    }

    let mut synced = 0usize;
    let mut failed: Vec<String> = Vec::new();

    for session_id in &sessions_to_sync {
        match offline::sync_session(&state.data_dir, learner_id, session_id, client).await {
            Ok(()) => {
                synced += 1;
                tracing::info!(
                    learner_id = %learner_id,
                    session_id,
                    "Session synced successfully"
                );
            }
            Err(e) => {
                tracing::warn!(
                    learner_id = %learner_id,
                    session_id,
                    error = %e,
                    "Session sync failed"
                );
                failed.push(session_id.clone());
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "sync_complete",
            "synced": synced,
            "failed": failed,
        })),
    )
        .into_response()
}

/// Query parameters for `GET /api/v1/learners/:id/sessions`.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SessionHistoryQuery {
    /// 1-based page number (default: 1).
    #[serde(default = "default_page")]
    page: usize,
    /// Items per page (default: 20, maximum 100).
    #[serde(default = "default_per_page")]
    per_page: usize,
}

fn default_page() -> usize {
    1
}
fn default_per_page() -> usize {
    20
}

/// `GET /api/v1/learners/:id/sessions` — list session history for a learner.
///
/// Returns paginated metadata only (dates, skills, accuracy, flags) — no full
/// assignment logs. Supports `page` and `perPage` query parameters.
/// Read lock: session history is read-only.
async fn list_session_history(
    State(state): State<AppState>,
    Path(learner_id): Path<Uuid>,
    Query(query): Query<SessionHistoryQuery>,
) -> impl IntoResponse {
    let _guard = state.locks.read(learner_id).await;

    let params = SessionListParams {
        page: query.page,
        per_page: query.per_page,
    };
    let result = session::list_sessions(&state.data_dir, learner_id, params).await;

    (StatusCode::OK, Json(serde_json::json!(result))).into_response()
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
// Dashboard route handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/learners/:id/dashboard/overview`
///
/// Returns a combined snapshot: learner info, streaks, recent badges, skill
/// radar, ZPD visualization, and session totals. Never exposes `learnerId`
/// or UUIDs (Constitution §4, §6).
/// Read lock: read-only view of profile and progress.
async fn get_dashboard_overview(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;

    // Load profile — required.
    let profile = match learner::read_profile(&state.data_dir, id).await {
        Ok(p) => p,
        Err(e) => {
            let (status, body) = learner_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    // Load progress — gracefully defaults for new learners with no progress.json yet.
    let prog = match progress::read_progress(&state.data_dir, id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => progress::LearnerProgress::default_for(id),
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    // Build skill radar — one entry per skill (sorted for deterministic output).
    let mut skill_radar: Vec<dashboard::SkillRadarEntry> = prog
        .skills
        .iter()
        .map(|(skill_id, s)| dashboard::SkillRadarEntry {
            skill_id: skill_id.clone(),
            level: s.level,
            xp: s.xp,
        })
        .collect();
    skill_radar.sort_by(|a, b| a.skill_id.cmp(&b.skill_id));

    // Build ZPD visualization — gap computed at runtime (never stored, Constitution §7).
    let mut zpd_visualization: Vec<dashboard::ZpdVisualizationEntry> = prog
        .skills
        .iter()
        .map(|(skill_id, s)| dashboard::ZpdVisualizationEntry {
            skill_id: skill_id.clone(),
            independent_level: s.zpd.independent_level,
            scaffolded_level: s.zpd.scaffolded_level,
            gap: s.zpd.gap(),
        })
        .collect();
    zpd_visualization.sort_by(|a, b| a.skill_id.cmp(&b.skill_id));

    // Return only the 5 most recently earned badges.
    let recent_badges: Vec<progress::EarnedBadge> =
        prog.badges.iter().rev().take(5).cloned().collect();

    let overview = dashboard::DashboardOverview {
        name: profile.name,
        age: profile.age,
        interests: profile.interests,
        current_streak_days: prog.streaks.current_days,
        longest_streak_days: prog.streaks.longest_days,
        recent_badges,
        skill_radar,
        zpd_visualization,
        total_sessions: prog.total_sessions,
        total_time_minutes: prog.total_time_minutes,
        total_assignments: prog.total_assignments,
    };

    (StatusCode::OK, Json(serde_json::json!(overview))).into_response()
}

/// `GET /api/v1/learners/:id/dashboard/skill/:skill_id`
///
/// Returns detailed data for a single skill: level, XP, ZPD (gap computed at
/// runtime), recent accuracy, working memory signal, and spaced-repetition health.
/// Read lock: read-only view of progress.
async fn get_skill_detail(
    State(state): State<AppState>,
    Path((id, skill_id)): Path<(Uuid, String)>,
) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;

    let prog = match progress::read_progress(&state.data_dir, id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => progress::LearnerProgress::default_for(id),
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    let skill = match prog.skills.get(&skill_id) {
        Some(s) => s,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": format!("Skill not found: {}", skill_id),
                    "code": "SKILL_NOT_FOUND"
                })),
            )
                .into_response();
        }
    };

    let today = chrono::Local::now().date_naive();
    let health = progress::classify_skill_health(&skill.spaced_repetition, today);

    // XP needed to reach the next level (0 at max level 10).
    // Formula: level = floor(xp/100) + 1, so next level boundary = level * 100.
    let xp_to_next_level = if skill.level >= 10 {
        0
    } else {
        (skill.level * 100).saturating_sub(skill.xp)
    };

    let detail = dashboard::SkillDetailView {
        skill_id: skill_id.clone(),
        level: skill.level,
        xp: skill.xp,
        xp_to_next_level,
        independent_level: skill.zpd.independent_level,
        scaffolded_level: skill.zpd.scaffolded_level,
        zpd_gap: skill.zpd.gap(), // computed at runtime
        recent_accuracy: skill.recent_accuracy.clone(),
        recent_accuracy_fraction: skill.recent_accuracy_fraction(),
        working_memory_signal: skill.working_memory_signal.clone(),
        spaced_repetition_health: health,
        last_practiced: skill.last_practiced.map(|d| d.to_string()),
    };

    (StatusCode::OK, Json(serde_json::json!(detail))).into_response()
}

/// `GET /api/v1/learners/:id/dashboard/behavioral-insights`
///
/// Returns observed behavioral dimensions (from the profile), metacognition
/// metrics (from progress), and recent session behavioral observations.
/// Missing session files are silently skipped — partial data preferred.
/// Read lock: read-only view of profile, progress, and session files.
async fn get_behavioral_insights(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;

    let profile = match learner::read_profile(&state.data_dir, id).await {
        Ok(p) => p,
        Err(e) => {
            let (status, body) = learner_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    let prog = match progress::read_progress(&state.data_dir, id).await {
        Ok(p) => p,
        Err(progress::ProgressError::NotFound(_)) => progress::LearnerProgress::default_for(id),
        Err(e) => {
            let (status, body) = progress_error_response(e);
            return (status, Json(serde_json::to_value(body.0).unwrap())).into_response();
        }
    };

    // Load recent session summaries (up to 3) for behavioral observations.
    // Missing or corrupt session files are handled gracefully by load_session_summaries.
    let summaries = session::load_session_summaries(&state.data_dir, id, 3).await;
    let recent_observations: Vec<dashboard::SessionObservation> = summaries
        .into_iter()
        .map(|s| dashboard::SessionObservation {
            session_date: s.date,
            observations: s.behavioral_observations,
            continuity_notes: s.continuity_notes,
        })
        .collect();

    let insights = dashboard::BehavioralInsightsView {
        frustration_response: profile.observed_behavior.frustration_response,
        effort_attribution: profile.observed_behavior.effort_attribution,
        hint_usage: profile.observed_behavior.hint_usage,
        optimal_session_minutes: profile
            .observed_behavior
            .attention_pattern
            .optimal_session_minutes,
        accuracy_decay_onset: profile
            .observed_behavior
            .attention_pattern
            .accuracy_decay_onset,
        self_correction_rate: prog.metacognition.self_correction_rate,
        hint_request_rate: prog.metacognition.hint_request_rate,
        metacognition_trend: prog.metacognition.trend,
        recent_observations,
    };

    (StatusCode::OK, Json(serde_json::json!(insights))).into_response()
}

/// `GET /api/v1/learners/:id/dashboard/review-queue`
///
/// Returns only pending review items (confirmed and overridden items are
/// not surfaced — they're already resolved).
/// Read lock: read-only view of the review queue file.
async fn get_review_queue(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;

    let queue = match dashboard::read_review_queue(&state.data_dir, id).await {
        Ok(q) => q,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to read review queue: {e}"),
                    "code": "REVIEW_QUEUE_ERROR"
                })),
            )
                .into_response();
        }
    };

    // Return only pending items — already-resolved items are not shown.
    let pending: Vec<&dashboard::ReviewQueueItem> = queue
        .items
        .iter()
        .filter(|item| item.status == dashboard::ReviewStatus::Pending)
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "items": pending,
            "total": pending.len()
        })),
    )
        .into_response()
}

/// JSON body for `POST /api/v1/learners/:id/dashboard/review-queue/:item_id`.
///
/// Only the parent's decision and optional notes can be set — assignment content,
/// correct answer, and Claude's assessment are immutable (Constitution §5).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewDecisionRequest {
    /// The parent's decision: `"confirmed"`, `"overridden"`, or `"discuss"`.
    decision: String,
    /// Optional notes explaining the decision.
    #[serde(default)]
    notes: Option<String>,
}

/// `POST /api/v1/learners/:id/dashboard/review-queue/:item_id`
///
/// Apply a parent decision (confirm / override / discuss) to a review queue item.
/// Only modifies `status` and `parentNotes` — all other fields are immutable.
/// Write lock: modifies the review queue file.
async fn update_review_item(
    State(state): State<AppState>,
    Path((id, item_id)): Path<(Uuid, String)>,
    Json(req): Json<ReviewDecisionRequest>,
) -> impl IntoResponse {
    // Write lock: read-modify-write on the review queue.
    let _guard = state.locks.write(id).await;

    let new_status = match req.decision.as_str() {
        "confirmed" => dashboard::ReviewStatus::Confirmed,
        "overridden" => dashboard::ReviewStatus::Overridden,
        "discuss" => dashboard::ReviewStatus::Discuss,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Invalid decision: '{}'. Must be 'confirmed', 'overridden', or 'discuss'", other),
                    "code": "INVALID_DECISION"
                })),
            )
                .into_response();
        }
    };

    let mut queue = match dashboard::read_review_queue(&state.data_dir, id).await {
        Ok(q) => q,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to read review queue: {e}"),
                    "code": "REVIEW_QUEUE_ERROR"
                })),
            )
                .into_response();
        }
    };

    // Find the item by ID — only status and parentNotes can be changed.
    // Collect the data we need before releasing the mutable borrow.
    let item_index = queue.items.iter().position(|i| i.id == item_id);

    match item_index {
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Review item not found: {}", item_id),
                "code": "REVIEW_ITEM_NOT_FOUND"
            })),
        )
            .into_response(),
        Some(idx) => {
            queue.items[idx].status = new_status;
            queue.items[idx].parent_notes = req.notes;

            // Capture the response fields before writing (avoids borrow-after-write).
            let resp_id = queue.items[idx].id.clone();
            let resp_status = queue.items[idx].status.clone();
            let resp_notes = queue.items[idx].parent_notes.clone();

            match dashboard::write_review_queue(&state.data_dir, id, &queue).await {
                Ok(_) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "id": resp_id,
                        "status": resp_status,
                        "parentNotes": resp_notes
                    })),
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Failed to write review queue: {e}"),
                        "code": "REVIEW_QUEUE_WRITE_ERROR"
                    })),
                )
                    .into_response(),
            }
        }
    }
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

    let dashboard_routes = Router::new()
        .route("/overview", get(get_dashboard_overview))
        .route("/skill/:skill_id", get(get_skill_detail))
        .route("/behavioral-insights", get(get_behavioral_insights))
        .route("/review-queue", get(get_review_queue))
        .route("/review-queue/:item_id", post(update_review_item));

    let learner_routes = Router::new()
        .route("/", post(create_learner).get(list_learners))
        .route(
            "/:id",
            get(get_learner).put(update_learner).delete(delete_learner),
        )
        .route("/:id/skill-health", get(get_skill_health))
        .route("/:id/buffer", get(get_buffer_status))
        .route("/:id/buffer/replenish", post(replenish_buffer_handler))
        .route("/:id/sync", post(sync_offline_sessions))
        .nest("/:id/assignments", assignment_routes)
        .nest("/:id/sessions", session_routes)
        .nest("/:id/dashboard", dashboard_routes);

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
