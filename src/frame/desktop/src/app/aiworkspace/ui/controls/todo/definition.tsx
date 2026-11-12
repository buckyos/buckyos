/* The official to-do control (控件机制 TODO §0.2, Q1, Q2, Q20): a code-registered data control showing a standard to-do
 * table (`buckyos.todo@1`). Which table is settled by on_bind from its contract (控件 on_bind TODO §2.5; data/binding.ts).
 * Its body is the list / board. */

import { ListChecks } from 'lucide-react'
import { TODO_CONTROL } from '@controls/schedule'
import { solveBox } from '@controls/solve'
import { aiwsT } from '../../../i18n'
import { controlKey } from '../data/keys'
import { DataFlow } from '../data/DataFlow'
import type { ContainerDefinition } from '../registry'
import { ITEMS_BINDING, TODO_KIND } from './model'
import { TodoBody } from './TodoBody'

export const todoControl: ContainerDefinition = {
  kind: TODO_KIND, version: 1, get title() { return aiwsT('aiworkspace.todo.title') }, icon: ListChecks,
  configSchema: {
    type: 'object',
    properties: {
      mode: { enum: ['list', 'board'] }, hide_done: { type: 'boolean' },
      collapsed: { type: 'array', maxItems: 64, items: { type: 'string' } },
    },
  },
  children: { accepts: [], containers: false },
  sizing: 'box',
  defaultSize: { w: 420, h: 520 },
  layout: { direction: 'bottom_up', algorithm: 'fixed', solve: solveBox },
  Body: TodoBody,
  Flow: ({ containerId, mode }) => <DataFlow containerId={containerId} mode={mode} height={480} />,
  // its table is settled by on_bind (the contract's `items`: none → created, one → shown, several → asked)
  create: (args) => [
    { op: 'entity.create', entity_id: args.cellId, type_id: 'buckyos.container', parent_id: args.parentId, order_key: args.orderKey,
      placement: { x: Math.round(args.placement.x), y: Math.round(args.placement.y), w: Math.round(args.placement.w), h: Math.round(args.placement.h) },
      payload: { kind: TODO_KIND, layout: { mode: 'data' }, title: args.title || aiwsT('aiworkspace.todo.title'), config: { mode: 'list' },
        ...(args.existingSourceId ? { bindings: { [ITEMS_BINDING]: { entity_id: args.existingSourceId } } } : {}) } },
  ] as never,
  keys: (ctx, event) => controlKey(ctx.containerId, event),
  agent: TODO_CONTROL,
}
