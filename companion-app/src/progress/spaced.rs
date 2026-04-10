/// SM-2 inspired spaced repetition scheduler.
///
/// See CLAUDE.md → "Spaced Repetition" for the full algorithm spec, session mix
/// strategy, and dashboard visibility tiers.
use chrono::{Days, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{LearnerProgress, SpacedRepetition};

/// Minimum ease factor — the SM-2 ease factor never drops below this value.
pub const MIN_EASE_FACTOR: f32 = 1.3;

/// Accuracy tier thresholds (as fractions in `0.0..=1.0`).
const HIGH_ACCURACY: f32 = 0.80;
const MODERATE_ACCURACY: f32 = 0.60;

/// Fraction of session slots dedicated to new/advancing content.
const NEW_CONTENT_RATIO: f32 = 0.70;

// ---------------------------------------------------------------------------
// SM-2 update algorithm
// ---------------------------------------------------------------------------

/// Update the [`SpacedRepetition`] fields for a skill after a practice session.
///
/// `session_accuracy` is a fraction in `0.0..=1.0` representing the fraction
/// of problems the learner answered correctly during the session.
/// `today` is passed in explicitly to keep the function pure/testable.
///
/// Three tiers as defined in CLAUDE.md:
/// - ≥ 80% accuracy → increase interval (multiply by ease factor), increment streak
/// - 60–79% accuracy → hold interval, reset streak to 0
/// - < 60% accuracy → reset interval to 1 day, decrease ease factor (floored at
///   [`MIN_EASE_FACTOR`])
pub fn update_spaced_repetition(
    sr: &mut SpacedRepetition,
    session_accuracy: f32,
    today: NaiveDate,
) {
    if session_accuracy >= HIGH_ACCURACY {
        // High accuracy: increase interval, increment consecutive correct count.
        let new_interval = ((sr.interval_days as f32) * sr.ease_factor).round() as u32;
        sr.interval_days = new_interval.max(1);
        sr.consecutive_correct += 1;
    } else if session_accuracy >= MODERATE_ACCURACY {
        // Moderate accuracy: hold interval, reset streak.
        sr.consecutive_correct = 0;
    } else {
        // Low accuracy: reset interval to 1 day, decrease ease factor.
        sr.interval_days = 1;
        sr.ease_factor = (sr.ease_factor - 0.2).max(MIN_EASE_FACTOR);
        sr.consecutive_correct = 0;
    }

    sr.next_review_date = today
        .checked_add_days(Days::new(sr.interval_days as u64))
        .unwrap_or(today);
}

// ---------------------------------------------------------------------------
// Skill health classification
// ---------------------------------------------------------------------------

/// Skill health tier as shown on the parent dashboard "Skill Health Map".
///
/// Tiers are defined in CLAUDE.md → "Dashboard Visibility":
/// - **Fresh** — practiced recently, next review not yet due
/// - **Due** — review date has arrived (0 days overdue)
/// - **Overdue** — missed review window (1–6 days overdue)
/// - **Rusty** — significantly overdue (7+ days overdue)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillHealth {
    Fresh,
    Due,
    Overdue,
    Rusty,
}

/// Classify the health of a skill given `today`.
///
/// - `days_until_review > 0` → Fresh
/// - `days_until_review == 0` → Due
/// - `days_overdue` in 1–6 → Overdue
/// - `days_overdue >= 7` → Rusty
pub fn classify_skill_health(sr: &SpacedRepetition, today: NaiveDate) -> SkillHealth {
    let next = sr.next_review_date;
    if next > today {
        SkillHealth::Fresh
    } else {
        let days_overdue = today
            .signed_duration_since(next)
            .num_days()
            .max(0) as u64;
        match days_overdue {
            0 => SkillHealth::Due,
            1..=6 => SkillHealth::Overdue,
            _ => SkillHealth::Rusty,
        }
    }
}

// ---------------------------------------------------------------------------
// Session mix planning
// ---------------------------------------------------------------------------

/// A skill nominated for review during session planning.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewCandidate {
    /// Skill identifier (e.g. `"pattern-recognition"`).
    pub skill_id: String,
    /// Difficulty level to target for the review assignment.
    /// Review assignments target `independentLevel` (confirming retention, not
    /// pushing difficulty — see CLAUDE.md).
    pub target_level: u32,
    /// How many days past the review date this skill is (0 = due today).
    pub days_overdue: i64,
    /// The skill's current level (used for priority calculation).
    pub skill_level: u32,
}

impl ReviewCandidate {
    /// Priority score: `days_overdue × skill_level`.
    ///
    /// Higher priority → should be reviewed sooner.
    /// When a skill is exactly due (days_overdue == 0) or fresh it is not
    /// included at all; this method is only called for due/overdue/rusty skills.
    pub fn priority(&self) -> i64 {
        self.days_overdue * (self.skill_level as i64)
    }
}

/// Plan the spaced repetition review portion of an upcoming session.
///
/// Returns an ordered list of [`ReviewCandidate`]s that should be included in
/// the next session, together with the number of *new/advancing* assignment
/// slots available.
///
/// ## Session mix
/// The session mix targets approximately **70% new content, 30% review** as
/// specified in CLAUDE.md. Given `total_slots` (the total number of
/// assignments for the session), this function returns:
/// - `new_slots` — number of slots for new/advancing content
/// - `review_candidates` — ordered list of skills to review (highest-priority
///   first), capped so they fill at most the remaining 30% of slots
///
/// When multiple skills are overdue, they are prioritised by
/// `days_overdue × skill_level` (higher-level skills decay are more costly).
///
/// `today` is passed explicitly for testability.
pub fn plan_session_mix(
    progress: &LearnerProgress,
    total_slots: usize,
    today: NaiveDate,
) -> (usize, Vec<ReviewCandidate>) {
    let new_slots = ((total_slots as f32) * NEW_CONTENT_RATIO).round() as usize;
    let review_slots = total_slots.saturating_sub(new_slots);

    // Collect all skills that are due, overdue, or rusty.
    let mut candidates: Vec<ReviewCandidate> = progress
        .skills
        .iter()
        .filter_map(|(skill_id, skill)| {
            let days_overdue = today
                .signed_duration_since(skill.spaced_repetition.next_review_date)
                .num_days();
            if days_overdue >= 0 {
                Some(ReviewCandidate {
                    skill_id: skill_id.clone(),
                    target_level: skill.zpd.independent_level,
                    days_overdue,
                    skill_level: skill.level,
                })
            } else {
                None
            }
        })
        .collect();

    // Sort by priority descending (highest priority first).
    candidates.sort_by_key(|c| std::cmp::Reverse(c.priority()));

    // Cap to the number of available review slots.
    candidates.truncate(review_slots);

    (new_slots, candidates)
}

// ---------------------------------------------------------------------------
// Skill health map (for API response)
// ---------------------------------------------------------------------------

/// One entry in the skill health map returned by the API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillHealthEntry {
    /// Skill identifier (e.g. `"pattern-recognition"`).
    pub skill_id: String,
    /// Current health classification.
    pub health: SkillHealth,
    /// Next scheduled review date.
    pub next_review_date: NaiveDate,
    /// Positive → days until the review is due.
    /// Zero → due today.
    /// Negative → days overdue.
    pub days_until_review: i64,
}

/// Build the full skill health map for a learner, keyed by skill ID.
///
/// `today` is passed explicitly for testability.
pub fn build_skill_health_map(
    progress: &LearnerProgress,
    today: NaiveDate,
) -> HashMap<String, SkillHealthEntry> {
    progress
        .skills
        .iter()
        .map(|(skill_id, skill)| {
            let health = classify_skill_health(&skill.spaced_repetition, today);
            let days_until_review = skill
                .spaced_repetition
                .next_review_date
                .signed_duration_since(today)
                .num_days();
            (
                skill_id.clone(),
                SkillHealthEntry {
                    skill_id: skill_id.clone(),
                    health,
                    next_review_date: skill.spaced_repetition.next_review_date,
                    days_until_review,
                },
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::{SkillProgress, SpacedRepetition, ZpdLevels};
    use chrono::NaiveDate;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn default_sr(interval: u32, ease: f32, next: NaiveDate) -> SpacedRepetition {
        SpacedRepetition {
            interval_days: interval,
            ease_factor: ease,
            next_review_date: next,
            consecutive_correct: 0,
        }
    }

    // --- SM-2 update ---

    #[test]
    fn test_high_accuracy_increases_interval() {
        let today = date(2026, 4, 10);
        let mut sr = default_sr(7, 2.5, today);
        update_spaced_repetition(&mut sr, 0.80, today);
        // 7 * 2.5 = 17.5, rounds to 18
        assert_eq!(sr.interval_days, 18);
        assert_eq!(sr.consecutive_correct, 1);
        assert_eq!(sr.next_review_date, date(2026, 4, 28));
    }

    #[test]
    fn test_high_accuracy_above_threshold() {
        let today = date(2026, 4, 10);
        let mut sr = default_sr(4, 2.0, today);
        update_spaced_repetition(&mut sr, 1.0, today);
        assert_eq!(sr.interval_days, 8); // 4 * 2.0 = 8
        assert_eq!(sr.consecutive_correct, 1);
    }

    #[test]
    fn test_moderate_accuracy_holds_interval() {
        let today = date(2026, 4, 10);
        let mut sr = SpacedRepetition {
            interval_days: 7,
            ease_factor: 2.5,
            next_review_date: today,
            consecutive_correct: 3,
        };
        update_spaced_repetition(&mut sr, 0.70, today);
        assert_eq!(sr.interval_days, 7);
        assert_eq!(sr.consecutive_correct, 0);
        assert!((sr.ease_factor - 2.5).abs() < f32::EPSILON);
        assert_eq!(sr.next_review_date, date(2026, 4, 17));
    }

    #[test]
    fn test_low_accuracy_resets_interval() {
        let today = date(2026, 4, 10);
        let mut sr = default_sr(14, 2.5, today);
        update_spaced_repetition(&mut sr, 0.50, today);
        assert_eq!(sr.interval_days, 1);
        assert_eq!(sr.next_review_date, date(2026, 4, 11));
    }

    #[test]
    fn test_low_accuracy_decreases_ease_factor() {
        let today = date(2026, 4, 10);
        let mut sr = default_sr(7, 2.5, today);
        update_spaced_repetition(&mut sr, 0.59, today);
        // 2.5 - 0.2 = 2.3
        assert!((sr.ease_factor - 2.3).abs() < 1e-6);
    }

    #[test]
    fn test_ease_factor_floor() {
        let today = date(2026, 4, 10);
        let mut sr = default_sr(7, 1.4, today);
        update_spaced_repetition(&mut sr, 0.0, today);
        // 1.4 - 0.2 = 1.2, but floor is 1.3
        assert!((sr.ease_factor - MIN_EASE_FACTOR).abs() < 1e-6);
    }

    #[test]
    fn test_ease_factor_at_floor_stays() {
        let today = date(2026, 4, 10);
        let mut sr = default_sr(7, MIN_EASE_FACTOR, today);
        update_spaced_repetition(&mut sr, 0.0, today);
        assert!((sr.ease_factor - MIN_EASE_FACTOR).abs() < 1e-6);
    }

    // --- Skill health classification ---

    #[test]
    fn test_classify_fresh() {
        let sr = default_sr(7, 2.5, date(2026, 4, 15));
        let health = classify_skill_health(&sr, date(2026, 4, 10));
        assert_eq!(health, SkillHealth::Fresh);
    }

    #[test]
    fn test_classify_due() {
        let sr = default_sr(7, 2.5, date(2026, 4, 10));
        let health = classify_skill_health(&sr, date(2026, 4, 10));
        assert_eq!(health, SkillHealth::Due);
    }

    #[test]
    fn test_classify_overdue() {
        let sr = default_sr(7, 2.5, date(2026, 4, 7));
        let health = classify_skill_health(&sr, date(2026, 4, 10));
        assert_eq!(health, SkillHealth::Overdue); // 3 days overdue
    }

    #[test]
    fn test_classify_rusty() {
        let sr = default_sr(7, 2.5, date(2026, 3, 20));
        let health = classify_skill_health(&sr, date(2026, 4, 10));
        assert_eq!(health, SkillHealth::Rusty); // 21 days overdue
    }

    #[test]
    fn test_classify_overdue_boundary_6_days() {
        let sr = default_sr(7, 2.5, date(2026, 4, 4));
        let health = classify_skill_health(&sr, date(2026, 4, 10));
        assert_eq!(health, SkillHealth::Overdue); // exactly 6 days overdue
    }

    #[test]
    fn test_classify_rusty_boundary_7_days() {
        let sr = default_sr(7, 2.5, date(2026, 4, 3));
        let health = classify_skill_health(&sr, date(2026, 4, 10));
        assert_eq!(health, SkillHealth::Rusty); // exactly 7 days overdue
    }

    // --- Session mix ---

    fn make_skill(level: u32, next_review: NaiveDate) -> SkillProgress {
        SkillProgress {
            level,
            xp: 100,
            last_practiced: None,
            zpd: ZpdLevels {
                independent_level: level,
                scaffolded_level: level + 1,
            },
            recent_accuracy: vec![],
            working_memory_signal: Default::default(),
            spaced_repetition: SpacedRepetition {
                interval_days: 7,
                ease_factor: 2.5,
                next_review_date: next_review,
                consecutive_correct: 0,
            },
        }
    }

    fn make_progress_with_skills(skills: Vec<(&str, SkillProgress)>) -> LearnerProgress {
        let mut skill_map = HashMap::new();
        for (id, skill) in skills {
            skill_map.insert(id.to_string(), skill);
        }
        LearnerProgress {
            schema_version: 1,
            learner_id: Uuid::new_v4(),
            skills: skill_map,
            badges: vec![],
            streaks: Default::default(),
            total_sessions: 0,
            total_time_minutes: 0,
            total_assignments: 0,
            metacognition: Default::default(),
            challenge_flags: HashMap::new(),
        }
    }

    #[test]
    fn test_session_mix_70_30_split() {
        let today = date(2026, 4, 10);
        // Need enough overdue skills to fill the 30% review slots (3 out of 10).
        let progress = make_progress_with_skills(vec![
            ("pattern-recognition", make_skill(4, date(2026, 4, 5))), // overdue
            ("sequential-logic", make_skill(2, date(2026, 4, 1))),    // rusty
            ("spatial-reasoning", make_skill(3, date(2026, 4, 3))),   // overdue
        ]);
        let (new_slots, review) = plan_session_mix(&progress, 10, today);
        assert_eq!(new_slots, 7); // 70% of 10
        assert_eq!(review.len(), 3); // capped at 30% of 10
        assert_eq!(new_slots + review.len(), 10);
    }

    #[test]
    fn test_session_mix_only_fresh_skills_no_review() {
        let today = date(2026, 4, 10);
        let progress = make_progress_with_skills(vec![
            ("pattern-recognition", make_skill(4, date(2026, 4, 20))), // fresh
        ]);
        let (_new_slots, review) = plan_session_mix(&progress, 10, today);
        assert!(review.is_empty());
    }

    #[test]
    fn test_session_mix_priority_order() {
        let today = date(2026, 4, 10);
        // skill-a: level 5, 9 days overdue → priority 45
        // skill-b: level 2, 5 days overdue → priority 10
        // skill-c: level 3, 3 days overdue → priority 9
        let progress = make_progress_with_skills(vec![
            ("skill-a", make_skill(5, date(2026, 4, 1))),
            ("skill-b", make_skill(2, date(2026, 4, 5))),
            ("skill-c", make_skill(3, date(2026, 4, 7))),
        ]);
        let (_new_slots, review) = plan_session_mix(&progress, 10, today);
        assert_eq!(review[0].skill_id, "skill-a");
        assert_eq!(review[1].skill_id, "skill-b");
        assert_eq!(review[2].skill_id, "skill-c");
    }

    #[test]
    fn test_session_mix_review_targets_independent_level() {
        let today = date(2026, 4, 10);
        let mut skill = make_skill(4, date(2026, 4, 1));
        skill.zpd.independent_level = 3;
        let progress = make_progress_with_skills(vec![("pattern-recognition", skill)]);
        let (_new_slots, review) = plan_session_mix(&progress, 10, today);
        assert_eq!(review[0].target_level, 3);
    }

    // --- Skill health map ---

    #[test]
    fn test_build_skill_health_map_days_until_review() {
        let today = date(2026, 4, 10);
        let progress = make_progress_with_skills(vec![
            ("pattern-recognition", make_skill(4, date(2026, 4, 15))), // 5 days away
            ("sequential-logic", make_skill(2, date(2026, 4, 7))),     // 3 days overdue
        ]);
        let map = build_skill_health_map(&progress, today);

        let pr = &map["pattern-recognition"];
        assert_eq!(pr.health, SkillHealth::Fresh);
        assert_eq!(pr.days_until_review, 5);

        let sl = &map["sequential-logic"];
        assert_eq!(sl.health, SkillHealth::Overdue);
        assert_eq!(sl.days_until_review, -3);
    }

    #[test]
    fn test_skill_health_kebab_case_serialization() {
        assert_eq!(
            serde_json::to_string(&SkillHealth::Fresh).unwrap(),
            "\"fresh\""
        );
        assert_eq!(
            serde_json::to_string(&SkillHealth::Due).unwrap(),
            "\"due\""
        );
        assert_eq!(
            serde_json::to_string(&SkillHealth::Overdue).unwrap(),
            "\"overdue\""
        );
        assert_eq!(
            serde_json::to_string(&SkillHealth::Rusty).unwrap(),
            "\"rusty\""
        );
    }
}
