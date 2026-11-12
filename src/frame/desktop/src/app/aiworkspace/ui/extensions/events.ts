/* Events of control instances (扩展控件 P2, Q26; README “Execution”): a control raises a declared event (`aiws.emit`, a
 * declarative `button`); in run mode the host runs the instance's `on` handlers in order — every `write` together as
 * one undoable Commit keyed `evt/<instance>/<event>/<event_id>`, a `wish` run with the same key, `open` / `notify` here —
 * then re-raises it through the composites around the instance (`events.<name>.from = { slot, event }`). A click is
 * the clicker's action with their rights; mounting, a sync or a reload never raises anything. In design mode nothing
 * runs: the Block shows a corner note instead. */

import { randomId } from '../../api/ids'
import type { BlockDefPayload, EntityEnvelope, EventBindings, EventHandler, Json, KeyedContent, Operation } from '../../api/types'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import type { CanvasMode } from '../blocks/registry'
import { bindingsOf, typedWriter, writeAllowed, type Binding } from './bridge'

export const MAX_HANDLERS = 4

export function handlersOf(v: EventHandler | EventHandler[] | undefined | null): EventHandler[] {
  if (!v) return []
  return (Array.isArray(v) ? v : [v]).filter((h) => h && typeof h === 'object')
}

// ---- design-mode notes: "event X (design mode, not run)" on the Block that raised it

const notes = new Map<string, { event: string; at: number }>()
const noteListeners = new Set<() => void>()
let notesVersion = 0
const NOTE_MS = 6000

export function subscribeDesignNotes(listener: () => void): () => void {
  noteListeners.add(listener)
  return () => { noteListeners.delete(listener) }
}
export function designNotesVersion(): number { return notesVersion }
export function designNote(cellId: string): { event: string; at: number } | null {
  const note = notes.get(cellId)
  return note && Date.now() - note.at < NOTE_MS ? note : null
}
function changedNotes() {
  notesVersion += 1
  for (const l of [...noteListeners]) l()
}
export function noteDesignEvent(cellId: string, event: string) {
  const at = Date.now()
  notes.set(cellId, { event, at })
  changedNotes()
  window.setTimeout(() => { if (notes.get(cellId)?.at === at) { notes.delete(cellId); changedNotes() } }, NOTE_MS)
}

// ---- templates: `{{event.<param>}}`, `{{config.<key>}}`; a string that is exactly one placeholder takes the value's type

const WHOLE = /^\s*\{\{\s*(event|config)\.([A-Za-z0-9_-]+)\s*\}\}\s*$/
const PART = /\{\{\s*(event|config)\.([A-Za-z0-9_-]+)\s*\}\}/g

export function fillTemplate(value: Json, scope: { event: Record<string, Json>; config: Record<string, Json> }): Json {
  if (typeof value === 'string') {
    const whole = WHOLE.exec(value)
    if (whole) return scope[whole[1] as 'event' | 'config'][whole[2]] ?? null
    return value.replace(PART, (_, where: 'event' | 'config', key: string) => { const v = scope[where][key]; return v === undefined || v === null ? '' : typeof v === 'string' ? v : JSON.stringify(v) })
  }
  if (Array.isArray(value)) return value.map((v) => fillTemplate(v, scope))
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, fillTemplate(v, scope)]))
  return value
}

/** A definition's `config_schema` defaults. */
function defaultsOf(def: BlockDefPayload | undefined): Record<string, Json> {
  const props = (def?.config_schema as { properties?: Record<string, { default?: Json }> } | undefined)?.properties ?? {}
  return Object.fromEntries(Object.entries(props).filter(([, p]) => p && p.default !== undefined).map(([k, p]) => [k, p.default as Json]))
}

interface InstancePayload { config?: Record<string, Json>; bindings?: Record<string, Binding>; source_ref?: { entity_id: string }; def_ref?: { entity_id: string }; on?: EventBindings; title?: string }

export interface RaiseArgs {
  store: WorkspaceStore
  instanceId: string
  event: string
  params?: Record<string, Json>
  mode: CanvasMode
  interactive: boolean
  /** Opens a data entity (the `open` handler). */
  openEntity?: (entityId: string) => void
  /** False once the runtime that raised it is gone: its late events are dropped. */
  alive?: () => boolean
}

async function readInstance(store: WorkspaceStore, id: string): Promise<{ payload: InstancePayload; def: BlockDefPayload | undefined }> {
  const read = await store.session.read<KeyedContent<InstancePayload>>(id)
  const payload = read.content.payload
  const defId = payload.def_ref?.entity_id
  const def = defId ? (await store.session.read<KeyedContent<BlockDefPayload>>(defId)).content.payload : undefined
  return { payload, def }
}

/** Raise `event` of the instance: in run mode its handlers run (and the composites around it re-raise it); in design
 * mode the Block notes it. An event the definition does not declare is refused. */
export async function raiseEvent(args: RaiseArgs): Promise<{ handled: boolean; event_id: string }> {
  const { store, instanceId, event } = args
  const eventId = randomId().slice(0, 20).toLowerCase()
  const { payload, def } = await readInstance(store, instanceId)
  if (!def?.events || !(event in def.events)) throw new Error(aiwsT('aiworkspace.ext.event.undeclared', { name: event }))
  if (!args.interactive) { noteDesignEvent(instanceId, event); return { handled: false, event_id: eventId } }
  const params = args.params ?? {}
  let handled = await runHandlers(args, instanceId, payload, def, event, params, eventId)
  // the composites around the instance re-raise it (`events.<name>.from = { slot, event }`), with the same event id
  let child = store.outline.get(instanceId)
  let childEvent = event
  for (let depth = 0; child?.parent_id && child.slot && depth < 16; depth++) {
    const parent: EntityEnvelope | undefined = store.outline.get(child.parent_id)
    if (!parent || parent.type_id !== 'buckyos.container' || parent.kind !== 'composite') break
    const outer = await readInstance(store, parent.entity_id)
    const slot = child.slot
    const name = Object.entries(outer.def?.events ?? {}).find(([, e]) => e.from?.slot === slot && e.from.event === childEvent)?.[0]
    if (!name) break
    if (await runHandlers(args, parent.entity_id, outer.payload, outer.def, name, params, eventId)) handled = true
    child = parent
    childEvent = name
  }
  return { handled, event_id: eventId }
}

async function runHandlers(args: RaiseArgs, entityId: string, payload: InstancePayload, def: BlockDefPayload | undefined, event: string, params: Record<string, Json>, eventId: string): Promise<boolean> {
  const { store } = args
  // the instance's own handlers, else the definition's default (a composite's `events.<e>.on`, manifest 4)
  const handlers = handlersOf(payload.on?.[event] ?? def?.events?.[event]?.on)
  if (handlers.length === 0) return false
  const scope = { event: params, config: { ...defaultsOf(def), ...(payload.config ?? {}) } }
  const filled = handlers.map((h) => fillTemplate(h as unknown as Json, scope) as unknown as EventHandler)
  const bindings = bindingsOf(payload)
  const anchor = store.outline.get(entityId)
  const label = aiwsT('aiworkspace.ext.event.label', { control: def?.title ?? def?.def_id ?? payload.title ?? entityId, event: def?.events?.[event]?.title ?? event })
  const key = `evt/${entityId}/${event}/${eventId}`
  let writesDone = false
  for (const h of filled) {
    if (args.alive && !args.alive()) return false
    switch (h.kind) {
      case 'write': {
        if (writesDone) break
        writesDone = true
        // every write handler of the event goes as one Commit, where the first one stands
        const ops: Operation[] = []
        try {
          if (!anchor) throw new Error(aiwsT('aiworkspace.ext.event.instanceGone'))
          const writer = typedWriter(store, anchor, bindings)
          for (const w of filled) {
            if (w.kind !== 'write') continue
            const target = writer.binding(w.target).entity_id
            if (!writeAllowed(store, args.mode, args.interactive, target)) throw new Error(aiwsT('aiworkspace.ext.event.writeRefused', { target: w.target }))
            ops.push(...(await writer.write(w.what, w.target, (w.args ?? {}) as Json)).ops)
          }
        } catch (error) {
          store.notify('error', aiwsT('aiworkspace.ext.event.failed', { label, error: error instanceof Error ? error.message : String(error) }))
          return true
        }
        if (ops.length === 0 || (args.alive && !args.alive())) break
        const outcome = await store.submit({ editId: `evt:${entityId}:${event}`, label, operations: ops, idempotencyKey: key })
        if (outcome.status !== 'accepted' && outcome.status !== 'saved_locally') store.notify('error', aiwsT('aiworkspace.ext.event.notWritten', { label, code: 'code' in outcome ? String(outcome.code) : outcome.status }))
        break
      }
      case 'wish':
        void store.wish.execute(h.wish_id, { event: { name: event, instance_id: entityId, params, inputs: h.inputs ?? null }, idempotencyKey: key })
          .catch((error: unknown) => store.notify('error', aiwsT('aiworkspace.ext.event.failed', { label, error: error instanceof Error ? error.message : String(error) })))
        break
      case 'open': {
        const target = h.target ? bindings[h.target]?.entity_id ?? h.target : bindings.source?.entity_id
        if (target) args.openEntity?.(target)
        break
      }
      case 'notify':
        if (typeof h.text === 'string' && h.text) store.notify('info', h.text)
        break
    }
  }
  return true
}
