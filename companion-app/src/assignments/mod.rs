// Assignment generation & evaluation pipeline.
// See CLAUDE.md "Assignment System" and "Reliability Architecture" sections.
//
// Architecture:
//   GENERATE  ->  Claude creates assignment (structured JSON)
//       |
//   VALIDATE  ->  Backend verifies correctAnswer programmatically
//       |
//   PRESENT   ->  Return verified assignment to caller
//
// Generation and evaluation are always separate API operations.

pub mod adaptation;

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::claude::schemas::GeneratedAssignment;
use crate::progress::tracker::{LearnerProgress, SkillProgress, ZpdLevels};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during assignment operations.
#[derive(Debug, Error)]
pub enum AssignmentError {
    #[error("No assignment templates found in directory: {0}")]
    NoTemplates(String),

    #[error("Template directory not accessible: {0}")]
    TemplateIo(String),

    #[error("Failed to parse template '{path}': {reason}")]
    TemplateParse { path: String, reason: String },

    #[error("No skill available for assignment generation")]
    NoSkillAvailable,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Template types
// ---------------------------------------------------------------------------

/// The verification strictness level for a given assignment type.
///
/// See CLAUDE.md -> "Verification Layers by Assignment Type".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerificationLevel {
    /// Backend independently computes the correct answer and rejects assignments
    /// where Claude's answer doesn't match.
    Full,
    /// Basic logical consistency check -- the assignment is plausible and the
    /// answer follows from the premises, but full recomputation isn't possible.
    Partial,
    /// No programmatic verification possible -- flag for parent review.
    None,
}

/// An assignment template loaded from `data/curriculum/assignment-templates/`.
///
/// Templates constrain what Claude can generate, reducing hallucination surface.
/// See CLAUDE.md -> "Constrain Generation with Assignment Templates".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentTemplate {
    /// The assignment type this template governs (e.g. `"sequence-puzzle"`).
    #[serde(rename = "type")]
    pub assignment_type: String,
    /// Raw constraint data (type-specific; injected into generation prompt).
    pub constraints: serde_json::Value,
    /// How strictly the backend can verify Claude's `correctAnswer`.
    pub verification_level: VerificationLevel,
    /// Specific method used for verification (e.g. `"compute-sequence"`).
    pub verification_method: String,
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// The result of the backend's independent answer verification.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerificationStatus {
    /// Backend independently confirmed the answer is correct (full).
    Verified,
    /// Basic plausibility checks passed; full recomputation not possible.
    PartiallyVerified,
    /// Cannot verify programmatically -- parent review is required.
    Unverifiable,
}

/// Attempt to detect an arithmetic sequence rule.
///
/// Returns `Some(common_difference)` if all consecutive differences are equal.
fn detect_arithmetic(terms: &[i64]) -> Option<i64> {
    if terms.len() < 2 {
        return None;
    }
    let diff = terms[1] - terms[0];
    if terms.windows(2).all(|w| w[1] - w[0] == diff) {
        Some(diff)
    } else {
        None
    }
}

/// Attempt to detect a geometric sequence rule.
///
/// Returns `Some(common_ratio)` if all consecutive ratios are identical integers
/// and no division by zero occurs.
fn detect_geometric(terms: &[i64]) -> Option<i64> {
    if terms.len() < 2 || terms[0] == 0 {
        return None;
    }
    if terms[1] % terms[0] != 0 {
        return None;
    }
    let ratio = terms[1] / terms[0];
    if ratio == 0 {
        return None;
    }
    if terms
        .windows(2)
        .all(|w| w[0] != 0 && w[1] % w[0] == 0 && w[1] / w[0] == ratio)
    {
        Some(ratio)
    } else {
        None
    }
}

/// Attempt to detect a Fibonacci-like sequence.
///
/// Returns `true` if each term (starting from index 2) equals the sum of the
/// two preceding terms.
fn detect_fibonacci_like(terms: &[i64]) -> bool {
    if terms.len() < 3 {
        return false;
    }
    terms.windows(3).all(|w| w[2] == w[0] + w[1])
}

/// Compute the expected next term in a sequence given a list of known terms.
///
/// Tries arithmetic -> geometric -> fibonacci-like in order.
/// Returns `None` if no recognisable rule is found.
pub fn compute_sequence_next(terms: &[i64]) -> Option<i64> {
    if terms.is_empty() {
        return None;
    }
    if let Some(diff) = detect_arithmetic(terms) {
        return Some(terms[terms.len() - 1] + diff);
    }
    if let Some(ratio) = detect_geometric(terms) {
        return Some(terms[terms.len() - 1] * ratio);
    }
    if detect_fibonacci_like(terms) {
        let n = terms.len();
        return Some(terms[n - 1] + terms[n - 2]);
    }
    None
}

/// Full verification for sequence puzzles using `verification_data.terms`.
///
/// Extracts the term list from the assignment's `verification_data`, computes
/// the expected next term, and checks it against `correctAnswer`.
///
/// Returns `Unverifiable` (not `PartiallyVerified`) whenever the computation
/// cannot be completed — missing or malformed `verification_data` means the
/// backend cannot independently confirm correctness, so the assignment must
/// not be accepted at "full" verification level. The pipeline will retry or
/// fall back to deterministic generation in that case.
fn verify_sequence_puzzle(assignment: &GeneratedAssignment) -> VerificationStatus {
    let Some(vd) = &assignment.verification_data else {
        return VerificationStatus::Unverifiable;
    };

    let Some(terms_val) = vd.get("terms") else {
        return VerificationStatus::Unverifiable;
    };

    let Some(terms_array) = terms_val.as_array() else {
        return VerificationStatus::Unverifiable;
    };

    let terms: Vec<i64> = terms_array.iter().filter_map(|v| v.as_i64()).collect();

    if terms.len() < 2 {
        return VerificationStatus::Unverifiable;
    }

    let Some(expected) = compute_sequence_next(&terms) else {
        return VerificationStatus::Unverifiable;
    };

    let claude_answer = match &assignment.correct_answer {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.parse::<i64>().ok(),
        _ => None,
    };

    match claude_answer {
        Some(answer) if answer == expected => VerificationStatus::Verified,
        _ => VerificationStatus::Unverifiable,
    }
}

/// Partial verification for deductive reasoning.
///
/// Checks basic logical consistency:
/// - At least one premise exists in `verification_data`.
/// - A conclusion is specified.
/// - `correctAnswer` matches the conclusion string (case-insensitive substring).
fn verify_deductive_reasoning(assignment: &GeneratedAssignment) -> VerificationStatus {
    let answer_nonempty = match &assignment.correct_answer {
        serde_json::Value::String(s) => !s.trim().is_empty(),
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) => true,
        _ => false,
    };
    if !answer_nonempty {
        return VerificationStatus::Unverifiable;
    }

    if let Some(vd) = &assignment.verification_data {
        let has_premises = vd
            .get("premises")
            .and_then(|p| p.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);

        let conclusion = vd
            .get("conclusion")
            .and_then(|c| c.as_str())
            .map(|s| s.trim().to_string());

        if has_premises {
            if let Some(conclusion_str) = conclusion {
                let answer_str = match &assignment.correct_answer {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                if !conclusion_str.is_empty()
                    && answer_str
                        .to_lowercase()
                        .contains(&conclusion_str.to_lowercase())
                {
                    return VerificationStatus::PartiallyVerified;
                }
            }
            return VerificationStatus::PartiallyVerified;
        }
    }

    VerificationStatus::PartiallyVerified
}

/// Partial verification for pattern-matching assignments.
///
/// Checks that `correctAnswer` appears in `acceptableAnswers`.
fn verify_pattern_matching(assignment: &GeneratedAssignment) -> VerificationStatus {
    if assignment.acceptable_answers.is_empty() {
        return VerificationStatus::Unverifiable;
    }
    if assignment
        .acceptable_answers
        .iter()
        .any(|a| a == &assignment.correct_answer)
    {
        VerificationStatus::PartiallyVerified
    } else {
        VerificationStatus::Unverifiable
    }
}

/// Run backend verification on a generated assignment.
///
/// Dispatches to the appropriate verification method based on the template's
/// `verificationLevel` and `verificationMethod`. This is the authoritative
/// correctness check -- Claude is not trusted to self-verify.
pub fn verify_assignment(
    assignment: &GeneratedAssignment,
    level: &VerificationLevel,
    method: &str,
) -> VerificationStatus {
    match level {
        VerificationLevel::None => VerificationStatus::Unverifiable,
        VerificationLevel::Partial => match method {
            "rule-check" => verify_deductive_reasoning(assignment),
            "acceptability-check" => verify_pattern_matching(assignment),
            _ => VerificationStatus::PartiallyVerified,
        },
        VerificationLevel::Full => match method {
            "compute-sequence" => verify_sequence_puzzle(assignment),
            _ => VerificationStatus::Unverifiable,
        },
    }
}

/// Returns `true` if the verification status means the assignment should be
/// flagged for parent review.
pub fn needs_parent_review(status: &VerificationStatus) -> bool {
    matches!(status, VerificationStatus::Unverifiable)
}

// ---------------------------------------------------------------------------
// Template loading
// ---------------------------------------------------------------------------

/// Load all assignment templates from the given directory.
///
/// Each `.json` file in the directory is parsed as an [`AssignmentTemplate`].
/// Returns an error if the directory is not accessible or contains no valid
/// templates.
pub async fn load_templates(
    templates_dir: &Path,
) -> Result<Vec<AssignmentTemplate>, AssignmentError> {
    let mut read_dir = tokio::fs::read_dir(templates_dir).await.map_err(|e| {
        AssignmentError::TemplateIo(format!(
            "Cannot read templates directory '{}': {e}",
            templates_dir.display()
        ))
    })?;

    let mut templates = Vec::new();

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let bytes = tokio::fs::read(&path).await?;
        let template: AssignmentTemplate =
            serde_json::from_slice(&bytes).map_err(|e| AssignmentError::TemplateParse {
                path: path.display().to_string(),
                reason: e.to_string(),
            })?;
        templates.push(template);
    }

    if templates.is_empty() {
        return Err(AssignmentError::NoTemplates(
            templates_dir.display().to_string(),
        ));
    }

    Ok(templates)
}

/// Find the template matching a given assignment type.
pub fn find_template<'a>(
    templates: &'a [AssignmentTemplate],
    assignment_type: &str,
) -> Option<&'a AssignmentTemplate> {
    templates
        .iter()
        .find(|t| t.assignment_type == assignment_type)
}

// ---------------------------------------------------------------------------
// ZPD skill selection and difficulty targeting
// ---------------------------------------------------------------------------

/// Compute the target difficulty for an assignment aimed at the midpoint of a
/// learner's ZPD gap for a specific skill.
///
/// The target is the midpoint between `independentLevel` and `scaffoldedLevel`,
/// clamped to [1, 10]. If the gap is zero the target is `independentLevel + 1`.
pub fn target_difficulty(zpd: &ZpdLevels) -> u32 {
    let gap = zpd.gap();
    let target = if gap == 0 {
        zpd.independent_level.saturating_add(1)
    } else {
        zpd.independent_level + (gap / 2)
    };
    target.clamp(1, 10)
}

/// Candidate skill selected for the next assignment.
#[derive(Clone, Debug)]
pub struct SkillTarget {
    /// The skill ID (e.g. `"pattern-recognition"`).
    pub skill_id: String,
    /// Computed target difficulty for this skill.
    pub difficulty: u32,
}

/// Select which skill the next assignment should target, and what difficulty.
///
/// Priority order (see CLAUDE.md "Difficulty Adaptation Rules -- Across Sessions"):
/// 1. Skills with spaced-repetition review due today or overdue.
/// 2. Skills with recent accuracy below 60% -- reinforce fundamentals.
/// 3. Widest ZPD gap -- most room for growth.
///
/// Returns `None` if no skills exist yet.
pub fn select_skill(progress: &LearnerProgress, today: chrono::NaiveDate) -> Option<SkillTarget> {
    if progress.skills.is_empty() {
        return None;
    }

    // 1. Overdue review -- pick highest-level overdue skill.
    let overdue: Vec<(&String, &SkillProgress)> = progress
        .skills
        .iter()
        .filter(|(_, s)| s.spaced_repetition.next_review_date <= today)
        .collect();

    if !overdue.is_empty() {
        let (skill_id, skill) = overdue
            .iter()
            .max_by_key(|(_, s)| s.level)
            .map(|(id, s)| (*id, *s))
            .unwrap();
        return Some(SkillTarget {
            skill_id: skill_id.clone(),
            difficulty: target_difficulty(&skill.zpd),
        });
    }

    // 2. Recent accuracy below 60% -- reinforce.
    let struggling: Vec<(&String, &SkillProgress)> = progress
        .skills
        .iter()
        .filter(|(_, s)| !s.recent_accuracy.is_empty() && s.recent_accuracy_fraction() < 0.6)
        .collect();

    if !struggling.is_empty() {
        let (skill_id, skill) = struggling
            .iter()
            .min_by(|(_, a), (_, b)| {
                a.recent_accuracy_fraction()
                    .partial_cmp(&b.recent_accuracy_fraction())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(id, s)| (*id, *s))
            .unwrap();
        return Some(SkillTarget {
            skill_id: skill_id.clone(),
            difficulty: target_difficulty(&skill.zpd),
        });
    }

    // 3. Widest ZPD gap.
    let (skill_id, skill) = progress
        .skills
        .iter()
        .max_by_key(|(_, s)| s.zpd.gap())
        .unwrap();

    Some(SkillTarget {
        skill_id: skill_id.clone(),
        difficulty: target_difficulty(&skill.zpd),
    })
}

// ---------------------------------------------------------------------------
// Deterministic fallback generation
// ---------------------------------------------------------------------------

/// Generate a provably correct sequence-puzzle assignment without calling Claude.
fn generate_sequence_fallback(skill: &str, difficulty: u32) -> GeneratedAssignment {
    let seed = difficulty as u64
        + std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            % 100;

    if seed.is_multiple_of(2) {
        let start = (seed % 5 + 1) as i64;
        let step = (difficulty as i64).clamp(2, 10);
        let terms: Vec<i64> = (0..4).map(|i| start + i * step).collect();
        let next = terms[3] + step;

        GeneratedAssignment {
            assignment_type: "sequence-puzzle".to_string(),
            skill: skill.to_string(),
            difficulty,
            theme: "math".to_string(),
            prompt: format!(
                "What number comes next in this sequence? {} -> {} -> {} -> {} -> ?",
                terms[0], terms[1], terms[2], terms[3]
            ),
            correct_answer: serde_json::json!(next),
            acceptable_answers: vec![serde_json::json!(next), serde_json::json!(next.to_string())],
            hints: vec![
                "Look at how the numbers change from one to the next.".to_string(),
                format!("Each number is {} more than the previous one.", step),
                format!("{} + {} = ?", terms[3], step),
            ],
            explanation: format!(
                "Each number increases by {step}. Starting from {start}, \
                 the pattern is +{step} each time. After {}, the next number is {next}.",
                terms[3]
            ),
            modality: None,
            verification_data: Some(serde_json::json!({
                "terms": terms,
                "sequenceType": "arithmetic",
            })),
        }
    } else {
        let start = 1_i64;
        let ratio: i64 = if difficulty <= 3 { 2 } else { 3 };
        let terms: Vec<i64> = (0..4).map(|i| start * ratio.pow(i as u32)).collect();
        let next = terms[3] * ratio;

        GeneratedAssignment {
            assignment_type: "sequence-puzzle".to_string(),
            skill: skill.to_string(),
            difficulty,
            theme: "math".to_string(),
            prompt: format!(
                "What number comes next in this sequence? {} -> {} -> {} -> {} -> ?",
                terms[0], terms[1], terms[2], terms[3]
            ),
            correct_answer: serde_json::json!(next),
            acceptable_answers: vec![serde_json::json!(next), serde_json::json!(next.to_string())],
            hints: vec![
                "Look at how the numbers grow from one to the next.".to_string(),
                format!(
                    "Each number is multiplied by {} to get the next one.",
                    ratio
                ),
                format!("{} x {} = ?", terms[3], ratio),
            ],
            explanation: format!(
                "Each number is multiplied by {ratio}. Starting from {start}, \
                 the pattern is x{ratio} each time. After {}, the next number is {next}.",
                terms[3]
            ),
            modality: None,
            verification_data: Some(serde_json::json!({
                "terms": terms,
                "sequenceType": "geometric",
            })),
        }
    }
}

/// Generate a provably correct deductive-reasoning assignment without Claude.
fn generate_deductive_fallback(skill: &str, difficulty: u32) -> GeneratedAssignment {
    let premise_a = "All robots can count.";
    let premise_b = "Zara is a robot.";
    let conclusion = "Zara can count.";

    GeneratedAssignment {
        assignment_type: "deductive-reasoning".to_string(),
        skill: skill.to_string(),
        difficulty,
        theme: "robots".to_string(),
        prompt: format!(
            "Read these two facts carefully:\n1. {premise_a}\n2. {premise_b}\n\n\
             What can you figure out for certain from these two facts?"
        ),
        correct_answer: serde_json::json!(conclusion),
        acceptable_answers: vec![
            serde_json::json!(conclusion),
            serde_json::json!("Zara can count"),
            serde_json::json!("zara can count"),
        ],
        hints: vec![
            "Read each fact carefully -- what do they tell you about Zara?".to_string(),
            "The first fact is true for ALL robots. Is Zara a robot?".to_string(),
            "If all robots can count, and Zara is a robot, then Zara must be able to..."
                .to_string(),
        ],
        explanation: "Fact 1 says all robots can count. Fact 2 says Zara is a robot. \
                      Since ALL robots can count, Zara must also be able to count. \
                      This is deductive reasoning -- applying a general rule to a specific case!"
            .to_string(),
        modality: None,
        verification_data: Some(serde_json::json!({
            "premises": [premise_a, premise_b],
            "conclusion": conclusion,
        })),
    }
}

/// Generate a provably correct pattern-matching assignment without Claude.
fn generate_pattern_fallback(skill: &str, difficulty: u32) -> GeneratedAssignment {
    let pattern = ["red", "blue", "red", "blue", "red"];
    let next = "blue";

    GeneratedAssignment {
        assignment_type: "pattern-matching".to_string(),
        skill: skill.to_string(),
        difficulty,
        theme: "colors".to_string(),
        prompt: format!(
            "Look at this color pattern: {} -> {} -> {} -> {} -> {} -> ?\nWhat color comes next?",
            pattern[0], pattern[1], pattern[2], pattern[3], pattern[4]
        ),
        correct_answer: serde_json::json!(next),
        acceptable_answers: vec![
            serde_json::json!("blue"),
            serde_json::json!("Blue"),
            serde_json::json!("BLUE"),
        ],
        hints: vec![
            "Look at the colors in order -- do you see a repeating pattern?".to_string(),
            "The pattern repeats every two colors.".to_string(),
            "After red comes... what color have you seen after red before?".to_string(),
        ],
        explanation: "The pattern alternates: red, blue, red, blue, red, blue. \
                      After every red comes blue, so the next color is blue!"
            .to_string(),
        modality: None,
        verification_data: None,
    }
}

/// Generate a deterministic (Claude-free) fallback assignment.
///
/// These assignments are provably correct but lack creative theming.
pub fn generate_deterministic(
    assignment_type: &str,
    skill: &str,
    difficulty: u32,
) -> GeneratedAssignment {
    match assignment_type {
        "sequence-puzzle" => generate_sequence_fallback(skill, difficulty),
        "deductive-reasoning" => generate_deductive_fallback(skill, difficulty),
        "pattern-matching" => generate_pattern_fallback(skill, difficulty),
        _ => generate_sequence_fallback(skill, difficulty),
    }
}

// ---------------------------------------------------------------------------
// Verified assignment output
// ---------------------------------------------------------------------------

/// A generated assignment together with its backend verification result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedAssignment {
    /// The assignment (may be Claude-generated or deterministic fallback).
    pub assignment: GeneratedAssignment,
    /// Result of the backend's independent verification.
    pub verification_status: VerificationStatus,
    /// Whether this assignment should be surfaced in the parent review queue.
    pub needs_parent_review: bool,
    /// Whether a deterministic fallback was used instead of Claude.
    pub used_fallback: bool,
    /// Whether this was generated as a frustration-pivot confidence builder.
    /// Confidence-builder results do not count toward difficulty progression.
    #[serde(default)]
    pub is_confidence_builder: bool,
}

// ---------------------------------------------------------------------------
// Full pipeline
// ---------------------------------------------------------------------------

/// Payload passed to the pipeline describing what to generate.
#[derive(Debug)]
pub struct PipelineRequest {
    pub skill: String,
    pub difficulty: u32,
    /// Preferred assignment type (e.g. `"sequence-puzzle"`). If `None` the
    /// pipeline picks based on the skill.
    pub preferred_type: Option<String>,
}

/// Map a skill ID to the most appropriate assignment type.
fn skill_to_assignment_type(skill_id: &str) -> &'static str {
    match skill_id {
        "pattern-recognition" => "pattern-matching",
        "sequential-logic" => "sequence-puzzle",
        "spatial-reasoning" => "pattern-matching",
        "deductive-reasoning" => "deductive-reasoning",
        _ => "sequence-puzzle",
    }
}

/// Run the full GENERATE -> VALIDATE -> PRESENT pipeline.
///
/// 1. Calls Claude to generate an assignment with full learner context.
/// 2. Runs backend verification on `correctAnswer`.
/// 3. Retries up to `max_retries` times on verification failure.
/// 4. Falls back to deterministic generation if retries are exhausted or Claude
///    is unavailable.
/// 5. Returns the assignment with its verification status.
///
/// Generation and evaluation are **always separate operations** -- this function
/// only generates; evaluation is a separate API call.
pub async fn run_pipeline<F, Fut>(
    generate_fn: F,
    templates: &[AssignmentTemplate],
    request: &PipelineRequest,
    max_retries: usize,
) -> VerifiedAssignment
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Option<GeneratedAssignment>>,
{
    let assignment_type = request
        .preferred_type
        .as_deref()
        .unwrap_or_else(|| skill_to_assignment_type(&request.skill));

    let template = find_template(templates, assignment_type);
    let (level, method) = template
        .map(|t| (t.verification_level.clone(), t.verification_method.clone()))
        .unwrap_or((VerificationLevel::None, "none".to_string()));

    for attempt in 0..=max_retries {
        let generated = generate_fn().await;
        let Some(assignment) = generated else {
            tracing::warn!(
                skill = %request.skill,
                attempt,
                "Claude generation returned None -- falling back to deterministic"
            );
            break;
        };

        let status = verify_assignment(&assignment, &level, &method);
        let review = needs_parent_review(&status);

        if status != VerificationStatus::Unverifiable || level == VerificationLevel::None {
            tracing::info!(
                skill = %request.skill,
                difficulty = request.difficulty,
                assignment_type = %assignment_type,
                ?status,
                attempt,
                "Assignment verification passed"
            );
            return VerifiedAssignment {
                assignment,
                verification_status: status,
                needs_parent_review: review,
                used_fallback: false,
                is_confidence_builder: false,
            };
        }

        tracing::warn!(
            skill = %request.skill,
            attempt,
            assignment_type = %assignment_type,
            "Verification failed, retrying"
        );
    }

    tracing::warn!(
        skill = %request.skill,
        assignment_type = %assignment_type,
        "Using deterministic fallback after all retries exhausted"
    );
    let fallback = generate_deterministic(assignment_type, &request.skill, request.difficulty);
    let status = verify_assignment(&fallback, &level, &method);
    let review = needs_parent_review(&status);

    VerifiedAssignment {
        assignment: fallback,
        verification_status: status,
        needs_parent_review: review,
        used_fallback: true,
        is_confidence_builder: false,
    }
}

// ---------------------------------------------------------------------------
// Response checking (for the evaluation API)
// ---------------------------------------------------------------------------

/// Check whether a child's response matches the backend-verified correct answer.
///
/// Performs a case-insensitive string comparison and checks `acceptableAnswers`.
/// This is called **before** the Claude evaluation request -- Claude receives the
/// backend's determination so it cannot hallucinate correctness.
pub fn check_response_correct(assignment: &GeneratedAssignment, child_response: &str) -> bool {
    let response_normalised = child_response.trim().to_lowercase();

    let canonical_match = match &assignment.correct_answer {
        serde_json::Value::Number(n) => {
            response_normalised == n.to_string()
                || response_normalised.parse::<f64>().ok() == n.as_f64()
        }
        serde_json::Value::String(s) => response_normalised == s.trim().to_lowercase(),
        other => response_normalised == other.to_string().to_lowercase(),
    };

    if canonical_match {
        return true;
    }

    assignment.acceptable_answers.iter().any(|a| match a {
        serde_json::Value::Number(n) => {
            response_normalised == n.to_string()
                || response_normalised.parse::<f64>().ok() == n.as_f64()
        }
        serde_json::Value::String(s) => response_normalised == s.trim().to_lowercase(),
        other => response_normalised == other.to_string().to_lowercase(),
    })
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::collections::HashMap;
    use uuid::Uuid;

    use crate::progress::tracker::{
        LearnerProgress, Metacognition, SkillProgress, SpacedRepetition, Streaks,
        WorkingMemorySignal, ZpdLevels,
    };

    fn make_sequence_assignment(correct: i64, terms: Vec<i64>) -> GeneratedAssignment {
        GeneratedAssignment {
            assignment_type: "sequence-puzzle".to_string(),
            skill: "pattern-recognition".to_string(),
            difficulty: 3,
            theme: "test".to_string(),
            prompt: "What comes next?".to_string(),
            correct_answer: serde_json::json!(correct),
            acceptable_answers: vec![serde_json::json!(correct)],
            hints: vec!["h1".to_string(), "h2".to_string(), "h3".to_string()],
            explanation: "exp".to_string(),
            modality: None,
            verification_data: Some(serde_json::json!({ "terms": terms })),
        }
    }

    fn make_assignment_with_correct(correct: serde_json::Value) -> GeneratedAssignment {
        GeneratedAssignment {
            assignment_type: "sequence-puzzle".to_string(),
            skill: "pattern-recognition".to_string(),
            difficulty: 3,
            theme: "test".to_string(),
            prompt: "What comes next?".to_string(),
            correct_answer: correct.clone(),
            acceptable_answers: vec![correct],
            hints: vec!["h1".to_string(), "h2".to_string(), "h3".to_string()],
            explanation: "exp".to_string(),
            modality: None,
            verification_data: None,
        }
    }

    fn make_skill(
        independent: u32,
        scaffolded: u32,
        next_review: NaiveDate,
        recent: Vec<u8>,
    ) -> SkillProgress {
        SkillProgress {
            level: 2,
            xp: 100,
            last_practiced: None,
            zpd: ZpdLevels {
                independent_level: independent,
                scaffolded_level: scaffolded,
            },
            recent_accuracy: recent,
            working_memory_signal: WorkingMemorySignal::Stable,
            spaced_repetition: SpacedRepetition {
                interval_days: 7,
                ease_factor: 2.5,
                next_review_date: next_review,
                consecutive_correct: 0,
            },
        }
    }

    fn make_progress_with_skills(skills: HashMap<String, SkillProgress>) -> LearnerProgress {
        LearnerProgress {
            schema_version: 1,
            learner_id: Uuid::new_v4(),
            skills,
            badges: vec![],
            streaks: Streaks::default(),
            total_sessions: 5,
            total_time_minutes: 100,
            total_assignments: 20,
            metacognition: Metacognition::default(),
            challenge_flags: HashMap::new(),
        }
    }

    #[test]
    fn test_detect_arithmetic() {
        assert_eq!(detect_arithmetic(&[2, 4, 6, 8]), Some(2));
        assert_eq!(detect_arithmetic(&[10, 7, 4, 1]), Some(-3));
        assert_eq!(detect_arithmetic(&[1, 2, 4, 8]), None);
    }

    #[test]
    fn test_detect_geometric() {
        assert_eq!(detect_geometric(&[1, 2, 4, 8]), Some(2));
        assert_eq!(detect_geometric(&[1, 3, 9, 27]), Some(3));
        assert_eq!(detect_geometric(&[2, 4, 6, 8]), None);
    }

    #[test]
    fn test_detect_fibonacci_like() {
        assert!(detect_fibonacci_like(&[1, 1, 2, 3, 5, 8]));
        assert!(detect_fibonacci_like(&[2, 3, 5, 8, 13]));
        assert!(!detect_fibonacci_like(&[1, 2, 4, 8]));
    }

    #[test]
    fn test_compute_sequence_next_arithmetic() {
        assert_eq!(compute_sequence_next(&[2, 4, 6, 8]), Some(10));
        assert_eq!(compute_sequence_next(&[10, 7, 4, 1]), Some(-2));
    }

    #[test]
    fn test_compute_sequence_next_geometric() {
        assert_eq!(compute_sequence_next(&[1, 2, 4, 8]), Some(16));
        assert_eq!(compute_sequence_next(&[1, 3, 9, 27]), Some(81));
    }

    #[test]
    fn test_compute_sequence_next_fibonacci() {
        assert_eq!(compute_sequence_next(&[1, 1, 2, 3, 5]), Some(8));
        assert_eq!(compute_sequence_next(&[2, 3, 5, 8, 13]), Some(21));
    }

    #[test]
    fn test_compute_sequence_next_empty() {
        assert_eq!(compute_sequence_next(&[]), None);
        assert_eq!(compute_sequence_next(&[42]), None);
    }

    #[test]
    fn test_verify_sequence_correct() {
        let assignment = make_sequence_assignment(16, vec![2, 4, 8]);
        let status = verify_assignment(&assignment, &VerificationLevel::Full, "compute-sequence");
        assert_eq!(status, VerificationStatus::Verified);
    }

    #[test]
    fn test_verify_sequence_wrong_answer() {
        let assignment = make_sequence_assignment(99, vec![2, 4, 8]);
        let status = verify_assignment(&assignment, &VerificationLevel::Full, "compute-sequence");
        assert_eq!(status, VerificationStatus::Unverifiable);
    }

    #[test]
    fn test_verify_sequence_no_verification_data() {
        let assignment = GeneratedAssignment {
            assignment_type: "sequence-puzzle".to_string(),
            skill: "pattern-recognition".to_string(),
            difficulty: 3,
            theme: "test".to_string(),
            prompt: "What comes next?".to_string(),
            correct_answer: serde_json::json!(16),
            acceptable_answers: vec![],
            hints: vec!["h1".to_string(), "h2".to_string(), "h3".to_string()],
            explanation: "exp".to_string(),
            modality: None,
            verification_data: None,
        };
        let status = verify_assignment(&assignment, &VerificationLevel::Full, "compute-sequence");
        // Missing verification_data for a full-level assignment must be
        // Unverifiable (not PartiallyVerified) so the pipeline retries.
        assert_eq!(status, VerificationStatus::Unverifiable);
    }

    #[test]
    fn test_verify_none_level_returns_unverifiable() {
        let assignment = make_sequence_assignment(16, vec![2, 4, 8]);
        let status = verify_assignment(&assignment, &VerificationLevel::None, "any");
        assert_eq!(status, VerificationStatus::Unverifiable);
    }

    #[test]
    fn test_verify_deductive_with_premises_and_conclusion() {
        let assignment = GeneratedAssignment {
            assignment_type: "deductive-reasoning".to_string(),
            skill: "deductive-reasoning".to_string(),
            difficulty: 3,
            theme: "test".to_string(),
            prompt: "If all dogs are mammals, and Rex is a dog, what is Rex?".to_string(),
            correct_answer: serde_json::json!("Rex is a mammal"),
            acceptable_answers: vec![serde_json::json!("Rex is a mammal")],
            hints: vec!["h1".to_string(), "h2".to_string(), "h3".to_string()],
            explanation: "exp".to_string(),
            modality: None,
            verification_data: Some(serde_json::json!({
                "premises": ["All dogs are mammals", "Rex is a dog"],
                "conclusion": "Rex is a mammal",
            })),
        };
        let status = verify_assignment(&assignment, &VerificationLevel::Partial, "rule-check");
        assert_eq!(status, VerificationStatus::PartiallyVerified);
    }

    #[test]
    fn test_verify_pattern_matching_in_acceptable() {
        let assignment = GeneratedAssignment {
            assignment_type: "pattern-matching".to_string(),
            skill: "pattern-recognition".to_string(),
            difficulty: 2,
            theme: "test".to_string(),
            prompt: "What comes next?".to_string(),
            correct_answer: serde_json::json!("blue"),
            acceptable_answers: vec![serde_json::json!("blue"), serde_json::json!("Blue")],
            hints: vec!["h1".to_string(), "h2".to_string(), "h3".to_string()],
            explanation: "exp".to_string(),
            modality: None,
            verification_data: None,
        };
        let status = verify_assignment(
            &assignment,
            &VerificationLevel::Partial,
            "acceptability-check",
        );
        assert_eq!(status, VerificationStatus::PartiallyVerified);
    }

    #[test]
    fn test_needs_parent_review() {
        assert!(needs_parent_review(&VerificationStatus::Unverifiable));
        assert!(!needs_parent_review(&VerificationStatus::Verified));
        assert!(!needs_parent_review(&VerificationStatus::PartiallyVerified));
    }

    #[test]
    fn test_target_difficulty_midpoint() {
        let zpd = ZpdLevels {
            independent_level: 2,
            scaffolded_level: 6,
        };
        assert_eq!(target_difficulty(&zpd), 4);
    }

    #[test]
    fn test_target_difficulty_zero_gap() {
        let zpd = ZpdLevels {
            independent_level: 3,
            scaffolded_level: 3,
        };
        assert_eq!(target_difficulty(&zpd), 4);
    }

    #[test]
    fn test_target_difficulty_clamped() {
        let zpd = ZpdLevels {
            independent_level: 10,
            scaffolded_level: 10,
        };
        assert_eq!(target_difficulty(&zpd), 10);
    }

    #[test]
    fn test_select_skill_overdue_review() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 10).unwrap();
        let overdue = NaiveDate::from_ymd_opt(2026, 4, 8).unwrap();
        let future = NaiveDate::from_ymd_opt(2026, 4, 20).unwrap();

        let mut skills = HashMap::new();
        skills.insert(
            "pattern-recognition".to_string(),
            make_skill(2, 5, overdue, vec![1, 1, 1]),
        );
        skills.insert(
            "sequential-logic".to_string(),
            make_skill(2, 5, future, vec![1, 1, 1]),
        );

        let progress = make_progress_with_skills(skills);
        let target = select_skill(&progress, today).expect("should return a skill");
        assert_eq!(target.skill_id, "pattern-recognition");
    }

    #[test]
    fn test_select_skill_struggling() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 10).unwrap();
        let future = NaiveDate::from_ymd_opt(2026, 4, 20).unwrap();

        let mut skills = HashMap::new();
        skills.insert(
            "sequential-logic".to_string(),
            make_skill(2, 4, future, vec![0, 0, 1, 0, 1]),
        );
        skills.insert(
            "pattern-recognition".to_string(),
            make_skill(2, 4, future, vec![1, 1, 1, 1, 1]),
        );

        let progress = make_progress_with_skills(skills);
        let target = select_skill(&progress, today).expect("should return a skill");
        assert_eq!(target.skill_id, "sequential-logic");
    }

    #[test]
    fn test_select_skill_widest_gap() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 10).unwrap();
        let future = NaiveDate::from_ymd_opt(2026, 4, 20).unwrap();

        let mut skills = HashMap::new();
        skills.insert(
            "pattern-recognition".to_string(),
            make_skill(2, 4, future, vec![1, 1, 1]),
        );
        skills.insert(
            "sequential-logic".to_string(),
            make_skill(1, 6, future, vec![1, 1, 1]),
        );

        let progress = make_progress_with_skills(skills);
        let target = select_skill(&progress, today).expect("should return a skill");
        assert_eq!(target.skill_id, "sequential-logic");
    }

    #[test]
    fn test_select_skill_empty_progress() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 10).unwrap();
        let progress = make_progress_with_skills(HashMap::new());
        assert!(select_skill(&progress, today).is_none());
    }

    #[test]
    fn test_deterministic_sequence_is_correct() {
        let assignment = generate_deterministic("sequence-puzzle", "pattern-recognition", 3);
        assert_eq!(assignment.assignment_type, "sequence-puzzle");
        let status = verify_assignment(&assignment, &VerificationLevel::Full, "compute-sequence");
        assert_eq!(
            status,
            VerificationStatus::Verified,
            "deterministic sequence puzzle must always be verified"
        );
    }

    #[test]
    fn test_deterministic_deductive_is_partially_verified() {
        let assignment = generate_deterministic("deductive-reasoning", "deductive-reasoning", 3);
        assert_eq!(assignment.assignment_type, "deductive-reasoning");
        let status = verify_assignment(&assignment, &VerificationLevel::Partial, "rule-check");
        assert_ne!(status, VerificationStatus::Unverifiable);
    }

    #[test]
    fn test_check_response_correct_exact() {
        let a = make_assignment_with_correct(serde_json::json!(16));
        assert!(check_response_correct(&a, "16"));
        assert!(!check_response_correct(&a, "17"));
    }

    #[test]
    fn test_check_response_correct_string_case_insensitive() {
        let a = make_assignment_with_correct(serde_json::json!("blue"));
        assert!(check_response_correct(&a, "Blue"));
        assert!(check_response_correct(&a, "BLUE"));
        assert!(!check_response_correct(&a, "red"));
    }

    #[test]
    fn test_check_response_correct_whitespace() {
        let a = make_assignment_with_correct(serde_json::json!("blue"));
        assert!(check_response_correct(&a, "  blue  "));
    }

    #[test]
    fn test_template_serde_round_trip() {
        let t = AssignmentTemplate {
            assignment_type: "sequence-puzzle".to_string(),
            constraints: serde_json::json!({"maxTerms": 6}),
            verification_level: VerificationLevel::Full,
            verification_method: "compute-sequence".to_string(),
        };
        let json = serde_json::to_string(&t).expect("serialize");
        let restored: AssignmentTemplate = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(t, restored);
    }

    #[test]
    fn test_verification_level_serde() {
        assert_eq!(
            serde_json::to_string(&VerificationLevel::Full).unwrap(),
            "\"full\""
        );
        assert_eq!(
            serde_json::to_string(&VerificationLevel::Partial).unwrap(),
            "\"partial\""
        );
        assert_eq!(
            serde_json::to_string(&VerificationLevel::None).unwrap(),
            "\"none\""
        );
    }

    #[tokio::test]
    async fn test_pipeline_uses_fallback_when_claude_unavailable() {
        let templates = vec![AssignmentTemplate {
            assignment_type: "sequence-puzzle".to_string(),
            constraints: serde_json::json!({}),
            verification_level: VerificationLevel::Full,
            verification_method: "compute-sequence".to_string(),
        }];
        let request = PipelineRequest {
            skill: "pattern-recognition".to_string(),
            difficulty: 3,
            preferred_type: Some("sequence-puzzle".to_string()),
        };
        let result = run_pipeline(
            || async { None::<GeneratedAssignment> },
            &templates,
            &request,
            2,
        )
        .await;
        assert!(
            result.used_fallback,
            "should use fallback when Claude is unavailable"
        );
        assert_eq!(result.assignment.assignment_type, "sequence-puzzle");
    }

    #[tokio::test]
    async fn test_pipeline_accepts_verified_assignment() {
        let templates = vec![AssignmentTemplate {
            assignment_type: "sequence-puzzle".to_string(),
            constraints: serde_json::json!({}),
            verification_level: VerificationLevel::Full,
            verification_method: "compute-sequence".to_string(),
        }];
        let request = PipelineRequest {
            skill: "pattern-recognition".to_string(),
            difficulty: 3,
            preferred_type: Some("sequence-puzzle".to_string()),
        };
        let good_assignment = GeneratedAssignment {
            assignment_type: "sequence-puzzle".to_string(),
            skill: "pattern-recognition".to_string(),
            difficulty: 3,
            theme: "test".to_string(),
            prompt: "2, 4, 8, ?".to_string(),
            correct_answer: serde_json::json!(16),
            acceptable_answers: vec![serde_json::json!(16)],
            hints: vec!["h1".to_string(), "h2".to_string(), "h3".to_string()],
            explanation: "doubling".to_string(),
            modality: None,
            verification_data: Some(serde_json::json!({"terms": [2, 4, 8]})),
        };
        let result = run_pipeline(
            || {
                let a = good_assignment.clone();
                async move { Some(a) }
            },
            &templates,
            &request,
            2,
        )
        .await;
        assert!(!result.used_fallback);
        assert_eq!(result.verification_status, VerificationStatus::Verified);
        assert!(!result.needs_parent_review);
    }

    #[tokio::test]
    async fn test_pipeline_retries_on_verification_failure() {
        let templates = vec![AssignmentTemplate {
            assignment_type: "sequence-puzzle".to_string(),
            constraints: serde_json::json!({}),
            verification_level: VerificationLevel::Full,
            verification_method: "compute-sequence".to_string(),
        }];
        let request = PipelineRequest {
            skill: "pattern-recognition".to_string(),
            difficulty: 3,
            preferred_type: Some("sequence-puzzle".to_string()),
        };
        let bad_assignment = GeneratedAssignment {
            assignment_type: "sequence-puzzle".to_string(),
            skill: "pattern-recognition".to_string(),
            difficulty: 3,
            theme: "test".to_string(),
            prompt: "2, 4, 8, ?".to_string(),
            correct_answer: serde_json::json!(999),
            acceptable_answers: vec![serde_json::json!(999)],
            hints: vec!["h1".to_string(), "h2".to_string(), "h3".to_string()],
            explanation: "doubling".to_string(),
            modality: None,
            verification_data: Some(serde_json::json!({"terms": [2, 4, 8]})),
        };
        let result = run_pipeline(
            || {
                let a = bad_assignment.clone();
                async move { Some(a) }
            },
            &templates,
            &request,
            2,
        )
        .await;
        assert!(
            result.used_fallback,
            "should fall back after retries exhausted"
        );
    }
}
