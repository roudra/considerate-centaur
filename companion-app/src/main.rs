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
use educational_companion::learner;
use educational_companion::learner::{
    InitialPreferences, LearnerError, LearnerProfile, ObservedBehavior,
};
use educational_companion::lock::LockManager;
use educational_companion::progress;

/// Server-side store of verified assignments awaiting child responses.
///
/// When the generate endpoint creates a verified assignment, it stores it here
/// keyed by a unique assignment ID. The evaluate endpoint looks it up by ID —
/// the client never supplies the correct answer. This prevents clients from
/// forging correctness (Constitution §5).
type AssignmentStore = Arc<Mutex<HashMap<String, VerifiedAssignment>>>;

/// Shared application state passed to every route handler.
#[derive(Clone)]
struct AppState {
    data_dir: Arc<PathBuf>,
    locks: LockManager,
    /// Assignment templates loaded at startup from the curriculum directory.
    templates: Arc<Vec<AssignmentTemplate>>,
    /// Server-side store of pending assignments — keyed by assignment ID.
    pending_assignments: AssignmentStore,
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

    let state = AppState {
        data_dir: Arc::new(data_dir),
        locks: LockManager::new(),
        templates: Arc::new(templates),
        pending_assignments: Arc::new(Mutex::new(HashMap::new())),
    };

    let assignment_routes = Router::new()
        .route("/generate", post(generate_assignment))
        .route("/evaluate", post(evaluate_response));

    let learner_routes = Router::new()
        .route("/", post(create_learner).get(list_learners))
        .route(
            "/:id",
            get(get_learner).put(update_learner).delete(delete_learner),
        )
        .route("/:id/skill-health", get(get_skill_health))
        .nest("/:id/assignments", assignment_routes);

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
