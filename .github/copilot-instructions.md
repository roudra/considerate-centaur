# Copilot Instructions — Educational Companion App

## What This Project Is

An adaptive educational companion that creates personalized learning experiences for children, with a focus on building logical reasoning skills. The system uses Claude (Anthropic API) to generate assignments, evaluate responses, and adapt to each child's unique way of thinking.

**Read before writing any code:**
1. `CONSTITUTION.md` — non-negotiable invariants. If your code violates any of these, the PR will be rejected.
2. `CLAUDE.md` — the living technical spec. Schemas, features, architecture. This is what you implement against.
3. This file — coding conventions, project layout, and established patterns.

## Tech Stack

- **Language**: Rust (2021 edition)
- **Web Framework**: Axum
- **Serialization**: serde / serde_json
- **AI Integration**: Anthropic Claude API via HTTP (reqwest)
- **Data Storage**: Local filesystem — JSON files + session markdown files (no database)
- **Frontend**: Web dashboard (parent view) + child-facing learning UI (TBD)
- **Testing**: `cargo test`, integration tests in `tests/`

## Project Layout

Files marked with * are planned but not yet created:

```
companion-app/
  Cargo.toml
  src/
    main.rs               # Axum server entrypoint + route handlers
    lib.rs                 # Re-exports all modules for integration tests
    learner/
      mod.rs               # Learner profile CRUD (filesystem persistence)
      profile.rs           # LearnerProfile struct, enums, serde, validation
      *onboarding.rs       # Onboarding & calibration flow
    assignments/
      mod.rs               # Assignment generation & evaluation pipeline
      *templates.rs        # Assignment template definitions
      *generator.rs        # Claude-powered assignment generation
      *evaluator.rs        # Claude-powered response evaluation
      *verifier.rs         # Backend answer verification (math, logic)
      *adaptation.rs       # Within-session and cross-session difficulty adaptation
    progress/
      mod.rs               # Progress persistence + badge eligibility checker
      tracker.rs           # LearnerProgress, SkillProgress, ZpdLevels, SpacedRepetition structs
      *spaced.rs           # Spaced repetition scheduling algorithm (SM-2)
      *gamification.rs     # Skill tree unlock, challenges, teach-back, streak shields
    session/
      mod.rs               # Session lifecycle management
      *markdown.rs         # Session markdown writer
      *buffer.rs           # Offline assignment buffer
    claude/
      mod.rs               # Claude API client
      *prompts.rs          # Prompt construction with context injection
      *schemas.rs          # Structured output schemas (serde types)
    dashboard/
      mod.rs               # Parent dashboard API routes
  data/
    learners/              # Per-learner directories (created at runtime, gitignored)
    curriculum/
      skill-tree.json      # Master skill/badge definitions
      assignment-templates/ # Assignment template JSON files
  tests/
    learner_profile_tests.rs  # Integration tests for learner CRUD
    progress_tests.rs         # Integration tests for progress + badge eligibility
```

## Key Architecture Rules

### 1. Bones & Soul

Every feature has two sides:
- **Bones**: the data schema, the API contract, the persistence format. These must be strict, validated, and tested.
- **Soul**: the adaptive, human-centered behavior. Claude's tone, the way feedback is phrased, the frustration detection. This must feel warm, never mechanical.

### 2. Claude Is the Creative Engine, Not the Source of Truth

- Claude generates assignments and evaluates responses — but the **backend verifies correctness**.
- Claude does not store state. Every API call includes fresh context from JSON files.
- **Separate generation from evaluation** — never use the same Claude call for both.
- All Claude responses use **structured JSON output** (serde-deserializable types), never freeform text.

### 3. Privacy: No Real Names

- The `name` field in `profile.json` is a **child-chosen display name** (any alias).
- The system never asks for or stores real names.
- The backend must **omit `id`, `learnerId`, UUIDs** from every Claude API call.
- Only pass: display name, age, interests, skill levels, ZPD, recent session summaries.

### 4. Data Model

All data structures must match the schemas defined in `CLAUDE.md`:
- `profile.json` — has `schemaVersion`, child-chosen `name`, `initialPreferences`, `observedBehavior`
- `progress.json` — has `schemaVersion`, per-skill ZPD (no stored `gap` — compute at runtime), `spacedRepetition` fields, `metacognition`
- Session markdown — written by the backend, not Claude. Claude provides narrative content as structured output.

### 5. ZPD Gap Is Computed, Never Stored

`gap = scaffoldedLevel - independentLevel` — always calculate at runtime. Storing it creates inconsistency risk.

## Coding Conventions

### Rust Style
- Use `thiserror` for custom error types, `anyhow` for application-level errors
- Prefer `Result<T, E>` over panics — never `unwrap()` in production code
- Use `serde::{Serialize, Deserialize}` for all data structures
- Use `#[serde(rename_all = "camelCase")]` to match JSON field naming in CLAUDE.md
- Derive `Clone, Debug` on all public types
- Use module-level `mod.rs` files that re-export public types
- Write doc comments (`///`) on all public functions and types
- Integration tests go in `tests/`, unit tests in `#[cfg(test)]` modules

### File I/O
- All file operations should be async (tokio::fs)
- Use proper error handling for missing files (a new learner won't have progress.json yet)
- Validate `schemaVersion` on read — return an error if it doesn't match expected version

### API Design
- REST API using Axum with JSON request/response bodies
- Consistent error responses: `{ "error": "message", "code": "ERROR_CODE" }`
- All business routes prefixed with `/api/v1/` — operational endpoints like `/health` are at the root
- Server reads `DATA_DIR` env var for data directory (default: `data`)
- Use `AppState` (in `main.rs`) with `Arc<PathBuf>` for the data directory, passed to handlers via Axum's `State` extractor

### Testing
- Every module should have unit tests for core logic
- Integration tests go in `tests/` (not `tests/integration/`) and use `tempfile::TempDir` for learner data
- Test both happy paths and error cases (missing files, invalid JSON, schema version mismatch)
- Use `cargo test` — no external test frameworks needed
- Import from `educational_companion::` (the lib crate) in integration tests

### Established Patterns (follow these in new modules)
- **Error types**: each module defines its own error enum via `thiserror` (e.g. `LearnerError`, `ProgressError`) with variants for NotFound, InvalidSchemaVersion, Io, Json
- **Persistence**: `read_*` validates schemaVersion, `write_*` enforces data invariants (e.g. ring buffer truncation)
- **Defaults**: behavioral enums default to `Unknown`, numeric fields to 0, optional fields to `None`
- **Schema version**: always validate `schemaVersion == 1` on read; return a typed error on mismatch
- **Badge eligibility**: use `check_new_badges()` from `progress` module with a `BadgeContext` for session-scoped conditions

## Build & Run

```bash
cargo build              # Build the project
cargo test               # Run all tests
cargo run                # Start the server
cargo clippy             # Lint
cargo fmt --check        # Check formatting
```

## Common Pitfalls

These will get your PR rejected. Each references a CONSTITUTION.md principle.

1. **Don't store ZPD gap** — compute it at runtime. (Constitution §7)
2. **Don't send UUIDs to Claude** — sanitize the profile before API calls. (Constitution §6)
3. **Don't let Claude decide correctness** — the backend verifies `correctAnswer` independently. (Constitution §5)
4. **Don't write session markdown from Claude output directly** — the backend assembles the file from structured data. (Constitution §5)
5. **Don't trust the client** — never accept `correctAnswer`, verification data, or assignment content from the client. Store verified assignments server-side; the client sends only an assignment ID. (Constitution §5)
6. **Don't use discouraging language** — no "Wrong", no "Incorrect", no comparisons. (Constitution §1, §2)
7. **Don't use VARK labels** — no "visual learner" or similar. Observe, don't label. (Constitution §3)
8. **Don't collect real names** — `name` is a child-chosen alias. (Constitution §4)
9. **Don't hardcode learning paths** — always adapt from observed behavioral data. (Constitution §10)
10. **Don't use `unwrap()` in production code** — use `?` or proper error handling.
11. **Don't use `//` comments in JSON files** — they're invalid JSON.
12. **Don't silently weaken verification** — if backend cannot verify a "full" level assignment, return `Unverifiable` and retry/fallback. Never downgrade to `PartiallyVerified` when the check couldn't actually run.

## Self-Review Checklist

Before marking your PR as ready, review your own code against these questions. Treat this as an adversarial analysis — actively try to break your implementation.

### Trust & Security
- [ ] Who is trusted in this flow? Who isn't? Where is the boundary?
- [ ] Can the client forge, replay, or manipulate any data that affects correctness?
- [ ] Does any endpoint accept data from the client that should only come from the server?
- [ ] Are correct answers, verification data, or internal state ever exposed to the client?

### Failure Semantics
- [ ] What happens when an external call fails (Claude API, file I/O, JSON parse)?
- [ ] What happens when data is missing, malformed, or has a wrong schema version?
- [ ] If a verification or check cannot be completed, does it fail safe (Unverifiable) or fail open (PartiallyVerified)?
- [ ] Are there any code paths where an error is silently swallowed?

### Concurrency & Data Integrity
- [ ] Do all route handlers that read learner data acquire a read lock via `state.locks.read(id)`?
- [ ] Do all route handlers that write learner data acquire a write lock via `state.locks.write(id)`?
- [ ] Is there any read-modify-write sequence that isn't protected by a single write lock?
- [ ] Could two concurrent requests corrupt state (e.g., both read, both modify, second write overwrites first)?

### Constitution Compliance
- [ ] Does any feedback text violate growth mindset principles? (Constitution §1, §2)
- [ ] Does any data sent to Claude contain UUIDs, learnerId, or schemaVersion? (Constitution §5, §6)
- [ ] Is ZPD gap stored anywhere, or always computed at runtime? (Constitution §7)
- [ ] Does the system degrade gracefully if Claude is unavailable? (Constitution §8)

### Data Invariants
- [ ] Is `recentAccuracy` ring buffer capped at 5 on every write path?
- [ ] Is `schemaVersion` validated on every file read?
- [ ] Are `observedBehavior` fields only modified by the session system, never by API input?
- [ ] Are all JSON field names camelCase and all behavioral enum values kebab-case?
