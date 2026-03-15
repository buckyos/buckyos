# Message Hub Documentation

## Purpose

- `doc/message_hub/` is the canonical documentation directory for `Message Hub`.
- `Message Hub` is the user-facing web product over BuckyOS messaging and communication aggregation.
- The current implementation is intentionally incremental: the first stable product shell is a multi-section Message Hub app under `/message-hub`, with realtime direct messaging still serving as the deepest implemented workflow.

## Product Positioning

- `Message Hub` is the AI-era information center for people and agents.
- It starts from chat, then expands toward a unified inbox for `Messages`, `Email`, `Calendar`, `Notifications`, `TODO`, and agent communication records.
- It should feel like an active communication workspace, not a narrow transport debugger.

## Current Scope

### Implemented

- Dedicated route surface under `/message-hub`.
- Landing page at `/message-hub/today`.
- Section routes for `Today`, `Chat`, `People`, `Tasks`, and `Agents`.
- `Today` now acts as a triage inbox with grouped lanes for reply candidates, follow-up candidates, and agent signals.
- Realtime direct message list, contact list, send flow, and NDJSON stream.
- Launch entry from the desktop shell in `control_panel`.

### Planned

- Email aggregation.
- Calendar and schedule integration.
- Notification center unification.
- TODO capture from communication context.
- AI reply, spam filtering, and cross-channel person merge.
- Agent-to-agent RAW communication record view.

## Current Runtime Note

- The product surface is now `Message Hub`, but the browser-safe adapter still runs through the existing control-panel service in the current implementation phase.
- This is a transition detail, not the intended long-term ownership boundary.

## Canonical Files

- `doc/message_hub/README.context.md`
- `doc/message_hub/ARCHITECTURE.context.md`
- `doc/message_hub/SPEC.context.md`
- `doc/message_hub/CONTEXT.context.md`
