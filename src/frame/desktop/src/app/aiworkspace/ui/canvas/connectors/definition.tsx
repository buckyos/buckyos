/* eslint-disable react-refresh/only-export-components -- a definition, not a component module */
/* The connector's Block definition (连接线实现方案 §9.1; 标准对象的交互改进 §5.2, §6; UI 框架 PRD §10.1): what the registry
 * knows of it — the catalog entry (the object toolbar turns it into the connector tool), its actions (edit the label,
 * reset the route) and its properties (route, caps, dash, width, colour as quick ones on the Block toolbar; all of them
 * in the properties panel). The canvas host draws and edits the line itself; outside a free canvas a connector is not
 * shown. */

import { ArrowLeft, ArrowRight, CornerDownRight, Minus, RotateCcw, Slash, Spline, Type } from 'lucide-react'
import type { CellPayload, KeyedContent } from '../../../api/types'
import { aiwsT } from '../../../i18n'
import type { WorkspaceStore } from '../../../state/store'
import { useAiwsText } from '../../../useAiwsText'
import { createOp } from '../../blocks/ops'
import type { BlockDefinition, RenderContext } from '../../blocks/registry'
import { connectorData, connectorStyle, CAPS, CONNECTOR_VIEW, DASHES, LABEL_SIZES, ROUTES, WIDTHS, type Route } from './model'
import { routeOps } from './ops'

const ROUTE_ICONS = { straight: Slash, elbow: CornerDownRight, curve: Spline }

function ConnectorStatic() {
  const { t } = useAiwsText()
  return <div className="aiws-muted" data-testid="aiws-connector-static">{t('aiworkspace.connector.staticNote')}</div>
}

const style = (context: RenderContext) => connectorStyle(context.payload.config)
const data = (context: RenderContext) => connectorData(context.payload as unknown as Record<string, unknown>)

/** The route: its keys (and the bends dropped), read again so the write follows what is stored now. */
async function routeChange(context: RenderContext, store: WorkspaceStore, route: Route) {
  const id = context.cell.entity_id
  const read = await store.session.read<KeyedContent<CellPayload>>(id)
  const stored = connectorData(read.content.payload as unknown as Record<string, unknown>)
  if (stored.route === route) return []
  // a straight self loop is not allowed (§4.3): such a line keeps its route
  if (route === 'straight' && stored.start && stored.end && stored.start.entity_id === stored.end.entity_id) { store.notify('info', aiwsT('aiworkspace.connector.straightSelfLoop')); return [] }
  return routeOps(id, route, stored.controls.length > 0, read.content.key_revs ?? {})
}

function resetRoute(context: RenderContext, store: WorkspaceStore) {
  const id = context.cell.entity_id
  void store.submit({ editId: `key:${id}:controls`, label: aiwsT('aiworkspace.connector.edit.resetRoute'), operations: [{ op: 'entity.unset_keys', entity_id: id, keys: [{ key: 'controls', expect: { rev: context.keyRevs.controls ?? 0 } }] }] })
}

export const connectorDefinition: BlockDefinition = {
  type: CONNECTOR_VIEW, version: 1, get title() { return aiwsT('aiworkspace.connector.title') }, accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 200, h: 0 }, cost: { editor: false, html: false },
  Static: ConnectorStatic, chrome: 'none',
  catalog: { group: 'layout', get description() { return aiwsT('aiworkspace.connector.description') }, needs: 'none', standard: true, toolPicker: 7 },
  // a flow page shows no lines (连接线实现方案 §8.3): its ＋ does not offer one (UI 框架 PRD §10.1)
  traits: { flowInsertable: false },
  actions: [
    { id: 'line-label', icon: Type, primary: true, get label() { return aiwsT('aiworkspace.connector.editLabel') }, modes: ['edit'], needs: ['update'], when: (context) => !context.editorActive,
      run: (context) => context.activateEditor(), key: 'Enter' },
    { id: 'line-reset', icon: RotateCcw, primary: true, get label() { return aiwsT('aiworkspace.connector.resetRoute') }, modes: ['edit'], needs: ['update'],
      disabled: (context) => (data(context).controls.length ? false : aiwsT('aiworkspace.connector.autoRouted')), run: resetRoute },
  ],
  // the look: the quick ones are the Block toolbar's line tools
  configFields: [
    { key: 'route', id: 'line-route', get label() { return aiwsT('aiworkspace.connector.route') }, kind: 'select', quick: true, default: 'straight',
      get options() { return ROUTES.map((r) => ({ value: r.value, label: r.label, icon: ROUTE_ICONS[r.value] })) }, get: (context) => data(context).route,
      ops: (context, store, value) => routeChange(context, store, (value ?? 'straight') as Route),
      editLabel: (value) => aiwsT('aiworkspace.connector.edit.route', { route: ROUTES.find((r) => r.value === value)?.label ?? String(value) }) },
    { key: 'start_cap', id: 'line-start-cap', get label() { return aiwsT('aiworkspace.connector.startCap') }, kind: 'select', quick: true, icon: ArrowLeft, default: 'none', options: CAPS, get: (context) => style(context).startCap },
    { key: 'end_cap', id: 'line-end-cap', get label() { return aiwsT('aiworkspace.connector.endCap') }, kind: 'select', quick: true, icon: ArrowRight, default: 'arrow', options: CAPS, get: (context) => style(context).endCap },
    { key: 'dash', id: 'line-dash', get label() { return aiwsT('aiworkspace.blocks.lineStyle') }, kind: 'select', quick: true, icon: Minus, default: 'solid', options: DASHES, get: (context) => style(context).dash },
    { key: 'width', id: 'line-width', get label() { return aiwsT('aiworkspace.connector.width') }, kind: 'number', quick: true, default: 2, min: 0.5, max: 24, options: WIDTHS, get: (context) => style(context).width },
    { key: 'stroke', id: 'line-color', get label() { return aiwsT('aiworkspace.blocks.color') }, kind: 'color', quick: true, palette: 'ink', get: (context) => style(context).stroke,
      editLabel: () => aiwsT('aiworkspace.connector.color') },
    { key: 'rounded', get label() { return aiwsT('aiworkspace.connector.rounded') }, kind: 'boolean', default: true, get: (context) => style(context).rounded },
    { key: 'label_size', get label() { return aiwsT('aiworkspace.connector.labelSize') }, kind: 'select', default: 'm', options: LABEL_SIZES, get: (context) => style(context).labelSize },
  ],
  // "insert" from a menu: a horizontal line from the spot (the object toolbar starts the connector tool instead)
  create: (args) => [createOp(args.cellId, 'buckyos.cell', args.parentId, args.orderKey, { view: { type: CONNECTOR_VIEW, version: 1 }, start: null, end: null }, undefined, { ...args.placement, h: 0, w: Math.max(40, args.placement.w) })],
}

