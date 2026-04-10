/// Data structures for the learner profile (`profile.json`).
/// See CLAUDE.md → "Learner Profile" for the full schema.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How a child prefers to be challenged — set by the parent at onboarding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChallengePreference {
    Independent,
    Guided,
    Collaborative,
}

/// How the child tends to respond when frustrated — observed from sessions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrustrationResponse {
    #[default]
    Unknown,
    Perseveres,
    SlowsDown,
    Rushes,
    Disengages,
}

/// Whether the child attributes effort to process or outcome — observed from sessions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffortAttribution {
    #[default]
    Unknown,
    ProcessOriented,
    OutcomeOriented,
}

/// How proactively the child requests hints — observed from sessions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HintUsage {
    #[default]
    Unknown,
    Proactive,
    Reactive,
    Avoidant,
}

/// Attention span data derived from accuracy-over-time curves.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionPattern {
    /// Minutes into a session where the child performs best — `null` until derived.
    pub optimal_session_minutes: Option<u32>,
    /// Minute mark where accuracy starts dropping — `null` until derived.
    pub accuracy_decay_onset: Option<u32>,
}

/// Behavioral signals observed from session data — never set via API.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedBehavior {
    pub frustration_response: FrustrationResponse,
    pub effort_attribution: EffortAttribution,
    pub hint_usage: HintUsage,
    pub attention_pattern: AttentionPattern,
}

/// Parent-set starting preferences for a learner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitialPreferences {
    pub session_length_minutes: u32,
    pub challenge_preference: ChallengePreference,
}

/// The full learner profile stored in `profile.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnerProfile {
    pub schema_version: u32,
    pub id: Uuid,
    /// Child-chosen display name (any alias — never a real name).
    pub name: String,
    pub age: u8,
    pub interests: Vec<String>,
    pub initial_preferences: InitialPreferences,
    /// Populated entirely by the session system — never set via API.
    pub observed_behavior: ObservedBehavior,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile() -> LearnerProfile {
        LearnerProfile {
            schema_version: 1,
            id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            name: "StarExplorer42".to_string(),
            age: 8,
            interests: vec!["dinosaurs".to_string(), "space".to_string()],
            initial_preferences: InitialPreferences {
                session_length_minutes: 25,
                challenge_preference: ChallengePreference::Guided,
            },
            observed_behavior: ObservedBehavior::default(),
        }
    }

    #[test]
    fn test_serde_round_trip() {
        let profile = sample_profile();
        let json = serde_json::to_string(&profile).expect("serialize");
        let restored: LearnerProfile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(profile, restored);
    }

    #[test]
    fn test_camel_case_fields() {
        let profile = sample_profile();
        let json = serde_json::to_string(&profile).expect("serialize");
        assert!(json.contains("\"schemaVersion\""));
        assert!(json.contains("\"initialPreferences\""));
        assert!(json.contains("\"sessionLengthMinutes\""));
        assert!(json.contains("\"challengePreference\""));
        assert!(json.contains("\"observedBehavior\""));
        assert!(json.contains("\"frustrationResponse\""));
        assert!(json.contains("\"effortAttribution\""));
        assert!(json.contains("\"hintUsage\""));
        assert!(json.contains("\"attentionPattern\""));
        assert!(json.contains("\"optimalSessionMinutes\""));
        assert!(json.contains("\"accuracyDecayOnset\""));
    }

    #[test]
    fn test_enum_kebab_case_serialization() {
        // ChallengePreference
        assert_eq!(
            serde_json::to_string(&ChallengePreference::Independent).unwrap(),
            "\"independent\""
        );
        assert_eq!(
            serde_json::to_string(&ChallengePreference::Guided).unwrap(),
            "\"guided\""
        );
        assert_eq!(
            serde_json::to_string(&ChallengePreference::Collaborative).unwrap(),
            "\"collaborative\""
        );

        // FrustrationResponse
        assert_eq!(
            serde_json::to_string(&FrustrationResponse::Unknown).unwrap(),
            "\"unknown\""
        );
        assert_eq!(
            serde_json::to_string(&FrustrationResponse::SlowsDown).unwrap(),
            "\"slows-down\""
        );
        assert_eq!(
            serde_json::to_string(&FrustrationResponse::Perseveres).unwrap(),
            "\"perseveres\""
        );
        assert_eq!(
            serde_json::to_string(&FrustrationResponse::Rushes).unwrap(),
            "\"rushes\""
        );
        assert_eq!(
            serde_json::to_string(&FrustrationResponse::Disengages).unwrap(),
            "\"disengages\""
        );

        // EffortAttribution
        assert_eq!(
            serde_json::to_string(&EffortAttribution::Unknown).unwrap(),
            "\"unknown\""
        );
        assert_eq!(
            serde_json::to_string(&EffortAttribution::ProcessOriented).unwrap(),
            "\"process-oriented\""
        );
        assert_eq!(
            serde_json::to_string(&EffortAttribution::OutcomeOriented).unwrap(),
            "\"outcome-oriented\""
        );

        // HintUsage
        assert_eq!(
            serde_json::to_string(&HintUsage::Unknown).unwrap(),
            "\"unknown\""
        );
        assert_eq!(
            serde_json::to_string(&HintUsage::Proactive).unwrap(),
            "\"proactive\""
        );
        assert_eq!(
            serde_json::to_string(&HintUsage::Reactive).unwrap(),
            "\"reactive\""
        );
        assert_eq!(
            serde_json::to_string(&HintUsage::Avoidant).unwrap(),
            "\"avoidant\""
        );
    }

    #[test]
    fn test_default_observed_behavior() {
        let ob = ObservedBehavior::default();
        assert_eq!(ob.frustration_response, FrustrationResponse::Unknown);
        assert_eq!(ob.effort_attribution, EffortAttribution::Unknown);
        assert_eq!(ob.hint_usage, HintUsage::Unknown);
        assert_eq!(ob.attention_pattern.optimal_session_minutes, None);
        assert_eq!(ob.attention_pattern.accuracy_decay_onset, None);
    }

    #[test]
    fn test_json_matches_schema() {
        let profile = sample_profile();
        let json = serde_json::to_string_pretty(&profile).expect("serialize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse");

        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["name"], "StarExplorer42");
        assert_eq!(value["age"], 8);
        assert_eq!(value["observedBehavior"]["frustrationResponse"], "unknown");
        assert_eq!(value["observedBehavior"]["effortAttribution"], "unknown");
        assert_eq!(value["observedBehavior"]["hintUsage"], "unknown");
        assert!(value["observedBehavior"]["attentionPattern"]["optimalSessionMinutes"].is_null());
        assert!(value["observedBehavior"]["attentionPattern"]["accuracyDecayOnset"].is_null());
        assert_eq!(value["initialPreferences"]["challengePreference"], "guided");
        assert_eq!(value["initialPreferences"]["sessionLengthMinutes"], 25);
    }
}
