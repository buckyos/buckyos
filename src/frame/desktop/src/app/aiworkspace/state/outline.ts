/* OutlineModel: the two trees of one open Workspace as an incrementally maintained map of envelopes
 * (phase two §9.3 rule 4). Structural change events are applied from the operations they carry —
 * a move or a placement never re-reads the whole outline — and only events the model cannot
 * interpret (no ops, an unknown entity, a restore) fall back to a full re-read.
 *
 * Every Block subscribes to its own entry (`subscribeEntity`), so one Block's change re-renders
 * that Block alone. Children lists are kept per parent for the trees and the RenderHost. */

import { isControlKind } from '@controls/contract'
import type { Capability, CommitEvent, ConnectorProjection, EntityEnvelope, Json, Operation, StoredPlacement } from '../api/types'
import { Emitter } from './emitter'

const COLOR = /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/

type Listener = () => void

function compareSiblings(a: EntityEnvelope, b: EntityEnvelope): number {
  const ka = a.order_key ?? ''
  const kb = b.order_key ?? ''
  return ka < kb ? -1 : ka > kb ? 1 : a.entity_id < b.entity_id ? -1 : a.entity_id > b.entity_id ? 1 : 0
}

const CONNECTOR_KEYS = ['flip', 'route', 'controls', 'label'] as const

function connectorProjection(payload: Record<string, unknown>): ConnectorProjection {
  const out: Record<string, unknown> = { start: payload.start ?? null, end: payload.end ?? null }
  for (const key of CONNECTOR_KEYS) if (payload[key] !== undefined && payload[key] !== null) out[key] = payload[key]
  return out as unknown as ConnectorProjection
}

/** Outline fields derived from a payload (what the backend's `outline_extras` adds). */
function extras(typeId: string, payload: Record<string, unknown>): Partial<EntityEnvelope> {
  const out: Partial<EntityEnvelope> = { title: (payload.title as string | undefined) ?? null }
  // a sourced entity (内核架构改进 D12): where it comes from; its content is read-only here
  const source = payload.source as Record<string, unknown> | undefined
  out.source = source && typeof source === 'object'
    ? { kind: source.kind as 'url' | 'workspace', mode: (source.mode as 'live' | 'fixed' | undefined) ?? 'live', url: (source.url as string | undefined) ?? null, workspace_id: (source.workspace_id as string | undefined) ?? null,
        entity_id: (source.entity_id as string | undefined) ?? null, zone: (source.zone as string | undefined) ?? null, refresh: (source.refresh as { interval_s: number } | undefined) ?? null, auth_mode: (source.auth_mode as 'none' | 'hosted' | undefined) ?? 'none' }
    : null
  if (typeId === 'buckyos.container') {
    out.kind = payload.kind as string
    out.layout = (payload.layout as EntityEnvelope['layout']) ?? { mode: 'flow' }
    for (const key of ['system', 'surface_id', 'content_folder_id'] as const) if (payload[key] !== undefined) (out as Record<string, unknown>)[key] = payload[key]
    out.icon = (payload.icon as string | undefined) ?? null
    out.locked = payload.locked === true ? true : null
    // a control's attributes drive its layout (脑图方案 §5.4)
    out.config = isControlKind(payload.kind as string) && payload.config && typeof payload.config === 'object' ? payload.config as Record<string, Json> : null
  } else if (typeId === 'buckyos.cell') {
    out.locked = payload.locked === true ? true : null
    const view = payload.view as { type?: string; version?: number } | undefined
    out.view_type = view?.type ?? null
    out.view_version = view?.version ?? null
    // a connector's geometry keys, as the backend's `outline_extras` projects them (连接线实现方案 §9.2)
    out.connector = view?.type === 'connector' ? connectorProjection(payload) : null
    const stroke = (payload.config as { stroke?: unknown } | undefined)?.stroke
    out.ink_stroke = view?.type === 'ink' && typeof stroke === 'string' ? stroke : null
    out.source_id = (payload.source_ref as { entity_id?: string } | undefined)?.entity_id ?? null
    out.def_id = (payload.def_ref as { entity_id?: string } | undefined)?.entity_id ?? null
    out.live = (payload.presentation as { live?: boolean } | undefined)?.live === true ? true : null
    // a Block's role in its control and a mind map node's layout hints, as the backend projects them
    out.slot = typeof payload.slot === 'string' ? payload.slot : null
    const config = payload.config as { side?: unknown; branch_color?: unknown } | undefined
    out.side = config?.side === 'left' || config?.side === 'right' ? config.side : null
    out.branch_color = typeof config?.branch_color === 'string' && COLOR.test(config.branch_color) ? config.branch_color : null
  } else if (typeId === 'buckyos.viewport') {
    out.viewport = {
      surface_id: (payload.surface_ref as { entity_id?: string } | undefined)?.entity_id ?? null,
      center: (payload.center as { x: number; y: number } | undefined) ?? null,
      zoom: (payload.zoom as number | undefined) ?? null,
    }
  } else if (typeId === 'buckyos.show-path') {
    out.show_path = {
      purpose: (payload.purpose as 'presentation' | 'guide' | undefined) ?? null,
      stage: (payload.stage as { w: number; h: number } | undefined) ?? null,
      steps: Array.isArray(payload.steps) ? payload.steps.length : 0,
    }
  } else if (typeId === 'buckyos.wish') {
    out.executor = (payload.executor as string) ?? null
    out.output_mode = (payload.output_mode as string) ?? null
  } else if (typeId === 'buckyos.block-def') {
    out.def_id = (payload.def_id as string) ?? null
    out.def_kind = (payload.kind as string) ?? null
    // a control's identity and where it came from (扩展控件 P0), as the backend projects them
    const ident = payload.ident as { author_did?: string; name?: string } | undefined
    out.ident = ident?.author_did && ident.name ? `${ident.author_did}#${ident.name}` : null
    out.manifest_version = typeof payload.manifest_version === 'number' ? payload.manifest_version : null
    out.def_version = typeof payload.version === 'number' ? payload.version : null
    out.provenance_pkg = (payload.provenance as { pkg_id?: string } | undefined)?.pkg_id ?? null
    out.source_dir_id = (payload.source_dir as { entity_id?: string } | undefined)?.entity_id ?? null
  } else if (typeId === 'buckyos.annotation') {
    out.target_id = (payload.target as { entity_id?: string } | undefined)?.entity_id ?? null
    out.annotation_kind = (payload.kind as string) ?? null
  } else if (typeId === 'buckyos.asset-ref') {
    out.media_type = (payload.media_type as string) ?? null
  }
  return out
}

export class OutlineModel {
  private entities = new Map<string, EntityEnvelope>()
  private children = new Map<string, Set<string>>()
  private readonly entityListeners = new Map<string, Set<Listener>>()
  private readonly emitter = new Emitter()
  private version = 0
  private loaded = false
  private reloadTimer: number | null = null
  private refreshTimer: number | null = null
  private readonly toRefresh = new Set<string>()
  private readonly load: () => Promise<EntityEnvelope[]>
  private readonly readEnvelopes: (ids: string[]) => Promise<(EntityEnvelope | null)[]>
  private readonly principal: string | null
  readonly subscribe = this.emitter.subscribe
  /** A remote structural change of a Block this window placed recently (layout conflict, D1). */
  onRemoteLayoutChange: ((entityId: string, author: string | null, placement: StoredPlacement | undefined) => void) | null = null

  constructor(load: () => Promise<EntityEnvelope[]>, readEnvelopes: (ids: string[]) => Promise<(EntityEnvelope | null)[]>, principal: string | null) {
    this.load = load
    this.readEnvelopes = readEnvelopes
    this.principal = principal
  }

  /** Monotonic counter: bumps on every applied change (for whole-tree subscribers). */
  snapshot = (): number => this.version
  isLoaded(): boolean { return this.loaded }

  get(id: string): EntityEnvelope | undefined { return this.entities.get(id) }
  all(): EntityEnvelope[] { return [...this.entities.values()] }
  /** Alive children of `parentId` in sibling order. */
  childrenOf(parentId: string): EntityEnvelope[] {
    const ids = this.children.get(parentId)
    if (!ids) return []
    const out: EntityEnvelope[] = []
    for (const id of ids) { const e = this.entities.get(id); if (e && !e.deleted) out.push(e) }
    return out.sort(compareSiblings)
  }
  /** Every alive descendant of `id`, depth first. */
  descendants(id: string): EntityEnvelope[] {
    const out: EntityEnvelope[] = []
    const walk = (parent: string) => { for (const child of this.childrenOf(parent)) { out.push(child); walk(child.entity_id) } }
    walk(id)
    return out
  }
  /** Path of ancestor ids from the entity up to the root (entity first). */
  ancestors(id: string): string[] {
    const out: string[] = []
    let cur: string | undefined = id
    for (let i = 0; cur && i < 4096; i++) { out.push(cur); cur = this.entities.get(cur)?.parent_id }
    return out
  }

  subscribeEntity(id: string, listener: Listener): () => void {
    let set = this.entityListeners.get(id)
    if (!set) { set = new Set(); this.entityListeners.set(id, set) }
    set.add(listener)
    return () => { set?.delete(listener); if (set && set.size === 0) this.entityListeners.delete(id) }
  }

  private notifyEntity(id: string) {
    const set = this.entityListeners.get(id)
    if (set) for (const listener of [...set]) listener()
  }

  private bump(ids: Iterable<string>) {
    this.version += 1
    for (const id of ids) this.notifyEntity(id)
    this.emitter.emit()
  }

  private link(entity: EntityEnvelope) {
    if (!entity.parent_id) return
    let set = this.children.get(entity.parent_id)
    if (!set) { set = new Set(); this.children.set(entity.parent_id, set) }
    set.add(entity.entity_id)
  }
  private unlink(entity: EntityEnvelope) {
    if (!entity.parent_id) return
    this.children.get(entity.parent_id)?.delete(entity.entity_id)
  }

  /** Replace everything with a fresh read (initial load and the fallback). */
  async reload(): Promise<void> {
    const list = await this.load()
    const next = new Map<string, EntityEnvelope>()
    for (const entity of list) next.set(entity.entity_id, entity)
    const touched = new Set<string>([...this.entities.keys(), ...next.keys()])
    this.entities = next
    this.children = new Map()
    for (const entity of list) this.link(entity)
    this.loaded = true
    this.bump(touched)
  }

  private scheduleReload() {
    if (this.reloadTimer !== null) return
    this.reloadTimer = window.setTimeout(() => {
      this.reloadTimer = null
      void this.reload().catch(() => { /* the next event or the caller's reload retries */ })
    }, 50)
  }

  /** Re-read the exact envelopes of entities that were patched optimistically (capabilities, lock holders…). */
  private scheduleRefresh(ids: string[]) {
    for (const id of ids) this.toRefresh.add(id)
    if (this.refreshTimer !== null) return
    this.refreshTimer = window.setTimeout(() => {
      this.refreshTimer = null
      const ids = [...this.toRefresh]
      this.toRefresh.clear()
      void this.readEnvelopes(ids).then((read) => {
        const changed: string[] = []
        read.forEach((entity, index) => {
          const id = ids[index]
          if (!entity) return
          const current = this.entities.get(id)
          if (!current) return
          // a newer local view of the structure wins over a read that overlapped it
          if ((entity.struct_rev ?? 0) < (current.struct_rev ?? 0)) return
          // `doc.read` envelopes carry no outline extras: keep the known ones, refresh them from the payload when it is there
          const payload = (entity as { content?: { payload?: Record<string, unknown> } }).content?.payload
          const merged: EntityEnvelope = { ...current, ...entity, ...(payload ? extras(entity.type_id, payload) : {}) }
          delete (merged as { content?: unknown }).content
          this.unlink(current)
          this.entities.set(id, merged)
          this.link(merged)
          changed.push(id)
        })
        if (changed.length > 0) this.bump(changed)
      }).catch(() => undefined)
    }, 80)
  }

  private patch(id: string, update: Partial<EntityEnvelope>): boolean {
    const current = this.entities.get(id)
    if (!current) return false
    const next = { ...current, ...update }
    if (next.parent_id !== current.parent_id) { this.unlink(current); this.link(next) }
    this.entities.set(id, next)
    return true
  }

  /** The capabilities of an entity this event created, until its envelope is read back: the parent's, and on the
   * caller's own writing with `comment` what the backend adds (`entity_caps`, 公开链接便签) — an annotation is
   * edited and deleted, a note Block shown with it in the same commit is also moved. */
  private createdCapabilities(event: CommitEvent, op: NonNullable<CommitEvent['ops']>[number], inherited: Capability[]): Capability[] {
    if (!this.principal || event.author !== this.principal || !inherited.includes('comment')) return inherited
    const payload = (op.payload as Record<string, unknown> | undefined) ?? {}
    const sourceId = (payload.source_ref as { entity_id?: string } | undefined)?.entity_id
    const extra: Capability[] = op.type_id === 'buckyos.annotation' ? ['update', 'delete']
      : op.type_id === 'buckyos.cell' && (payload.view as { type?: string } | undefined)?.type === 'note'
        && (event.ops ?? []).some((other) => other.op === 'entity.create' && other.entity_id === sourceId && other.type_id === 'buckyos.annotation') ? ['update', 'delete', 'structure']
        : []
    return [...new Set([...inherited, ...extra])]
  }

  /** Apply one change-stream event. Returns false when a full reload was scheduled instead. */
  applyEvent(event: CommitEvent): boolean {
    if (!this.loaded) return false
    const touched = event.touched ?? []
    if (touched.length === 0) return true
    const ops = event.ops ?? []
    const opsFor = (id: string) => ops.filter((op) => op.entity_id === id)
    const remote = event.author !== undefined && event.author !== this.principal && !event.local
    const changed = new Set<string>()
    const refresh: string[] = []
    let reload = false
    for (const item of touched) {
      const id = item.entity_id
      switch (item.change) {
        case 'moved':
        case 'placed': {
          const op = opsFor(id).find((candidate) => candidate.op === 'tree.move' || candidate.op === 'tree.place')
          if (!op) { if (event.local) reload = true; else reload = true; break }
          const update: Partial<EntityEnvelope> = { struct_rev: item.rev }
          if (op.op === 'tree.move') { update.parent_id = op.new_parent_id as string; update.order_key = op.order_key as string; update.placement = (op.placement as StoredPlacement | null | undefined) ?? undefined }
          else {
            if (typeof op.order_key === 'string') update.order_key = op.order_key
            if ('placement' in op) update.placement = (op.placement as StoredPlacement | null) ?? undefined
          }
          if (!this.patch(id, update)) { reload = true; break }
          changed.add(id)
          if (remote) this.onRemoteLayoutChange?.(id, event.author ?? null, update.placement)
          break
        }
        case 'created': {
          if (this.entities.has(id)) { refresh.push(id); changed.add(id); break }
          const op = opsFor(id).find((candidate) => candidate.op === 'entity.create')
          const parent = op ? this.entities.get(op.parent_id as string) : undefined
          if (!op || !parent) { reload = true; break }
          const payload = (op.payload as Record<string, unknown> | undefined) ?? {}
          const entity: EntityEnvelope = {
            entity_id: id, type_id: op.type_id as string, schema_version: (op.schema_version as number | undefined) ?? 1,
            name: (op.name as string | undefined) ?? null, parent_id: op.parent_id as string, order_key: op.order_key as string,
            placement: (op.placement as StoredPlacement | undefined) ?? undefined, scope: (op.scope as string | undefined) === 'personal' ? 'personal' : 'shared', deleted: false,
            content_rev: item.rev, meta_rev: item.rev, life_rev: item.rev, struct_rev: item.rev, write_policy: 'open',
            capabilities: this.createdCapabilities(event, op, parent.capabilities), ...extras(op.type_id as string, payload),
          }
          this.entities.set(id, entity)
          this.link(entity)
          changed.add(id)
          refresh.push(id)
          break
        }
        case 'deleted': {
          const current = this.entities.get(id)
          if (current) { this.unlink(current); this.entities.delete(id); changed.add(id) }
          break
        }
        case 'restored':
          reload = true
          break
        case 'renamed': {
          const op = opsFor(id).find((candidate) => candidate.op === 'entity.rename' || candidate.op === 'entity.set_write_policy')
          if (!op) { reload = true; break }
          const update: Partial<EntityEnvelope> = { meta_rev: item.rev }
          if (op.op === 'entity.rename') update.name = (op.name as string | null) ?? null
          else update.write_policy = op.policy as 'open' | 'lock_required'
          if (!this.patch(id, update)) { reload = true; break }
          changed.add(id)
          break
        }
        case 'derived': {
          const op = opsFor(id).find((candidate) => candidate.op === 'entity.set_derived')
          if (!this.patch(id, { meta_rev: item.rev, derived: (op?.derived as EntityEnvelope['derived']) ?? null })) break
          changed.add(id)
          break
        }
        default: {
          // content changes: bump the version and refresh outline extras touched by keyed writes
          const current = this.entities.get(id)
          if (!current) break
          const update: Partial<EntityEnvelope> = { content_rev: Math.max(current.content_rev, item.rev) }
          const keyed = opsFor(id).filter((candidate) => candidate.op === 'entity.set_keys' || candidate.op === 'entity.unset_keys')
          if (keyed.length > 0) {
            const payload: Record<string, unknown> = {}
            let touchedExtras = false
            for (const op of keyed) for (const key of (op.keys as { key: string; value?: unknown }[] | undefined) ?? []) {
              if (['title', 'source', 'view', 'source_ref', 'def_ref', 'kind', 'layout', 'executor', 'output_mode', 'target', 'media_type', 'def_id', 'icon', 'locked', 'start', 'end', 'presentation', 'config', 'slot',
                'surface_ref', 'center', 'zoom', 'purpose', 'stage', 'steps', 'ident', 'manifest_version', 'version', 'provenance', 'source_dir', ...CONNECTOR_KEYS].includes(key.key)) touchedExtras = true
              if (op.op === 'entity.set_keys') payload[key.key] = key.value
            }
            if (touchedExtras) {
              // the ops carry only the changed keys: rebuild extras from the known envelope plus the change
              const known: Record<string, unknown> = {
                title: current.title, source: current.source ?? undefined, kind: current.kind, layout: current.layout, system: current.system, surface_id: current.surface_id, content_folder_id: current.content_folder_id,
                icon: current.icon ?? undefined, locked: current.locked ?? undefined,
                view: current.view_type ? { type: current.view_type, version: current.view_version ?? undefined } : undefined,
                source_ref: current.source_id ? { entity_id: current.source_id } : undefined,
                def_ref: current.def_id ? { entity_id: current.def_id } : undefined,
                executor: current.executor, output_mode: current.output_mode, def_id: current.def_id, media_type: current.media_type,
                target: current.target_id ? { entity_id: current.target_id } : undefined,
                ...(current.connector ?? {}),
                config: current.type_id === 'buckyos.container' ? current.config ?? undefined
                  : current.ink_stroke || current.side || current.branch_color ? { ...(current.ink_stroke ? { stroke: current.ink_stroke } : {}), ...(current.side ? { side: current.side } : {}), ...(current.branch_color ? { branch_color: current.branch_color } : {}) } : undefined,
                slot: current.slot ?? undefined,
                presentation: current.live ? { live: true } : undefined,
                surface_ref: current.viewport?.surface_id ? { entity_id: current.viewport.surface_id } : undefined,
                center: current.viewport?.center ?? undefined, zoom: current.viewport?.zoom ?? undefined,
                purpose: current.show_path?.purpose ?? undefined, stage: current.show_path?.stage ?? undefined,
                steps: current.show_path ? new Array(current.show_path.steps) : undefined,
                ident: current.ident ? (() => { const cut = current.ident.lastIndexOf('#'); return { author_did: current.ident.slice(0, cut), name: current.ident.slice(cut + 1) } })() : undefined,
                manifest_version: current.manifest_version ?? undefined, version: current.def_version ?? undefined,
                provenance: current.provenance_pkg ? { pkg_id: current.provenance_pkg } : undefined,
                source_dir: current.source_dir_id ? { entity_id: current.source_dir_id } : undefined,
              }
              for (const op of keyed) if (op.op === 'entity.unset_keys') for (const key of (op.keys as { key: string }[] | undefined) ?? []) delete known[key.key]
              Object.assign(update, extras(current.type_id, { ...known, ...payload }))
            }
          }
          this.patch(id, update)
          changed.add(id)
        }
      }
    }
    if (reload) { console.debug('[aiworkspace] outline: event not interpretable, reloading', event.seq, touched.map((t) => `${t.entity_id}:${t.change}`).join(',')); this.scheduleReload(); return false }
    if (refresh.length > 0) this.scheduleRefresh(refresh)
    if (changed.size > 0) this.bump(changed)
    return true
  }

  /** Apply a local optimistic patch (e.g. an accepted move before the stream brings it back): nothing is persisted here. */
  patchLocal(id: string, update: Partial<EntityEnvelope>) {
    if (this.patch(id, update)) this.bump([id])
  }

  dispose() {
    if (this.reloadTimer !== null) window.clearTimeout(this.reloadTimer)
    if (this.refreshTimer !== null) window.clearTimeout(this.refreshTimer)
    this.entityListeners.clear()
  }
}

/** The operation(s) of one accepted event concerning `entityId` (used by tests and tools). */
export function opsOf(event: CommitEvent, entityId: string): Operation[] {
  return (event.ops ?? []).filter((op) => op.entity_id === entityId)
}
