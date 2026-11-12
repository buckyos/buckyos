/* The official to-do control (控件机制 TODO §0.2, Q1, Q2, Q20): a code-registered data control showing a standard to-do
 * table (`buckyos.todo@1`). Inserted, it shows the workspace's only to-do table, asks which one among several, and
 * creates one in the canvas's content folder when there is none (data/source.tsx). Its body is the list / board. */

import { ListChecks } from 'lucide-react'
import { standardTableOp, TODO_SCHEMA_ID } from 'buckyos'
import { TODO_CONTROL } from '@controls/schedule'
import { solveBox } from '@controls/solve'
import { randomId } from '../../../api/ids'
import { aiwsT, currentAiwsLocale } from '../../../i18n'
import { controlKey } from '../data/keys'
import { DataFlow } from '../data/DataFlow'
import { freeTableName } from '../data/source'
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
  source: { binding: ITEMS_BINDING, schemaId: TODO_SCHEMA_ID, table: 'todo' },
  Flow: ({ containerId, mode }) => <DataFlow containerId={containerId} mode={mode} height={480} />,
  create: (args) => {
    const containerId = args.cellId
    const tableId = args.existingSourceId ?? args.dataId ?? randomId('t')
    const locale = currentAiwsLocale().startsWith('zh') ? 'zh' : 'en'
    const title = args.title || aiwsT('aiworkspace.todo.title')
    return [
      ...(args.existingSourceId ? [] : [standardTableOp('todo', { entity_id: tableId, parent_id: args.contentFolderId, order_key: args.dataOrderKey, name: freeTableName(args.store, args.contentFolderId, title), locale })]),
      { op: 'entity.create', entity_id: containerId, type_id: 'buckyos.container', parent_id: args.parentId, order_key: args.orderKey,
        placement: { x: Math.round(args.placement.x), y: Math.round(args.placement.y), w: Math.round(args.placement.w), h: Math.round(args.placement.h) },
        payload: { kind: TODO_KIND, layout: { mode: 'data' }, title, bindings: { [ITEMS_BINDING]: { entity_id: tableId } }, config: { mode: 'list' } } },
    ] as never
  },
  keys: (ctx, event) => controlKey(ctx.containerId, event),
  agent: TODO_CONTROL,
}
