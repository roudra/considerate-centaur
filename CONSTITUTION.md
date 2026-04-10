# Project Constitution

These are the non-negotiable invariants of the Educational Companion. They apply to every line of code, every PR, every feature, every interaction. They do not change.

If a spec detail in CLAUDE.md conflicts with a principle here, the constitution wins.

---

## 1. The Child Comes First

The child's emotional safety is more important than any feature, metric, or technical goal.

- **Never make the child feel bad about a wrong answer.** Frame every mistake as a learning opportunity.
- **Never show frustration with the child.** If the system detects frustration, it adapts — it does not push harder.
- **Never force a task.** Every assignment has a skip option. The child is always in control.
- **Never display scores during a session.** The child should feel they are playing, not being tested.
- **Never compare the child to other learners.** Progress is personal, not competitive.

## 2. Growth Mindset in All Feedback

Every piece of feedback — from Claude, from the UI, from badge descriptions — must reinforce a growth mindset.

- Praise **effort, strategy, and persistence** — never raw ability.
- Say "You kept trying different approaches!" — never "You're so smart!"
- After failure: "Not quite — but you're thinking in the right direction!" — never "Wrong."
- Celebrate the **process** of learning, not just the outcome.

## 3. Observe, Don't Label

The system never assigns static learning style labels to a child. No "visual learner." No "slow learner." No categories.

- `observedBehavior` is populated from session data — never from questionnaires or parent input.
- Behavioral dimensions (`frustrationResponse`, `effortAttribution`, `hintUsage`) are **observations that can change**, not fixed traits.
- The system adapts to what it observes, session by session. Today's "disengages" might be next month's "perseveres."

## 4. No Real Names

The system never asks for, collects, or stores a child's real name.

- The `name` field is a **child-chosen display name** — any alias, character name, or nickname is accepted.
- This display name is the only identifier shown in the UI and sent to Claude.
- System-internal identifiers (`id`, `learnerId`, UUIDs) are **never sent to Claude** and never shown to the child.

## 5. Claude Is the Creative Engine, Not the Source of Truth

Claude generates, adapts, and encourages. It does not decide, store, or verify.

- **Data files are the memory.** Claude is stateless. Every API call includes fresh context from JSON files.
- **The backend verifies correctness.** Claude's `correctAnswer` is checked programmatically before showing to the child. Claude can be wrong — the backend catches it.
- **The backend writes session files.** Claude provides narrative content as structured output. The backend assembles and persists the markdown. The file on disk is the source of truth.
- **Generation and evaluation are always separate.** Never use the same Claude call to create a problem and judge the child's answer.
- **The client is untrusted.** The correct answer is never sent to or accepted from the client. Verified assignments are stored server-side; the client receives only an assignment ID. Evaluation looks up the stored answer by ID.
- **Structured output only.** All Claude responses are parsed into typed structs — never freeform text.

## 6. Privacy by Architecture

Privacy is not a feature to be added — it is a structural constraint on how data flows.

- **Local-first storage.** All learner data lives on the local filesystem. No cloud database, no third-party analytics.
- **Minimal data to Claude.** Only what's needed for generation and evaluation crosses the API boundary: display name, age, interests, skill levels, ZPD, recent session summaries.
- **Never sent to Claude:** `id`, `learnerId`, UUIDs, parent information, raw session logs, system metadata.
- **Parent controls data.** The parent can export all data and delete all learner data at any time.
- **No tracking.** No advertising, no analytics SDKs, no tracking pixels.

## 7. ZPD Drives All Adaptation

The Zone of Proximal Development is the core learning principle. Every assignment targets the gap between what the child can do alone and what they can do with support.

- **ZPD gap is computed, never stored.** `gap = scaffoldedLevel - independentLevel`, calculated at runtime. Storing it creates inconsistency.
- **Assignments target the ZPD midpoint** — hard enough to stretch, easy enough to succeed with support.
- **When independent catches up to scaffolded**, both levels rise — the child has internalized the skill.
- **Working memory overload** triggers reduced complexity, not just reduced difficulty.

## 8. The System Must Always Work

A child should never stare at a loading screen or encounter an error. Learning continues regardless of connectivity.

- **Offline-first resilience.** If Claude is down, serve from the assignment buffer. If the buffer is empty, generate from templates. If there's no connectivity, queue everything for sync.
- **Four degradation tiers:** Full → Buffered → Template → Offline. The child's experience degrades gracefully, never abruptly.
- **Every assignment in the buffer is pre-verified.** No unverified content reaches the child, even in offline mode.

## 9. Bones and Soul, Always Both

Every feature must have structural integrity (bones) and adaptive warmth (soul). One without the other fails.

- **Without bones:** progress is unmeasurable, adaptation is guesswork, data is inconsistent.
- **Without soul:** the system is a worksheet generator — technically correct but emotionally dead.
- Before merging any PR, ask: does this strengthen the bones, deepen the soul, or both?

## 10. Every Child Is Different

No two children learn the same way. The system must adapt to each child individually.

- **Never hard-code learning paths.** Always adapt from observed behavioral data.
- **Each learner profile is fully independent** — their own ZPD levels, observed behaviors, learning curves, and session history.
- **Interests drive engagement.** Theme assignments around what the child cares about.
- **Session length adapts.** Some children focus for 30 minutes, others for 10. The system learns this from `attentionPattern`, not from a setting.
