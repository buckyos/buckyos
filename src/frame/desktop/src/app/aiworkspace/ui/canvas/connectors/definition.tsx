/* eslint-disable react-refresh/only-export-components -- a definition, not a component module */
/* The connector's Block definition (连接线实现方案 §9.1; 标准对象的交互改进 §5.2, §6): what the registry knows of
 * it — the catalog entry (the object toolbar turns it into the connector tool), the near toolbar's type section
 * (route, caps, dash, width, colour, label, reset route), actions and inspector fields. The canvas host draws
 * and edits the line itself; outside a free canvas a connector is not shown. */

import { ArrowLeft, ArrowRight, CornerDownRight, Minus, RotateCcw, Slash, Spline, Type } from 'lucide-react'
import type { CellPayload, KeyedContent } from '../../../api/types'
import type { WorkspaceStore } from '../../../state/store'
import { createOp, setCellConfig } from '../../blocks/ops'
import type { BlockDefinition, RenderContext, ToolbarItem } from '../../blocks/registry'
import { connectorData, connectorStyle, CAPS, CONNECTOR_VIEW, DASHES, ROUTES, WIDTHS, type Route } from './model'
import { routeOps } from './ops'

const ROUTE_ICONS = { straight: Slash, elbow: CornerDownRight, curve: Spline }

function ConnectorStatic() {
  return <div className="aiws-muted" data-testid="aiws-connector-static">连接线只在自由画布上显示</div>
}

const canChange = (context: RenderContext): string | false => (context.capabilities.includes('update') && context.mode === 'edit' && context.readOnlyReason === null ? false : '没有修改此连接线的权限')

async function setRoute(context: RenderContext, store: WorkspaceStore, route: Route) {
  const id = context.cell.entity_id
  const read = await store.session.read<KeyedContent<CellPayload>>(id)
  const data = connectorData(read.content.payload as unknown as Record<string, unknown>)
  if (data.route === route) return
  // a straight self loop is not allowed (§4.3): such a line keeps its route
  if (route === 'straight' && data.start && data.end && data.start.entity_id === data.end.entity_id) { store.notify('info', '首尾连在同一个对象上的连接线不能是直线。'); return }
  void store.submit({ editId: `key:${id}:route`, label: `连接线：${ROUTES.find((r) => r.value === route)?.label ?? route}`, operations: routeOps(id, route, data.controls.length > 0, read.content.key_revs ?? {}) })
}

function resetRoute(context: RenderContext, store: WorkspaceStore) {
  const id = context.cell.entity_id
  void store.submit({ editId: `key:${id}:controls`, label: '连接线：重置路由', operations: [{ op: 'entity.unset_keys', entity_id: id, keys: [{ key: 'controls', expect: { rev: context.keyRevs.controls ?? 0 } }] }] })
}

function connectorToolbar(context: RenderContext, store: WorkspaceStore): ToolbarItem[] {
  // while the label is edited the editor's own tools (size, colour) are shown instead
  if (context.editorActive) return []
  const data = connectorData(context.payload as unknown as Record<string, unknown>)
  const style = connectorStyle(context.payload.config)
  const reason = canChange(context)
  const config = (patch: Record<string, string | number | null>, label: string) => setCellConfig(context, store, patch, label)
  return [
    { kind: 'menu', id: 'line-route', icon: ROUTE_ICONS[data.route], label: '路由', value: data.route, disabled: reason,
      items: ROUTES.map((r) => ({ ...r, icon: ROUTE_ICONS[r.value] })), onPick: (value) => { void setRoute(context, store, value as Route) } },
    { kind: 'menu', id: 'line-start-cap', icon: ArrowLeft, label: '起点线帽', value: style.startCap, disabled: reason, items: CAPS,
      onPick: (value) => config({ start_cap: value === 'none' ? null : value }, `起点线帽 → ${CAPS.find((c) => c.value === value)?.label ?? value}`) },
    { kind: 'menu', id: 'line-end-cap', icon: ArrowRight, label: '终点线帽', value: style.endCap, disabled: reason, items: CAPS,
      onPick: (value) => config({ end_cap: value === 'arrow' ? null : value }, `终点线帽 → ${CAPS.find((c) => c.value === value)?.label ?? value}`) },
    { kind: 'menu', id: 'line-dash', icon: Minus, label: '线型', value: style.dash, disabled: reason, items: DASHES,
      onPick: (value) => config({ dash: value === 'solid' ? null : value }, `线型 → ${DASHES.find((d) => d.value === value)?.label ?? value}`) },
    { kind: 'menu', id: 'line-width', label: '粗细', value: WIDTHS.find((w) => Number(w.value) === style.width)?.value, disabled: reason, items: WIDTHS,
      onPick: (value) => config({ width: Number(value) === 2 ? null : Number(value) }, `粗细 → ${WIDTHS.find((w) => w.value === value)?.label ?? value}`) },
    { kind: 'color', id: 'line-color', label: '颜色', palette: 'ink', value: style.stroke, disabled: reason,
      onPick: (value) => config({ stroke: value || null }, '连接线颜色') },
    { kind: 'button', id: 'line-label', icon: Type, label: '标签', key: 'Enter', disabled: reason, run: () => context.activateEditor() },
    { kind: 'button', id: 'line-reset', icon: RotateCcw, label: '重置路由', disabled: reason || (data.controls.length ? false : '已是自动路由'), run: () => resetRoute(context, store) },
  ]
}

export const connectorDefinition: BlockDefinition = {
  type: CONNECTOR_VIEW, version: 1, title: '连接线', accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 200, h: 0 }, cost: { editor: false, html: false },
  Static: ConnectorStatic, chrome: 'none',
  catalog: { group: 'layout', description: '连接两个对象的线，可带箭头和文字。选中对象后从边上的蓝点拖出，或用这个工具在画布上拖画（L）。', needs: 'none', standard: true },
  actions: [{ id: 'label', label: '编辑标签', modes: ['edit'], needs: ['update'], run: (context) => context.activateEditor(), key: 'Enter' }],
  toolbar: connectorToolbar,
  configFields: [
    { key: 'stroke', label: '颜色', kind: 'color' },
    { key: 'width', label: '粗细（0.5–24）', kind: 'number' },
    { key: 'dash', label: '线型', kind: 'select', options: DASHES },
    { key: 'start_cap', label: '起点线帽', kind: 'select', options: CAPS },
    { key: 'end_cap', label: '终点线帽', kind: 'select', options: CAPS },
    { key: 'rounded', label: '直角圆角', kind: 'boolean' },
    { key: 'label_size', label: '标签字号', kind: 'select', options: [{ value: 's', label: '小' }, { value: 'm', label: '中' }, { value: 'l', label: '大' }] },
  ],
  // "insert" from a menu: a horizontal line from the spot (the object toolbar starts the connector tool instead)
  create: (args) => [createOp(args.cellId, 'buckyos.cell', args.parentId, args.orderKey, { view: { type: CONNECTOR_VIEW, version: 1 }, start: null, end: null }, undefined, { ...args.placement, h: 0, w: Math.max(40, args.placement.w) })],
}

