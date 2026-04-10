// Learner profile management — CRUD operations for learner profiles.
// See CLAUDE.md "Learner Profile" section for schema details.

pub mod profile;

pub use profile::*;

use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

const EXPECTED_SCHEMA_VERSION: u32 = 1;

/// Errors that can occur during learner profile operations.
#[derive(Debug, Error)]
pub enum LearnerError {
    #[error("Learner not found: {0}")]
    NotFound(Uuid),

    #[error("Schema version mismatch: expected {expected}, got {actual}")]
    InvalidSchemaVersion { expected: u32, actual: u32 },

    #[error("Invalid profile: {0}")]
    InvalidProfile(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Returns the path to a learner's directory: `{data_dir}/learners/{learner_id}/`
fn learner_dir(data_dir: &Path, learner_id: Uuid) -> std::path::PathBuf {
    data_dir.join("learners").join(learner_id.to_string())
}

/// Returns the path to a learner's profile file: `{data_dir}/learners/{learner_id}/profile.json`
fn profile_path(data_dir: &Path, learner_id: Uuid) -> std::path::PathBuf {
    learner_dir(data_dir, learner_id).join("profile.json")
}

/// Creates a new learner profile, writing `profile.json` and creating the `sessions/` subdirectory.
pub async fn create_profile(data_dir: &Path, profile: &LearnerProfile) -> Result<(), LearnerError> {
    if profile.name.trim().is_empty() {
        return Err(LearnerError::InvalidProfile(
            "name must not be empty".to_string(),
        ));
    }

    let dir = learner_dir(data_dir, profile.id);
    tokio::fs::create_dir_all(dir.join("sessions")).await?;

    let json = serde_json::to_string_pretty(profile)?;
    tokio::fs::write(profile_path(data_dir, profile.id), json).await?;

    Ok(())
}

/// Reads a learner profile from disk, validating the schema version.
pub async fn read_profile(
    data_dir: &Path,
    learner_id: Uuid,
) -> Result<LearnerProfile, LearnerError> {
    let path = profile_path(data_dir, learner_id);

    let bytes = tokio::fs::read(&path).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            LearnerError::NotFound(learner_id)
        } else {
            LearnerError::Io(e)
        }
    })?;

    let profile: LearnerProfile = serde_json::from_slice(&bytes)?;

    if profile.schema_version != EXPECTED_SCHEMA_VERSION {
        return Err(LearnerError::InvalidSchemaVersion {
            expected: EXPECTED_SCHEMA_VERSION,
            actual: profile.schema_version,
        });
    }

    Ok(profile)
}

/// Overwrites an existing learner's `profile.json` with updated data.
pub async fn update_profile(data_dir: &Path, profile: &LearnerProfile) -> Result<(), LearnerError> {
    if profile.name.trim().is_empty() {
        return Err(LearnerError::InvalidProfile(
            "name must not be empty".to_string(),
        ));
    }

    // Ensure the learner exists before updating.
    let path = profile_path(data_dir, profile.id);
    if !tokio::fs::try_exists(&path).await? {
        return Err(LearnerError::NotFound(profile.id));
    }

    let json = serde_json::to_string_pretty(profile)?;
    tokio::fs::write(path, json).await?;

    Ok(())
}

/// Deletes a learner's entire directory (profile, sessions, and all data).
pub async fn delete_profile(data_dir: &Path, learner_id: Uuid) -> Result<(), LearnerError> {
    let dir = learner_dir(data_dir, learner_id);

    if !tokio::fs::try_exists(&dir).await? {
        return Err(LearnerError::NotFound(learner_id));
    }

    tokio::fs::remove_dir_all(dir).await?;

    Ok(())
}

/// Returns all learner profiles found under `{data_dir}/learners/`.
pub async fn list_profiles(data_dir: &Path) -> Result<Vec<LearnerProfile>, LearnerError> {
    let learners_dir = data_dir.join("learners");

    // If the directory doesn't exist yet, return an empty list.
    if !tokio::fs::try_exists(&learners_dir).await? {
        return Ok(vec![]);
    }

    let mut entries = tokio::fs::read_dir(&learners_dir).await?;
    let mut profiles = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let entry_path = entry.path();

        if !entry_path.is_dir() {
            continue;
        }

        let profile_file = entry_path.join("profile.json");
        if !profile_file.exists() {
            continue;
        }

        let bytes = tokio::fs::read(&profile_file).await?;
        match serde_json::from_slice::<LearnerProfile>(&bytes) {
            Ok(p) if p.schema_version == EXPECTED_SCHEMA_VERSION => profiles.push(p),
            Ok(p) => {
                tracing::warn!(
                    "Skipping learner at {:?}: schema version {} (expected {})",
                    entry_path,
                    p.schema_version,
                    EXPECTED_SCHEMA_VERSION
                );
            }
            Err(e) => {
                tracing::warn!("Skipping learner at {:?}: JSON error: {}", entry_path, e);
            }
        }
    }

    Ok(profiles)
}
