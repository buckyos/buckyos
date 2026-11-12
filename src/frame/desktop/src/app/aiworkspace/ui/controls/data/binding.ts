/* What a placed control binds (控件 on_bind TODO): the Desktop side of buckyos-websdk `resolveBindings` — the controls an
 * insertion creates as a tree (a composite and the first Blocks of its slots), their declarations (a built-in control's
 * contract, a definition's `bindings` / `on_bind` / `on_init_control`), the Workspace's data for the queries, the
 * dialogs of BindDialogs.tsx, and the result folded into the insertion's Commit; a re-bind of a control on the canvas the
 * same way. */

import { bindingsPayload, boundIds, createOps, resolveBindings, type BindDialogs, type BindFailure, type BindingDecl, type BindNode, type Bound, type DataInfo, type DataQuery, type OnBindStep, type OnInitStep } from 'buckyos'
import { uniqueName } from '@table/operations'
import { CONTRACTS } from '@controls/contracts'
import { isDataControl } from '@controls/contract'
import { randomId } from '../../../api/ids'
import type { EntityEnvelope, Json, Operation } from '../../../api/types'
import { aiwsT, currentAiwsLocale } from '../../../i18n'
import type { WorkspaceStore } from '../../../state/store'
import { controlRegistry } from '../registry'

/** Entities that are data a control can bind (not Blocks, canvases or definitions). */
const NOT_DATA = new Set(['buckyos.cell', 'buckyos.container', 'buckyos.block-def', 'buckyos.viewport', 'buckyos.show-path'])

/** Where data lives, for people: its canvas, else its folder. */
export function placeOf(store: WorkspaceStore, e: EntityEnvelope): string {
  const parent = e.parent_id ? store.outline.get(e.parent_id) : undefined
  if (!parent) return ''
  const surface = parent.system === 'surface_content' && parent.surface_id ? store.outline.get(parent.surface_id) : undefined
  return (surface ?? parent).title ?? (surface ?? parent).name ?? ''
}

export function findData(store: WorkspaceStore, q: DataQuery): DataInfo[] {
  return store.outline.all()
    .filter((e) => !e.deleted && !NOT_DATA.has(e.type_id) && (!q.accepts?.length || q.accepts.includes(e.type_id)) && (!q.schema_id || e.schema_id === q.schema_id))
    .map((e) => ({ entity_id: e.entity_id, type_id: e.type_id, schema_id: e.schema_id ?? null, title: e.title ?? e.name ?? null, name: e.name ?? null, place: placeOf(store, e) }))
}

/** A control's declarations: a built-in data control's contract, or its definition's payload. */
interface Declared { decls: Record<string, BindingDecl>; onBind?: OnBindStep[]; onInit?: OnInitStep[]; actions: { id: string }[]; title: string }

/** The built-in controls' binding titles in the person's language (their contracts are written in English). */
const BUILTIN_TITLES: Record<string, Record<string, string>> = {
  todo: { items: 'aiworkspace.datactl.binding.todoList' },
  calendar: { events: 'aiworkspace.datactl.binding.calendars', todo: 'aiworkspace.datactl.binding.todoLists' },
}

async function declaredOf(store: WorkspaceStore, kind: string | null, defId: string | null): Promise<Declared | null> {
  if (kind && isDataControl(kind)) {
    const contract = CONTRACTS[kind]?.() as { bindings?: Record<string, BindingDecl>; title?: string } | undefined
    if (!contract) return null
    const decls = Object.fromEntries(Object.entries(contract.bindings ?? {}).map(([name, d]) => [name, { ...d, title: BUILTIN_TITLES[kind]?.[name] ? aiwsT(BUILTIN_TITLES[kind][name]) : d.title }]))
    return { decls, actions: [], title: controlRegistry.get(kind)?.title ?? contract.title ?? kind }
  }
  if (!defId) return null
  try {
    const read = await store.readBatched<{ payload: { bindings?: Record<string, BindingDecl>; on_bind?: OnBindStep[]; on_init_control?: OnInitStep[]; actions?: { id: string }[]; title?: string } }>(defId)
    const p = read.content.payload
    return { decls: p.bindings ?? {}, onBind: p.on_bind, onInit: p.on_init_control, actions: p.actions ?? [], title: p.title ?? defId }
  } catch { return null }
}

const boundOf = (raw: unknown): Bound => {
  const out: Bound = {}
  for (const [name, v] of Object.entries((raw as Record<string, unknown> | undefined) ?? {})) {
    if (Array.isArray(v)) out[name] = v.map((x) => (x as { entity_id?: string })?.entity_id).filter((id): id is string => typeof id === 'string')
    else if (typeof (v as { entity_id?: unknown })?.entity_id === 'string') out[name] = (v as { entity_id: string }).entity_id
  }
  return out
}

interface PlacedNode { op: Operation; declared: Declared; node: BindNode }

/** Why a placement or a re-bind did not happen, for people (null: they cancelled). */
function failureText(error: BindFailure, declsOf: (node: string) => Record<string, BindingDecl> | undefined): string | null {
  if (error.code === 'CANCELLED') return null
  const name = error.binding ? declsOf(error.node ?? '')?.[error.binding]?.title ?? error.binding : ''
  if (error.code === 'NO_DATA') return aiwsT('aiworkspace.datactl.source.noData', { name })
  if (error.code === 'NOT_READY') return aiwsT('aiworkspace.datactl.source.notReady', { reason: error.message })
  return error.message
}

export type Placement = { ok: true; operations: Operation[]; then: { node: string; action: string }[] } | { ok: false; message: string | null }

/** Settle the bindings of the controls `operations` create (from parent to child, then their last checks from child to
 * parent) and fold what they create into them: one Commit. `message` null: the person cancelled. */
export async function bindPlacement(store: WorkspaceStore, operations: Operation[], opts: { folderId: string; dialogs: BindDialogs; noUi?: boolean }): Promise<Placement> {
  const placed = new Map<string, PlacedNode>()
  for (const op of operations) {
    if (op.op !== 'entity.create') continue
    const payload = (op.payload ?? {}) as Record<string, Json>
    const kind = op.type_id === 'buckyos.container' ? String(payload.kind ?? '') : null
    const defId = (payload.def_ref as { entity_id?: string } | undefined)?.entity_id ?? null
    if (op.type_id === 'buckyos.container' ? !(kind && (isDataControl(kind) || kind === 'composite')) : !(op.type_id === 'buckyos.cell' && defId)) continue
    const declared = await declaredOf(store, kind, defId)
    if (!declared) continue
    const id = String(op.entity_id)
    placed.set(id, { op, declared, node: { id, title: String(payload.title ?? declared.title), decls: declared.decls, given: boundOf(payload.bindings), onBind: declared.onBind, onInit: declared.onInit, children: [] } })
  }
  if (placed.size === 0) return { ok: true, operations, then: [] }
  const roots: BindNode[] = []
  for (const p of placed.values()) {
    const parent = placed.get(String(p.op.parent_id))
    if (parent) parent.node.children.push(p.node)
    else roots.push(p.node)
  }
  // a control put into one on the canvas sees what that one bound
  const outer = roots.length === 1 ? store.outline.get(String(placed.get(roots[0].id)!.op.parent_id)) : undefined
  const outerDeclared = outer && (isDataControl(outer.kind) || outer.kind === 'composite') ? await declaredOf(store, outer.kind ?? null, outer.def_id ?? null) : null
  const root: BindNode = roots.length === 1 ? roots[0] : { id: '', title: '', decls: {}, children: roots }
  const outcome = await resolveBindings(root, {
    reason: 'insert', no_ui: opts.noUi === true, dialogs: opts.dialogs, find: (q) => findData(store, q),
    parent: outer && outerDeclared ? { decls: outerDeclared.decls, bound: (outer.bindings ?? {}) as Bound } : null,
  })
  if (!outcome.ok) return { ok: false, message: failureText(outcome.error, (node) => placed.get(node)?.declared.decls) }
  const made = createData(store, outcome.results, opts.folderId, operations)
  for (const [id, result] of outcome.results) {
    const p = placed.get(id)
    if (!p) continue
    const payload = { ...((p.op.payload ?? {}) as Record<string, Json>) }
    const bindings = bindingsPayload(result.bindings, p.declared.decls, made.ids)
    if (Object.keys(bindings).length > 0) payload.bindings = bindings as Json
    else delete payload.bindings
    p.op.payload = payload
  }
  return { ok: true, operations: [...made.ops, ...operations], then: outcome.then }
}

/** Operations creating what the bindings asked for, in `folderId`: after its children and whatever `alongside` puts there,
 * names free among them. */
function createData(store: WorkspaceStore, results: Parameters<typeof createOps>[0], folderId: string, alongside: Operation[] = []) {
  const siblings = alongside.filter((op) => op.op === 'entity.create' && op.parent_id === folderId)
  const taken = new Set([...store.outline.childrenOf(folderId).filter((e) => !e.deleted).map((e) => e.name), ...siblings.map((op) => op.name as string | undefined)].filter((n): n is string => Boolean(n)))
  let last = [store.outline.childrenOf(folderId).at(-1)?.order_key, ...siblings.map((op) => op.order_key as string | undefined)].filter((k): k is string => Boolean(k)).sort().at(-1)
  return createOps(results, {
    folderId, newId: () => randomId('d'),
    orderKey: () => { last = store.core.order_key_between(last, undefined); return last },
    freeName: (name) => { const n = uniqueName(name, taken); taken.add(n); return n },
    locale: currentAiwsLocale().startsWith('zh') ? 'zh' : 'en',
  }) as { ops: Operation[]; ids: Record<string, string> }
}

/** The content folder of the canvas an entity is on. */
export function surfaceFolderOf(store: WorkspaceStore, id: string): string | null {
  let at = store.outline.get(id)
  for (let i = 0; at && at.kind !== 'surface' && i < 64; i++) at = at.parent_id ? store.outline.get(at.parent_id) : undefined
  return at?.content_folder_id ?? null
}

/** Bind a control on the canvas again: `rebind` asks for each binding (its current data shown), `insert` settles what is
 * missing the way an insertion would. One Commit: data it creates and an `entity.set_props` of its `bindings`. */
export async function bindControl(store: WorkspaceStore, id: string, reason: 'insert' | 'rebind', dialogs: BindDialogs, label: string): Promise<'done' | 'cancelled' | string> {
  const entity = store.outline.get(id)
  const folderId = surfaceFolderOf(store, id)
  if (!entity || !folderId) return 'missing'
  const declared = await declaredOf(store, entity.type_id === 'buckyos.container' ? entity.kind ?? null : null, entity.def_id ?? null)
  if (!declared) return 'missing'
  const current = entity.bindings ? (entity.bindings as Bound) : boundOf((await store.readBatched<{ payload: { bindings?: unknown } }>(id)).content.payload.bindings)
  const outcome = await resolveBindings(
    { id, title: entity.title ?? declared.title, decls: declared.decls, given: current, onBind: reason === 'insert' ? declared.onBind : undefined, onInit: undefined, children: [] },
    { reason, no_ui: false, dialogs, find: (q) => findData(store, q) },
  )
  if (!outcome.ok) return failureText(outcome.error, () => declared.decls) ?? 'cancelled'
  const result = outcome.results.get(id)!
  const made = createData(store, outcome.results, folderId)
  const next = bindingsPayload(result.bindings, declared.decls, made.ids)
  const props: Record<string, Json> = {}
  for (const name of Object.keys(declared.decls)) {
    const before = boundIds(current[name])
    const after = next[name]
    if (after === undefined) { if (before.length > 0) props[name] = null }
    else props[name] = after
  }
  if (Object.keys(props).length === 0 && made.ops.length === 0) return 'done'
  const outcomeCommit = await store.submit({ editId: `bind:${id}`, label, operations: [...made.ops, { op: 'entity.set_props', entity_id: id, key: 'bindings', props } as unknown as Operation] })
  if (outcomeCommit.status === 'rejected') return outcomeCommit.errors?.[0]?.detail ?? outcomeCommit.detail ?? outcomeCommit.code ?? 'rejected'
  return 'done'
}
