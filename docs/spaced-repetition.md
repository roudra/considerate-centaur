# Spaced Repetition

## The Problem

We track `lastPracticed` per skill but have no mechanism to schedule reviews. A skill mastered 3 weeks ago and never revisited will decay. The child might appear to have a high level but can't actually perform at it anymore.

## SM-2 Inspired Algorithm

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

## How It Works

1. **After each practice session**, update the spaced repetition fields based on performance:
   - Session accuracy >= 80% → increase `intervalDays` by multiplying with `easeFactor`, increment `consecutiveCorrect`
   - Session accuracy 60-79% → keep `intervalDays` the same, reset `consecutiveCorrect` to 0
   - Session accuracy < 60% → reset `intervalDays` to 1 (review tomorrow), decrease `easeFactor` by 0.2 (min 1.3)
2. **Session planning** checks all skills' `nextReviewDate` and mixes review assignments into new-skill sessions:
   - Typically 70% new/advancing content, 30% spaced review
   - If multiple skills are overdue for review, prioritize by: days overdue x skill level (higher-level skills are more costly to lose)
3. **Review assignments are shorter** — 2-3 quick problems at the `independentLevel` to confirm retention, not full difficulty progression

## Dashboard Visibility

The parent dashboard shows a "Skill Health" indicator per skill:
- **Fresh** — practiced recently, next review not due yet
- **Due** — review date has arrived, will be included in next session
- **Overdue** — missed review window, skill may be decaying
- **Rusty** — significantly overdue, will get priority review
