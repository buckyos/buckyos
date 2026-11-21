---
name: owner
description: Use the signed-in owner's personal calendar, to-dos, contacts, notifications and answers. Find a person's shared availability and reuse its stable Object URL.
requires: a verified Zone account or a bound Agent session and installed personal resource aliases
baseline: standard
commands: xcall
scope: ui, work
---

Use the complete local alias that matches the request. Each object declares its own actions and inputs in its Profile. For a known action, invoke it directly. Store a fresh idempotency key before sending a write and reuse the same key and parameters when retrying it.

```sh
xcall owner.cal add --params '{"title":"Design review","start":"2026-11-20T14:00:00+08:00","end":"2026-11-20T14:30:00+08:00"}' --idempotency-key review-1120
xcall owner.cal list --params '{"from":"2026-11-20T00:00:00+08:00","to":"2026-11-21T00:00:00+08:00","tz":"Asia/Shanghai"}'
xcall owner.todo add --params '{"title":"Send the review notes","due":"2026-11-20"}'
xcall owner notify --params '{"text":"Your review notes are ready."}'
xcall owner ask --params '{"text":"Should the review go ahead on Friday?"}'
```

- Calendar windows include the start and exclude the end. Use offset-bearing instants. For all-day events use `start_date` and inclusive `end_date`. Resolve “Friday” in the owner's known time zone. Ask for the calendar date and time zone when unclear. For changes to a recurring event keep the original occurrence reference and choose the requested scope.
- Use `owner.todo list` with `include_done=true` to find completed items. Use `done` and `undone` for the human's to-do list. Work-session plan steps are managed with `agent-session todo`.
- Read `detail.result`. Give the person the record's `open_url` and retain `record_ref` for later edits. Pass the original reference and a `patch` to the appropriate update action. A workspace ID or item title alone cannot identify the original provider. For a pending question retain the `notify_id` and use `owner notification_status` to check for the answer.
- `xcall owner.contacts find name=Lin` can return several people. Ask the owner to choose when necessary. Use the selected person's DID and visible published URL. Aliases like `owner.cal` resolve in the current user's context.
- Given a visible availability URL and the known action, use one `xcall "$availability_ref" free_busy --params '{"from":"2026-11-20T13:00:00+08:00","to":"2026-11-20T17:00:00+08:00"}'`. Report free time only for complete coverage. Use `detail.error` to distinguish denied access, unavailable resources and failed writes. On `WRITE_OUTCOME_UNKNOWN`, retain the original key and report the uncertainty.

### Remembering a shared availability URL

When the owner explicitly asks in private to remember how to schedule with Lin, use that conversation's `agent-session`. Set `person_did`, `zone_did` and `availability_ref` from the selected contact's `did`, `zone_did` and `availability_ref`. If the contact has no visible availability URL, skip the note.

```sh
agent-session ctl perceive "Use $availability_ref in Zone $zone_did for Lin ($person_did) when scheduling meetings with free_busy." --object "$availability_ref" --tag availability --suggest fact --explicit
```

Memory stamps the observation time and the conversation's subjects. Keep Lin's DID in the text so the owner's private note retains that visibility. On recall, use a Zone-local `obj://` URL in its recorded Zone. The availability provider checks the caller's current sharing grant on each call.
