/* Declared actions of a control (控件机制 TODO H2; manifest 4 `component.json` `actions`): what the host offers on the Block
 * toolbar of an instance (as primary actions) and in its menu — the context menu, the toolbar's "more", a flow page's title
 * row (UI 框架 PRD §10.1) — in the modes it names, to whoever has the capabilities it needs — also before an HTML control
 * runs. The host runs them: handlers as an event's (`write`, `wish`, `open`, `notify`), plus `config` (the instance's
 * own properties, one undoable `entity.set_props`) and `forward` (handed to the running HTML as the request
 * `action:<id>`; a control that is not running is started first). Strings may hold `{{now}}` — the service's clock — and
 * `{{config.<key>}}`. Like an event, an action is the person's own act, with their rights. */

import type { EventHandler, Json, KeyedContent, Operation } from '../../api/types'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import { serverClock } from '../../state/serverClock'
import type { CanvasMode } from '../blocks/registry'
import { bindingsOf, typedWriter, writeAllowed } from './bridge'

export interface DeclaredAction {
  id: string
  title: string
  /** One of the icon names the host knows (`blockui/icons.ts`); others show the title. */
  icon?: string
  primary?: boolean
  modes?: ('edit' | 'view')[]
  needs?: string[]
  run: ActionHandler | ActionHandler[]
}

type ActionHandler = EventHandler | { kind: 'config'; props: Record<string, Json> } | { kind: 'forward' }

/** The actions a definition declares (an HTML Block's definition payload, or a composite's `def_actions` in the outline). */
export function declaredActions(list: unknown): DeclaredAction[] {
  // the declarative interpreter's own actions (`command`) are not these
  return Array.isArray(list) ? (list as unknown as DeclaredAction[]).filter((a) => a && typeof a.id === 'string' && typeof a.title === 'string' && a.run !== undefined) : []
}

/** A declared action is a primary action of its control (a toolbar button, UI 框架 PRD §10.1) unless it says `primary:
 * false`; every declared action is in the Block's menu. (`primary` is read once `component.json` may carry it.) */
export function declaredPrimary(a: DeclaredAction): boolean {
  return a.primary !== false
}

/** Is the action offered to this person here (mode and capabilities)? */
export function actionOffered(a: DeclaredAction, mode: CanvasMode, capabilities: readonly string[]): boolean {
  const modes = a.modes ?? ['edit', 'view']
  if (!modes.includes(mode === 'show' ? 'view' : mode as 'edit' | 'view')) return false
  return (a.needs ?? []).every((n) => capabilities.includes(n))
}

const PLACE = /\{\{\s*(now|config\.([A-Za-z0-9_-]+))\s*\}\}/g
const WHOLE = /^\s*\{\{\s*(now|config\.([A-Za-z0-9_-]+))\s*\}\}\s*$/

function fill(v: Json, now: number, config: Record<string, Json>): Json {
  if (typeof v === 'string') {
    const whole = WHOLE.exec(v)
    if (whole) return whole[1] === 'now' ? now : config[whole[2]] ?? null
    return v.replace(PLACE, (_m, what: string, key?: string) => (what === 'now' ? String(now) : String(config[key ?? ''] ?? '')))
  }
  if (Array.isArray(v)) return v.map((x) => fill(x, now, config))
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, fill(x, now, config)]))
  return v
}

// ---- `forward`: handed to the running HTML (started first when needed)

const forwards = new Map<string, string[]>()
const forwardListeners = new Map<string, (id: string) => void>()

/** The running HTML of a Block receives forwarded actions (its runtime registers; queued ones arrive at once). */
export function onForwardedAction(cellId: string, listener: (id: string) => void): () => void {
  forwardListeners.set(cellId, listener)
  for (const id of forwards.get(cellId) ?? []) listener(id)
  forwards.delete(cellId)
  return () => { if (forwardListeners.get(cellId) === listener) forwardListeners.delete(cellId) }
}

/** The instance an action runs on. */
export interface ActionHost {
  instanceId: string
  mode: CanvasMode
  /** The control's name (in the undo label). */
  title: string
  /** The definition's config defaults. */
  defaults: Record<string, Json>
  openEntity: (id: string) => void
  /** Starts the control (a `forward` to a control that is not running); absent: it has nothing to start. */
  activate?: () => void
}

/** The defaults of a `config_schema`. */
export function configDefaults(schema: unknown): Record<string, Json> {
  const props = (schema as { properties?: Record<string, { default?: Json }> } | undefined)?.properties ?? {}
  return Object.fromEntries(Object.entries(props).filter(([, p]) => p?.default !== undefined).map(([k, p]) => [k, p.default as Json]))
}

/** Run an action of the instance. */
export async function runAction(store: WorkspaceStore, host: ActionHost, action: DeclaredAction): Promise<void> {
  const { instanceId } = host
  const read = await store.session.read<KeyedContent<Record<string, Json>>>(instanceId)
  const payload = read.content.payload as { config?: Record<string, Json> }
  const config = { ...host.defaults, ...(payload.config ?? {}) }
  const clock = serverClock(store.session)
  if (!clock.synced) await clock.sync()
  const handlers = (Array.isArray(action.run) ? action.run : [action.run]).map((h) => fill(h as unknown as Json, clock.now(), config) as unknown as ActionHandler)
  const label = aiwsT('aiworkspace.ext.action.label', { control: host.title, action: action.title })
  const ops: Operation[] = []
  const cell = store.outline.get(instanceId)
  if (!cell) throw new Error(aiwsT('aiworkspace.extension.noRecord', { id: instanceId }))
  const writer = typedWriter(store, cell, bindingsOf(payload as never))
  for (const h of handlers) {
    switch (h.kind) {
      case 'config': {
        const props = h.props ?? {}
        ops.push({ op: 'entity.set_props', entity_id: instanceId, key: 'config', props, expect: Object.fromEntries(Object.keys(props).map((k) => [k, read.content.key_revs[`p:config.${k}`] ?? 0])) })
        break
      }
      case 'write': {
        const target = writer.binding(h.target).entity_id
        if (!writeAllowed(store, host.mode, true, target)) throw new Error(aiwsT('aiworkspace.ext.event.writeRefused', { target: h.target }))
        ops.push(...(await writer.write(h.what, h.target, (h.args ?? {}) as Json)).ops)
        break
      }
      case 'wish':
        void store.wish.next(h.wish_id, { event: { name: `action:${action.id}`, instance_id: instanceId, params: {}, inputs: h.inputs ?? null } })
        break
      case 'open': {
        const b = h.target ? bindingsOf(payload as never)[h.target]?.entity_id ?? h.target : undefined
        if (b) host.openEntity(b)
        break
      }
      case 'notify':
        if (typeof h.text === 'string' && h.text) store.notify('info', h.text)
        break
      case 'forward': {
        const listener = forwardListeners.get(instanceId)
        if (listener) listener(action.id)
        else if (host.activate) { forwards.set(instanceId, [...(forwards.get(instanceId) ?? []), action.id]); host.activate() }
        break
      }
    }
  }
  if (ops.length) {
    const outcome = await store.submit({ editId: `action:${instanceId}:${action.id}`, label, operations: ops })
    if (outcome.status !== 'accepted' && outcome.status !== 'saved_locally') store.notify('error', aiwsT('aiworkspace.ext.event.notWritten', { label, code: 'code' in outcome ? String(outcome.code) : outcome.status }))
  }
}
