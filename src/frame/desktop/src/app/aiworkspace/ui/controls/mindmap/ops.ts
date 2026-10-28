/* Mind map operations and tree navigation (脑图方案 §7): every write is an ordinary commit on the BlockTree — a node
 * is a Cell under its parent node, placement "auto" (the control lays it out), a pin writes numbers, "back to
 * automatic layout" writes "auto" again. Navigation follows the laid-out tree: towards the children along the way
 * a branch grows, back to the parent the other way, siblings across. */

import { isControlKind } from '@controls/contract'
import { BRANCH_PALETTE } from '@controls/edges'
import { ALL_AUTO, isAuto } from '@controls/placement'
import { mindmapConfig, type MindmapConfig } from '@controls/tree'
import { randomId } from '../../../api/ids'
import type { EntityEnvelope, Json, Operation } from '../../../api/types'
import type { OutlineModel } from '../../../state/outline'
import type { WorkspaceStore } from '../../../state/store'
import type { Laid } from '../../canvas/layout'
import { isCanvasContainer } from '../registry'

export const NODE_VIEW = 'mindnode'

/** The control laying `id` out: its nearest container, walking up through Blocks, when that is a control. */
export function controlOf(outline: OutlineModel, id: string): EntityEnvelope | undefined {
  let cur = outline.get(id)?.parent_id
  for (let i = 0; cur && i < 4096; i++) {
    const e = outline.get(cur)
    if (!e) return undefined
    if (e.type_id === 'buckyos.container') return isControlKind(e.kind) ? e : undefined
    cur = e.parent_id
  }
  return undefined
}

/** The Blocks a node (or a control) holds, in order. */
export function blockChildren(outline: OutlineModel, id: string): EntityEnvelope[] {
  return outline.childrenOf(id).filter((c) => c.type_id === 'buckyos.cell' || isCanvasContainer(c))
}

/** Where a node sits in its control: depth (0: a root), its branch colour, the control and its config. */
export function nodeInfo(outline: OutlineModel, id: string): { control: EntityEnvelope; config: MindmapConfig; depth: number; color: string | null } | null {
  const chain: EntityEnvelope[] = []
  let cur: EntityEnvelope | undefined = outline.get(id)
  for (let i = 0; cur && i < 4096; i++) {
    if (cur.type_id === 'buckyos.container' && chain.length > 0) break
    chain.push(cur)
    cur = cur.parent_id ? outline.get(cur.parent_id) : undefined
  }
  if (!cur || !isControlKind(cur.kind)) return null
  const config = mindmapConfig(cur.config)
  const depth = chain.length - 1
  if (depth === 0) return { control: cur, config, depth, color: null }
  const own = chain.slice(0, depth).find((e) => e.branch_color)?.branch_color
  if (own) return { control: cur, config, depth, color: own }
  // the first-level node's turn among its root's children
  const level1 = chain[depth - 1]
  const root = chain[depth]
  const palette = config.branch_colors?.length ? config.branch_colors : BRANCH_PALETTE
  const index = blockChildren(outline, root.entity_id).findIndex((c) => c.entity_id === level1.entity_id)
  return { control: cur, config, depth, color: palette[Math.max(0, index) % palette.length] }
}

const after = (store: WorkspaceStore, parentId: string) => store.core.order_key_between(store.outline.childrenOf(parentId).at(-1)?.order_key ?? undefined, undefined)

/** A new text node under `parentId` (a node or the control), last among its children. */
export function addChildOp(store: WorkspaceStore, parentId: string, title = ''): { op: Operation; id: string } {
  const id = randomId('n')
  return { id, op: { op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: parentId, order_key: after(store, parentId), placement: { ...ALL_AUTO }, payload: { view: { type: NODE_VIEW }, ...(title ? { title } : {}) } } }
}

/** A new text node right after `nodeId` among its siblings; a root gets a child instead. */
export function addSiblingOp(store: WorkspaceStore, nodeId: string): { op: Operation; id: string } {
  const node = store.outline.get(nodeId)
  const parent = node?.parent_id ? store.outline.get(node.parent_id) : undefined
  if (!node || !parent || parent.type_id === 'buckyos.container') return addChildOp(store, nodeId)
  const siblings = store.outline.childrenOf(parent.entity_id)
  const index = siblings.findIndex((s) => s.entity_id === nodeId)
  const id = randomId('n')
  const key = store.core.order_key_between(siblings[index]?.order_key ?? undefined, siblings[index + 1]?.order_key ?? undefined)
  return { id, op: { op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: parent.entity_id, order_key: key, placement: { ...ALL_AUTO }, payload: { view: { type: NODE_VIEW } } } }
}

/** `nodeId` (with its subtree) re-hung under `parentId`, before `beforeId` / at the end; laid out again (x / y "auto"). */
export function moveNodeOp(store: WorkspaceStore, nodeId: string, parentId: string, beforeId: string | null): Operation {
  const node = store.outline.get(nodeId)
  const siblings = store.outline.childrenOf(parentId).filter((s) => s.entity_id !== nodeId)
  const index = beforeId ? siblings.findIndex((s) => s.entity_id === beforeId) : -1
  const key = index < 0
    ? store.core.order_key_between(siblings.at(-1)?.order_key ?? undefined, undefined)
    : store.core.order_key_between(siblings[index - 1]?.order_key ?? undefined, siblings[index].order_key ?? undefined)
  const stored = node?.placement
  return { op: 'tree.move', entity_id: nodeId, new_parent_id: parentId, order_key: key, placement: { x: 'auto', y: 'auto', w: stored?.w ?? 'auto', h: stored?.h ?? 'auto' } }
}

/** Back to the automatic layout (§5.5): the pinned position goes, the size components stay as stored. */
export function resetLayoutOps(store: WorkspaceStore, ids: string[]): Operation[] {
  return ids.flatMap((id) => {
    const p = store.outline.get(id)?.placement
    if (!p || (isAuto(p.x) && isAuto(p.y))) return []
    return [{ op: 'tree.place', entity_id: id, placement: { x: 'auto', y: 'auto', w: p.w, h: p.h } }]
  })
}

/** A key of a Block or control set from what is stored now (one key, its own version). */
export async function setKeyOps(store: WorkspaceStore, id: string, key: string, value: Json | null): Promise<Operation[]> {
  const read = await store.session.read<{ key_revs: Record<string, number> }>(id)
  const expect = { rev: read.content.key_revs[key] ?? 0 }
  return [value === null ? { op: 'entity.unset_keys', entity_id: id, keys: [{ key, expect }] } : { op: 'entity.set_keys', entity_id: id, keys: [{ key, value, expect }] }]
}

/** The control's config with `patch` applied (a null value removes the key). */
export async function controlConfigOps(store: WorkspaceStore, containerId: string, patch: Record<string, Json | null>): Promise<Operation[]> {
  const read = await store.session.read<{ payload: { config?: Record<string, Json> }; key_revs: Record<string, number> }>(containerId)
  const config: Record<string, Json> = { ...(read.content.payload.config ?? {}) }
  for (const [k, v] of Object.entries(patch)) { if (v === null) delete config[k]; else config[k] = v }
  return [{ op: 'entity.set_keys', entity_id: containerId, keys: [{ key: 'config', value: config, expect: { rev: read.content.key_revs.config ?? 0 } }] }]
}

/** A node's own config with `patch` applied (branch colour, side). */
export async function nodeConfigOps(store: WorkspaceStore, nodeId: string, patch: Record<string, Json | null>): Promise<Operation[]> {
  return controlConfigOps(store, nodeId, patch)
}

/** The node an arrow key goes to from `nodeId` (§7.1): along the branch's growth to the child nearest across,
 * against it to the parent, across to the previous / next sibling on the same side. */
export function navigate(laid: Map<string, Laid>, nodeId: string, key: string): string | null {
  const me = laid.get(nodeId)
  const node = me?.node
  if (!me || !node) return null
  const kids = [...laid.values()].filter((l) => l.node?.parent === nodeId && l.node.control === node.control)
  const center = (l: Laid) => ({ x: l.rect.x + l.rect.w / 2, y: l.rect.y + l.rect.h / 2 })
  const c = center(me)
  const nearest = (list: Laid[], axis: 'x' | 'y') => list.sort((a, b) => Math.abs(center(a)[axis] - c[axis]) - Math.abs(center(b)[axis] - c[axis]))[0]?.entity.entity_id ?? null
  const toward = (grow: string) => nearest(kids.filter((k) => k.node?.grow === grow), grow === 'down' ? 'x' : 'y')
  const vertical = node.grow === 'down'
  if (node.parent === null) {
    // a root: its children on the side of the arrow
    if (key === 'ArrowRight') return toward('right')
    if (key === 'ArrowLeft') return toward('left')
    if (key === 'ArrowDown') return toward('down')
    return null
  }
  const inward = node.grow === 'right' ? 'ArrowRight' : node.grow === 'left' ? 'ArrowLeft' : 'ArrowDown'
  const outward = node.grow === 'right' ? 'ArrowLeft' : node.grow === 'left' ? 'ArrowRight' : 'ArrowUp'
  if (key === inward) return nearest(kids, vertical ? 'x' : 'y')
  if (key === outward) return node.parent
  const across = vertical ? (key === 'ArrowLeft' ? -1 : key === 'ArrowRight' ? 1 : 0) : (key === 'ArrowUp' ? -1 : key === 'ArrowDown' ? 1 : 0)
  if (!across) return null
  const siblings = [...laid.values()].filter((l) => l.node?.parent === node.parent && l.node.control === node.control && l.node.grow === node.grow)
    .sort((a, b) => (vertical ? a.rect.x - b.rect.x : a.rect.y - b.rect.y))
  const i = siblings.findIndex((s) => s.entity.entity_id === nodeId)
  return siblings[i + across]?.entity.entity_id ?? null
}
