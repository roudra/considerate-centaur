# Educational Companion App — Technical Spec

> **This is the living technical spec.** It defines schemas, features, and architecture.
> For non-negotiable invariants that never change, see **[CONSTITUTION.md](CONSTITUTION.md)**.
> If anything here conflicts with the constitution, the constitution wins.

## Project Vision

An adaptive educational companion that creates personalized learning experiences for children. The system uses Claude to generate assignments, track progress, and adapt to each child's unique way of thinking — with a focus on building logical reasoning skills.

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

## Onboarding & First Session

The first session has no behavioral data — no ZPD baselines, no observed frustration patterns, no accuracy history. A cold start risks assignments that are too hard (discouraging) or too easy (boring). The onboarding session must feel like **play, not a test**.

### First Session Flow

1. **Choose Your Name** — the child picks their display name. Can be anything: "DragonMaster", "Luna", "Captain Cheese". This is their identity in the system.
2. **Interest Discovery** — quick, visual selection of topics they like (dinosaurs, space, animals, robots, art, music, sports, etc.). Presented as icons/images, not a text form.
3. **Calibration Puzzles** — a series of 8-10 short, varied puzzles across skill categories. Each starts at low difficulty and adapts up/down within the puzzle:
   - 2 pattern recognition (number sequences, visual patterns)
   - 2 sequential logic (ordering steps, simple algorithms)
   - 2 spatial reasoning (shape matching, rotation)
   - 2 deductive reasoning (simple if-then, elimination)
4. **Baseline Seeding** — from the calibration results, the backend populates initial ZPD values for each skill:
   - Highest difficulty solved without help → `independentLevel`
   - Highest difficulty solved with a hint → `scaffoldedLevel`
   - Skills not attempted → left at defaults (independentLevel: 1, scaffoldedLevel: 2)
5. **Welcome Badge** — the child earns their first badge ("Explorer" or similar) just for completing onboarding. Immediate positive reinforcement.

### Design Principles for Onboarding
- No scores shown — the child should not feel evaluated
- Every puzzle has a "skip" option — no forced frustration
- Calibration puzzles are themed around the interests the child just selected
- The session is shorter than a normal session (10-15 minutes)
- The tone is exploratory: "Let's see what kinds of puzzles you like!"

## Assignment System

### Multi-Modal Assignment Types

Children engage differently with different modalities. The assignment system supports multiple interaction types beyond text-only problems.

#### Interaction Modalities

| Modality | Examples | Implementation |
|---|---|---|
| **Text** | "What comes next: 2, 4, 8, ?" | Standard text input/multiple choice |
| **Visual-Interactive** | Drag shapes into position, complete a pattern grid, rotate a shape to match | Canvas-based UI components with drag/drop |
| **Sequencing** | Arrange steps in order, sort items by rule | Drag-to-reorder list components |
| **Drawing** | Draw the missing shape, sketch a pattern continuation | Simple drawing canvas with shape tools |
| **Audio-Enhanced** | Spoken instructions for younger children, sound-pattern puzzles | Text-to-speech for prompts, audio playback for patterns |
| **Teach-Back** | "Explain to your friend why 16 comes next" | Free-text or voice input, evaluated for reasoning quality |

#### Assignment Template with Modality

```json
{
  "type": "pattern-completion",
  "modality": "visual-interactive",
  "skill": "spatial-reasoning",
  "difficulty": 3,
  "theme": "space",
  "prompt": "The spaceship is flying in a pattern. Drag it to where it goes next!",
  "interactionType": "drag-drop",
  "visualAssets": ["grid-3x3", "spaceship-sprite"],
  "correctAnswer": {"position": [2, 2]},
  "hints": [
    "Look at the path the spaceship has taken so far...",
    "It's moving diagonally — down and to the right!",
    "Where would it land if it keeps going the same way?"
  ],
  "explanation": "The spaceship moves one square down and one square right each time. That's a diagonal pattern!"
}
```

#### Age-Based Modality Weighting
- **Ages 5-6**: Heavy visual-interactive and audio-enhanced, minimal text input
- **Ages 7-8**: Mix of visual and text, introduce sequencing and drawing
- **Ages 9-10**: More text-based and teach-back, visual for spatial reasoning
- **Ages 11+**: Primarily text and teach-back, visual only for complex spatial problems

The system tracks which modalities produce higher engagement (longer time-on-task, higher accuracy) per learner and weights future assignments accordingly.

## Spaced Repetition

### The Problem
We track `lastPracticed` per skill but have no mechanism to schedule reviews. A skill mastered 3 weeks ago and never revisited will decay. The child might appear to have a high level but can't actually perform at it anymore.

### SM-2 Inspired Algorithm

Each skill in `progress.json` gains spaced repetition fields:

```json
{
  "pattern-recognition": {
    "level": 4,
    "xp": 340,
    "lastPracticed": "2026-04-07",
    "zpd": { "independentLevel": 3, "scaffoldedLevel": 5 },
    "recentAccuracy": [1, 1, 0, 1, 1],
    "workingMemorySignal": "stable",
    "spacedRepetition": {
      "intervalDays": 7,
      "easeFactor": 2.5,
      "nextReviewDate": "2026-04-14",
      "consecutiveCorrect": 4
    }
  }
}
```

### How It Works

1. **After each practice session**, update the spaced repetition fields based on performance:
   - Session accuracy >= 80% → increase `intervalDays` by multiplying with `easeFactor`, increment `consecutiveCorrect`
   - Session accuracy 60-79% → keep `intervalDays` the same, reset `consecutiveCorrect` to 0
   - Session accuracy < 60% → reset `intervalDays` to 1 (review tomorrow), decrease `easeFactor` by 0.2 (min 1.3)
2. **Session planning** checks all skills' `nextReviewDate` and mixes review assignments into new-skill sessions:
   - Typically 70% new/advancing content, 30% spaced review
   - If multiple skills are overdue for review, prioritize by: days overdue x skill level (higher-level skills are more costly to lose)
3. **Review assignments are shorter** — 2-3 quick problems at the `independentLevel` to confirm retention, not full difficulty progression

### Dashboard Visibility
The parent dashboard shows a "Skill Health" indicator per skill:
- **Fresh** — practiced recently, next review not due yet
- **Due** — review date has arrived, will be included in next session
- **Overdue** — missed review window, skill may be decaying
- **Rusty** — significantly overdue, will get priority review

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
  STORE     →  Backend stores the verified assignment server-side (keyed by UUID)
      ↓
  PRESENT   →  Return to client: assignment ID + prompt/hints (NO correct answer)
      ↓
  CAPTURE   →  Client sends back: assignment ID + child's response
      ↓
  LOOKUP    →  Backend retrieves stored assignment by ID (client never supplies correctAnswer)
      ↓
  EVALUATE  →  Claude evaluates (given: problem, correct answer, child's response, behavioral context)
      ↓
  RECORD    →  Backend writes session markdown (source of truth, not Claude)
```

**Trust boundary:** The client is untrusted. It never receives or sends back the `correctAnswer`. The generate endpoint returns only the assignment ID, prompt, hints, and difficulty. The evaluate endpoint accepts only the assignment ID and the child's response — the backend looks up the stored verified assignment to determine correctness. This prevents any client-side forgery of answers.

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
  "verificationLevel": "full",
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
  "verificationLevel": "partial",
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
- **Skill Health Map**: spaced repetition status per skill — fresh, due, overdue, rusty — so parents can see which skills need review

### Parent-Child Shared Sessions

The dashboard isn't just for observation — parents can actively participate in learning.

#### Shared Session Mode
- Parent activates "Join Session" from the dashboard
- The system presents a **co-solve challenge** — a harder problem designed for two people to work through together
- Both parent and child see the problem; the child leads the solving, the parent provides support
- The system observes the collaborative dynamic and records it in the session markdown:
  - Did the parent take over, or did the child lead?
  - Did the child ask for help proactively, or only after struggling?
  - How did the child respond to the parent's hints vs. the system's hints?

#### Why This Matters
- Directly supports the `collaborative` challenge preference
- Gives parents firsthand insight into how their child thinks (not just dashboard data)
- Creates a shared positive experience around learning
- The system can calibrate its own scaffolding by observing how a skilled human (the parent) scaffolds

#### Session Markdown Addition
```markdown
## Shared Session: Parent Co-Solve
- **Mode**: collaborative
- **Parent role observed**: guide (let child lead, asked questions rather than giving answers)
- **Child response to parent scaffolding**: positive — built on parent's hints, arrived at answer independently
- **Comparison to system scaffolding**: child more willing to take risks with parent present
```

## Offline & Resilience Architecture

The child should never stare at a loading screen. If Claude API is down or slow, learning continues.

### Assignment Buffer

The backend maintains a pre-generated buffer of assignments per learner:

```json
{
  "learnerId": "learner-uuid",
  "generatedAt": "2026-04-07T15:00:00Z",
  "bufferSize": 10,
  "assignments": [
    {
      "type": "sequence-puzzle",
      "skill": "pattern-recognition",
      "difficulty": 4,
      "theme": "dinosaurs",
      "prompt": "...",
      "correctAnswer": 16,
      "hints": ["...", "...", "..."],
      "explanation": "..."
    }
  ]
}
```

### Buffer Strategy
- After each session, the backend requests Claude to generate the **next session's worth of assignments** (5-8 problems) and stores them in the buffer
- Buffer covers a mix of skills based on the spaced repetition schedule and ZPD targets
- Buffer assignments are pre-validated by the backend (correctAnswer verified)
- When a session starts, the system tries Claude first for fresh, contextual assignments; falls back to buffer if Claude is unavailable

### Graceful Degradation Tiers

| Tier | Condition | Behavior |
|---|---|---|
| **Full** | Claude API responsive | Fresh assignments, real-time evaluation, behavioral feedback |
| **Buffered** | Claude API slow (>5s) or down | Serve from pre-generated buffer; defer evaluation to when API returns |
| **Template** | Buffer empty + Claude down | Generate from assignment templates using deterministic rules (no Claude); basic right/wrong feedback only |
| **Offline Practice** | No connectivity at all | Template-generated assignments; all session data queued for sync when connectivity returns |

### Data Sync on Reconnect
- Session data recorded during offline/buffered mode is stored locally
- When connectivity returns, the backend syncs session data and requests Claude to generate behavioral observations retroactively
- The parent dashboard marks offline sessions with a note: "Session completed offline — behavioral observations generated after sync"

## Privacy & Safety Architecture

### Core Privacy Principles

1. **No real names** — the system never asks for or stores a child's real name. The `name` field is a child-chosen display name (alias, character name, anything)
2. **Local-first storage** — all learner data lives on the local filesystem. No cloud database, no third-party analytics
3. **Minimal data to Claude** — only what's needed for assignment generation and evaluation crosses the API boundary

### Data Flow: What Goes Where

```
LOCAL STORAGE (never leaves the device):
├── profile.json      → includes id, age, interests, observed behavior
├── progress.json     → skill levels, ZPD, badges, metacognition, spaced repetition
├── session markdowns  → full session logs with behavioral observations
└── buffer.json       → pre-generated assignments

SENT TO CLAUDE API (sanitized):
├── display name only  → "StarExplorer42" (child-chosen, not real name)
├── age               → needed for age-appropriate language calibration
├── interests         → needed for theming assignments
├── skill levels + ZPD → needed for difficulty calibration
├── recent session summaries → behavioral observations + continuity notes only
└── current session context → assignments and responses so far

NEVER SENT TO CLAUDE:
├── id / learnerId / UUIDs
├── any parent information
├── raw session logs (only summaries)
└── system-internal metadata
```

### COPPA Considerations
- The system is designed for use by a parent with their child — the parent is the account holder and data controller
- No data is transmitted to third parties beyond the Claude API calls (which contain only the sanitized fields above)
- No advertising, no tracking pixels, no analytics SDKs
- Parent can export all data (JSON + markdown files) at any time
- Parent can delete all learner data with a single action — deletes the `<learner-id>/` directory entirely
- Session data retention is controlled by the parent — no automatic cloud backups

### Content Safety
- All Claude-generated content passes through the feedback guardrails (see Reliability Architecture)
- Assignment content is constrained by templates — Claude cannot introduce arbitrary topics
- The parent review queue catches any edge cases in evaluation
- No user-generated content is shared between learners or exposed externally

## Deeper Gamification

Badges and streaks provide basic motivation. Deeper gamification creates a sense of **journey, mastery, and agency**.

### Skill Tree Visualization

The child sees their skills as a visual **unlock tree** — not a flat list:

```
                    [Critical Thinking]
                    /                 \
        [Deductive Reasoning]    [Problem Decomposition]
              |                        |
      [Sequential Logic]        [Spatial Reasoning]
              \                  /
           [Pattern Recognition]
                  (start here)
```

- Skills at the bottom are foundational; skills higher up require prerequisites
- Each skill node shows: current level, XP progress bar, badges earned
- Locked skills are visible but grayed out — the child can see what's coming and what they need to unlock it
- Unlocking a new skill is a major event with celebration animation

### Challenge Modes

Beyond regular sessions, special challenge modes add variety:

#### Timed Challenges
- "Speed Round" — 5 problems, 60 seconds each, at the child's independent level
- Focus on fluency and confidence, not pushing difficulty
- Earn a unique "Lightning" badge for completing with 80%+ accuracy

#### Boss Battles
- A multi-step, multi-skill problem that combines 2-3 skill areas
- Example: "The Dinosaur Maze" — uses spatial reasoning to navigate, sequential logic to follow clues, pattern recognition to decode a message
- Only appears when the child has sufficient levels in all required skills
- Earns a special "Boss Defeated" badge with the challenge name

#### Daily Puzzle
- One optional puzzle per day, outside of regular sessions
- Rotates across skill areas
- Maintains a separate streak counter ("Daily Puzzle Streak")
- Low pressure — no impact on skill levels, just XP and badges

### Teach-Back Moments

The deepest form of learning is explaining a concept to someone else. The system periodically asks the child to teach back:

- After mastering a concept (3+ consecutive correct at a level), the system says: "You're really good at this! Can you explain how you figured it out? Pretend you're teaching a friend."
- The child responds via text or voice input
- Claude evaluates the explanation for:
  - **Accuracy** — did they describe the actual concept correctly?
  - **Completeness** — did they cover the key steps?
  - **Clarity** — would a peer understand this?
- Successful teach-backs earn a unique "Teacher" badge per skill
- The explanation is recorded in the session markdown as a metacognition signal
- This data feeds into `metacognition.trend` — children who can teach back are demonstrating deep understanding

### Progression Feel

- **XP notifications** are shown after each assignment ("Pattern Recognition +20 XP!")
- **Level-up celebrations** are prominent — full-screen animation, badge award
- **Streak protection** — if the child misses one day, they get a "streak shield" (once per week) to maintain momentum without punishment
- **Milestone timeline** — the dashboard shows a visual timeline of achievements, so the child can look back at how far they've come

## Development Guidelines

### Commands
- Build: `cargo build`
- Test: `cargo test`
- Lint: `cargo clippy`
- Format check: `cargo fmt --check`
- Run: `cargo run`

### Environment Variables
- `DATA_DIR` — path to the data directory (default: `data`). All learner profiles, progress files, session markdowns, and buffers are stored here.
- `ANTHROPIC_API_KEY` — API key for Claude. Required for assignment generation and evaluation. Not needed for offline/template-only mode.
- `RUST_LOG` — log level filter (e.g. `info`, `debug`, `educational_companion=debug`). Uses `tracing-subscriber` with `EnvFilter`.

### Key Principles

All non-negotiable principles live in [CONSTITUTION.md](CONSTITUTION.md). The following are technical guidelines for this codebase:

- Session markdowns are the source of truth for what happened; JSON tracks aggregate state
- Keep the child-facing UI simple, colorful, and distraction-free
- Parent dashboard shows insights (behavioral trends, ZPD growth), not raw AI output
- All Claude interactions must go through a central service layer for consistency and cost control
- Every module defines its own error enum via `thiserror` — no panics in production code
- All data structures use `#[serde(rename_all = "camelCase")]`; all behavioral enums use `#[serde(rename_all = "kebab-case")]`
- Schema version is validated on every file read; mismatches return typed errors
