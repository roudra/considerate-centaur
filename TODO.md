# Educational Companion App — Pending Work

## CLAUDE.md Rewrite — COMPLETED

### Batch 1: Structural Fixes & Foundation
- [x] Add Project Quick Reference section with hallucination risk table (from Copilot)
- [x] Update tech stack from Kotlin/Gradle to Rust
- [x] Update directory structure for Rust project layout
- [x] Fix invalid `//` JSON comments — move to field notes below blocks (from Copilot)
- [x] Add `schemaVersion: 1` to profile.json and progress.json schemas (from Copilot)
- [x] Remove redundant `gap` field from ZPD — compute at runtime (from Copilot)
- [x] Update `name` field: child chooses their own display name, any alias accepted, system never asks for real name
- [x] Fix session markdown authorship contradiction: Claude generates narrative, backend writes the file (from Copilot)
- [x] Update privacy rule: backend must omit `id`, `learnerId`, UUIDs from Claude API calls (from Copilot)
- [x] Remove old sync-service build commands from Development Guidelines

### Batch 2: New Learning Features
- [x] Add Onboarding & First Session section — calibration session that feels like play, seeds ZPD baselines across all skill areas, no cold-start problem
- [x] Add Multi-Modal Assignment Types — visual puzzles (drag/drop shapes, draw patterns), interactive sequences, audio cues, not just text-heavy problems
- [x] Add Spaced Repetition System — SM-2 algorithm for scheduling review assignments, prevent skill decay, track intervals per skill in progress.json

### Batch 3: Experience & Safety
- [x] Add Parent-Child Shared Sessions — parents can join a session, co-solve challenge problems, system observes collaborative dynamic, supports "collaborative" challenge preference
- [x] Add Offline / Resilience Architecture — pre-generated assignment buffer from templates, graceful degradation when Claude API is down/slow, child never sees a loading screen
- [x] Add Privacy & Safety Architecture — full data flow diagram (what's stored where, what leaves the device), COPPA considerations, no real names collected, local-first data storage
- [x] Add Deeper Gamification — skill tree visualization with unlock paths, challenge modes (timed puzzles, boss battles), teach-back moments where child explains a concept (deepest form of learning)

### Other Completed
- [x] Remove unrelated sync-service code (old MySQL-to-MongoDB Java service)

## Next Steps
- [x] Initialize Rust project structure (`cargo init`)
- [ ] Implement learner profile management
- [ ] Implement Claude API integration layer
- [ ] Build assignment generation pipeline
- [ ] Build session management & persistence
- [ ] Build parent dashboard
- [ ] Build child-facing learning UI
