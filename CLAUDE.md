# Educational Companion App

## Project Vision

An adaptive educational companion that creates personalized learning experiences for children. The system uses Claude to generate assignments, track progress, and adapt to each child's unique learning style — with a focus on building logical thinking skills.

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
```json
{
  "id": "learner-uuid",
  "name": "Display Name",
  "age": 8,
  "learningStyle": {
    "primary": "visual",          // visual | auditory | kinesthetic | reading-writing
    "pacing": "steady",           // quick | steady | deliberate
    "challengePreference": "guided" // independent | guided | collaborative
  },
  "strengths": ["pattern-recognition", "spatial-reasoning"],
  "growthAreas": ["sequential-logic", "word-problems"],
  "interests": ["dinosaurs", "space", "building"],
  "adaptiveParameters": {
    "difficultyLevel": 3,         // 1-10 scale, auto-adjusted
    "hintFrequency": "moderate",
    "repetitionFactor": 1.2,
    "sessionLengthMinutes": 25
  }
}
```

### Progress Tracking (`progress.json`)
```json
{
  "learnerId": "learner-uuid",
  "skills": {
    "pattern-recognition": { "level": 4, "xp": 340, "lastPracticed": "2026-04-07" },
    "sequential-logic": { "level": 2, "xp": 120, "lastPracticed": "2026-04-05" }
  },
  "badges": [
    { "id": "first-puzzle", "name": "Puzzle Pioneer", "earnedDate": "2026-03-15", "category": "logic" }
  ],
  "streaks": { "currentDays": 5, "longestDays": 12 },
  "totalSessions": 28,
  "totalTimeMinutes": 680
}
```

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
- Observations: Strong with number patterns, struggled with shape rotation sequences
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
- After 3 consecutive correct answers at a level → increase difficulty
- After 2 consecutive incorrect answers → decrease difficulty by 1, offer hints
- After a wrong answer followed by a correct with hints → maintain level, reduce hints gradually
- Weekly review: if average accuracy > 85% → push toward new skill areas
- Weekly review: if average accuracy < 60% → reinforce fundamentals with varied approaches

## Claude Integration

### How Claude Is Used
1. **Assignment Generation**: Claude generates age-appropriate assignments tailored to the learner's profile, current skill levels, and interests (e.g., framing logic puzzles with dinosaur themes)
2. **Response Evaluation**: Claude evaluates free-form answers, provides encouraging feedback, and identifies misconceptions
3. **Adaptive Recommendations**: After each session, Claude analyzes performance and recommends next-session focus areas
4. **Session Summaries**: Claude produces the session markdown with observations about learning patterns

### Prompt Design Principles
- Always age-appropriate language
- Encouraging tone — celebrate effort, not just correctness
- Connect to learner's stated interests where possible
- When a learner struggles, provide scaffolded hints rather than answers
- Explain *why* an answer is correct to reinforce learning

## Parent Dashboard

### Views
- **Overview**: current streaks, recent badges, skill radar chart
- **Skill Detail**: drill into any skill — see level progression over time, common error patterns
- **Session History**: browse past session markdowns
- **Learner Settings**: adjust profile, interests, session length, difficulty preferences
- **Multi-Learner Support**: switch between learner profiles (each child gets their own persona with independent learning curves)

## Development Guidelines

### Commands
- Build: `cd sync-service && ./gradlew build`
- Test: `cd sync-service && ./gradlew test`

### Key Principles
- Every learner is different — never hard-code learning paths; always adapt from profile data
- Session markdowns are the source of truth for what happened; JSON tracks aggregate state
- Keep the child-facing UI simple, colorful, and distraction-free
- Parent dashboard shows data, not raw AI output
- All Claude interactions must go through a central service layer for consistency and cost control
- Learner data is private — never send identifiable info beyond first name to Claude
