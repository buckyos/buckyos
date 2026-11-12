/* The official calendar control (控件机制 TODO §0.1, Q1, Q20): a code-registered data control showing standard calendar
 * tables (`buckyos.calendar@1`). Inserted, it binds as `events` the workspace's only calendar table, asks which one
 * among several, and creates one in the canvas's content folder when there is none (data/source.ts), and shows the due
 * dates of every to-do table of the workspace (`todo`, `todo_2` …); its menu adds further calendars and to-do tables. */

import { CalendarDays } from 'lucide-react'
import { CALENDAR_SCHEMA_ID, standardTableOp, TODO_SCHEMA_ID } from 'buckyos'
import { CALENDAR_CONTROL } from '@controls/schedule'
import { solveBox } from '@controls/solve'
import { randomId } from '../../../api/ids'
import { aiwsT, currentAiwsLocale } from '../../../i18n'
import { controlKey } from '../data/keys'
import { DataFlow } from '../data/DataFlow'
import { freeBindingName, freeTableName, sourceTables } from '../data/source'
import type { ContainerDefinition } from '../registry'
import { CalendarBody } from './CalendarBody'
import { CALENDAR_KIND, EVENTS_BINDING, TODO_BINDING } from './model'

export const calendarControl: ContainerDefinition = {
  kind: CALENDAR_KIND, version: 1, get title() { return aiwsT('aiworkspace.calendar.title') }, icon: CalendarDays,
  configSchema: {
    type: 'object',
    properties: {
      view: { enum: ['day', 'week', 'month', 'agenda'] }, week_start: { type: 'integer', minimum: 0, maximum: 6 }, day_start: { type: 'integer', minimum: 0, maximum: 23 },
      show_todo: { type: 'boolean' }, colors: { type: 'object' }, hidden: { type: 'array', items: { type: 'string' } },
    },
  },
  children: { accepts: [], containers: false },
  sizing: 'box',
  defaultSize: { w: 860, h: 620 },
  layout: { direction: 'bottom_up', algorithm: 'fixed', solve: solveBox },
  Body: CalendarBody,
  source: { binding: EVENTS_BINDING, schemaId: CALENDAR_SCHEMA_ID, table: 'calendar' },
  Flow: ({ containerId, mode }) => <DataFlow containerId={containerId} mode={mode} height={560} />,
  create: (args) => {
    const containerId = args.cellId
    const tableId = args.existingSourceId ?? args.dataId ?? randomId('t')
    const locale = currentAiwsLocale().startsWith('zh') ? 'zh' : 'en'
    const title = args.title || aiwsT('aiworkspace.calendar.title')
    // every to-do table of the workspace shows its due dates at once (`todo`, `todo_2` …); the menu stops showing one
    const todos: Record<string, { entity_id: string }> = {}
    for (const table of sourceTables(args.store, TODO_SCHEMA_ID)) todos[freeBindingName(todos, TODO_BINDING)] = { entity_id: table.entity_id }
    return [
      ...(args.existingSourceId ? [] : [standardTableOp('calendar', { entity_id: tableId, parent_id: args.contentFolderId, order_key: args.dataOrderKey, name: freeTableName(args.store, args.contentFolderId, title), locale })]),
      { op: 'entity.create', entity_id: containerId, type_id: 'buckyos.container', parent_id: args.parentId, order_key: args.orderKey,
        placement: { x: Math.round(args.placement.x), y: Math.round(args.placement.y), w: Math.round(args.placement.w), h: Math.round(args.placement.h) },
        payload: { kind: CALENDAR_KIND, layout: { mode: 'data' }, title, bindings: { [EVENTS_BINDING]: { entity_id: tableId }, ...todos } } },
    ] as never
  },
  keys: (ctx, event) => controlKey(ctx.containerId, event),
  agent: CALENDAR_CONTROL,
}
