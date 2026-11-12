/* The public contracts of the official data controls (控件机制 TODO G13): the to-do control and the calendar declared in
 * the shape of an extension's `component.json` — identity, version, properties (the instance's defaults; which of them
 * each person keeps for themself, G6), bindings (standard tables), the templates items are projected with, the
 * layouts, what a press or a drag on a template's role writes, the menu actions and the self-description an Agent
 * reads. Generated into `aiws/contracts/<kind>.json` (tools/build_catalog.ts), returned by `control.describe`. What a
 * composite extension can declare of the same mechanisms is composition.json's manifest 4 (`projections`, events from
 * a projection's role, `media`, a slot's `visible`); the board's columns, the time grid and pop-overs stay the host's. */

import { CALENDAR_CONTROL, AGENDA_TEMPLATE, BAR_TEMPLATE, BLOCK_TEMPLATE, CARD_TEMPLATE, LIST_TEMPLATE, TODO_CONTROL } from './schedule.ts'

/** Raised when a contract changes in a way a reader must know (a template's roles, a gesture's write, a property, what a
 * binding name means). */
export const CONTRACT_VERSION = { todo: 1, calendar: 2 }
const AUTHOR = 'did:bns:buckyos'

export function todoContract(): Record<string, unknown> {
  return {
    manifest_version: 4,
    ident: { author_did: AUTHOR, name: 'todo' },
    kind: 'builtin',
    version: CONTRACT_VERSION.todo,
    title: 'To-dos',
    description: TODO_CONTROL.summary,
    default_size: { w: 420, h: 520 },
    config_schema: {
      type: 'object',
      properties: {
        mode: { enum: ['list', 'board'], default: 'list', title: 'View' },
        hide_done: { type: 'boolean', default: false, title: 'Hide completed' },
        collapsed: { type: 'array', items: { type: 'string' }, title: 'Folded columns (status options)' },
      },
    },
    /** The properties each person keeps as their own (user state `block:<id>/view`); the instance's are the default. */
    user_state: ['mode', 'hide_done', 'collapsed'],
    bindings: {
      items: { accepts: ['buckyos.table-source'], schema_id: 'buckyos.todo@1', required: true, title: 'To-do table', owned: 'created with the control' },
    },
    templates: { list_row: LIST_TEMPLATE, card: CARD_TEMPLATE },
    layouts: {
      list: { algorithm: 'list', template: 'list_row', item: { h: 38 }, gap: 2, order: 'order', groups: 'open, then completed (folded)' },
      board: { algorithm: 'columns', template: 'card', by: 'status', column: { w: 260 }, gap: 8, order: 'order' },
    },
    gestures: [
      { role: 'check', press: 'complete or reopen', writes: 'table.set_values status (done / the first other option), done_at' },
      { role: 'title', edit: 'title', writes: 'table.set_values title' },
      { role: 'body', press: 'select; a second press opens the details', drag: 'reorder (list, column) or move to another column', writes: 'table.set_values order (an order key between the neighbours), status' },
    ],
    events: {},
    actions: [
      { id: 'add', title: 'Add a to-do', writes: 'table.insert_records { title, status, order }' },
      { id: 'add_column', title: 'Add a column', needs: ['structure'], writes: 'table.add_option on status' },
      { id: 'copy_link', title: 'Copy link', link: '?block=<control>&record=<record>' },
      { id: 'annotate', title: 'Annotate', anchor: { kind: 'table_record' } },
      { id: 'open_page', title: 'Open as a page', link: '?block=<control>&page=control' },
      { id: 'delete', title: 'Delete', writes: 'table.delete_records' },
    ],
    agent: TODO_CONTROL.agent,
  }
}

export function calendarContract(): Record<string, unknown> {
  return {
    manifest_version: 4,
    ident: { author_did: AUTHOR, name: 'calendar' },
    kind: 'builtin',
    version: CONTRACT_VERSION.calendar,
    title: 'Calendar',
    description: CALENDAR_CONTROL.summary,
    default_size: { w: 860, h: 620 },
    config_schema: {
      type: 'object',
      properties: {
        view: { enum: ['day', 'week', 'month', 'agenda'], default: 'week', title: 'View', description: 'agenda on a phone unless chosen' },
        week_start: { type: 'integer', minimum: 0, maximum: 6, title: 'First day of the week (0 = Monday … 6 = Sunday; default by language)' },
        day_start: { type: 'integer', minimum: 0, maximum: 23, default: 8, title: 'First hour shown' },
        show_todo: { type: 'boolean', default: true, title: 'Show due to-dos' },
        colors: { type: 'object', title: 'Colour per bound table, by its entity id (#rrggbb)' },
        hidden: { type: 'array', items: { type: 'string' }, title: 'Tables not shown (entity ids)' },
      },
    },
    user_state: ['view', 'date', 'week_start', 'day_start', 'show_todo', 'hidden'],
    bindings: {
      events: { accepts: ['buckyos.table-source'], schema_id: 'buckyos.calendar@1', required: true, many: true, title: 'Calendars', owned: 'created with the control' },
      todo: { accepts: ['buckyos.table-source'], schema_id: 'buckyos.todo@1', many: true, title: 'To-do lists whose due dates are shown (read only)' },
    },
    templates: { bar: BAR_TEMPLATE, block: BLOCK_TEMPLATE, agenda_row: AGENDA_TEMPLATE },
    layouts: {
      month: { algorithm: 'month', template: 'bar', lanes: 'per week, "+N more" beyond the cell', order: 'all-day and long first, then by start' },
      week: { algorithm: 'time_grid', template: 'block', days: 7, all_day_row: 'bar', overlap: 'side by side, widening to the right' },
      day: { algorithm: 'time_grid', template: 'block', days: 1, all_day_row: 'bar' },
      agenda: { algorithm: 'agenda', template: 'agenda_row', group: 'by day' },
    },
    /** What a recurring event's write asks first: this occurrence, this and the following, or all (RFC 5545 semantics). */
    recurrence_scopes: ['one', 'following', 'all'],
    gestures: [
      { role: 'body', press: 'select; a second press opens the details', drag: 'move (15 minutes, across days; Alt copies)', writes: 'table.set_values start, end (or start_date, end_date) in one Commit; a recurring event asks the scope' },
      { role: 'resize-end', drag: 'change the end', writes: 'table.set_values end' },
      { role: 'background', press: 'create in place', drag: 'create over the dragged time', writes: 'table.insert_records { title, start, end }' },
    ],
    events: {},
    actions: [
      { id: 'skip', title: 'Skip this occurrence', writes: 'table.set_values ext.exdate (or a cancelled override)' },
      { id: 'duplicate', title: 'Duplicate', writes: 'table.insert_records' },
      { id: 'copy_link', title: 'Copy link', link: '?block=<control>&record=<record>&occ=<original start>' },
      { id: 'annotate', title: 'Annotate', anchor: { kind: 'table_record' } },
      { id: 'delete', title: 'Delete', writes: 'table.delete_records (a recurring event asks the scope)' },
    ],
    agent: CALENDAR_CONTROL.agent,
  }
}

export const CONTRACTS: Record<string, () => Record<string, unknown>> = { todo: todoContract, calendar: calendarContract }

export function contractJson(kind: string): string {
  return JSON.stringify(CONTRACTS[kind](), null, 2) + '\n'
}
