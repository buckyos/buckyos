/* eslint-disable react-refresh/only-export-components -- a definition, not a component module */
/* The connector's Block definition (连接线实现方案 §9.1; 标准对象的交互改进 §5.2, §6): what the registry knows of
 * it — the catalog entry (the object toolbar turns it into the connector tool), the near toolbar's type section
 * (route, caps, dash, width, colour, label, reset route), actions and inspector fields. The canvas host draws
 * and edits the line itself; outside a free canvas a connector is not shown. */

import { ArrowLeft, ArrowRight, CornerDownRight, Minus, RotateCcw, Slash, Spline, Type } from 'lucide-react'
import type { CellPayload, KeyedContent } from '../../../api/types'
import { aiwsT } from '../../../i18n'
import type { WorkspaceStore } from '../../../state/store'
import { useAiwsText } from '../../../useAiwsText'
import { createOp, setCellConfig } from '../../blocks/ops'
import type { BlockDefinition, RenderContext, ToolbarItem } from '../../blocks/registry'
import { connectorData, connectorStyle, CAPS, CONNECTOR_VIEW, DASHES, LABEL_SIZES, ROUTES, WIDTHS, type Route } from './model'
import { routeOps } from './ops'

const ROUTE_ICONS = { straight: Slash, elbow: CornerDownRight, curve: Spline }

function ConnectorStatic() {
  const { t } = useAiwsText()
  return <div className="aiws-muted" data-testid="aiws-connector-static">{t('aiworkspace.connector.staticNote')}</div>
}

const canChange = (context: RenderContext): string | false => (context.capabilities.includes('update') && context.mode === 'edit' && context.readOnlyReason === null ? false : aiwsT('aiworkspace.connector.noPermission'))
/** "<what> → <value>": the label of a look change in the edit history. */
const changed = (label: string, value: string) => aiwsT('aiworkspace.blocks.edit.change', { label, value })

async function setRoute(context: RenderContext, store: WorkspaceStore, route: Route) {
  const id = context.cell.entity_id
  const read = await store.session.read<KeyedContent<CellPayload>>(id)
  const data = connectorData(read.content.payload as unknown as Record<string, unknown>)
  if (data.route === route) return
  // a straight self loop is not allowed (§4.3): such a line keeps its route
  if (route === 'straight' && data.start && data.end && data.start.entity_id === data.end.entity_id) { store.notify('info', aiwsT('aiworkspace.connector.straightSelfLoop')); return }
  void store.submit({ editId: `key:${id}:route`, label: aiwsT('aiworkspace.connector.edit.route', { route: ROUTES.find((r) => r.value === route)?.label ?? route }), operations: routeOps(id, route, data.controls.length > 0, read.content.key_revs ?? {}) })
}

function resetRoute(context: RenderContext, store: WorkspaceStore) {
  const id = context.cell.entity_id
  void store.submit({ editId: `key:${id}:controls`, label: aiwsT('aiworkspace.connector.edit.resetRoute'), operations: [{ op: 'entity.unset_keys', entity_id: id, keys: [{ key: 'controls', expect: { rev: context.keyRevs.controls ?? 0 } }] }] })
}

function connectorToolbar(context: RenderContext, store: WorkspaceStore): ToolbarItem[] {
  // while the label is edited the editor's own tools (size, colour) are shown instead
  if (context.editorActive) return []
  const data = connectorData(context.payload as unknown as Record<string, unknown>)
  const style = connectorStyle(context.payload.config)
  const reason = canChange(context)
  const config = (patch: Record<string, string | number | null>, label: string) => setCellConfig(context, store, patch, label)
  return [
    { kind: 'menu', id: 'line-route', icon: ROUTE_ICONS[data.route], label: aiwsT('aiworkspace.connector.route'), value: data.route, disabled: reason,
      items: ROUTES.map((r) => ({ ...r, icon: ROUTE_ICONS[r.value] })), onPick: (value) => { void setRoute(context, store, value as Route) } },
    { kind: 'menu', id: 'line-start-cap', icon: ArrowLeft, label: aiwsT('aiworkspace.connector.startCap'), value: style.startCap, disabled: reason, items: CAPS,
      onPick: (value) => config({ start_cap: value === 'none' ? null : value }, changed(aiwsT('aiworkspace.connector.startCap'), CAPS.find((c) => c.value === value)?.label ?? value)) },
    { kind: 'menu', id: 'line-end-cap', icon: ArrowRight, label: aiwsT('aiworkspace.connector.endCap'), value: style.endCap, disabled: reason, items: CAPS,
      onPick: (value) => config({ end_cap: value === 'arrow' ? null : value }, changed(aiwsT('aiworkspace.connector.endCap'), CAPS.find((c) => c.value === value)?.label ?? value)) },
    { kind: 'menu', id: 'line-dash', icon: Minus, label: aiwsT('aiworkspace.blocks.lineStyle'), value: style.dash, disabled: reason, items: DASHES,
      onPick: (value) => config({ dash: value === 'solid' ? null : value }, changed(aiwsT('aiworkspace.blocks.lineStyle'), DASHES.find((d) => d.value === value)?.label ?? value)) },
    { kind: 'menu', id: 'line-width', label: aiwsT('aiworkspace.connector.width'), value: WIDTHS.find((w) => Number(w.value) === style.width)?.value, disabled: reason, items: WIDTHS,
      onPick: (value) => config({ width: Number(value) === 2 ? null : Number(value) }, changed(aiwsT('aiworkspace.connector.width'), WIDTHS.find((w) => w.value === value)?.label ?? value)) },
    { kind: 'color', id: 'line-color', label: aiwsT('aiworkspace.blocks.color'), palette: 'ink', value: style.stroke, disabled: reason,
      onPick: (value) => config({ stroke: value || null }, aiwsT('aiworkspace.connector.color')) },
    { kind: 'button', id: 'line-label', icon: Type, label: aiwsT('aiworkspace.connector.label'), key: 'Enter', disabled: reason, run: () => context.activateEditor() },
    { kind: 'button', id: 'line-reset', icon: RotateCcw, label: aiwsT('aiworkspace.connector.resetRoute'), disabled: reason || (data.controls.length ? false : aiwsT('aiworkspace.connector.autoRouted')), run: () => resetRoute(context, store) },
  ]
}

export const connectorDefinition: BlockDefinition = {
  type: CONNECTOR_VIEW, version: 1, get title() { return aiwsT('aiworkspace.connector.title') }, accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 200, h: 0 }, cost: { editor: false, html: false },
  Static: ConnectorStatic, chrome: 'none',
  catalog: { group: 'layout', get description() { return aiwsT('aiworkspace.connector.description') }, needs: 'none', standard: true },
  actions: [{ id: 'label', get label() { return aiwsT('aiworkspace.connector.editLabel') }, modes: ['edit'], needs: ['update'], run: (context) => context.activateEditor(), key: 'Enter' }],
  toolbar: connectorToolbar,
  configFields: [
    { key: 'stroke', get label() { return aiwsT('aiworkspace.blocks.color') }, kind: 'color' },
    { key: 'width', get label() { return aiwsT('aiworkspace.connector.widthField') }, kind: 'number' },
    { key: 'dash', get label() { return aiwsT('aiworkspace.blocks.lineStyle') }, kind: 'select', options: DASHES },
    { key: 'start_cap', get label() { return aiwsT('aiworkspace.connector.startCap') }, kind: 'select', options: CAPS },
    { key: 'end_cap', get label() { return aiwsT('aiworkspace.connector.endCap') }, kind: 'select', options: CAPS },
    { key: 'rounded', get label() { return aiwsT('aiworkspace.connector.rounded') }, kind: 'boolean' },
    { key: 'label_size', get label() { return aiwsT('aiworkspace.connector.labelSize') }, kind: 'select', options: LABEL_SIZES },
  ],
  // "insert" from a menu: a horizontal line from the spot (the object toolbar starts the connector tool instead)
  create: (args) => [createOp(args.cellId, 'buckyos.cell', args.parentId, args.orderKey, { view: { type: CONNECTOR_VIEW, version: 1 }, start: null, end: null }, undefined, { ...args.placement, h: 0, w: Math.max(40, args.placement.w) })],
}

