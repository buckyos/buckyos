/* Writing presentation paths (第三期规划 §6.3, §7.1). The whole `steps` array is one version cell, so every edit is
 * expressed as step commands by id: a conflict re-reads the newest steps, replays the commands and commits again (at
 * most three times). Only a command whose step or anchor was deleted cannot be replayed; that edit then stays in
 * "需要处理". Creating a Viewport or a Frame and adding its step, or changing the stage size together with the
 * Frames' aspect, is one commit and one undo step. */

import { randomId } from '../api/ids'
import type { CommitOutcome, Json, KeyedContent, Operation, Placement, ShowPathPayload, ShowStep, StageSize } from '../api/types'
import type { WorkspaceStore } from '../state/store'
import { applyStepCommands, DEFAULT_BACKGROUND, DEFAULT_STAGE, frameAtRatio, sameRatio, worldRectOf, type StepCommand } from './model'

const REPLAYS = 3

export function okOutcome(outcome: CommitOutcome): boolean {
  return outcome.status === 'accepted' || outcome.status === 'saved_locally'
}

export async function readPath(store: WorkspaceStore, pathId: string): Promise<{ payload: ShowPathPayload; revs: Record<string, number> }> {
  const read = await store.session.read<KeyedContent<ShowPathPayload>>(pathId)
  return { payload: read.content.payload, revs: read.content.key_revs ?? {} }
}

/** Commit `commands` on the path's steps (plus `extra` operations computed from the new steps), replaying them on
 * conflicts. Resolves with the outcome of the last attempt. */
export async function commitSteps(
  store: WorkspaceStore,
  pathId: string,
  commands: StepCommand[],
  label: string,
  extra?: (steps: ShowStep[], path: ShowPathPayload) => Operation[],
): Promise<CommitOutcome | null> {
  const editId = `path:${pathId}:${randomId().slice(0, 8)}`
  let outcome: CommitOutcome | null = null
  for (let attempt = 0; attempt <= REPLAYS; attempt++) {
    let path: { payload: ShowPathPayload; revs: Record<string, number> }
    try { path = await readPath(store, pathId) } catch (error) { store.notify('error', `无法读取演讲路径：${error instanceof Error ? error.message : String(error)}`); return null }
    const next = applyStepCommands(path.payload.steps, commands)
    if ('unreplayable' in next) {
      store.notify('error', `「${label}」无法应用到最新的路径上：${next.unreplayable}。请检查后重新操作。`)
      return outcome
    }
    const operations: Operation[] = [
      ...(extra?.(next, path.payload) ?? []),
      { op: 'entity.set_keys', entity_id: pathId, keys: [{ key: 'steps', value: next as unknown as Json, expect: { rev: path.revs.steps ?? 0 } }] },
    ]
    outcome = await store.submit({ editId, label, operations })
    // someone changed the path meanwhile: replay the commands on what is there now
    if (outcome.status === 'conflict' && attempt < REPLAYS && outcome.conflicts?.every((c) => c.entity_id === pathId)) continue
    return outcome
  }
  return outcome
}

/** Set path-level keys (title, purpose, background) in one commit. */
export async function setPathKeys(store: WorkspaceStore, pathId: string, values: Partial<Omit<ShowPathPayload, 'steps' | 'stage'>>, label: string): Promise<CommitOutcome | null> {
  const { revs } = await readPath(store, pathId)
  const set = Object.entries(values).filter(([, v]) => v !== undefined && v !== null && v !== '')
  const unset = Object.entries(values).filter(([, v]) => v === undefined || v === null || v === '')
  const operations: Operation[] = []
  if (set.length) operations.push({ op: 'entity.set_keys', entity_id: pathId, keys: set.map(([key, value]) => ({ key, value: value as Json, expect: { rev: revs[key] ?? 0 } })) })
  if (unset.length) operations.push({ op: 'entity.unset_keys', entity_id: pathId, keys: unset.map(([key]) => ({ key, expect: { rev: revs[key] ?? 0 } })) })
  if (operations.length === 0) return null
  return store.submit({ editId: `path:${pathId}:keys`, label, operations })
}

/** A new path in the `shows` folder. */
export function createPathOp(store: WorkspaceStore, title: string, purpose: 'presentation' | 'guide'): { op: Operation; id: string } {
  const id = randomId('p')
  const last = store.outline.childrenOf('shows').at(-1)?.order_key
  const payload: ShowPathPayload = { title, purpose, stage: DEFAULT_STAGE, background: DEFAULT_BACKGROUND, steps: [] }
  return { id, op: { op: 'entity.create', entity_id: id, type_id: 'buckyos.show-path', parent_id: 'shows', order_key: store.core.order_key_between(last ?? undefined, undefined), payload: payload as unknown as Json } }
}

/** A new Viewport on `surfaceId`. */
export function createViewportOp(store: WorkspaceStore, surfaceId: string, view: { center: { x: number; y: number }; zoom: number }, title: string): { op: Operation; id: string } {
  const id = randomId('v')
  const last = store.outline.childrenOf('shows').at(-1)?.order_key
  return {
    id,
    op: { op: 'entity.create', entity_id: id, type_id: 'buckyos.viewport', parent_id: 'shows', order_key: store.core.order_key_between(last ?? undefined, undefined),
      payload: { title, surface_ref: { entity_id: surfaceId }, center: view.center, zoom: view.zoom } },
  }
}

/** A new Frame Cell on `surfaceId` at the world rect `rect`. */
export function createFrameOp(store: WorkspaceStore, surfaceId: string, rect: Placement, title: string): { op: Operation; id: string } {
  const id = randomId('c')
  const last = store.outline.childrenOf(surfaceId).at(-1)?.order_key
  return {
    id,
    op: { op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: surfaceId, order_key: store.core.order_key_between(last ?? undefined, undefined),
      placement: { x: Math.round(rect.x), y: Math.round(rect.y), w: Math.round(rect.w), h: Math.round(rect.h) }, payload: { view: { type: 'frame', version: 1 }, title, config: { color: '#4f8df7' } } },
  }
}

/** `tree.place` of a Frame brought to the stage's aspect (keeping centre and width), or nothing when it already fits. */
export function fitFrameOps(store: WorkspaceStore, frameId: string, stage: StageSize): Operation[] {
  const frame = store.outline.get(frameId)
  const placed = worldRectOf(store.outline, frameId)
  if (!frame?.placement || !placed || sameRatio(placed.rect, stage)) return []
  const next = frameAtRatio(frame.placement, stage)
  const placement: Placement = { ...frame.placement, x: next.x, y: next.y, w: next.w, h: next.h }
  store.noteLayoutIntent(frameId, placement)
  return [{ op: 'tree.place', entity_id: frameId, placement }]
}

export function newStep(kind: 'frame' | 'viewport', entityId: string): ShowStep {
  return { id: randomId('st'), target: { kind, entity_id: entityId } }
}

/** Change the stage size; every Frame of the path takes the new aspect in the same commit (§7.1). */
export async function setStage(store: WorkspaceStore, pathId: string, stage: StageSize): Promise<CommitOutcome | null> {
  const { payload, revs } = await readPath(store, pathId)
  const frames = [...new Set(payload.steps.filter((s) => s.target.kind === 'frame').map((s) => s.target.entity_id))]
  const operations: Operation[] = [
    { op: 'entity.set_keys', entity_id: pathId, keys: [{ key: 'stage', value: stage as unknown as Json, expect: { rev: revs.stage ?? 0 } }] },
    ...frames.flatMap((id) => (store.outline.get(id)?.capabilities.includes('structure') || store.outline.get(store.outline.get(id)?.parent_id ?? '')?.capabilities.includes('structure') ? fitFrameOps(store, id, stage) : [])),
  ]
  return store.submit({ editId: `path:${pathId}:stage`, label: `舞台尺寸 → ${stage.w} × ${stage.h}`, operations })
}

/** Write a target's presentation property: a Frame's `presentation.<key>`, a Viewport's own key. */
export async function setTargetText(store: WorkspaceStore, targetId: string, key: 'notes' | 'caption' | 'title' | 'background', value: string): Promise<CommitOutcome | null> {
  const target = store.outline.get(targetId)
  if (!target) return null
  const read = await store.session.read<KeyedContent<Record<string, Json>>>(targetId)
  const revs = read.content.key_revs ?? {}
  const payload = read.content.payload
  const text = value.trim() === '' ? null : value
  let operation: Operation
  if (target.type_id === 'buckyos.viewport' || key === 'title') {
    if ((payload[key] ?? null) === text) return null
    operation = text === null
      ? { op: 'entity.unset_keys', entity_id: targetId, keys: [{ key, expect: { rev: revs[key] ?? 0 } }] }
      : { op: 'entity.set_keys', entity_id: targetId, keys: [{ key, value: text, expect: { rev: revs[key] ?? 0 } }] }
  } else {
    const current = (payload.presentation as Record<string, Json> | undefined) ?? {}
    if ((current[key] ?? null) === text) return null
    const next: Record<string, Json> = { ...current }
    if (text === null) delete next[key]
    else next[key] = text
    operation = { op: 'entity.set_keys', entity_id: targetId, keys: [{ key: 'presentation', value: Object.keys(next).length ? next : null, expect: { rev: revs.presentation ?? 0 } }] }
  }
  const label = { notes: '讲解备注', caption: '说明', title: '标题', background: '背景色' }[key]
  return store.submit({ editId: `target:${targetId}:${key}`, label: `${label}：${target.title ?? target.entity_id}`, operations: [operation] })
}

/** Turn "operable on stage" on or off for a Block (`presentation.live`, §10.2). */
export async function setLive(store: WorkspaceStore, cellId: string, live: boolean): Promise<CommitOutcome | null> {
  const read = await store.session.read<KeyedContent<Record<string, Json>>>(cellId)
  const current = (read.content.payload.presentation as Record<string, Json> | undefined) ?? {}
  const next: Record<string, Json> = { ...current }
  if (live) next.live = true
  else delete next.live
  return store.submit({
    editId: `live:${cellId}`, label: live ? '放映时可操作' : '放映时不可操作',
    operations: [{ op: 'entity.set_keys', entity_id: cellId, keys: [{ key: 'presentation', value: Object.keys(next).length ? next : null, expect: { rev: read.content.key_revs?.presentation ?? 0 } }] }],
  })
}
