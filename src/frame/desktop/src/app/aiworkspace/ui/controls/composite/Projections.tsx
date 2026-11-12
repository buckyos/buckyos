/* eslint-disable react-refresh/only-export-components -- the projections with their viewer-clock helpers */
/* The projections of a composite (控件机制 TODO G2, G13, G16, G18, G21; manifest 4): Blocks a definition declares as a
 * template over data — the records of a bound table, the members of a bound directory, the elements of a list field
 * counted (most first), or the viewer — instantiated per item, never stored, laid out in the regions the composite's
 * layout reserved. In run mode a press on a node of a role raises the composite's event that names it
 * (`events.<e>.from = { projection, role }`), with the item's key: the instance's `on` handlers do the rest (a `write`,
 * a `wish` …), as for any event. The viewer's item carries their first opening (user state, by the service's clock) and
 * the time a slot of theirs still shows. */

import { useCallback, useContext, useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import { compositeOf, projectionLayout, templated, type Composition, type CompositionProjection } from '@controls/composite'
import { instantiate, type ProjectedNode, type Template } from '@controls/projection'
import type { FieldDef, Json, KeyedContent, QueryRow } from '../../../api/types'
import { useEntity, useLoad, useStore, useUserState, useVersion } from '../../../state/hooks'
import { RunModeContext } from '../../../state/runMode'
import { serverClock } from '../../../state/serverClock'
import type { WorkspaceStore } from '../../../state/store'
import { raiseEvent } from '../../extensions/events'
import type { CanvasMode } from '../../blocks/registry'
import { ProjectedView, projectedAt } from '../data/ProjectedView'
import { useRecordSets } from '../data/records'
import { projectionCounts } from './counts'

export const firstOpenKey = (containerId: string) => `block:${containerId}/first_open`

interface Item { key: string; scope: Record<string, unknown> }

type DirMember = { kind: string; object_id?: string; media_type?: string; size?: number }

/** The members of a bound directory. */
function useDirMembers(store: WorkspaceStore, dirId: string | null): Record<string, DirMember> {
  const version = useVersion(dirId ? `e:${dirId}` : '')
  const entity = useEntity(dirId ?? '')
  const load = useCallback(async () => (dirId ? (await store.session.read<KeyedContent<{ members?: Record<string, DirMember> }>>(dirId)).content.payload.members ?? {} : {}), [store, dirId])
  return useLoad(load, `${version}:${entity?.content_rev ?? 0}`).data ?? {}
}

const fieldOf = (row: QueryRow, field: string) => row.values[field]

/** A select's elements are keyed by their option's label (a photo's name), other values as they are. */
function labelsOf(field: FieldDef | undefined): (v: string) => string {
  const labels = new Map((field?.options ?? []).map((o) => [o.option_id, o.label]))
  return (v) => labels.get(v) ?? v
}

/** The viewer's time: their first opening of this composite (written once, by the service's clock) and the clock now. */
export function useViewerClock(containerId: string, active: boolean) {
  const store = useStore()
  const clock = serverClock(store.session)
  useSyncExternalStore(clock.subscribe, clock.snapshot)
  const firstOpen = useUserState<number>(firstOpenKey(containerId))
  const [now, setNow] = useState(() => clock.now())
  useEffect(() => {
    if (!active) return
    if (typeof firstOpen !== 'number') void clock.sync().then(() => { if (typeof store.userState.get(firstOpenKey(containerId)) !== 'number') store.userState.set(firstOpenKey(containerId), clock.now()) })
    const timer = window.setInterval(() => setNow(clock.now()), 1000)
    return () => window.clearInterval(timer)
  }, [active, firstOpen, clock, store, containerId])
  return { firstOpen: typeof firstOpen === 'number' ? firstOpen : null, now }
}

/** How long a slot's rule still shows it to this viewer (ms; null: the slot has no rule or the viewer has not opened). */
export function remainingOf(composition: Composition | null, config: Record<string, unknown>, firstOpen: number | null, now: number): number | null {
  const rules = Object.values(composition?.slots ?? {}).map((s) => s.visible).filter((v): v is NonNullable<typeof v> => Boolean(v))
  if (!rules.length || firstOpen === null) return null
  const forMs = Math.max(...rules.map((r) => Number(templated(r.for_ms, config)) || 0))
  return Math.max(0, firstOpen + forMs - now)
}

export function CompositeProjections({ containerId, regions, mode }: { containerId: string; regions: Map<string, { x: number; y: number; w: number; h: number }> | undefined; mode: CanvasMode }) {
  const store = useStore()
  const { interactive } = useContext(RunModeContext)
  const container = useEntity(containerId)
  const { composition, config } = compositeOf(store.outline, containerId)
  const projections = useMemo(() => Object.entries(composition?.projections ?? {}), [composition])
  const bindings = useMemo(() => container?.bindings ?? {}, [container?.bindings])
  const tableIds = useMemo(() => [...new Set(projections.flatMap(([, p]) => {
    const ids: string[] = []
    if (p.source.kind === 'records' || p.source.kind === 'counts') { const id = bindings[p.source.binding]; if (id) ids.push(id) }
    if (p.mark) { const id = bindings[p.mark.binding]; if (id) ids.push(id) }
    return ids
  }))].filter((id) => store.outline.get(id)?.type_id === 'buckyos.table-source'), [projections, bindings, store])
  const { snapshots } = useRecordSets(tableIds)
  const dirId = projections.map(([, p]) => (p.source.kind === 'members' ? bindings[p.source.binding] : undefined)).find(Boolean) ?? null
  const members = useDirMembers(store, dirId ?? null)
  const viewerClock = useViewerClock(containerId, mode !== 'edit' && mode !== 'presentation_edit')
  const principal = store.session.info().principal ?? null

  // the items of every projection (counts may join another projection's items, so members / records come first)
  const items = useMemo(() => {
    const out = new Map<string, Item[]>()
    const mineOf = (p: CompositionProjection): Set<string> => {
      if (!p.mark) return new Set()
      const tableId = bindings[p.mark.binding]
      const snap = tableId ? snapshots.get(tableId) : undefined
      const submitter = snap?.fields.find((f) => f.submitter)?.field_id
      const own = submitter && principal ? snap?.rows.find((r) => r.values[submitter] === principal) : undefined
      const v = own ? fieldOf(own, p.mark.field) : undefined
      const label = labelsOf(snap?.fields.find((f) => f.field_id === p.mark!.field || f.name === p.mark!.field))
      return new Set((Array.isArray(v) ? v.map(String) : typeof v === 'string' ? [v] : []).map(label))
    }
    const base = (name: string, p: CompositionProjection): Item[] => {
      const marked = mineOf(p)
      const mark = (key: string, scope: Record<string, unknown>) => (p.mark ? { ...scope, [p.mark.as]: marked.has(key) ? true : null } : scope)
      const src = p.source
      if (src.kind === 'members') {
        const accept = src.accept ?? 'any'
        return Object.entries(members).filter(([, m]) => m.kind === 'file' && (accept === 'any' || (m.media_type ?? '').startsWith(`${accept}/`)))
          .sort((a, b) => a[0].localeCompare(b[0]))
          .map(([key, m]) => ({ key, scope: { item: mark(key, { key, name: key, object_id: m.object_id, media_type: m.media_type, size: m.size }) } }))
      }
      if (src.kind === 'records') {
        const rows = [...(snapshots.get(bindings[src.binding] ?? '')?.rows ?? [])]
        if (src.sort) {
          const desc = src.sort.startsWith('-')
          const f = desc ? src.sort.slice(1) : src.sort
          rows.sort((a, b) => { const x = String(a.values[f] ?? ''), y = String(b.values[f] ?? ''); return (x < y ? -1 : x > y ? 1 : 0) * (desc ? -1 : 1) })
        }
        return rows.slice(0, src.limit ?? 200).map((r) => ({ key: r.record_id, scope: { item: mark(r.record_id, { key: r.record_id, id: r.record_id, ...r.values }) } }))
      }
      if (src.kind === 'viewer') {
        const remaining = remainingOf(composition, config, viewerClock.firstOpen, viewerClock.now)
        return [{ key: 'viewer', scope: { item: { principal, first_open: viewerClock.firstOpen, remaining_ms: remaining, expired: remaining === 0 } } }]
      }
      void name
      return []
    }
    for (const [name, p] of projections) if (p.source.kind !== 'counts') out.set(name, base(name, p))
    for (const [name, p] of projections) {
      if (p.source.kind !== 'counts') continue
      const src = p.source
      const snap = snapshots.get(bindings[src.binding] ?? '')
      const field = snap?.fields.find((f) => f.field_id === src.field || f.name === src.field)
      const label = labelsOf(field)
      const counts = new Map<string, number>()
      for (const r of snap?.rows ?? []) {
        const v = r.values[field?.field_id ?? src.field]
        for (const k of Array.isArray(v) ? v : v === undefined || v === null ? [] : [v]) counts.set(label(String(k)), (counts.get(label(String(k))) ?? 0) + 1)
      }
      const joined = src.of ? new Map((out.get(src.of) ?? []).map((i, index) => [i.key, { i, index }])) : null
      // most first; ties in the joined projection's order (the directory's), else by key
      const ranked = [...counts.entries()].filter(([k]) => !joined || joined.has(k))
        .sort((a, b) => b[1] - a[1] || (joined ? (joined.get(a[0])!.index - joined.get(b[0])!.index) : a[0].localeCompare(b[0])))
        .slice(0, src.top ?? 10)
      out.set(name, ranked.map(([key, count], rank) => ({ key, scope: { item: { ...((joined?.get(key)?.i.scope.item as Record<string, unknown> | undefined) ?? {}), key, count, rank: rank + 1 } } })))
    }
    return out
  }, [projections, members, snapshots, bindings, principal, viewerClock.firstOpen, viewerClock.now, composition, config])

  // the canvas layout reserves room for the items there are
  useEffect(() => {
    const book = projectionCounts(store)
    for (const [name, list] of items) book.set(containerId, name, list.length)
  }, [items, store, containerId])

  const nodes = useMemo(() => {
    const out: ProjectedNode[] = []
    for (const [name, p] of projections) {
      const region = regions?.get(name)
      const list = items.get(name) ?? []
      if (!region) continue
      const laid = projectionLayout(p, config, list.length, { x: region.x, y: region.y }, region.w)
      list.forEach((item, i) => {
        const rect = laid.items[i]
        if (!rect) return
        out.push(...instantiate(`${containerId}.${name}`, p.template as Template, { key: item.key, scope: { ...item.scope, config, viewer: { principal } } }, rect))
      })
    }
    return out
  }, [projections, regions, items, config, containerId, principal])

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!interactive) return
    const hit = projectedAt(event.target, event.currentTarget)
    if (!hit) return
    const projection = hit.id.slice(containerId.length + 1, hit.id.indexOf('~'))
    const name = Object.entries(composition?.events ?? {}).find(([, e]) => e.from?.projection === projection && (e.from.role ?? 'body') === hit.role)?.[0]
    if (!name) return
    event.stopPropagation()
    void raiseEvent({ store, instanceId: containerId, event: name, params: { item: hit.item, projection } as Record<string, Json>, mode, interactive })
      .catch((error: unknown) => store.notify('error', error instanceof Error ? error.message : String(error)))
  }

  if (!nodes.length) return null
  return (
    <div className="aiws-composite-projections" data-role={interactive ? 'editor' : undefined} data-active={interactive ? 'true' : undefined} onPointerDown={onPointerDown} data-testid={`aiws-composite-projections-${containerId}`}>
      {nodes.map((n) => <ProjectedView key={n.id} node={n} />)}
    </div>
  )
}
