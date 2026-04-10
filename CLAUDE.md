# Educational Companion App

## Project Vision

An adaptive educational companion that creates personalized learning experiences for children. The system uses Claude to generate assignments, track progress, and adapt to each child's unique way of thinking — with a focus on building logical reasoning skills.

### Design Philosophy: Bones & Soul

This project follows a "bones and soul" principle — both halves are essential:

- **Bones** (Structure): The rigid, load-bearing skeleton — data schemas, skill trees, session formats, APIs, progress tracking. These must be stable and well-defined because everything hangs off them. Without strong bones, progress is unmeasurable and adaptation is guesswork.
- **Soul** (Adaptive Intelligence): The living, breathing part that makes this more than a quiz engine. Claude's ability to *see* the child — to adapt tone, theme assignments around their interests, recognize frustration, celebrate effort, and write observations that help parents understand how their child thinks. Without soul, this is just a worksheet generator.

Every design decision should ask: does this strengthen the bones, deepen the soul, or both?

---

## Project Quick Reference

### 1. Purpose of This Repository

This repository is an **adaptive educational companion** for children. Its purpose is to:

- Deliver personalized logic and reasoning assignments to individual learners
- Track progress across a structured skill tree (ZPD-driven, never one-size-fits-all)
- Surface behavioral insights for parents so they understand *how* their child thinks, not just what they scored
- Build a "bones and soul" platform: stable data schemas plus Claude's adaptive intelligence

### 2. What Claude Is Expected to Do

Claude is the **creative and adaptive engine** — not the source of truth. Concretely, Claude:

1. **Generates assignments** — themed to the child's interests, calibrated to their ZPD, structured as verifiable JSON
2. **Evaluates answers** — given the problem, the verified correct answer, and the child's response; provides encouraging, growth-mindset feedback
3. **Produces session narrative content** — behavioral observation text and continuity notes that the **backend then writes** into session markdown files
4. **Drives adaptation** — after each session, recommends next-session focus areas, difficulty adjustments, and theme choices

Claude does **not** store state, decide correctness (the backend does), or write files. Every Claude API call is stateless and grounded in fresh data from the JSON files.

### 3. Where Claude Can Hallucinate and How We Mitigate

| Risk Area | Why It Can Hallucinate | Mitigation |
|---|---|---|
| **correctAnswer in generated assignments** | Claude may produce a wrong answer for a sequence or arithmetic problem | Backend independently computes and verifies the answer before showing to child |
| **Session behavioral observations** | Claude may invent observations not supported by the recorded signals | Backend writes session markdown using structured data; Claude's narrative is labeled as "AI observation" and surfaced for parent review |
| **Cross-session memory** | Claude has no memory between API calls; may confuse learners or invent prior events | Every call includes profile.json, progress.json, and last 2-3 session summaries as grounding context |
| **Skill/level references** | Claude may reference skill names or levels that don't exist in the skill tree | skill-tree.json is injected into every generation prompt; Claude must reference only valid skill IDs |
| **Free-form answer evaluation** | For open-ended questions, Claude may incorrectly assess correctness | High-risk types are flagged for parent review; Claude reports a confidence level; backend never auto-confirms ambiguous evaluations |

---

## Core Architecture

### Tech Stack
- **Backend**: Rust (Actix-Web or Axum for HTTP, serde for JSON serialization)
- **AI Engine**: Claude API (Anthropic SDK) for assignment generation, evaluation, and adaptive learning
- **Data Storage**: Structured JSON + session markdown files (local filesystem, no external database)
- **Frontend**: Web dashboard (parent view + child-facing learning interface)

### Directory Structure
```
/companion-app
  /src
    /learner          # Learner profile management
    /assignments      # Assignment generation & evaluation
    /progress         # Progress tracking, skill trees, badges
    /dashboard        # Parent dashboard API
    /session          # Session management & persistence
    /claude           # Claude API integration layer
    /spaced           # Spaced repetition scheduler
    /offline          # Assignment buffer & offline fallback
  /data
    /learners         # Per-learner directories
      /<learner-id>
        profile.json          # Display name, age, interests, preferences, observed behavior
        progress.json         # Cumulative skill scores, ZPD, badges, metacognition
        /sessions
          session-YYYY-MM-DD-HHmm.md  # Session logs (human-readable)
    /curriculum
      skill-tree.json         # Master skill/badge definitions
      assignment-templates/   # Reusable assignment templates
    /buffer
      /<learner-id>-buffer.json  # Pre-generated assignments for offline use
  /web
    /parent-dashboard  # Progress dashboard UI
    /learner-ui        # Child-facing learning interface
```

## Data Model

### Learner Profile (`profile.json`)

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

### Progress Tracking (`progress.json`)

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
  "metacognition": {
    "selfCorrectionRate": 0.3,
    "hintRequestRate": 0.15,
    "trend": "improving"
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

**ZPD (Zone of Proximal Development)**: The gap between `independentLevel` and `scaffoldedLevel` is where learning actually happens. The system should always target assignments within this zone — hard enough to stretch, easy enough to succeed with support.

### Session Markdown (`session-YYYY-MM-DD-HHmm.md`)
Session files are human-readable logs that capture what happened in each learning session:
```markdown
# Session: 2026-04-07 15:30
## Learner: [Name]
## Focus: Sequential Logic — Level 2

### Assignment 1: Pattern Completion
- **Type**: sequence-puzzle
- **Difficulty**: 3/10
- **Prompt**: "What comes next: 2, 4, 8, 16, ?"
- **Response**: 32
- **Result**: correct
- **Time**: 45s
- **Notes**: Solved quickly, showed understanding of doubling pattern

### Assignment 2: ...

## Session Summary
- Correct: 4/5
- Difficulty adjustment: 3 → 4 (trending up)
- Skills practiced: sequential-logic (+40xp), pattern-recognition (+20xp)
- Badge earned: "Logic Streak" (3 sessions in a row with 80%+)

## Behavioral Observations
- **Frustration signals**: None — stayed engaged throughout
- **Self-correction**: Changed answer on Assignment 3 before submitting (metacognition positive)
- **Accuracy over time**: Consistent across 25 minutes, no decay detected
- **Hint usage**: Requested one hint proactively on Assignment 4 (healthy behavior)
- **Interest engagement**: Dinosaur-themed problems held attention longer (+15s avg time-on-task)
- **Growth mindset note**: After getting Assignment 2 wrong, said "let me try again" — praise effort

## Continuity Notes
- Last session struggled with shape rotations → today tried a simpler rotation warm-up → succeeded
- Ready to introduce 3-step sequential problems next session (ZPD gap supports it)
- Consider introducing "Problem Decomposition" as a new skill area — showed intuitive breaking-apart behavior
```

## Skill & Badge System

### Skill Tree Categories (Logic Focus)
- **Pattern Recognition**: sequences, visual patterns, analogies
- **Deductive Reasoning**: if-then logic, elimination, syllogisms
- **Sequential Logic**: ordering, step-by-step processes, algorithms
- **Spatial Reasoning**: shapes, rotations, maps, symmetry
- **Problem Decomposition**: breaking problems into parts
- **Critical Thinking**: evaluating claims, finding errors, cause-and-effect

### Badge Types
- **Milestone Badges**: first correct answer, first perfect session, 10 sessions completed
- **Skill Badges**: reach level 3/5/7/10 in any skill
- **Streak Badges**: 3-day, 7-day, 30-day learning streaks
- **Challenge Badges**: complete special challenge assignments
- **Explorer Badges**: try a new skill category for the first time

### Difficulty Adaptation Rules

Adaptation targets the **Zone of Proximal Development** — always working in the gap between what the child can do alone and what they can do with support.

#### Within a Session
- After 3 consecutive correct answers at independent level → increase difficulty toward scaffolded level
- After 2 consecutive incorrect answers → decrease difficulty by 1, offer scaffolded hints
- After a wrong answer followed by a correct with hints → maintain level, gradually reduce hint detail
- If frustration signals detected (rapid wrong answers, long pauses, disengagement) → pivot to an easier "confidence builder" assignment, then return

#### Across Sessions
- Weekly review: if average accuracy > 85% → push toward new skill areas or increase ZPD ceiling
- Weekly review: if average accuracy < 60% → reinforce fundamentals with varied problem formats
- If `independentLevel` catches up to `scaffoldedLevel` → the child has internalized the skill; raise both
- If `workingMemorySignal` is "overloaded" for a skill → reduce multi-step complexity, focus on single-concept problems

#### Emotional Adaptation
- After session abandonment → next session starts with a familiar, confidence-building warm-up
- If `effortAttribution` trends toward "outcome-oriented" → Claude feedback shifts to emphasize process ("You tried three different approaches — that's real problem-solving!")
- If `frustrationResponse` is "disengages" → shorter sessions, more frequent badges, lower initial difficulty

## Claude Integration

### How Claude Is Used
1. **Assignment Generation**: Claude generates age-appropriate assignments tailored to the learner's profile, current ZPD levels, and interests (e.g., framing logic puzzles with dinosaur themes for a dinosaur-loving child)
2. **Response Evaluation**: Claude evaluates free-form answers, provides encouraging feedback, identifies misconceptions, and notes behavioral signals (hesitation, self-correction, speed)
3. **Adaptive Recommendations**: After each session, Claude analyzes performance and behavioral observations to recommend next-session focus areas, difficulty adjustments, and theme choices
4. **Session Summaries**: Claude generates the behavioral observation text and continuity notes as structured output; the **backend then assembles and writes** the session markdown file. Claude provides the narrative; the backend is the author of the file on disk (the source of truth).
5. **Session-to-Session Memory**: Claude references prior sessions to build narrative continuity ("Remember yesterday when you cracked that tricky pattern? Let's build on that today")

### Prompt Design Principles
- Always age-appropriate language calibrated to the child's age
- **Growth mindset reinforcement** — celebrate effort, strategy, and persistence, never raw ability ("You kept trying different approaches!" not "You're so smart!")
- Connect to learner's stated interests where possible to increase engagement
- When a learner struggles, provide scaffolded hints (not answers) — start vague, get specific only if needed
- Explain *why* an answer is correct to reinforce learning and build metacognition
- **Frustration awareness** — if behavioral signals suggest frustration, pivot to encouragement and easier problems before returning to the challenge
- Never make the child feel bad about wrong answers — frame them as learning opportunities

### Reliability Architecture: Preventing Hallucination & Memory Lapses

**Core principle: Claude is the creative engine, not the source of truth.** Data files are the memory, the backend validates correctness, templates constrain generation, and the dashboard surfaces uncertainty.

#### 1. Structured Output, Not Freeform Generation

All Claude responses must use structured JSON schemas, never freeform text that gets parsed.

Example — assignment object returned by Claude:

```json
{
  "type": "sequence-puzzle",
  "skill": "pattern-recognition",
  "difficulty": 4,
  "theme": "dinosaurs",
  "prompt": "A T-Rex takes 2 steps, then 4, then 8. How many steps next?",
  "correctAnswer": 16,
  "acceptableAnswers": [16, "16", "sixteen"],
  "hints": [
    "Look at how the number changes each time...",
    "Each time, the number doubles!",
    "8 × 2 = ?"
  ],
  "explanation": "Each step count doubles the previous one: 2→4→8→16. This is called a geometric sequence!"
}
```

The `correctAnswer` field is **verified programmatically by the backend** before the assignment is shown to the child.

#### 2. Ground Every Prompt in Source-of-Truth Data

Claude does not remember across API calls. Every call includes relevant context from data files:

```
Every Claude API call includes:
├── profile.json         → who this child is, interests, observed behaviors
├── progress.json        → current skill levels, ZPD, recent accuracy
├── Last 2-3 session summaries → Behavioral Observations + Continuity Notes sections only
│   (NOT full assignment logs — keep context focused)
└── skill-tree.json      → valid skills, levels, badge definitions
```

**The data files are the memory.** Claude reads them fresh every time. This is why the session markdown format captures continuity notes — they are the retrieval layer for session-to-session coherence.

#### 3. Separate Generation from Evaluation

Never use the same Claude call to generate a problem and evaluate the child's answer:

```
Pipeline (each step is a separate concern):

  GENERATE  →  Claude creates assignment (structured JSON)
      ↓
  VALIDATE  →  Backend verifies correctAnswer programmatically
      ↓
  PRESENT   →  Show assignment to child
      ↓
  CAPTURE   →  Record child's response + timing + behavioral signals
      ↓
  EVALUATE  →  Claude evaluates (given: problem, correct answer, child's response, behavioral context)
      ↓
  RECORD    →  Backend writes session markdown (source of truth, not Claude)
```

By giving Claude the correct answer during evaluation, it cannot hallucinate whether the child is right. Claude's job at that stage is tone and explanation, not correctness judgment.

#### 4. Constrain Generation with Assignment Templates

Templates bound what Claude can produce, reducing the hallucination surface.

`assignment-templates/sequence-puzzle.json`:

```json
{
  "type": "sequence-puzzle",
  "constraints": {
    "sequenceTypes": ["arithmetic", "geometric", "fibonacci-like"],
    "maxTerms": 6,
    "numberRange": [1, 100],
    "operations": ["add", "multiply", "power"]
  },
  "verifiable": true,
  "verificationMethod": "compute-sequence"
}
```

`assignment-templates/deductive-reasoning.json`:

```json
{
  "type": "deductive-reasoning",
  "constraints": {
    "maxPremises": 3,
    "logicTypes": ["if-then", "elimination", "syllogism"],
    "domainVocabulary": "age-appropriate"
  },
  "verifiable": "partial",
  "verificationMethod": "rule-check"
}
```

Claude fills in templates creatively (theming, wording). The backend verifies the underlying logic is sound.

#### 5. Verification Layers by Assignment Type

Not all assignments are equally hallucination-prone. Verify accordingly:

| Assignment Type | Risk Level | Verification Method |
|---|---|---|
| Arithmetic / sequences | Low | Backend computes answer independently |
| Pattern matching | Low | Predefined pattern banks; Claude selects and themes |
| If-then / elimination | Medium | Encode rules as constraints; verify conclusion follows from premises |
| Spatial reasoning | Medium | Use validated visual templates; Claude describes, doesn't create images |
| Free-form reasoning | High | Claude evaluates, backend flags low-confidence for parent review |
| Creative / open-ended | High | No single correct answer; evaluate for effort and reasoning, not correctness |

**Weight assignment mix toward verifiable types**, especially for new learners. Introduce higher-risk types gradually as the parent builds trust in the system.

#### 6. Session Context Window Management

Tiered context strategy to keep Claude focused and reduce noise:

```
ALWAYS include (compact):
  └── profile.json
  └── progress.json (current skill snapshot)

INCLUDE SUMMARIZED (from last 2-3 sessions):
  └── Session Summary section
  └── Behavioral Observations section
  └── Continuity Notes section
  (NOT full assignment-by-assignment logs)

INCLUDE FULL (current session only):
  └── All assignments so far in this session
  └── Child's responses and behavioral signals
```

This prevents context dilution — Claude sees what matters, not everything that ever happened.

#### 7. Feedback Guardrails

Enforce at the prompt level for every evaluation call:

```
Feedback rules (non-negotiable):
- NEVER say "correct" unless the backend has confirmed correctness
- NEVER invent facts — only explain using concepts present in the assignment
- If uncertain about an evaluation, respond with curiosity:
  "That's an interesting approach! Let's look at it together..."
  and flag for parent review
- NEVER use discouraging language
- NEVER compare the child to other learners
- Frame all wrong answers as learning: "Not quite — but you're thinking
  in the right direction! Here's a hint..."
```

#### 8. Parent Review Queue

For cases that can't be fully verified programmatically, surface on the dashboard:

```
Flagged for Review:
┌─────────────────────────────────────────────────────────┐
│ Session 2026-04-07, Assignment 4                        │
│ Type: free-form reasoning                               │
│ Claude's evaluation confidence: medium                  │
│ Child's answer: "Because the big one eats the small one"│
│ Claude's assessment: "Creative reasoning, partially     │
│   correct — understood the elimination concept"         │
│ Actions: [✓ Confirm] [✏ Override] [💬 Discuss]          │
└─────────────────────────────────────────────────────────┘
```

The parent is the final verification layer for edge cases. Over time, the review queue shrinks as the system learns which types of evaluations the parent consistently confirms.

## Parent Dashboard

### Views
- **Overview**: current streaks, recent badges, skill radar chart, ZPD visualization (what they can do alone vs. with help)
- **Skill Detail**: drill into any skill — see level progression over time, ZPD gap trends, common error patterns, working memory signals
- **Behavioral Insights**: frustration response trends, metacognition growth, attention patterns over time — the "soul" data that helps parents understand *how* their child learns, not just *what* they scored
- **Session History**: browse past session markdowns with behavioral observations and continuity notes
- **Learner Settings**: adjust profile, interests, session length, challenge preferences
- **Multi-Learner Support**: switch between learner profiles — each child gets their own persona with fully independent observed behaviors, ZPD levels, and learning curves

## Development Guidelines

### Commands
- Build: `cargo build`
- Test: `cargo test`
- Run: `cargo run`

### Key Principles
- Every learner is different — never hard-code learning paths; always adapt from observed behavioral data
- **No static labels** — never categorize a child as a "visual learner" or similar; observe, don't label
- Session markdowns are the source of truth for what happened; JSON tracks aggregate state
- **ZPD-driven**: always target the zone between independent and scaffolded ability
- Keep the child-facing UI simple, colorful, and distraction-free
- Parent dashboard shows insights (behavioral trends, ZPD growth), not raw AI output
- **Growth mindset in all feedback** — praise process, not talent
- All Claude interactions must go through a central service layer for consistency and cost control
- Learner data is private — the system never collects or stores real names. The `name` field is always a child-chosen display name (any alias is accepted). The backend must omit `id`, `learnerId`, UUIDs, and any other system-internal identifiers from every Claude API call; only the display name and non-identifying personalization fields are passed.
