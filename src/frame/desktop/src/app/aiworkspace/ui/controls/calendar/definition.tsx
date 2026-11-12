/* The official calendar control (控件机制 TODO §0.1, Q1, Q20): a code-registered data control. Inserting one creates a
 * standard calendar table (`buckyos.calendar@1`) in the canvas's content folder — owned by the control — and the
 * control bound to it as `events`; further calendars and a to-do table (`todo`, its due dates) can be bound later. */

import { CalendarDays } from 'lucide-react'
import { standardTableOp } from 'buckyos'
import { CALENDAR_CONTROL } from '@controls/schedule'
import { solveBox } from '@controls/solve'
import { randomId } from '../../../api/ids'
import { aiwsT, currentAiwsLocale } from '../../../i18n'
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
  create: (args) => {
    const containerId = args.cellId
    const tableId = args.existingSourceId ?? args.dataId ?? randomId('t')
    const locale = currentAiwsLocale().startsWith('zh') ? 'zh' : 'en'
    const title = args.title || aiwsT('aiworkspace.calendar.title')
    return [
      ...(args.existingSourceId ? [] : [standardTableOp('calendar', { entity_id: tableId, parent_id: args.contentFolderId, order_key: args.dataOrderKey, name: title, locale })]),
      { op: 'entity.create', entity_id: containerId, type_id: 'buckyos.container', parent_id: args.parentId, order_key: args.orderKey,
        placement: { x: Math.round(args.placement.x), y: Math.round(args.placement.y), w: Math.round(args.placement.w), h: Math.round(args.placement.h) },
        payload: { kind: CALENDAR_KIND, layout: { mode: 'data' }, title, bindings: { [EVENTS_BINDING]: { entity_id: tableId } } } },
    ] as never
  },
  keys: (ctx, event) => controlKey(ctx.containerId, event),
  agent: CALENDAR_CONTROL,
}
