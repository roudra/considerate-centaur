# Deeper Gamification

Badges and streaks provide basic motivation. Deeper gamification creates a sense of **journey, mastery, and agency**.

## Skill Tree Visualization

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

## Challenge Modes

Beyond regular sessions, special challenge modes add variety:

### Timed Challenges
- "Speed Round" — 5 problems, 60 seconds each, at the child's independent level
- Focus on fluency and confidence, not pushing difficulty
- Earn a unique "Lightning" badge for completing with 80%+ accuracy

### Boss Battles
- A multi-step, multi-skill problem that combines 2-3 skill areas
- Example: "The Dinosaur Maze" — uses spatial reasoning to navigate, sequential logic to follow clues, pattern recognition to decode a message
- Only appears when the child has sufficient levels in all required skills
- Earns a special "Boss Defeated" badge with the challenge name

### Daily Puzzle
- One optional puzzle per day, outside of regular sessions
- Rotates across skill areas
- Maintains a separate streak counter ("Daily Puzzle Streak")
- Low pressure — no impact on skill levels, just XP and badges

## Teach-Back Moments

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

## Progression Feel

- **XP notifications** are shown after each assignment ("Pattern Recognition +20 XP!")
- **Level-up celebrations** are prominent — full-screen animation, badge award
- **Streak protection** — if the child misses one day, they get a "streak shield" (once per week) to maintain momentum without punishment
- **Milestone timeline** — the dashboard shows a visual timeline of achievements, so the child can look back at how far they've come
