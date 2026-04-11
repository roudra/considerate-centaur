# Onboarding & First Session

The first session has no behavioral data — no ZPD baselines, no observed frustration patterns, no accuracy history. A cold start risks assignments that are too hard (discouraging) or too easy (boring). The onboarding session must feel like **play, not a test**.

## First Session Flow

1. **Choose Your Name** — the child picks their display name. Can be anything: "DragonMaster", "Luna", "Captain Cheese". This is their identity in the system.
2. **Interest Discovery** — quick, visual selection of topics they like (dinosaurs, space, animals, robots, art, music, sports, etc.). Presented as icons/images, not a text form.
3. **Calibration Puzzles** — a series of 8-10 short, varied puzzles across skill categories. Each starts at low difficulty and adapts up/down within the puzzle:
   - 2 pattern recognition (number sequences, visual patterns)
   - 2 sequential logic (ordering steps, simple algorithms)
   - 2 spatial reasoning (shape matching, rotation)
   - 2 deductive reasoning (simple if-then, elimination)
4. **Baseline Seeding** — from the calibration results, the backend populates initial ZPD values for each skill:
   - Highest difficulty solved without help → `independentLevel`
   - Highest difficulty solved with a hint → `scaffoldedLevel`
   - Skills not attempted → left at defaults (independentLevel: 1, scaffoldedLevel: 2)
5. **Welcome Badge** — the child earns their first badge ("Explorer" or similar) just for completing onboarding. Immediate positive reinforcement.

## Design Principles for Onboarding

- No scores shown — the child should not feel evaluated
- Every puzzle has a "skip" option — no forced frustration
- Calibration puzzles are themed around the interests the child just selected
- The session is shorter than a normal session (10-15 minutes)
- The tone is exploratory: "Let's see what kinds of puzzles you like!"
