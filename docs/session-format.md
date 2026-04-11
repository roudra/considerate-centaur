# Session Markdown Format

Session files (`session-YYYY-MM-DD-HHmm.md`) are human-readable logs that capture what happened in each learning session. They are the source of truth for what occurred — JSON tracks aggregate state.

## Template

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

## Shared Session Addition

When a parent joins a session in collaborative mode:

```markdown
## Shared Session: Parent Co-Solve
- **Mode**: collaborative
- **Parent role observed**: guide (let child lead, asked questions rather than giving answers)
- **Child response to parent scaffolding**: positive — built on parent's hints, arrived at answer independently
- **Comparison to system scaffolding**: child more willing to take risks with parent present
```

## Key Design Points

- Claude generates the behavioral observation text and continuity notes as structured output
- The **backend assembles and writes** the markdown file — Claude provides narrative, the backend is the author on disk
- The Behavioral Observations and Continuity Notes sections are included in subsequent Claude API calls as session-to-session memory
- Full assignment-by-assignment logs are NOT sent to Claude — only the summary sections
