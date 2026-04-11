# Privacy & Safety Architecture

## Core Privacy Principles

1. **No real names** — the system never asks for or stores a child's real name. The `name` field is a child-chosen display name (alias, character name, anything)
2. **Local-first storage** — all learner data lives on the local filesystem. No cloud database, no third-party analytics
3. **Minimal data to Claude** — only what's needed for assignment generation and evaluation crosses the API boundary

## Data Flow: What Goes Where

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

## COPPA Considerations

- The system is designed for use by a parent with their child — the parent is the account holder and data controller
- No data is transmitted to third parties beyond the Claude API calls (which contain only the sanitized fields above)
- No advertising, no tracking pixels, no analytics SDKs
- Parent can export all data (JSON + markdown files) at any time
- Parent can delete all learner data with a single action — deletes the `<learner-id>/` directory entirely
- Session data retention is controlled by the parent — no automatic cloud backups

## Content Safety

- All Claude-generated content passes through the feedback guardrails (see [Claude Integration](claude-integration.md))
- Assignment content is constrained by templates — Claude cannot introduce arbitrary topics
- The parent review queue catches any edge cases in evaluation
- No user-generated content is shared between learners or exposed externally
