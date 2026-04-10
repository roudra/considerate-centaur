use anyhow::Context;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use educational_companion::learner;
use educational_companion::learner::{InitialPreferences, LearnerError, LearnerProfile, ObservedBehavior};
use educational_companion::lock::LockManager;

/// Shared application state passed to every route handler.
#[derive(Clone)]
struct AppState {
    data_dir: Arc<PathBuf>,
    locks: LockManager,
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
            Json(ErrorResponse::new(
                format!("I/O error: {e}"),
                "IO_ERROR",
            )),
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
        Ok(()) => (StatusCode::CREATED, Json(serde_json::to_value(&profile).unwrap())).into_response(),
        Err(e) => {
            let (status, body) = learner_error_response(e);
            (status, Json(serde_json::to_value(body.0).unwrap())).into_response()
        }
    }
}

/// `GET /api/v1/learners` — list all learners.
async fn list_learners(State(state): State<AppState>) -> impl IntoResponse {
    match learner::list_profiles(&state.data_dir).await {
        Ok(profiles) => (StatusCode::OK, Json(serde_json::to_value(profiles).unwrap())).into_response(),
        Err(e) => {
            let (status, body) = learner_error_response(e);
            (status, Json(serde_json::to_value(body.0).unwrap())).into_response()
        }
    }
}

/// `GET /api/v1/learners/:id` — get a learner profile.
async fn get_learner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.read(id).await;
    match learner::read_profile(&state.data_dir, id).await {
        Ok(profile) => (StatusCode::OK, Json(serde_json::to_value(profile).unwrap())).into_response(),
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
        Ok(()) => (StatusCode::OK, Json(serde_json::to_value(&updated).unwrap())).into_response(),
        Err(e) => {
            let (status, body) = learner_error_response(e);
            (status, Json(serde_json::to_value(body.0).unwrap())).into_response()
        }
    }
}

/// `DELETE /api/v1/learners/:id` — delete a learner and all their data.
async fn delete_learner(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let _guard = state.locks.write(id).await;
    match learner::delete_profile(&state.data_dir, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let (status, body) = learner_error_response(e);
            (status, Json(serde_json::to_value(body.0).unwrap())).into_response()
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tracing::info!("Educational Companion starting up");

    let data_dir = PathBuf::from(
        std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string()),
    );

    let state = AppState {
        data_dir: Arc::new(data_dir),
        locks: LockManager::new(),
    };

    let learner_routes = Router::new()
        .route("/", post(create_learner).get(list_learners))
        .route("/:id", get(get_learner).put(update_learner).delete(delete_learner));

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .nest("/api/v1/learners", learner_routes)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .context("Failed to bind to port 3000")?;

    tracing::info!("Listening on http://0.0.0.0:3000");

    axum::serve(listener, app)
        .await
        .context("Server error")?;

    Ok(())
}
