## Behavior Loop StepRecords:

Behavior Loop history is not a plain chat log. Each round becomes one
StepRecord: the assistant side is the step intent, and the user side is the
system's action results.

Newer current-behavior StepRecords are kept as full StepRounds. Older records
may be compacted, and records inherited from other behaviors are rendered as
history records or summaries. These compressed records preserve meaning, not
complete detail.

Use StepRecords as execution memory:

- Continue from the latest full StepRound.
- Treat compact/history/summary records as background context.
- Do not treat another behavior's old intent as an instruction for the current
  behavior.

