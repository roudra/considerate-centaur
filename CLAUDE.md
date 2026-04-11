# Educational Companion App

An adaptive educational companion that creates personalized learning experiences for children. Claude generates assignments, tracks progress, and adapts to each child's unique way of thinking — with a focus on building logical reasoning skills.

For non-negotiable invariants, see [CONSTITUTION.md](CONSTITUTION.md). If anything here conflicts with the constitution, the constitution wins.

## Architecture Overview

- **Backend**: Rust (Axum, serde)
- **AI Engine**: Claude API — generates assignments, evaluates answers, drives adaptation
- **Storage**: JSON files + session markdown on local filesystem (no external database)
- **Frontend**: Web dashboard (parent view + child-facing learning interface)

**Claude's role**: Creative engine, not source of truth. Data files are memory. Backend verifies correctness. Generation and evaluation are always separate API calls. The client is untrusted — correct answers are never sent to or accepted from it.

**Pipeline**: GENERATE → VALIDATE → STORE (server-side) → PRESENT (no answer) → CAPTURE → LOOKUP → EVALUATE → RECORD

## Commands

```bash
cargo build              # Build
cargo test               # Test
cargo clippy             # Lint
cargo fmt --check        # Format check
cargo run                # Run
```

## Environment Variables

- `DATA_DIR` — path to the data directory (default: `data`)
- `ANTHROPIC_API_KEY` — Claude API key (not needed for offline/template-only mode)
- `RUST_LOG` — log level filter (e.g. `info`, `educational_companion=debug`)

## Coding Conventions

- Every module defines its own error enum via `thiserror` — no panics in production code
- All data structures use `#[serde(rename_all = "camelCase")]`
- All behavioral enums use `#[serde(rename_all = "kebab-case")]`
- Schema version is validated on every file read; mismatches return typed errors
- Session markdowns are the source of truth for what happened; JSON tracks aggregate state
- All Claude interactions go through a central service layer for consistency and cost control
- ZPD gap is computed at runtime (`scaffoldedLevel - independentLevel`), never stored

## Hallucination Risks

| Risk Area | Mitigation |
|---|---|
| `correctAnswer` in generated assignments | Backend independently computes and verifies |
| Session behavioral observations | Labeled as "AI observation", surfaced for parent review |
| Cross-session memory | Every API call includes fresh profile, progress, and last 2-3 session summaries |
| Skill/level references | skill-tree.json injected into every generation prompt |
| Free-form answer evaluation | Flagged for parent review; confidence level reported |

## Detailed Specs (read on demand)

| Topic | File |
|---|---|
| Learner profile & progress schemas | [docs/data-model.md](docs/data-model.md) |
| Session markdown format | [docs/session-format.md](docs/session-format.md) |
| Assignment types, badges, difficulty adaptation | [docs/assignment-system.md](docs/assignment-system.md) |
| Claude prompts, reliability, feedback guardrails | [docs/claude-integration.md](docs/claude-integration.md) |
| SM-2 spaced repetition algorithm | [docs/spaced-repetition.md](docs/spaced-repetition.md) |
| Skill tree, challenges, teach-back | [docs/gamification.md](docs/gamification.md) |
| First session & calibration flow | [docs/onboarding.md](docs/onboarding.md) |
| Assignment buffer & degradation tiers | [docs/offline-resilience.md](docs/offline-resilience.md) |
| Data flow, COPPA, content safety | [docs/privacy.md](docs/privacy.md) |
| Parent dashboard views & shared sessions | [docs/dashboard.md](docs/dashboard.md) |
