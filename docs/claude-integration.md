# Claude Integration

## How Claude Is Used

1. **Assignment Generation**: Claude generates age-appropriate assignments tailored to the learner's profile, current ZPD levels, and interests (e.g., framing logic puzzles with dinosaur themes for a dinosaur-loving child)
2. **Response Evaluation**: Claude evaluates free-form answers, provides encouraging feedback, identifies misconceptions, and notes behavioral signals (hesitation, self-correction, speed)
3. **Adaptive Recommendations**: After each session, Claude analyzes performance and behavioral observations to recommend next-session focus areas, difficulty adjustments, and theme choices
4. **Session Summaries**: Claude generates the behavioral observation text and continuity notes as structured output; the **backend then assembles and writes** the session markdown file. Claude provides the narrative; the backend is the author of the file on disk (the source of truth).
5. **Session-to-Session Memory**: Claude references prior sessions to build narrative continuity ("Remember yesterday when you cracked that tricky pattern? Let's build on that today")

## Prompt Design Principles

- Always age-appropriate language calibrated to the child's age
- **Growth mindset reinforcement** — celebrate effort, strategy, and persistence, never raw ability ("You kept trying different approaches!" not "You're so smart!")
- Connect to learner's stated interests where possible to increase engagement
- When a learner struggles, provide scaffolded hints (not answers) — start vague, get specific only if needed
- Explain *why* an answer is correct to reinforce learning and build metacognition
- **Frustration awareness** — if behavioral signals suggest frustration, pivot to encouragement and easier problems before returning to the challenge
- Never make the child feel bad about wrong answers — frame them as learning opportunities

## Reliability Architecture: Preventing Hallucination & Memory Lapses

**Core principle: Claude is the creative engine, not the source of truth.** Data files are the memory, the backend validates correctness, templates constrain generation, and the dashboard surfaces uncertainty.

### 1. Structured Output, Not Freeform Generation

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
    "8 x 2 = ?"
  ],
  "explanation": "Each step count doubles the previous one: 2->4->8->16. This is called a geometric sequence!"
}
```

The `correctAnswer` field is **verified programmatically by the backend** before the assignment is shown to the child.

### 2. Ground Every Prompt in Source-of-Truth Data

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

### 3. Separate Generation from Evaluation

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

### 4. Session Context Window Management

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

### 5. Feedback Guardrails

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

### 6. Parent Review Queue

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
