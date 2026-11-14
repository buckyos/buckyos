/* The `mindmap` control (脑图方案 §6.2, R2): the first built-in Block-control. Its kind is the container's, its layout
 * the shared `tree` algorithm, its chrome the implicit edges and fold handles; inserting one creates the container and
 * a root to type into. With one of its nodes selected it takes the keys (§7.1): Tab adds a child, Enter a sibling, the
 * arrows walk the tree, Ctrl+/ folds. Its properties are the direction, the edges and the spacing (quick ones on the
 * Block toolbar); its actions turn it into an outline beside it, copy it or save it as Markdown / OPML. */

import { ArrowDown, ArrowLeft, ArrowLeftRight, ArrowRight, ClipboardCopy, FileDown, ListTree, Network, Spline } from 'lucide-react'
import { MINDMAP } from '@controls/mindmap'
import { ALL_AUTO } from '@controls/placement'
import { solveControl } from '@controls/solve'
import { mindmapConfig, type Direction, type EdgeStyle } from '@controls/tree'
import { randomId } from '../../../api/ids'
import type { Json } from '../../../api/types'
import { aiwsT } from '../../../i18n'
import type { WorkspaceStore } from '../../../state/store'
import type { BlockAction, ConfigField, RenderContext } from '../../blocks/registry'
import { saveTextFile } from '../../canvas/drawioExport'
import { controlCommands } from '../commands'
import { foldBook } from '../folds'
import type { ContainerDefinition, ControlCommandContext } from '../registry'
import { MindmapChrome } from './Chrome'
import { FlowMindmap } from './FlowMindmap'
import { mindmapMarkdown, mindmapOpml, mindmapToRichtext } from './convert'
import { addChildOp, addSiblingOp, blockChildren, controlConfigOps, navigate, NODE_VIEW } from './ops'

const SPACINGS: Record<string, { sibling: number; level: number }> = { compact: { sibling: 6, level: 28 }, normal: { sibling: 12, level: 48 }, loose: { sibling: 24, level: 80 } }

function spacingName(s: { sibling: number; level: number }): string {
  return Object.entries(SPACINGS).find(([, v]) => v.sibling === s.sibling && v.level === s.level)?.[0] ?? 'normal'
}

/** Create a node next to / under the selection, select it and type into it. */
export function addNode(ctx: ControlCommandContext, kind: 'child' | 'sibling', from: string) {
  if (!ctx.canEdit) return
  const { op, id } = kind === 'child' ? addChildOp(ctx.store, from) : addSiblingOp(ctx.store, from)
  // a folded parent opens to show what was added
  if (kind === 'child') foldBook(ctx.store).set(ctx.containerId, [from], false)
  void ctx.store.submit({ editId: `insert:${id}`, label: aiwsT(kind === 'child' ? 'aiworkspace.mindmap.addChild' : 'aiworkspace.mindmap.addSibling'), operations: [op] }).then((outcome) => {
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') { ctx.select([id]); ctx.edit(id) }
  })
}

/** The map as an outline, beside it as rich text (脑图方案 §8): placed by the canvas's layout, then selected. */
function toRichtext(ctx: ControlCommandContext) {
  const container = ctx.store.outline.get(ctx.containerId)
  if (!container || !ctx.canEdit) return
  void mindmapToRichtext(ctx.store, ctx.laid, container, ctx.surface).then((id) => { if (id) ctx.select([id]) })
}

/** The file name of the map's export. */
const fileName = (store: WorkspaceStore, id: string) => (store.outline.get(id)?.title || aiwsT('aiworkspace.mindmap.title')).replace(/[\\/:*?"<>|]/g, '_')

const config = (context: RenderContext) => mindmapConfig(context.cell.config)
const configOps = (patch: (value: string) => Record<string, Json | null>) => (context: RenderContext, store: WorkspaceStore, value: Json | null) => controlConfigOps(store, context.cell.entity_id, patch(String(value ?? '')))
const dirIcons = { right: ArrowRight, left: ArrowLeft, both: ArrowLeftRight, down: ArrowDown }

const fields: ConfigField[] = [
  { key: 'direction', id: 'mindmap-direction', get label() { return aiwsT('aiworkspace.mindmap.direction') }, kind: 'select', quick: true, default: 'right',
    get options() { return (['right', 'left', 'both', 'down'] as Direction[]).map((d) => ({ value: d, label: aiwsT(`aiworkspace.mindmap.dir.${d}`), icon: dirIcons[d] })) },
    get: (context) => config(context).direction, ops: configOps((value) => ({ direction: value })), editLabel: () => aiwsT('aiworkspace.mindmap.direction') },
  { key: 'edge', id: 'mindmap-edge', get label() { return aiwsT('aiworkspace.mindmap.edge') }, kind: 'select', quick: true, icon: Spline, default: 'curve',
    get options() { return (['curve', 'elbow', 'straight'] as EdgeStyle[]).map((e) => ({ value: e, label: aiwsT(`aiworkspace.mindmap.edgeStyle.${e}`) })) },
    get: (context) => config(context).edge, ops: configOps((value) => ({ edge: value })), editLabel: () => aiwsT('aiworkspace.mindmap.edge') },
  { key: 'spacing', id: 'mindmap-spacing', get label() { return aiwsT('aiworkspace.mindmap.spacing') }, kind: 'select', quick: true, default: 'normal',
    get options() { return Object.keys(SPACINGS).map((k) => ({ value: k, label: aiwsT(`aiworkspace.mindmap.spacingName.${k}`) })) },
    get: (context) => spacingName(config(context).spacing), ops: configOps((value) => ({ spacing: SPACINGS[value] ?? SPACINGS.normal })), editLabel: () => aiwsT('aiworkspace.mindmap.spacing') },
]

const actions: BlockAction[] = [
  { id: 'mindmap-outline', icon: ListTree, primary: true, get label() { return aiwsT('aiworkspace.mindmap.toOutline') }, modes: ['edit'], needs: ['update'],
    run: (context) => controlCommands.send({ kind: 'run', containerId: context.cell.entity_id, run: toRichtext }) },
  { id: 'mindmap-copy-markdown', icon: ClipboardCopy, get label() { return aiwsT('aiworkspace.mindmap.copyMarkdown') }, modes: ['edit', 'view'],
    run: (context, store) => { void navigator.clipboard?.writeText(mindmapMarkdown(store, context.cell.entity_id)).then(() => store.notify('info', aiwsT('aiworkspace.mindmap.copied'))) } },
  { id: 'mindmap-export-markdown', icon: FileDown, get label() { return aiwsT('aiworkspace.mindmap.exportMarkdown') }, modes: ['edit', 'view'],
    run: (context, store) => saveTextFile(`${fileName(store, context.cell.entity_id)}.md`, mindmapMarkdown(store, context.cell.entity_id), 'text/markdown') },
  { id: 'mindmap-export-opml', icon: FileDown, get label() { return aiwsT('aiworkspace.mindmap.exportOpml') }, modes: ['edit', 'view'],
    run: (context, store) => saveTextFile(`${fileName(store, context.cell.entity_id)}.opml`, mindmapOpml(store, context.cell.entity_id), 'text/x-opml') },
]

function keys(ctx: ControlCommandContext, event: KeyboardEvent): boolean {
  const mod = event.ctrlKey || event.metaKey
  const node = ctx.nodeId
  if (!node) {
    // the container selected: Tab / Enter go into its first root
    if ((event.key === 'Tab' || event.key === 'Enter') && !mod) {
      const root = blockChildren(ctx.store.outline, ctx.containerId)[0]
      if (root) ctx.select([root.entity_id])
      return true
    }
    return false
  }
  if (event.key === 'Tab' && !mod) {
    if (event.shiftKey) { const parent = ctx.laid.get(node)?.node?.parent; if (parent) ctx.select([parent]) } else addNode(ctx, 'child', node)
    return true
  }
  if (event.key === 'Enter' && !mod && !event.shiftKey) { addNode(ctx, ctx.laid.get(node)?.node?.parent ? 'sibling' : 'child', node); return true }
  if (event.key.startsWith('Arrow') && !mod && !event.altKey) {
    const next = navigate(ctx.laid, node, event.key)
    if (next) ctx.select([next])
    return true
  }
  if (mod && event.key === '/') {
    if (blockChildren(ctx.store.outline, node).length > 0) foldBook(ctx.store).toggle(ctx.containerId, node)
    return true
  }
  return false
}

export const mindmapControl: ContainerDefinition = {
  kind: 'mindmap', version: 1, get title() { return aiwsT('aiworkspace.mindmap.title') }, icon: Network,
  configSchema: {
    type: 'object', additionalProperties: false,
    properties: {
      direction: { enum: ['right', 'left', 'both', 'down'] }, edge: { enum: ['curve', 'elbow', 'straight'] },
      spacing: { type: 'object', properties: { sibling: { type: 'number', minimum: 0, maximum: 2000 }, level: { type: 'number', minimum: 0, maximum: 2000 } }, additionalProperties: false },
      branch_colors: { type: 'array', maxItems: 64, items: { type: 'string' } },
    },
  },
  children: { accepts: 'any', containers: true },
  layout: { direction: 'bottom_up', algorithm: 'tree', solve: solveControl },
  Chrome: MindmapChrome,
  Flow: FlowMindmap,
  // the container and a root to type into: the root is the Block selected and edited after insertion
  create: (args) => {
    const containerId = args.dataId || randomId('m')
    const title = args.title || aiwsT('aiworkspace.mindmap.rootDefault')
    return [
      { op: 'entity.create', entity_id: containerId, type_id: 'buckyos.container', parent_id: args.parentId, order_key: args.orderKey,
        placement: { x: Math.round(args.placement.x), y: Math.round(args.placement.y), w: 'auto', h: 'auto' },
        payload: { kind: 'mindmap', layout: { mode: 'tree' }, title, config: { direction: 'right', edge: 'curve' } } },
      { op: 'entity.create', entity_id: args.cellId, type_id: 'buckyos.cell', parent_id: containerId, order_key: 'a', placement: { ...ALL_AUTO },
        payload: { view: { type: NODE_VIEW }, title } },
    ]
  },
  actions,
  configFields: fields,
  keys,
  agent: MINDMAP,
}
