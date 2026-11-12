/* The official calendar control (控件机制 TODO §0.1, Q1, Q20): a code-registered data control showing standard calendar
 * tables (`events`, `buckyos.calendar@1`) and the due dates of to-do tables (`todo`, `buckyos.todo@1`), both lists. Which
 * tables is settled by on_bind from its contract (控件 on_bind TODO §2.5; data/binding.ts); “Tables shown…” re-binds. */

import { CalendarDays } from 'lucide-react'
import { CALENDAR_CONTROL } from '@controls/schedule'
import { solveBox } from '@controls/solve'
import { aiwsT } from '../../../i18n'
import { controlKey } from '../data/keys'
import { DataFlow } from '../data/DataFlow'
import type { ContainerDefinition } from '../registry'
import { CalendarBody } from './CalendarBody'
import { CALENDAR_KIND, EVENTS_BINDING } from './model'

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
  Flow: ({ containerId, mode }) => <DataFlow containerId={containerId} mode={mode} height={560} />,
  // its tables are settled by on_bind (the contract's `events`: none → created, one → shown, several → asked; `todo`: all)
  create: (args) => [
    { op: 'entity.create', entity_id: args.cellId, type_id: 'buckyos.container', parent_id: args.parentId, order_key: args.orderKey,
      placement: { x: Math.round(args.placement.x), y: Math.round(args.placement.y), w: Math.round(args.placement.w), h: Math.round(args.placement.h) },
      payload: { kind: CALENDAR_KIND, layout: { mode: 'data' }, title: args.title || aiwsT('aiworkspace.calendar.title'),
        ...(args.existingSourceId ? { bindings: { [EVENTS_BINDING]: [{ entity_id: args.existingSourceId }] } } : {}) } },
  ] as never,
  keys: (ctx, event) => controlKey(ctx.containerId, event),
  agent: CALENDAR_CONTROL,
}
