# Educational Companion App

## Project Vision

An adaptive educational companion that creates personalized learning experiences for children. The system uses Claude to generate assignments, track progress, and adapt to each child's unique way of thinking — with a focus on building logical reasoning skills.

### Design Philosophy: Bones & Soul

This project follows a "bones and soul" principle — both halves are essential:

- **Bones** (Structure): The rigid, load-bearing skeleton — data schemas, skill trees, session formats, APIs, progress tracking. These must be stable and well-defined because everything hangs off them. Without strong bones, progress is unmeasurable and adaptation is guesswork.
- **Soul** (Adaptive Intelligence): The living, breathing part that makes this more than a quiz engine. Claude's ability to *see* the child — to adapt tone, theme assignments around their interests, recognize frustration, celebrate effort, and write observations that help parents understand how their child thinks. Without soul, this is just a worksheet generator.

Every design decision should ask: does this strengthen the bones, deepen the soul, or both?

## Core Architecture

### Tech Stack
- **Backend**: Kotlin/Gradle (existing sync-service as foundation)
- **AI Engine**: Claude API (Anthropic SDK) for assignment generation, evaluation, and adaptive learning
- **Data Storage**: Structured JSON + session markdown files (no external database)
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
  /data
    /learners         # Per-learner directories
      /<learner-id>
        profile.json          # Learning style, preferences, strengths/weaknesses
        progress.json         # Cumulative skill scores and badge inventory
        /sessions
          session-YYYY-MM-DD-HHmm.md  # Session logs (human-readable)
    /curriculum
      skill-tree.json         # Master skill/badge definitions
      assignment-templates/   # Reusable assignment templates
  /web
    /parent-dashboard  # Progress dashboard UI
    /learner-ui        # Child-facing learning interface
```

## Data Model

### Learner Profile (`profile.json`)

The profile avoids static "learning style" labels (the VARK model is not supported by research). Instead, it captures initial preferences and lets the system **observe and adapt** through behavioral dimensions tracked in `progress.json`.

```json
{
  "id": "learner-uuid",
  "name": "Display Name",
  "age": 8,
  "interests": ["dinosaurs", "space", "building"],
  "initialPreferences": {
    "sessionLengthMinutes": 25,
    "challengePreference": "guided"   // independent | guided | collaborative — parent-set starting point
  },
  "observedBehavior": {
    "frustrationResponse": "unknown",   // perseveres | slows-down | rushes | disengages — learned from sessions
    "effortAttribution": "unknown",     // process-oriented | outcome-oriented — learned from feedback patterns
    "hintUsage": "unknown",             // proactive | reactive | avoidant — learned from sessions
    "attentionPattern": {
      "optimalSessionMinutes": null,    // null until enough data; derived from accuracy-over-time curves
      "accuracyDecayOnset": null        // minute mark where accuracy starts dropping
    }
  }
}
```

**Key principle**: `initialPreferences` are set by the parent. `observedBehavior` is populated entirely by the system from session data — never by questionnaire.

### Progress Tracking (`progress.json`)

Tracks both the **bones** (XP, levels, badges) and the **soul** (ZPD gaps, behavioral signals).

```json
{
  "learnerId": "learner-uuid",
  "skills": {
    "pattern-recognition": {
      "level": 4,
      "xp": 340,
      "lastPracticed": "2026-04-07",
      "zpd": {
        "independentLevel": 3,        // can solve alone at this difficulty
        "scaffoldedLevel": 5,         // can solve with hints at this difficulty
        "gap": 2                      // the zone of proximal development
      },
      "recentAccuracy": [1, 1, 0, 1, 1],   // last 5 attempts (1=correct, 0=incorrect)
      "workingMemorySignal": "stable"       // stable | overloaded — from multi-step problem performance
    },
    "sequential-logic": {
      "level": 2,
      "xp": 120,
      "lastPracticed": "2026-04-05",
      "zpd": {
        "independentLevel": 1,
        "scaffoldedLevel": 3,
        "gap": 2
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
    "selfCorrectionRate": 0.3,        // how often they change an answer before submitting
    "hintRequestRate": 0.15,          // proactive hint requests per assignment
    "trend": "improving"              // improving | stable | declining
  }
}
```

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
4. **Session Summaries**: Claude produces the session markdown with behavioral observations and continuity notes that reference previous sessions
5. **Session-to-Session Memory**: Claude references prior sessions to build narrative continuity ("Remember yesterday when you cracked that tricky pattern? Let's build on that today")

### Prompt Design Principles
- Always age-appropriate language calibrated to the child's age
- **Growth mindset reinforcement** — celebrate effort, strategy, and persistence, never raw ability ("You kept trying different approaches!" not "You're so smart!")
- Connect to learner's stated interests where possible to increase engagement
- When a learner struggles, provide scaffolded hints (not answers) — start vague, get specific only if needed
- Explain *why* an answer is correct to reinforce learning and build metacognition
- **Frustration awareness** — if behavioral signals suggest frustration, pivot to encouragement and easier problems before returning to the challenge
- Never make the child feel bad about wrong answers — frame them as learning opportunities

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
- Build: `cd sync-service && ./gradlew build`
- Test: `cd sync-service && ./gradlew test`

### Key Principles
- Every learner is different — never hard-code learning paths; always adapt from observed behavioral data
- **No static labels** — never categorize a child as a "visual learner" or similar; observe, don't label
- Session markdowns are the source of truth for what happened; JSON tracks aggregate state
- **ZPD-driven**: always target the zone between independent and scaffolded ability
- Keep the child-facing UI simple, colorful, and distraction-free
- Parent dashboard shows insights (behavioral trends, ZPD growth), not raw AI output
- **Growth mindset in all feedback** — praise process, not talent
- All Claude interactions must go through a central service layer for consistency and cost control
- Learner data is private — never send identifiable info beyond first name to Claude
