# Standard business tables: `buckyos.todo@1`, `buckyos.calendar@1`

The official to-do and calendar controls (`notepads/aiworkspace-control-mechanism-todo.md` G8–G10) read and write ordinary
TableSources marked with a `schema_id`. The core checks the business rules below at admission only (schema class, like the
other built-in business rules: replay and import never run them). The same definitions live in buckyos-websdk
`src/aiws_schedule.ts` (`TODO_SCHEMA`, `CALENDAR_SCHEMA`), which creates the tables; keep the two in step.

A table is standard when its entity's `schema_id` is one of the two ids. Its **standard fields** keep their ids and types:
creating such a table needs all of them (with these types; options of `status` may differ), `table.delete_field` refuses
them, `table.update_field` / `table.migrate_field` cannot change their type. Other fields are free (the user may add
columns). Rules on a standard field that a table lacks are skipped.

## `buckyos.todo@1`

| field_id | type | rule |
| --- | --- | --- |
| `title` | text, required | the table's `title_field_id` |
| `status` | select, required | the option list is the board's columns, in order; option `done` means completed (initial options `todo`, `doing`, `done`) |
| `due` | date, nullable | |
| `order` | text, nullable | an order key (`order_key::check_order_key`): the list / column order, one per record so concurrent moves of different cards do not conflict |
| `done_at` | datetime, nullable | set when completed |
| `ext` | json, nullable | extension fields, see below |

`ext` keys with a meaning (others are kept as they are): `priority` (`low` \| `normal` \| `high` \| `urgent`), `tags`
(≤ 32 strings of ≤ 64 characters), `assignee` (string ≤ 256), `reminders` (as on the calendar), `source` (object ≤ 4 KiB).

## `buckyos.calendar@1`

| field_id | type | rule |
| --- | --- | --- |
| `title` | text, required | the table's `title_field_id` |
| `all_day` | boolean, nullable | absent = false |
| `start` | datetime, nullable | a timed event's start; required when not `all_day`, absent when `all_day` |
| `end` | datetime, nullable | a timed event's end; not before `start`; absent when `all_day` |
| `start_date` | date, nullable | an all-day event's first day; required when `all_day`, absent otherwise |
| `end_date` | date, nullable | an all-day event's last day (inclusive); not before `start_date`; absent when not `all_day` |
| `status` | select, nullable | `confirmed`, `tentative`, `cancelled` |
| `ext` | json, nullable | extension fields, see below |

`ext` keys with a meaning (others are kept):

| key | shape |
| --- | --- |
| `rrule` | an iCalendar RRULE (RFC 5545) of the subset below, e.g. `FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR` |
| `exdate` | ≤ 1000 original starts excluded from the series: datetimes (timed) or dates (all-day), as the event's start is written |
| `recurrence_id` | `{ master: <record_id>, original: <datetime \| date> }`: this record overrides one occurrence of a series (“仅此一次”); never together with `rrule` |
| `tz` | the IANA time zone the series repeats in (`Asia/Shanghai`, `UTC`; ≤ 64 characters, `[A-Za-z0-9_+/-]`) |
| `location` | string ≤ 1024 |
| `color` | `#rrggbb` |
| `busy` | boolean (absent: a timed event is busy, an all-day one free) |
| `reminders` | ≤ 16 of `{ before_min: 0…40320, channel?: string ≤ 64 }` |
| `rule_text` | string ≤ 256: a rule the subset cannot express (“每月最后一个法定工作日”), shown as is |
| `occurrences` | ≤ 1000 starts (datetime or date) an Agent wrote for a `rule_text` series |
| `covered_until` | datetime or date: how far `occurrences` (or the reminders an Agent scheduled) reach |
| `maintenance` | object ≤ 4 KiB: `{ covered_until?, last_success_at?, last_error?, maintainer? }` (D4 §5.4) |

### The RRULE subset

Parts separated by `;`, each `NAME=VALUE`, every name at most once, upper case:

- `FREQ` (required): `DAILY` \| `WEEKLY` \| `MONTHLY` \| `YEARLY`
- `INTERVAL`: 1…999
- `COUNT`: 1…1000, or `UNTIL`: `YYYYMMDD` or `YYYYMMDDTHHMMSSZ` — not both
- `BYDAY`: ≤ 7 comma-separated `MO`…`SU`, each with an optional ordinal `-5`…`-1`, `1`…`5` (ordinals only with `MONTHLY` / `YEARLY`)
- `BYMONTHDAY`: ≤ 31 of `1`…`31`, `-1` (`MONTHLY` / `YEARLY` only)
- `BYMONTH`: ≤ 12 of `1`…`12` (`YEARLY` only)
- `BYSETPOS`: one of `-5`…`-1`, `1`…`5`, only with `BYDAY` and `MONTHLY` (“每月最后一个工作日”)
- `WKST`: `MO`…`SU`

Anything else (`BYHOUR`, `BYWEEKNO`, `RDATE`, secondly / minutely / hourly frequencies, lower case) is refused with a
message naming the part: rules outside the subset go to `rule_text` + `occurrences`, never silently simplified.
