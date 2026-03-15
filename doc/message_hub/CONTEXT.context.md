# Message Hub Context

## Naming Rules

- Product name: `Message Hub`.
- Service/module name: `msg-center` / `msg_center`.
- Current route prefix: `/message-hub`.
- Current landing page: `/message-hub/today`.
- Current Today model: frontend-derived triage lanes over existing chat/contact data plus product blueprints for tasks and agents.

## Non-Obvious Facts

- The product surface and the current backend owner are temporarily different.
- `Message Hub` is already the user-facing route and launch target.
- The current browser-safe chat adapter still reuses logic hosted by the Rust `control_panel` service.
- Raw `/kapi/msg-center` remains lower-level and should not be exposed directly to the browser UI as the primary app contract.

## Safe Change Rules

- Preserve `/message-hub` as the stable product URL prefix during migration.
- Keep auth and owner DID derivation on the server side.
- Treat chat as the first implemented slice of a broader inbox product, not the permanent full definition of Message Hub.
