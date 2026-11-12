/* The official data controls (控件机制 TODO §0.1, §0.2, G13): the to-do list / board and the calendar. They show the
 * records of standard tables (`buckyos.todo@1`, `buckyos.calendar@1`; schemas/standard-tables.md) as projected Blocks
 * — nothing about an item is stored in the control. What every reader shares: their catalogue entries and the
 * self-descriptions an Agent reads; the Desktop's ContainerDefinitions add the canvas behaviour. */

import type { ControlDescriptor } from './catalog.ts'
import type { Template } from './projection.ts'

const TODO_AGENT = `# todo — the to-do control (list and board)

A \`buckyos.container\` with \`kind: "todo"\` on a canvas. It shows one to-do table (a \`buckyos.table-source\` with
\`schema_id: "buckyos.todo@1"\`) bound as \`bindings.items\`. **The items are the table's records**: to add, complete,
move or rename a to-do, write the table (\`table.insert_records\` / \`table.set_values\`), never Blocks — the control
shows the change at once. Nothing is stored per item in the control.

## The table (fields by id)

| field | type | meaning |
| --- | --- | --- |
| \`title\` | text, required | the to-do |
| \`status\` | select, required | a column of the board, in option order; option \`done\` means completed (default options \`todo\`, \`doing\`, \`done\`) |
| \`due\` | date | due date |
| \`order\` | text | an order key (base-36 fractional index, as \`order_key\`): list and column order |
| \`done_at\` | datetime | when it was completed |
| \`ext\` | json object | \`priority\` (low / normal / high / urgent), \`tags\` (strings), \`assignee\`, \`reminders\` |

A record's body (\`table.set_body\`, a rich text under the table) holds its notes.

## The control

\`payload: { kind: "todo", layout: { mode: "data" }, title?, bindings: { items: { entity_id: <table> } }, config? }\` with a
numeric \`placement { x, y, w, h }\` (it holds no Blocks). \`config\` is the default view: \`mode\` (\`list\` | \`board\`),
\`hide_done\` (boolean), \`collapsed\` (status option ids of folded columns). Each person's own view is their user state.

## Create one with doc.commit

\`\`\`json
{ "op": "entity.create", "entity_id": "todo1", "type_id": "buckyos.table-source", "schema_id": "buckyos.todo@1", "parent_id": "<content folder>",
  "order_key": "a", "payload": { "title_field_id": "title", "fields": [
    { "field_id": "title", "name": "Title", "type": "text", "required": true },
    { "field_id": "status", "name": "Status", "type": "select", "required": true, "options": [{ "option_id": "todo", "label": "To do" }, { "option_id": "doing", "label": "In progress" }, { "option_id": "done", "label": "Done" }] },
    { "field_id": "due", "name": "Due", "type": "date", "nullable": true }, { "field_id": "order", "name": "Order", "type": "text", "nullable": true },
    { "field_id": "done_at", "name": "Completed", "type": "datetime", "nullable": true }, { "field_id": "ext", "name": "Extension", "type": "json", "nullable": true }] } }
{ "op": "entity.create", "entity_id": "todoctl1", "type_id": "buckyos.container", "parent_id": "<surface>", "order_key": "b",
  "placement": { "x": 40, "y": 40, "w": 420, "h": 560 }, "payload": { "kind": "todo", "layout": { "mode": "data" }, "bindings": { "items": { "entity_id": "todo1" } } } }
{ "op": "table.insert_records", "source_id": "todo1", "records": [{ "record_id": "t1", "values": { "title": "Buy milk", "status": "todo", "order": "i" } }] }
\`\`\`

The placement is numbers (never \`"auto"\`): a data control is sized like a Block.

## The private schedule

Every person has a Workspace named \`schedule\` (\`ws.ensure { name: "schedule" }\`) whose to-do table has the fixed id
\`todo\` (and calendar \`calendar\`). Use the \`buckyos aiws todo\` commands (or buckyos-websdk \`ScheduleClient\`) to
read and change it.
`

const CALENDAR_AGENT = `# calendar — the calendar control

A \`buckyos.container\` with \`kind: "calendar"\`. It shows calendar tables (\`schema_id: "buckyos.calendar@1"\`) bound
as \`bindings.events\` (and more bindings for more calendars; a \`buckyos.todo@1\` table bound as \`bindings.todo\` shows
due dates). **Events are the table's records**: write the table, never Blocks.

## The table (fields by id)

| field | type | meaning |
| --- | --- | --- |
| \`title\` | text, required | |
| \`all_day\` | boolean | an all-day event (absent: timed) |
| \`start\` / \`end\` | datetime (RFC 3339) | a timed event; \`end\` not before \`start\` |
| \`start_date\` / \`end_date\` | date | an all-day event, \`end_date\` inclusive |
| \`status\` | select | confirmed / tentative / cancelled |
| \`ext\` | json object | \`rrule\` (iCalendar RRULE subset), \`exdate\` (excluded original starts), \`recurrence_id\` \`{ master, original }\` (overrides one occurrence), \`tz\` (IANA zone the series repeats in), \`location\`, \`color\`, \`reminders\`, \`rule_text\` + \`occurrences\` + \`covered_until\` (rules outside the subset, occurrences an Agent wrote), \`maintenance\` |

Repeating: \`FREQ=DAILY|WEEKLY|MONTHLY|YEARLY\`, \`INTERVAL\`, \`COUNT\` or \`UNTIL\`, \`BYDAY\` (ordinals with monthly /
yearly: \`-1FR\` = last Friday), \`BYMONTHDAY\`, \`BYMONTH\`, \`BYSETPOS\`, \`WKST\`. Every weekday:
\`FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR\`. An occurrence's id is \`<record_id>@<original start>\`. "Only this one" writes an
override record; "skip today" adds the original to \`ext.exdate\`.

## The control

\`payload: { kind: "calendar", layout: { mode: "data" }, title?, bindings: { events: { entity_id } }, config? }\` with a
numeric placement. \`config\`: \`view\` (\`day\` | \`week\` | \`month\` | \`agenda\`), \`week_start\` (0 Monday … 6 Sunday),
\`day_start\` (first hour shown), \`colors\` (\`{ <binding>: "#rrggbb" }\`).

## Create one with doc.commit

\`\`\`json
{ "op": "entity.create", "entity_id": "calctl1", "type_id": "buckyos.container", "parent_id": "<surface>", "order_key": "c",
  "placement": { "x": 40, "y": 40, "w": 880, "h": 640 }, "payload": { "kind": "calendar", "layout": { "mode": "data" }, "bindings": { "events": { "entity_id": "<calendar table>" } }, "config": { "view": "week" } } }
{ "op": "table.insert_records", "source_id": "<calendar table>", "records": [
  { "record_id": "e1", "values": { "title": "Stand-up", "start": "2026-11-16T01:00:00Z", "end": "2026-11-16T01:15:00Z", "ext": { "rrule": "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "tz": "Asia/Shanghai" } } },
  { "record_id": "e2", "values": { "title": "Trip", "all_day": true, "start_date": "2026-11-20", "end_date": "2026-11-22" } }] }
\`\`\`

The placement is numbers (never \`"auto"\`): a data control is sized like a Block.

Use \`buckyos aiws cal\` (or buckyos-websdk \`ScheduleClient\`) to read and change the private schedule.
`

export const TODO_CONTROL: ControlDescriptor = {
  kind: 'todo',
  title: 'To-do',
  summary: 'A to-do list that is also a board: the records of a to-do table, ticked, reordered and moved between status columns.',
  when: 'To show and work through a to-do table (buckyos.todo@1) on a canvas: a checklist, a task board.',
  formats: [],
  agent: TODO_AGENT,
}

export const CALENDAR_CONTROL: ControlDescriptor = {
  kind: 'calendar',
  title: 'Calendar',
  summary: 'A day / week / month / agenda calendar of the events in calendar tables, repeating events included.',
  when: 'To show and edit events of a calendar table (buckyos.calendar@1) on a canvas: meetings, plans, a team calendar.',
  formats: [],
  agent: CALENDAR_AGENT,
}

// ---- the templates their items are projected with (label, shape; Q1) — part of their public contracts (contracts.ts)

/** The check box (a circle) and its tick, drawn as svg shapes. */
const CHECK_SVG = { viewBox: '0 0 20 20', body: '<circle cx="10" cy="10" r="8.5" style="fill:var(--aiws-shape-fill);stroke:var(--aiws-shape-stroke);stroke-width:1.5"/>' }
const TICK_SVG = { viewBox: '0 0 20 20', body: '<path d="M6 10.4l2.6 2.6L14 7.6" style="fill:none;stroke:var(--aiws-shape-stroke);stroke-width:2;stroke-linecap:round;stroke-linejoin:round"/>' }

/** A list row: background, check box, title (edited in place), status, due date. */
export const LIST_TEMPLATE: Template = {
  nodes: [
    { id: 'bg', view: 'shape', role: 'body', config: { shape: 'rect', radius: 6, fill: '{{item.row_fill}}', stroke: '{{item.row_stroke}}', stroke_width: 1 } },
    { id: 'check', view: 'shape', role: 'check', box: { x: 8, y: 9, w: 20, h: 20 }, config: { shape: 'svg', svg: CHECK_SVG, fill: '{{item.check_fill}}', stroke: '{{item.check_stroke}}' } },
    { id: 'tick', view: 'shape', role: 'check', when: '{{item.done}}', box: { x: 8, y: 9, w: 20, h: 20 }, config: { shape: 'svg', svg: TICK_SVG, fill: 'none', stroke: '#ffffff' } },
    { id: 'prio', view: 'shape', role: 'body', when: '{{item.priority_color}}', box: { x: 34, y: 15, w: 4, h: 8 }, config: { shape: 'rect', radius: 2, fill: '{{item.priority_color}}', stroke: 'none' } },
    { id: 'title', view: 'label', role: 'title', edit: 'title', box: { x: 42, r: 104 }, config: { value: '{{item.title}}', font_size: 14, font_color: '{{item.title_color}}', strike: '{{item.done}}' } },
    { id: 'status', view: 'label', role: 'body', when: '{{item.doing}}', box: { w: 72, r: 100 }, config: { value: '{{item.status_label}}', font_size: 11, font_color: 'accent', align: 'right' } },
    { id: 'due', view: 'label', role: 'body', box: { w: 88, r: 8 }, config: { value: '{{item.due}}', format: 'relative', font_size: 12, font_color: '{{item.due_color}}', align: 'right' } },
    { id: 'note', view: 'shape', role: 'body', when: '{{item.noted}}', box: { x: 26, y: 8, w: 7, h: 7 }, config: { shape: 'ellipse', fill: 'warning', stroke: 'surface', stroke_width: 1.5 } },
  ],
}

/** A board card: background, title (wrapping), check box, due date, priority bar, tags. */
export const CARD_TEMPLATE: Template = {
  nodes: [
    { id: 'bg', view: 'shape', role: 'body', config: { shape: 'rect', radius: 8, fill: 'surface', stroke: '{{item.card_stroke}}', stroke_width: 1 } },
    { id: 'prio', view: 'shape', role: 'body', when: '{{item.priority_color}}', box: { x: 0, y: 8, w: 3, b: 8 }, config: { shape: 'rect', radius: 1.5, fill: '{{item.priority_color}}', stroke: 'none' } },
    { id: 'check', view: 'shape', role: 'check', box: { x: 10, y: 10, w: 18, h: 18 }, config: { shape: 'svg', svg: CHECK_SVG, fill: '{{item.check_fill}}', stroke: '{{item.check_stroke}}' } },
    { id: 'tick', view: 'shape', role: 'check', when: '{{item.done}}', box: { x: 10, y: 10, w: 18, h: 18 }, config: { shape: 'svg', svg: TICK_SVG, fill: 'none', stroke: '#ffffff' } },
    { id: 'title', view: 'label', role: 'title', edit: 'title', box: { x: 36, y: 9, r: 10, b: 6 }, config: { value: '{{item.title}}', font_size: 13, font_color: '{{item.title_color}}', strike: '{{item.done}}', wrap: true, valign: 'top' } },
    { id: 'due', view: 'label', role: 'body', when: '{{item.due}}', box: { x: 36, h: 18, b: 6, w: 110 }, config: { value: '{{item.due}}', format: 'relative', font_size: 11, font_color: '{{item.due_color}}' } },
    { id: 'tags', view: 'label', role: 'body', when: '{{item.tags}}', box: { r: 10, h: 18, b: 6, w: 110 }, config: { value: '{{item.tags}}', font_size: 11, font_color: 'muted', align: 'right' } },
    { id: 'note', view: 'shape', role: 'body', when: '{{item.noted}}', box: { r: 6, y: 6, w: 7, h: 7 }, config: { shape: 'ellipse', fill: 'warning', stroke: 'none' } },
  ],
}

/** A bar of the month grid / the all-day row: a filled strip with the title (a timed one starts with its time). */
export const BAR_TEMPLATE: Template = {
  nodes: [
    { id: 'bg', view: 'shape', role: 'body', config: { shape: 'rect', radius: 4, fill: '{{item.fill}}', stroke: '{{item.stroke}}', stroke_width: 1, opacity: '{{item.opacity}}' } },
    { id: 'dot', view: 'shape', role: 'body', when: '{{item.dot}}', box: { x: 5, y: 6, w: 7, h: 7 }, config: { shape: 'ellipse', fill: '{{item.color}}', stroke: 'none' } },
    { id: 'title', view: 'label', role: 'body', box: { x: '{{item.pad}}', r: 4 }, config: { value: '{{item.text}}', font_size: 12, font_color: '{{item.ink}}', weight: 'medium', strike: '{{item.cancelled}}' } },
  ],
}

/** A timed event in the time grid: a block with the title and its time; its bottom edge resizes it. */
export const BLOCK_TEMPLATE: Template = {
  nodes: [
    { id: 'bg', view: 'shape', role: 'body', config: { shape: 'rect', radius: 6, fill: '{{item.fill}}', stroke: '{{item.stroke}}', stroke_width: 1, opacity: '{{item.opacity}}' } },
    { id: 'edge', view: 'shape', role: 'body', box: { x: 0, y: 3, w: 3, b: 3 }, config: { shape: 'rect', radius: 1.5, fill: '{{item.color}}', stroke: 'none' } },
    { id: 'title', view: 'label', role: 'body', box: { x: 8, y: 2, r: 4, h: '{{item.title_h}}' }, config: { value: '{{item.title}}', font_size: 12, font_color: '{{item.ink}}', weight: 'medium', wrap: '{{item.wrap}}', valign: 'top', strike: '{{item.cancelled}}' } },
    { id: 'time', view: 'label', role: 'body', when: '{{item.show_time}}', box: { x: 8, y: '{{item.time_y}}', r: 4, h: 16 }, config: { value: '{{item.time}}', font_size: 11, font_color: '{{item.ink2}}' } },
    { id: 'resize', view: 'shape', role: 'resize-end', when: '{{item.resizable}}', box: { x: 0, b: 0, r: 0, h: 6 }, config: { shape: 'rect', fill: 'none', stroke: 'none' } },
  ],
}

/** An agenda row: the time, a colour dot and the title. */
export const AGENDA_TEMPLATE: Template = {
  nodes: [
    { id: 'bg', view: 'shape', role: 'body', config: { shape: 'rect', radius: 6, fill: '{{item.row_fill}}', stroke: 'none' } },
    { id: 'time', view: 'label', role: 'body', box: { x: 8, w: 96 }, config: { value: '{{item.time}}', font_size: 12, font_color: 'muted' } },
    { id: 'dot', view: 'shape', role: 'body', box: { x: 108, y: '50%', w: 9, h: 9 }, config: { shape: 'ellipse', fill: '{{item.color}}', stroke: 'none' } },
    { id: 'title', view: 'label', role: 'body', box: { x: 126, r: 8 }, config: { value: '{{item.title}}', font_size: 14, font_color: '{{item.title_ink}}', strike: '{{item.cancelled}}' } },
  ],
}
