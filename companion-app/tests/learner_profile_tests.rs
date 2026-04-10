/// Integration tests for learner profile file persistence (create, read, update, delete, list).
use educational_companion::learner::{
    self, AttentionPattern, ChallengePreference, EffortAttribution, FrustrationResponse, HintUsage,
    InitialPreferences, LearnerProfile, ObservedBehavior,
};
use tempfile::TempDir;
use uuid::Uuid;

fn make_profile() -> LearnerProfile {
    LearnerProfile {
        schema_version: 1,
        id: Uuid::new_v4(),
        name: "TestExplorer".to_string(),
        age: 9,
        interests: vec!["robots".to_string(), "math".to_string()],
        initial_preferences: InitialPreferences {
            session_length_minutes: 20,
            challenge_preference: ChallengePreference::Guided,
        },
        observed_behavior: ObservedBehavior::default(),
    }
}

#[tokio::test]
async fn test_create_and_read_profile() {
    let tmp = TempDir::new().unwrap();
    let profile = make_profile();

    learner::create_profile(tmp.path(), &profile).await.unwrap();

    let read_back = learner::read_profile(tmp.path(), profile.id).await.unwrap();
    assert_eq!(read_back, profile);
}

#[tokio::test]
async fn test_create_creates_sessions_dir() {
    let tmp = TempDir::new().unwrap();
    let profile = make_profile();

    learner::create_profile(tmp.path(), &profile).await.unwrap();

    let sessions_dir = tmp
        .path()
        .join("learners")
        .join(profile.id.to_string())
        .join("sessions");
    assert!(sessions_dir.is_dir(), "sessions/ directory must be created");
}

#[tokio::test]
async fn test_read_missing_profile_returns_not_found() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();

    let result = learner::read_profile(tmp.path(), id).await;
    assert!(
        matches!(result, Err(learner::LearnerError::NotFound(got_id)) if got_id == id),
        "expected NotFound, got: {:?}",
        result
    );
}

#[tokio::test]
async fn test_read_wrong_schema_version_returns_error() {
    let tmp = TempDir::new().unwrap();
    let mut profile = make_profile();
    profile.schema_version = 99;

    // Write the profile with wrong schema version directly to disk.
    let dir = tmp.path().join("learners").join(profile.id.to_string());
    tokio::fs::create_dir_all(dir.join("sessions"))
        .await
        .unwrap();
    let json = serde_json::to_string_pretty(&profile).unwrap();
    tokio::fs::write(dir.join("profile.json"), json)
        .await
        .unwrap();

    let result = learner::read_profile(tmp.path(), profile.id).await;
    assert!(
        matches!(
            result,
            Err(learner::LearnerError::InvalidSchemaVersion {
                expected: 1,
                actual: 99
            })
        ),
        "expected InvalidSchemaVersion, got: {:?}",
        result
    );
}

#[tokio::test]
async fn test_update_profile() {
    let tmp = TempDir::new().unwrap();
    let mut profile = make_profile();
    learner::create_profile(tmp.path(), &profile).await.unwrap();

    profile.name = "UpdatedName".to_string();
    profile.age = 10;
    learner::update_profile(tmp.path(), &profile).await.unwrap();

    let read_back = learner::read_profile(tmp.path(), profile.id).await.unwrap();
    assert_eq!(read_back.name, "UpdatedName");
    assert_eq!(read_back.age, 10);
}

#[tokio::test]
async fn test_update_nonexistent_profile_returns_not_found() {
    let tmp = TempDir::new().unwrap();
    let profile = make_profile();

    let result = learner::update_profile(tmp.path(), &profile).await;
    assert!(
        matches!(result, Err(learner::LearnerError::NotFound(_))),
        "expected NotFound, got: {:?}",
        result
    );
}

#[tokio::test]
async fn test_delete_profile() {
    let tmp = TempDir::new().unwrap();
    let profile = make_profile();
    learner::create_profile(tmp.path(), &profile).await.unwrap();

    learner::delete_profile(tmp.path(), profile.id)
        .await
        .unwrap();

    let result = learner::read_profile(tmp.path(), profile.id).await;
    assert!(
        matches!(result, Err(learner::LearnerError::NotFound(_))),
        "expected NotFound after delete, got: {:?}",
        result
    );
}

#[tokio::test]
async fn test_delete_nonexistent_returns_not_found() {
    let tmp = TempDir::new().unwrap();
    let id = Uuid::new_v4();

    let result = learner::delete_profile(tmp.path(), id).await;
    assert!(
        matches!(result, Err(learner::LearnerError::NotFound(_))),
        "expected NotFound, got: {:?}",
        result
    );
}

#[tokio::test]
async fn test_list_profiles_empty_dir() {
    let tmp = TempDir::new().unwrap();
    let profiles = learner::list_profiles(tmp.path()).await.unwrap();
    assert!(profiles.is_empty());
}

#[tokio::test]
async fn test_list_profiles_returns_all() {
    let tmp = TempDir::new().unwrap();
    let p1 = make_profile();
    let p2 = make_profile();
    learner::create_profile(tmp.path(), &p1).await.unwrap();
    learner::create_profile(tmp.path(), &p2).await.unwrap();

    let mut profiles = learner::list_profiles(tmp.path()).await.unwrap();
    assert_eq!(profiles.len(), 2);

    profiles.sort_by_key(|p| p.id);
    let mut expected_ids = vec![p1.id, p2.id];
    expected_ids.sort();
    let got_ids: Vec<Uuid> = profiles.iter().map(|p| p.id).collect();
    assert_eq!(got_ids, expected_ids);
}

#[tokio::test]
async fn test_list_profiles_skips_bad_schema_version() {
    let tmp = TempDir::new().unwrap();

    // Create one valid profile.
    let good = make_profile();
    learner::create_profile(tmp.path(), &good).await.unwrap();

    // Write one with wrong schema version.
    let mut bad = make_profile();
    bad.schema_version = 99;
    let dir = tmp.path().join("learners").join(bad.id.to_string());
    tokio::fs::create_dir_all(dir.join("sessions"))
        .await
        .unwrap();
    let json = serde_json::to_string_pretty(&bad).unwrap();
    tokio::fs::write(dir.join("profile.json"), json)
        .await
        .unwrap();

    let profiles = learner::list_profiles(tmp.path()).await.unwrap();
    assert_eq!(profiles.len(), 1, "bad schema version should be skipped");
    assert_eq!(profiles[0].id, good.id);
}

#[tokio::test]
async fn test_observed_behavior_preserved_across_update() {
    let tmp = TempDir::new().unwrap();
    let mut profile = make_profile();
    learner::create_profile(tmp.path(), &profile).await.unwrap();

    // Manually set observed_behavior fields (as the session system would).
    profile.observed_behavior = ObservedBehavior {
        frustration_response: FrustrationResponse::Perseveres,
        effort_attribution: EffortAttribution::ProcessOriented,
        hint_usage: HintUsage::Proactive,
        attention_pattern: AttentionPattern {
            optimal_session_minutes: Some(20),
            accuracy_decay_onset: Some(18),
        },
    };
    learner::update_profile(tmp.path(), &profile).await.unwrap();

    let read_back = learner::read_profile(tmp.path(), profile.id).await.unwrap();
    assert_eq!(
        read_back.observed_behavior.frustration_response,
        FrustrationResponse::Perseveres
    );
    assert_eq!(
        read_back
            .observed_behavior
            .attention_pattern
            .optimal_session_minutes,
        Some(20)
    );
}
