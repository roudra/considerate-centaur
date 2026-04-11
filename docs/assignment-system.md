# Assignment System

## Multi-Modal Assignment Types

Children engage differently with different modalities. The assignment system supports multiple interaction types beyond text-only problems.

### Interaction Modalities

| Modality | Examples | Implementation |
|---|---|---|
| **Text** | "What comes next: 2, 4, 8, ?" | Standard text input/multiple choice |
| **Visual-Interactive** | Drag shapes into position, complete a pattern grid, rotate a shape to match | Canvas-based UI components with drag/drop |
| **Sequencing** | Arrange steps in order, sort items by rule | Drag-to-reorder list components |
| **Drawing** | Draw the missing shape, sketch a pattern continuation | Simple drawing canvas with shape tools |
| **Audio-Enhanced** | Spoken instructions for younger children, sound-pattern puzzles | Text-to-speech for prompts, audio playback for patterns |
| **Teach-Back** | "Explain to your friend why 16 comes next" | Free-text or voice input, evaluated for reasoning quality |

### Assignment Template with Modality

```json
{
  "type": "pattern-completion",
  "modality": "visual-interactive",
  "skill": "spatial-reasoning",
  "difficulty": 3,
  "theme": "space",
  "prompt": "The spaceship is flying in a pattern. Drag it to where it goes next!",
  "interactionType": "drag-drop",
  "visualAssets": ["grid-3x3", "spaceship-sprite"],
  "correctAnswer": {"position": [2, 2]},
  "hints": [
    "Look at the path the spaceship has taken so far...",
    "It's moving diagonally — down and to the right!",
    "Where would it land if it keeps going the same way?"
  ],
  "explanation": "The spaceship moves one square down and one square right each time. That's a diagonal pattern!"
}
```

### Age-Based Modality Weighting
- **Ages 5-6**: Heavy visual-interactive and audio-enhanced, minimal text input
- **Ages 7-8**: Mix of visual and text, introduce sequencing and drawing
- **Ages 9-10**: More text-based and teach-back, visual for spatial reasoning
- **Ages 11+**: Primarily text and teach-back, visual only for complex spatial problems

The system tracks which modalities produce higher engagement (longer time-on-task, higher accuracy) per learner and weights future assignments accordingly.

## Skill & Badge System

### Skill Tree Categories (Logic Focus)
- **Pattern Recognition**: sequences, visual patterns, analogies
- **Deductive Reasoning**: if-then logic, elimination, syllogisms
- **Sequential Logic**: ordering, step-by-step processes, algorithms
- **Spatial Reasoning**: shapes, rotations, maps, symmetry
- **Problem Decomposition**: breaking problems into parts
- **Critical Thinking**: evaluating claims, finding errors, cause-and-effect

### Badge Types
- **Milestone Badges**: first correct answer, first perfect session, 10 sessions completed
- **Skill Badges**: reach level 3/5/7/10 in any skill
- **Streak Badges**: 3-day, 7-day, 30-day learning streaks
- **Challenge Badges**: complete special challenge assignments
- **Explorer Badges**: try a new skill category for the first time

## Difficulty Adaptation Rules

Adaptation targets the **Zone of Proximal Development** — always working in the gap between what the child can do alone and what they can do with support.

### Within a Session
- After 3 consecutive correct answers at independent level → increase difficulty toward scaffolded level
- After 2 consecutive incorrect answers → decrease difficulty by 1, offer scaffolded hints
- After a wrong answer followed by a correct with hints → maintain level, gradually reduce hint detail
- If frustration signals detected (rapid wrong answers, long pauses, disengagement) → pivot to an easier "confidence builder" assignment, then return

### Across Sessions
- Weekly review: if average accuracy > 85% → push toward new skill areas or increase ZPD ceiling
- Weekly review: if average accuracy < 60% → reinforce fundamentals with varied problem formats
- If `independentLevel` catches up to `scaffoldedLevel` → the child has internalized the skill; raise both
- If `workingMemorySignal` is "overloaded" for a skill → reduce multi-step complexity, focus on single-concept problems

### Emotional Adaptation
- After session abandonment → next session starts with a familiar, confidence-building warm-up
- If `effortAttribution` trends toward "outcome-oriented" → Claude feedback shifts to emphasize process ("You tried three different approaches — that's real problem-solving!")
- If `frustrationResponse` is "disengages" → shorter sessions, more frequent badges, lower initial difficulty

## Verification Layers by Assignment Type

Not all assignments are equally hallucination-prone. Verify accordingly:

| Assignment Type | Risk Level | Verification Method |
|---|---|---|
| Arithmetic / sequences | Low | Backend computes answer independently |
| Pattern matching | Low | Predefined pattern banks; Claude selects and themes |
| If-then / elimination | Medium | Encode rules as constraints; verify conclusion follows from premises |
| Spatial reasoning | Medium | Use validated visual templates; Claude describes, doesn't create images |
| Free-form reasoning | High | Claude evaluates, backend flags low-confidence for parent review |
| Creative / open-ended | High | No single correct answer; evaluate for effort and reasoning, not correctness |

**Weight assignment mix toward verifiable types**, especially for new learners. Introduce higher-risk types gradually as the parent builds trust in the system.

## Assignment Templates

Templates bound what Claude can produce, reducing the hallucination surface.

`assignment-templates/sequence-puzzle.json`:
```json
{
  "type": "sequence-puzzle",
  "constraints": {
    "sequenceTypes": ["arithmetic", "geometric", "fibonacci-like"],
    "maxTerms": 6,
    "numberRange": [1, 100],
    "operations": ["add", "multiply", "power"]
  },
  "verificationLevel": "full",
  "verificationMethod": "compute-sequence"
}
```

`assignment-templates/deductive-reasoning.json`:
```json
{
  "type": "deductive-reasoning",
  "constraints": {
    "maxPremises": 3,
    "logicTypes": ["if-then", "elimination", "syllogism"],
    "domainVocabulary": "age-appropriate"
  },
  "verificationLevel": "partial",
  "verificationMethod": "rule-check"
}
```

Claude fills in templates creatively (theming, wording). The backend verifies the underlying logic is sound.
