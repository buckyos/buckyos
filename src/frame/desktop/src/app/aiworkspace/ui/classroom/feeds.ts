/* The classroom extension's side of the session feeds (第四期规划 §1, §5.3): a classroom Block binds the feeds it reads
 * (`bindings: { tasks, my_responses, responses, results, participants }`) to the courseware's sample entities, which
 * the session service places (`session.feed_sample`); in a session the same bindings read the live feeds (a
 * participant's own `my_responses` instead of the shared sample). Rules — what counts as a question, who may be
 * picked — are this extension's, not the kernel's. */

import { useCallback } from 'react'
import type { Json, QueryRow, Result } from '../../api/types'
import { useLoad, useVersion } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { feedTarget, type SessionUi } from '../../session/context'
import type { RenderContext } from '../blocks/registry'

export type BindingName = 'tasks' | 'my_responses' | 'responses' | 'results' | 'participants' | 'roster'

const pending = new Map<string, Promise<void>>()

/** Bind the feeds a classroom Block reads to the courseware's samples (one write; once per Block and session). */
export function ensureBindings(context: RenderContext, store: WorkspaceStore, feeds: { name: BindingName; feed: string; public?: boolean }[]) {
  const id = context.cell.entity_id
  const have = context.payload.bindings ?? {}
  const missing = feeds.filter((f) => !have[f.name] || !store.outline.get(have[f.name].entity_id))
  if (missing.length === 0 || pending.has(id) || !context.capabilities.includes('update')) return
  const run = (async () => {
    const bindings: Record<string, Json> = { ...(have as unknown as Record<string, Json>) }
    for (const f of missing) {
      const r = await store.session.call<Result<{ entity_id: string }>>('session.feed_sample', { workspace_id: store.session.workspaceId, feed: f.feed, ...(f.public ? { public: true } : {}) })
      if (r?.ok) bindings[f.name] = { entity_id: r.entity_id }
    }
    await store.outline.reload()
    await store.submit({ editId: `bindings:${id}`, label: 'bind session feeds', operations: [{ op: 'entity.set_keys', entity_id: id, keys: [{ key: 'bindings', value: bindings, expect: { rev: context.keyRevs.bindings ?? 0 } }] }] })
  })()
  pending.set(id, run.finally(() => pending.delete(id)))
}

/** The entity a binding reads in this context (a session redirects `my_responses`). */
export function boundId(context: RenderContext, store: WorkspaceStore, session: SessionUi | null, name: BindingName): string | undefined {
  return feedTarget(session, store, context.payload.bindings?.[name]?.entity_id)
}

/** The rows of a feed (or any table), re-read when it changes. `null` while loading or without access. */
export function useRows(store: WorkspaceStore, entityId: string | undefined): QueryRow[] | null {
  const version = useVersion(`e:${entityId ?? ''}`)
  const load = useCallback(async () => {
    if (!entityId || !store.outline.get(entityId)) return null
    try {
      return (await store.session.query({ source_id: entityId, limit: 1000 })).rows
    } catch {
      return null
    }
  }, [store, entityId])
  return useLoad(load, `${entityId}:${version}`).data ?? null
}

export function text(row: QueryRow | undefined, field: string): string {
  const v = row?.values[field]
  return typeof v === 'string' ? v : v === undefined || v === null ? '' : String(v)
}

/** A feed row's JSON column (answers, contents are stored as JSON text). */
export function json(row: QueryRow | undefined, field: string): Json | null {
  const t = text(row, field)
  if (!t) return null
  try { return JSON.parse(t) as Json } catch { return t }
}

/** A small seeded generator (xorshift32): a pick is reproducible from its seed, which goes into the task (§5.4). */
export function seeded(seed: number): () => number {
  let x = seed >>> 0 || 0x9e3779b9
  return () => {
    x ^= x << 13; x >>>= 0
    x ^= x >>> 17
    x ^= x << 5; x >>>= 0
    return x / 0x1_0000_0000
  }
}

/** The principal column of a roster table (Q17: names are matched by DID or user name only). */
export function rosterColumns(rows: QueryRow[] | null, fieldNames: Record<string, string>): { principal: string | null; name: string | null } {
  // the field names when the table was read, else the field ids the rows carry
  const ids = Object.keys(fieldNames).length ? Object.keys(fieldNames) : [...new Set((rows ?? []).flatMap((r) => Object.keys(r.values)))]
  const find = (re: RegExp) => ids.find((id) => re.test(id) || re.test(fieldNames[id] ?? '')) ?? null
  // i18n-exempt: column names of rosters people import (Chinese headers included)
  const principal = find(/^(principal|did|user|user_id|账号|学号)$/i)
  // i18n-exempt: column names of rosters people import (Chinese headers included)
  const name = find(/^(name|姓名|名字|display_name)$/i)
  return { principal, name }
}
