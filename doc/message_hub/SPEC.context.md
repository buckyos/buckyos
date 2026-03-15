# Message Hub Specification

## Route Surface

### Implemented

- `/message-hub`
  - redirects to `/message-hub/today`
- `/message-hub/today`
  - landing page with a triage inbox, grouped lanes, suggested tasks, and agent queue preview
- `/message-hub/chat`
  - primary Message Hub chat page
- `/message-hub/people`
  - people graph seed view based on current contact list
- `/message-hub/tasks`
  - follow-up extraction and TODO workflow blueprint
- `/message-hub/agents`
  - agent inbox and RAW record blueprint

## API Surface

### Implemented

- `chat.bootstrap` on `/kapi/message-hub`
- `chat.contact.list` on `/kapi/message-hub`
- `chat.message.list` on `/kapi/message-hub`
- `chat.message.send` on `/kapi/message-hub`
- `POST /kapi/message-hub/chat/stream`

## Current Contract Notes

- The browser must use the Message Hub wrapper surface, not raw `/kapi/msg-center`.
- Session validation and owner DID derivation stay server-side.
- Realtime transport remains `fetch` + streamed NDJSON.
- The current realtime semantic is message-level record updates, not token-level LLM delta streaming.
- The current `Today` inbox is frontend-derived from contact freshness and capability state; it is not yet backed by a dedicated inbox-ranking API.

## Product Notes

- `Message Hub` is wider than chat as a product.
- The current implementation now includes a product shell and section navigation, but only the `Chat` section is deeply live with backend-backed interaction.
- `Today`, `People`, `Tasks`, and `Agents` are meaningful first-class product surfaces. `Today` now behaves like a real triage page, but it still derives its signals heuristically from existing contact/chat data instead of dedicated aggregation APIs.
- Email, calendar, notifications, TODO extraction, AI reply, and agent RAW communication ingestion are still planned expansions.
