/* The `mindmap` control (脑图方案 §6.2, R2): the first built-in Block-control. Its kind is the container's, its layout
 * the shared `tree` algorithm, its chrome the implicit edges and fold handles; inserting one creates the container and
 * a root to type into. With one of its nodes selected it takes the keys (§7.1): Tab adds a child, Enter a sibling, the
 * arrows walk the tree, Ctrl+/ folds. The container's toolbar sets the direction, the edges and the spacing. */

import { ArrowDown, ArrowLeft, ArrowLeftRight, ArrowRight, ListTree, Network, Spline } from 'lucide-react'
import { MINDMAP } from '@controls/mindmap'
import { ALL_AUTO } from '@controls/placement'
import { solveControl } from '@controls/solve'
import { mindmapConfig, type Direction, type EdgeStyle } from '@controls/tree'
import { randomId } from '../../../api/ids'
import { aiwsT } from '../../../i18n'
import type { ToolbarItem } from '../../blocks/registry'
import { saveTextFile } from '../../canvas/drawioExport'
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

function setConfig(ctx: ControlCommandContext, patch: Record<string, string | { sibling: number; level: number } | null>, label: string) {
  void controlConfigOps(ctx.store, ctx.containerId, patch).then((operations) => ctx.store.submit({ editId: `key:${ctx.containerId}:config`, label, operations }))
}

/** The map as an outline: beside it as rich text, on the clipboard, or saved as Markdown / OPML (脑图方案 §8, §9). */
function outlineAction(ctx: ControlCommandContext, value: string) {
  const container = ctx.store.outline.get(ctx.containerId)
  if (!container) return
  const name = (container.title || aiwsT('aiworkspace.mindmap.title')).replace(/[\\/:*?"<>|]/g, '_')
  if (value === 'richtext') void mindmapToRichtext(ctx.store, ctx.laid, container, ctx.surface).then((id) => { if (id) ctx.select([id]) })
  else if (value === 'copy') void navigator.clipboard?.writeText(mindmapMarkdown(ctx.store, ctx.containerId)).then(() => ctx.store.notify('info', aiwsT('aiworkspace.mindmap.copied')))
  else if (value === 'markdown') saveTextFile(`${name}.md`, mindmapMarkdown(ctx.store, ctx.containerId), 'text/markdown')
  else if (value === 'opml') saveTextFile(`${name}.opml`, mindmapOpml(ctx.store, ctx.containerId), 'text/x-opml')
}

function toolbar(ctx: ControlCommandContext): ToolbarItem[] {
  if (ctx.nodeId) return []
  const config = mindmapConfig(ctx.store.outline.get(ctx.containerId)?.config)
  const disabled = ctx.canEdit ? false : aiwsT('aiworkspace.blocks.noPermission')
  const dirIcon = { right: ArrowRight, left: ArrowLeft, both: ArrowLeftRight, down: ArrowDown }
  return [
    { kind: 'menu', id: 'mindmap-direction', icon: dirIcon[config.direction], label: aiwsT('aiworkspace.mindmap.direction'), value: config.direction, disabled,
      items: (['right', 'left', 'both', 'down'] as Direction[]).map((d) => ({ value: d, label: aiwsT(`aiworkspace.mindmap.dir.${d}`), icon: dirIcon[d] })),
      onPick: (value) => setConfig(ctx, { direction: value }, aiwsT('aiworkspace.mindmap.direction')) },
    { kind: 'menu', id: 'mindmap-edge', icon: Spline, label: aiwsT('aiworkspace.mindmap.edge'), value: config.edge, disabled,
      items: (['curve', 'elbow', 'straight'] as EdgeStyle[]).map((e) => ({ value: e, label: aiwsT(`aiworkspace.mindmap.edgeStyle.${e}`) })),
      onPick: (value) => setConfig(ctx, { edge: value }, aiwsT('aiworkspace.mindmap.edge')) },
    { kind: 'menu', id: 'mindmap-spacing', label: aiwsT('aiworkspace.mindmap.spacing'), value: spacingName(config.spacing), disabled,
      items: Object.keys(SPACINGS).map((k) => ({ value: k, label: aiwsT(`aiworkspace.mindmap.spacingName.${k}`) })),
      onPick: (value) => setConfig(ctx, { spacing: SPACINGS[value] ?? SPACINGS.normal }, aiwsT('aiworkspace.mindmap.spacing')) },
    { kind: 'menu', id: 'mindmap-outline', icon: ListTree, label: aiwsT('aiworkspace.mindmap.toOutline'),
      items: [
        ...(ctx.canEdit ? [{ value: 'richtext', label: aiwsT('aiworkspace.mindmap.toOutline') }] : []),
        { value: 'copy', label: aiwsT('aiworkspace.mindmap.copyMarkdown') },
        { value: 'markdown', label: aiwsT('aiworkspace.mindmap.exportMarkdown') },
        { value: 'opml', label: aiwsT('aiworkspace.mindmap.exportOpml') },
      ],
      onPick: (value) => outlineAction(ctx, value) },
  ]
}

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
  toolbar,
  keys,
  agent: MINDMAP,
}
