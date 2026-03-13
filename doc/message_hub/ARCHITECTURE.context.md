# Message Hub Architecture

## Runtime Shape

- `Message Hub` is currently exposed as a dedicated route surface inside the main web SPA: `/message-hub`.
- The landing page is `/message-hub/today`.
- The current section routes are `/message-hub/today`, `/message-hub/chat`, `/message-hub/people`, `/message-hub/tasks`, and `/message-hub/agents`.
- `Today` is the product orchestration surface: it derives a first-pass inbox from current contact freshness, reply capability, and planned task/agent workflows.
- The frontend currently uses a browser-safe chat adapter mounted at `/kapi/message-hub` plus a streaming helper at `/kapi/message-hub/chat/stream`.
- In the current transition phase, those routes are served by the existing Rust `control_panel` service.

## Current Ownership Split

| Surface | Current owner | Role |
| --- | --- | --- |
| `/message-hub` | control-panel SPA | dedicated Message Hub route space |
| `/kapi/message-hub` | Rust `control_panel` service | browser-safe chat wrapper |
| `POST /kapi/message-hub/chat/stream` | Rust `control_panel` service | message-level realtime NDJSON stream |
| raw `/kapi/msg-center` | Rust `msg-center` service | low-level message service API |

## Transition Direction

- The product boundary now belongs to `Message Hub`, not `control_panel`.
- The current wrapper stays in place only to preserve auth, owner DID derivation, and browser safety during migration.
- Future work should move the browser-safe adapter from `control_panel` into `msg-center` or a thin sibling facade without changing the `/message-hub` product URL shape.

## Frontend Direction

- The UI remains in the same repository.
- The current implementation reuses the existing chat UI logic for the `Chat` section, then wraps it inside a dedicated Message Hub shell with section navigation and product-level overview cards.
- The current `Today` section is still heuristic and frontend-derived, but it already establishes the intended UX model for a compact high-signal inbox.
- `control_panel` keeps only a launch entry and should not remain the primary message shell.
