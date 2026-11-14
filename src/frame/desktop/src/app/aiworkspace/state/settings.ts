/* Shared configuration of an open Workspace (UI 框架 PRD §4.6, §12; K4). It is content: committed, undoable, carried by
 * clones, packages and templates; personal preferences are elsewhere (state/prefs.ts).
 *
 * - The Workspace layer is the root's `config` (`toolbar` and whatever else the shell keeps there). Every member reads
 *   it — `ws.get_info.settings` when the session opened, the root itself afterwards when the caller may read it — and
 *   only `manage` on `/` writes it.
 * - The canvas layer is `config.toolbar` on the Surface (next to `background` and `page_size`); `structure` on that
 *   Surface writes it.
 *
 * Writes are `entity.set_props { key: config }` (one version cell per property, expected at the version last read), so
 * two managers changing two different settings both keep theirs, and the same one conflicts. */

import { useCallback } from 'react'
import { aiwsT } from '../i18n'
import type { ReadOk } from '../api/session'
import type { Capability, CommitOutcome, Json, Operation, WorkspaceSettings } from '../api/types'
import { useEntity, useLoad, useReadOnlyReason, useStore, useVersion } from './hooks'
import type { WorkspaceStore } from './store'

type ConfigRead = ReadOk<{ payload: { config?: Record<string, Json> }; key_revs?: Record<string, number> }>

export interface SharedConfig<T> {
  /** The current value (`{}` / null before anything is set). */
  value: T
  loading: boolean
  error: string | null
  /** Why the caller cannot change it now (no permission, read-only), or null. */
  writeReason: string | null
}

export interface WorkspaceSettingsState extends SharedConfig<WorkspaceSettings> {
  /** The `config` key's version (`ws.get_info.settings_rev`, then the root's `key_revs.config`). */
  rev: number
  /** Set (null: remove) top-level settings: one undoable Commit. */
  update(changes: Record<string, Json | null>): Promise<CommitOutcome>
}

export interface SurfaceToolbarState extends SharedConfig<Json | null> {
  /** Replace the canvas's toolbar configuration (null: remove it, inherit the Workspace's): one undoable Commit. */
  set(toolbar: Json | null): Promise<CommitOutcome>
}

/** The operation writing `changes` into `entity_id`'s `config`: each property expected at the version of the entity's
 * `key_revs` just read (a property never written is at 0); without a read (a member who cannot read the root) the
 * properties merge unconditionally. */
function setConfigOp(entityId: string, changes: Record<string, Json | null>, revs: Record<string, number> | null): Operation {
  const expect = revs ? Object.fromEntries(Object.keys(changes).map((name) => [name, revs[`p:config.${name}`] ?? 0])) : null
  return { op: 'entity.set_props', entity_id: entityId, key: 'config', props: changes, ...(expect ? { expect } : {}) } as Operation
}

async function readConfig(store: WorkspaceStore, entityId: string): Promise<{ config: Record<string, Json>; revs: Record<string, number> } | null> {
  try {
    const read = await store.readBatched<ConfigRead['content']>(entityId)
    return { config: read.content.payload.config ?? {}, revs: read.content.key_revs ?? {} }
  } catch {
    return null
  }
}

/** The Workspace's shared settings (needs an open Workspace: StoreContext). */
export function useWorkspaceSettings(): WorkspaceSettingsState {
  const store = useStore()
  const version = useVersion('e:root')
  const readOnly = useReadOnlyReason()
  const load = useCallback(async () => {
    const info = store.session.info()
    // the session's opening answer until the root changes; a member who cannot read the root keeps that one
    const fromInfo = { config: (info.settings ?? {}) as Record<string, Json>, revs: { config: info.settings_rev ?? 0 } as Record<string, number> }
    if (version === 0) return fromInfo
    return (await readConfig(store, 'root')) ?? fromInfo
  }, [store, version])
  const read = useLoad(load, version)
  const caps = store.session.info().capabilities as Capability[]
  const writeReason = !caps.includes('manage') ? aiwsT('aiworkspace.settings.needManage') : readOnly
  const update = useCallback(async (changes: Record<string, Json | null>) => {
    const latest = (await readConfig(store, 'root'))?.revs ?? null
    return store.submit({ editId: 'workspace:settings', label: aiwsT('aiworkspace.settings.workspaceLabel'), operations: [setConfigOp('root', changes, latest)] })
  }, [store])
  return { value: (read.data?.config ?? {}) as WorkspaceSettings, rev: read.data?.revs.config ?? 0, loading: read.loading, error: read.error, writeReason, update }
}

/** A canvas's own toolbar layer (`config.toolbar` on the Surface). */
export function useSurfaceToolbar(surfaceId: string | null): SurfaceToolbarState {
  const store = useStore()
  const surface = useEntity(surfaceId ?? '')
  const version = useVersion(`e:${surfaceId ?? ''}`)
  const readOnly = useReadOnlyReason()
  const load = useCallback(async () => (surfaceId ? readConfig(store, surfaceId) : null), [store, surfaceId])
  const read = useLoad(load, version)
  const writeReason = !surfaceId || !surface ? aiwsT('aiworkspace.settings.noCanvas')
    : !surface.capabilities.includes('structure') ? aiwsT('aiworkspace.settings.needStructure') : readOnly
  const set = useCallback(async (toolbar: Json | null) => {
    if (!surfaceId) throw new Error(aiwsT('aiworkspace.settings.noCanvas'))
    const latest = (await readConfig(store, surfaceId))?.revs ?? null
    return store.submit({ editId: `surface:${surfaceId}:toolbar`, label: aiwsT('aiworkspace.settings.surfaceToolbarLabel'), operations: [setConfigOp(surfaceId, { toolbar }, latest)] })
  }, [store, surfaceId])
  const value = (read.data?.config.toolbar ?? null) as Json | null
  return { value, loading: read.loading, error: read.error, writeReason, set }
}

export type WorkspaceRole = 'owner' | 'editor' | 'commenter' | 'viewer'

/** The caller's role on a Workspace card (§9.2 “我的角色”), from its Workspace-level capabilities (`ws.list`):
 * `manage` counts as the owner's role only for the owner itself (`isOwner`). */
export function roleOf(capabilities: readonly string[], isOwner: boolean): WorkspaceRole {
  if (isOwner) return 'owner'
  if (capabilities.some((c) => c === 'update' || c === 'append' || c === 'delete' || c === 'structure')) return 'editor'
  if (capabilities.includes('comment')) return 'commenter'
  return 'viewer'
}
