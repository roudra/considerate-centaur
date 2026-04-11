/// Data structures for learner progress (`progress.json`).
/// See CLAUDE.md → "Progress Tracking" for the full schema and field notes.
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Maximum number of entries in the `recentAccuracy` ring buffer.
pub const RECENT_ACCURACY_MAX: usize = 5;

/// Working memory load signal derived from multi-step problem performance.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkingMemorySignal {
    #[default]
    Stable,
    Overloaded,
}

/// Trend in metacognitive skill observed across recent sessions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetacognitionTrend {
    Improving,
    #[default]
    Stable,
    Declining,
}

/// Zone of Proximal Development levels for a skill.
///
/// The ZPD gap (`scaffoldedLevel - independentLevel`) is **always computed at runtime** —
/// it is never stored in JSON to avoid inconsistency when either level is updated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZpdLevels {
    /// Difficulty level the child can solve without help.
    pub independent_level: u32,
    /// Difficulty level the child can reach with hints.
    pub scaffolded_level: u32,
}

impl ZpdLevels {
    /// Compute the ZPD gap at runtime (`scaffoldedLevel - independentLevel`).
    ///
    /// This is always computed — never stored — to prevent inconsistency.
    pub fn gap(&self) -> u32 {
        self.scaffolded_level.saturating_sub(self.independent_level)
    }
}

impl Default for ZpdLevels {
    fn default() -> Self {
        Self {
            independent_level: 1,
            scaffolded_level: 2,
        }
    }
}

/// Spaced repetition scheduling data for a skill (SM-2 inspired).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpacedRepetition {
    /// Current review interval in days.
    pub interval_days: u32,
    /// SM-2 ease factor (minimum 1.3).
    pub ease_factor: f32,
    /// Next scheduled review date.
    pub next_review_date: NaiveDate,
    /// Consecutive correct answers at the current level.
    pub consecutive_correct: u32,
}

impl Default for SpacedRepetition {
    fn default() -> Self {
        Self {
            interval_days: 1,
            ease_factor: 2.5,
            next_review_date: chrono::Local::now().date_naive(),
            consecutive_correct: 0,
        }
    }
}

/// Progress record for a single skill.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillProgress {
    pub level: u32,
    pub xp: u32,
    /// Date of last practice — `null` for a skill that has never been practiced.
    pub last_practiced: Option<NaiveDate>,
    pub zpd: ZpdLevels,
    /// Ring buffer of the last [`RECENT_ACCURACY_MAX`] attempts: `1` = correct, `0` = incorrect.
    /// Never exceeds [`RECENT_ACCURACY_MAX`] entries.
    pub recent_accuracy: Vec<u8>,
    pub working_memory_signal: WorkingMemorySignal,
    pub spaced_repetition: SpacedRepetition,
}

impl SkillProgress {
    /// Record a new accuracy result, maintaining the ring buffer of at most
    /// [`RECENT_ACCURACY_MAX`] entries (oldest entries are evicted first).
    pub fn record_accuracy(&mut self, correct: bool) {
        self.recent_accuracy.push(u8::from(correct));
        if self.recent_accuracy.len() > RECENT_ACCURACY_MAX {
            let excess = self.recent_accuracy.len() - RECENT_ACCURACY_MAX;
            self.recent_accuracy.drain(..excess);
        }
    }

    /// Compute recent accuracy as a fraction in `0.0..=1.0`.
    /// Returns `0.0` when no attempts have been recorded yet.
    pub fn recent_accuracy_fraction(&self) -> f32 {
        if self.recent_accuracy.is_empty() {
            return 0.0;
        }
        let sum: u32 = self.recent_accuracy.iter().map(|&v| v as u32).sum();
        sum as f32 / self.recent_accuracy.len() as f32
    }
}

/// A badge that has been earned by a learner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EarnedBadge {
    pub id: String,
    pub name: String,
    pub earned_date: NaiveDate,
    pub category: String,
}

/// Streak tracking data.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Streaks {
    pub current_days: u32,
    pub longest_days: u32,
    /// Date the streak shield was last used. `None` means never used.
    /// A shield can only be used once per 7-day window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shield_last_used: Option<NaiveDate>,
}

/// Metacognition signals derived from session behavior.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metacognition {
    /// Fraction of assignments where the child changed their answer before submitting.
    pub self_correction_rate: f32,
    /// Proactive hint requests per assignment.
    pub hint_request_rate: f32,
    pub trend: MetacognitionTrend,
}

impl Default for Metacognition {
    fn default() -> Self {
        Self {
            self_correction_rate: 0.0,
            hint_request_rate: 0.0,
            trend: MetacognitionTrend::default(),
        }
    }
}

/// The full progress record for a learner, stored in `progress.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnerProgress {
    pub schema_version: u32,
    /// Internal learner UUID — **never sent to Claude** (omit when building API payloads).
    pub learner_id: Uuid,
    /// Per-skill progress keyed by skill ID (e.g. `"pattern-recognition"`).
    pub skills: HashMap<String, SkillProgress>,
    pub badges: Vec<EarnedBadge>,
    pub streaks: Streaks,
    pub total_sessions: u32,
    pub total_time_minutes: u32,
    /// Total individual assignments completed across all sessions.
    pub total_assignments: u32,
    pub metacognition: Metacognition,
    /// Boolean flags for challenge/milestone badges
    /// (e.g. `"onboardingComplete"`, `"timedChallenge80"`, `"bossComplete"`, `"teachBackSuccess"`).
    #[serde(default)]
    pub challenge_flags: HashMap<String, bool>,
}

impl LearnerProgress {
    /// Create a minimal default progress record for a learner that has no
    /// existing `progress.json` (e.g. a brand-new learner).
    pub fn default_for(learner_id: Uuid) -> Self {
        Self {
            schema_version: 1,
            learner_id,
            skills: HashMap::new(),
            badges: Vec::new(),
            streaks: Streaks::default(),
            total_sessions: 0,
            total_time_minutes: 0,
            total_assignments: 0,
            metacognition: Metacognition::default(),
            challenge_flags: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_progress(learner_id: Uuid) -> LearnerProgress {
        let mut skills = HashMap::new();
        skills.insert(
            "pattern-recognition".to_string(),
            SkillProgress {
                level: 4,
                xp: 340,
                last_practiced: Some(NaiveDate::from_ymd_opt(2026, 4, 7).unwrap()),
                zpd: ZpdLevels {
                    independent_level: 3,
                    scaffolded_level: 5,
                },
                recent_accuracy: vec![1, 1, 0, 1, 1],
                working_memory_signal: WorkingMemorySignal::Stable,
                spaced_repetition: SpacedRepetition {
                    interval_days: 7,
                    ease_factor: 2.5,
                    next_review_date: NaiveDate::from_ymd_opt(2026, 4, 14).unwrap(),
                    consecutive_correct: 4,
                },
            },
        );
        LearnerProgress {
            schema_version: 1,
            learner_id,
            skills,
            badges: vec![EarnedBadge {
                id: "first-puzzle".to_string(),
                name: "Puzzle Pioneer".to_string(),
                earned_date: NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
                category: "milestone".to_string(),
            }],
            streaks: Streaks {
                current_days: 5,
                longest_days: 12,
                shield_last_used: None,
            },
            total_sessions: 28,
            total_time_minutes: 680,
            total_assignments: 140,
            metacognition: Metacognition {
                self_correction_rate: 0.3,
                hint_request_rate: 0.15,
                trend: MetacognitionTrend::Improving,
            },
            challenge_flags: HashMap::new(),
        }
    }

    #[test]
    fn test_serde_round_trip() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let progress = sample_progress(id);
        let json = serde_json::to_string(&progress).expect("serialize");
        let restored: LearnerProgress = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(progress, restored);
    }

    #[test]
    fn test_no_gap_field_in_json() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let progress = sample_progress(id);
        let json = serde_json::to_string(&progress).expect("serialize");
        // ZPD gap must NOT appear in the serialized JSON.
        assert!(!json.contains("\"gap\""), "gap must not be stored in JSON");
    }

    #[test]
    fn test_zpd_gap_computed() {
        let zpd = ZpdLevels {
            independent_level: 3,
            scaffolded_level: 5,
        };
        assert_eq!(zpd.gap(), 2);
    }

    #[test]
    fn test_zpd_gap_no_underflow() {
        let zpd = ZpdLevels {
            independent_level: 5,
            scaffolded_level: 3,
        };
        assert_eq!(zpd.gap(), 0);
    }

    #[test]
    fn test_ring_buffer_max_5() {
        let mut skill = SkillProgress::default();
        for i in 0..10 {
            skill.record_accuracy(i % 2 == 0);
        }
        assert_eq!(skill.recent_accuracy.len(), RECENT_ACCURACY_MAX);
    }

    #[test]
    fn test_ring_buffer_evicts_oldest() {
        let mut skill = SkillProgress::default();
        // Push 6 entries: first entry (1) should be evicted.
        skill.record_accuracy(true); // index 0 → will be evicted
        skill.record_accuracy(false); // index 1
        skill.record_accuracy(true); // index 2
        skill.record_accuracy(false); // index 3
        skill.record_accuracy(true); // index 4
        skill.record_accuracy(false); // index 5 → pushes out index 0

        assert_eq!(skill.recent_accuracy.len(), RECENT_ACCURACY_MAX);
        assert_eq!(skill.recent_accuracy[0], 0); // was index 1
        assert_eq!(skill.recent_accuracy[4], 0); // last entry
    }

    #[test]
    fn test_recent_accuracy_fraction() {
        let mut skill = SkillProgress::default();
        skill.recent_accuracy = vec![1, 1, 0, 1, 1];
        assert!((skill.recent_accuracy_fraction() - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_recent_accuracy_fraction_empty() {
        let skill = SkillProgress::default();
        assert_eq!(skill.recent_accuracy_fraction(), 0.0);
    }

    #[test]
    fn test_camel_case_fields() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let progress = sample_progress(id);
        let json = serde_json::to_string(&progress).expect("serialize");
        assert!(json.contains("\"schemaVersion\""));
        assert!(json.contains("\"learnerId\""));
        assert!(json.contains("\"totalSessions\""));
        assert!(json.contains("\"totalTimeMinutes\""));
        assert!(json.contains("\"totalAssignments\""));
        assert!(json.contains("\"selfCorrectionRate\""));
        assert!(json.contains("\"hintRequestRate\""));
        assert!(json.contains("\"currentDays\""));
        assert!(json.contains("\"longestDays\""));
        assert!(json.contains("\"lastPracticed\""));
        assert!(json.contains("\"recentAccuracy\""));
        assert!(json.contains("\"workingMemorySignal\""));
        assert!(json.contains("\"spacedRepetition\""));
        assert!(json.contains("\"intervalDays\""));
        assert!(json.contains("\"easeFactor\""));
        assert!(json.contains("\"nextReviewDate\""));
        assert!(json.contains("\"consecutiveCorrect\""));
        assert!(json.contains("\"independentLevel\""));
        assert!(json.contains("\"scaffoldedLevel\""));
    }

    #[test]
    fn test_enum_kebab_case() {
        assert_eq!(
            serde_json::to_string(&WorkingMemorySignal::Stable).unwrap(),
            "\"stable\""
        );
        assert_eq!(
            serde_json::to_string(&WorkingMemorySignal::Overloaded).unwrap(),
            "\"overloaded\""
        );
        assert_eq!(
            serde_json::to_string(&MetacognitionTrend::Improving).unwrap(),
            "\"improving\""
        );
        assert_eq!(
            serde_json::to_string(&MetacognitionTrend::Stable).unwrap(),
            "\"stable\""
        );
        assert_eq!(
            serde_json::to_string(&MetacognitionTrend::Declining).unwrap(),
            "\"declining\""
        );
    }
}
