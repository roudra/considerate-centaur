# Offline & Resilience Architecture

The child should never stare at a loading screen. If Claude API is down or slow, learning continues.

## Assignment Buffer

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

## Buffer Strategy

- After each session, the backend requests Claude to generate the **next session's worth of assignments** (5-8 problems) and stores them in the buffer
- Buffer covers a mix of skills based on the spaced repetition schedule and ZPD targets
- Buffer assignments are pre-validated by the backend (correctAnswer verified)
- When a session starts, the system tries Claude first for fresh, contextual assignments; falls back to buffer if Claude is unavailable

## Graceful Degradation Tiers

| Tier | Condition | Behavior |
|---|---|---|
| **Full** | Claude API responsive | Fresh assignments, real-time evaluation, behavioral feedback |
| **Buffered** | Claude API slow (>5s) or down | Serve from pre-generated buffer; defer evaluation to when API returns |
| **Template** | Buffer empty + Claude down | Generate from assignment templates using deterministic rules (no Claude); basic right/wrong feedback only |
| **Offline Practice** | No connectivity at all | Template-generated assignments; all session data queued for sync when connectivity returns |

## Data Sync on Reconnect

- Session data recorded during offline/buffered mode is stored locally
- When connectivity returns, the backend syncs session data and requests Claude to generate behavioral observations retroactively
- The parent dashboard marks offline sessions with a note: "Session completed offline — behavioral observations generated after sync"
