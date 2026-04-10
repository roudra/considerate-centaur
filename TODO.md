# Educational Companion App — Pending Work

## CLAUDE.md Rewrite

### Batch 1: Structural Fixes & Foundation
- [ ] Add Project Quick Reference section with hallucination risk table (from Copilot)
- [ ] Update tech stack from Kotlin/Gradle to Rust
- [ ] Update directory structure for Rust project layout
- [ ] Fix invalid `//` JSON comments — move to field notes below blocks (from Copilot)
- [ ] Add `schemaVersion: 1` to profile.json and progress.json schemas (from Copilot)
- [ ] Remove redundant `gap` field from ZPD — compute at runtime (from Copilot)
- [ ] Update `name` field: child chooses their own display name, any alias accepted, system never asks for real name
- [ ] Fix session markdown authorship contradiction: Claude generates narrative, backend writes the file (from Copilot)
- [ ] Update privacy rule: backend must omit `id`, `learnerId`, UUIDs from Claude API calls (from Copilot)
- [ ] Remove old sync-service build commands from Development Guidelines

### Batch 2: New Learning Features
- [ ] Add Onboarding & First Session section — calibration session that feels like play, seeds ZPD baselines across all skill areas, no cold-start problem
- [ ] Add Multi-Modal Assignment Types — visual puzzles (drag/drop shapes, draw patterns), interactive sequences, audio cues, not just text-heavy problems
- [ ] Add Spaced Repetition System — SM-2 algorithm for scheduling review assignments, prevent skill decay, track intervals per skill in progress.json

### Batch 3: Experience & Safety
- [ ] Add Parent-Child Shared Sessions — parents can join a session, co-solve challenge problems, system observes collaborative dynamic, supports "collaborative" challenge preference
- [ ] Add Offline / Resilience Architecture — pre-generated assignment buffer from templates, graceful degradation when Claude API is down/slow, child never sees a loading screen
- [ ] Add Privacy & Safety Architecture — full data flow diagram (what's stored where, what leaves the device), COPPA considerations, no real names collected, local-first data storage
- [ ] Add Deeper Gamification — skill tree visualization with unlock paths, challenge modes (timed puzzles, boss battles), teach-back moments where child explains a concept (deepest form of learning)

## GitHub Issues to Create
Once GitHub MCP reconnects, create one issue per item above for tracking.
