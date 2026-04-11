# Data Model

## Learner Profile (`profile.json`)

The profile avoids static "learning style" labels (the VARK model is not supported by research). Instead, it captures initial preferences and lets the system **observe and adapt** through behavioral dimensions tracked in `progress.json`.

> **Note:** The JSON examples below are valid JSON and are intended to be copy/pasteable. Annotations are provided in the surrounding text and field notes; actual stored files must remain valid JSON (no `//` comments).

```json
{
  "schemaVersion": 1,
  "id": "learner-uuid",
  "name": "StarExplorer42",
  "age": 8,
  "interests": ["dinosaurs", "space", "building"],
  "initialPreferences": {
    "sessionLengthMinutes": 25,
    "challengePreference": "guided"
  },
  "observedBehavior": {
    "frustrationResponse": "unknown",
    "effortAttribution": "unknown",
    "hintUsage": "unknown",
    "attentionPattern": {
      "optimalSessionMinutes": null,
      "accuracyDecayOnset": null
    }
  }
}
```

**Field notes:**
- `schemaVersion` — increment when the schema changes; used for safe migration of existing files.
- `name` — **display name chosen by the child**. The system never asks for or stores the learner's real name. The child picks whatever they want to be called — a nickname, a character name, anything. This display name is safe to include in Claude API calls as-is.
- `initialPreferences.challengePreference` — valid values: `independent` | `guided` | `collaborative` (parent-set starting point).
- `observedBehavior.frustrationResponse` — valid values: `unknown` | `perseveres` | `slows-down` | `rushes` | `disengages` (learned from sessions, never set by questionnaire).
- `observedBehavior.effortAttribution` — valid values: `unknown` | `process-oriented` | `outcome-oriented`.
- `observedBehavior.hintUsage` — valid values: `unknown` | `proactive` | `reactive` | `avoidant`.
- `observedBehavior.attentionPattern.optimalSessionMinutes` — `null` until enough sessions; derived from accuracy-over-time curves.
- `observedBehavior.attentionPattern.accuracyDecayOnset` — minute mark where accuracy starts dropping; `null` until derived.

**Key principle**: `initialPreferences` are set by the parent. `observedBehavior` is populated entirely by the system from session data — never by questionnaire.

## Progress Tracking (`progress.json`)

Tracks both the **bones** (XP, levels, badges) and the **soul** (ZPD gaps, behavioral signals).

```json
{
  "schemaVersion": 1,
  "learnerId": "learner-uuid",
  "skills": {
    "pattern-recognition": {
      "level": 4,
      "xp": 340,
      "lastPracticed": "2026-04-07",
      "zpd": {
        "independentLevel": 3,
        "scaffoldedLevel": 5
      },
      "recentAccuracy": [1, 1, 0, 1, 1],
      "workingMemorySignal": "stable"
    },
    "sequential-logic": {
      "level": 2,
      "xp": 120,
      "lastPracticed": "2026-04-05",
      "zpd": {
        "independentLevel": 1,
        "scaffoldedLevel": 3
      },
      "recentAccuracy": [0, 1, 0, 1, 0],
      "workingMemorySignal": "overloaded"
    }
  },
  "badges": [
    { "id": "first-puzzle", "name": "Puzzle Pioneer", "earnedDate": "2026-03-15", "category": "logic" }
  ],
  "streaks": { "currentDays": 5, "longestDays": 12 },
  "totalSessions": 28,
  "totalTimeMinutes": 680,
  "totalAssignments": 140,
  "metacognition": {
    "selfCorrectionRate": 0.3,
    "hintRequestRate": 0.15,
    "trend": "improving"
  },
  "challengeFlags": {
    "onboardingComplete": true,
    "timedChallenge80": false,
    "bossComplete": false,
    "teachBackSuccess": false
  }
}
```

**Field notes:**
- `schemaVersion` — increment when the schema changes; used for safe migration of existing files.
- `zpd.independentLevel` — difficulty level the child can solve without help.
- `zpd.scaffoldedLevel` — difficulty level the child can reach with hints.
- **ZPD gap is computed, not stored** — always calculate `gap = scaffoldedLevel - independentLevel` at runtime. Storing it creates inconsistency risk when either level is updated independently.
- `recentAccuracy` — ring buffer of the last 5 attempts (1 = correct, 0 = incorrect). Backend must truncate to 5 entries on every write.
- `workingMemorySignal` — valid values: `stable` | `overloaded` (derived from multi-step problem performance).
- `metacognition.selfCorrectionRate` — fraction of assignments where the child changed their answer before submitting.
- `metacognition.hintRequestRate` — proactive hint requests per assignment.
- `metacognition.trend` — valid values: `improving` | `stable` | `declining`.
- `totalAssignments` — total individual assignments completed across all sessions. Needed for badge conditions like `totalAssignments >= 1`.
- `challengeFlags` — a map of boolean flags for challenge/milestone badge conditions. Known keys: `onboardingComplete`, `timedChallenge80`, `bossComplete`, `teachBackSuccess`. The badge eligibility system evaluates these as bare identifiers in condition strings.

**ZPD (Zone of Proximal Development)**: The gap between `independentLevel` and `scaffoldedLevel` is where learning actually happens. The system should always target assignments within this zone — hard enough to stretch, easy enough to succeed with support.
