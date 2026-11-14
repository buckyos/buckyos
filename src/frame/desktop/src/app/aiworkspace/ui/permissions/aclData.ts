/* The permission calls of one open workspace (UI 框架 PRD §8, K8): the `acl.*` client methods bound to the workspace,
 * the permissions' revision the views reload by (权限需求 R4: the change stream's `acl_rev`, and the revision every
 * write answers), and the names of principals (Zone users from control-panel, system user groups from the service).
 * Permission operations run online only: never queued, never on the document's undo stack. */

import { useCallback, useMemo, useSyncExternalStore } from 'react'
import { fetchUserList } from '../../../../api/user_mgr'
import { unwrap } from '../../api/client'
import type { WorkspaceSession } from '../../api/session'
import type { AclAuditItem, AclChange, Json, Result, ShareEntry, Subject, WorkspaceInfo } from '../../api/types'
import { useLoad, useSessionStatus, useStore, type Loaded } from '../../state/hooks'
import { useShell } from '../shell/shellContext'

// ---- the permissions' revision

interface RevState { local: number; listeners: Set<() => void>; pollers: number; timer: number | null }
const revs = new WeakMap<WorkspaceSession, RevState>()

function revState(session: WorkspaceSession): RevState {
  let state = revs.get(session)
  if (!state) { state = { local: 0, listeners: new Set(), pollers: 0, timer: null }; revs.set(session, state) }
  return state
}

/** A revision learnt from a write's answer: the views reload at once, without waiting for the change stream. */
export function noteAclRev(session: WorkspaceSession, rev: number | undefined) {
  const state = revState(session)
  if (rev === undefined || rev <= Math.max(state.local, session.info().acl_rev ?? 0)) return
  state.local = rev
  for (const listener of [...state.listeners]) listener()
}

function currentRev(session: WorkspaceSession): number {
  return Math.max(session.info().acl_rev ?? 0, revState(session).local)
}

/** A session that does not follow the permissions itself (a replica window) is asked now and then. */
async function pollRev(session: WorkspaceSession) {
  try {
    const info = await session.call<Result<WorkspaceInfo>>('ws.get_info', { workspace_id: session.workspaceId })
    if (info.ok) noteAclRev(session, info.acl_rev)
  } catch { /* offline: the next poll asks again */ }
}

/** The permissions' revision: it moves whenever an entry, a group, a member, anonymous access or a binding changes. */
export function useAclRev(): number {
  const { session } = useStore()
  const subscribe = useCallback((listener: () => void) => {
    const state = revState(session)
    state.listeners.add(listener)
    const off = session.subscribeAcl ? session.subscribeAcl(listener) : session.subscribeStatus(listener)
    if (!session.subscribeAcl && ++state.pollers === 1) state.timer = window.setInterval(() => { void pollRev(session) }, 15000)
    return () => {
      state.listeners.delete(listener)
      off()
      if (!session.subscribeAcl && --state.pollers === 0 && state.timer !== null) { window.clearInterval(state.timer); state.timer = null }
    }
  }, [session])
  return useSyncExternalStore(subscribe, () => currentRev(session))
}

// ---- reads shared by the views of one revision

const reads = new WeakMap<WorkspaceSession, Map<string, { rev: number; promise: Promise<unknown> }>>()

function cachedRead<T>(session: WorkspaceSession, key: string, rev: number, load: () => Promise<T>): Promise<T> {
  let map = reads.get(session)
  if (!map) { map = new Map(); reads.set(session, map) }
  const known = map.get(key)
  if (known && known.rev === rev) return known.promise as Promise<T>
  const promise = load()
  map.set(key, { rev, promise })
  promise.catch(() => { if (map.get(key)?.promise === promise) map.delete(key) })
  return promise
}

/** A permission read, loaded again whenever the revision moves; views asking the same `key` share one request.
 * `key` null: nothing is loaded. */
export function useAclRead<T>(key: string | null, load: () => Promise<T>): Loaded<T> {
  const { session } = useStore()
  const rev = useAclRev()
  // `key` names what `load` reads: a new closure of the same read is not a new read
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const run = useCallback(() => (key === null ? Promise.resolve(undefined as T) : cachedRead(session, key, rev, load)), [session, key, rev])
  return useLoad<T>(run, `${key}:${rev}`)
}

// ---- the calls

export type AclApi = ReturnType<typeof makeApi>

function makeApi(shell: ReturnType<typeof useShell>, session: WorkspaceSession) {
  const client = shell.client
  const ws = { workspace_id: session.workspaceId }
  const wrote = <T extends { acl_rev?: number }>(result: Result<T>) => { const r = unwrap(result); noteAclRev(session, r.acl_rev); return r }
  return {
    ws,
    client,
    // reads
    all: async () => unwrap(await client.aclQueryAll(ws)),
    groups: async () => unwrap(await client.aclGroups(ws)),
    entity: async (resource: string) => unwrap(await client.aclQueryEntity(ws, resource)),
    subject: async (subject?: string) => unwrap(await client.aclQuerySubject(ws, subject)),
    explain: async (resource: string, principal?: string) => unwrap(await client.aclExplain(ws, resource, principal)),
    entries: async (): Promise<ShareEntry[]> => unwrap(await client.wsListEntries(ws)).entries,
    /** The whole operation record, oldest first (at most `max` records). */
    audit: async (max = 5000): Promise<AclAuditItem[]> => {
      const out: AclAuditItem[] = []
      for (let after = 0; out.length < max;) {
        const page = unwrap(await client.aclAudit(ws, after, 1000)).items
        out.push(...page)
        if (page.length < 1000) break
        after = page[page.length - 1].seq
      }
      return out
    },
    // writes (each answers the new revision)
    set: async (changes: AclChange[]) => wrote(await client.aclSet(ws, changes)),
    setGroup: async (groupId: string, title?: string, description?: string) => wrote(await client.aclSetGroup(ws, groupId, title, description)),
    deleteGroup: async (groupId: string) => wrote(await client.aclDeleteGroup(ws, groupId)),
    members: async (groupId: string, add: string[], remove: string[] = []) => wrote(await client.aclMembers(ws, groupId, add, remove)),
    setAnonymous: async (enabled: boolean) => wrote(await client.aclSetAnonymous(ws, enabled)),
    bindMacro: async (wish: string, groupId: string | null, mode: 'union' | 'intersection') => wrote(await client.aclBindMacro(ws, wish, groupId, mode)),
  }
}

/** The `acl.*` calls of this workspace, and whether they can be made now (online). */
export function useAcl(): { api: AclApi; online: boolean } {
  const shell = useShell()
  const { session } = useStore()
  const status = useSessionStatus()
  const api = useMemo(() => makeApi(shell, session), [shell, session])
  return { api, online: status.kind === 'live' }
}

// ---- names

/** Who a principal or member spec is, by name (§8.2): Zone users from control-panel, system user groups and Agents from
 * the service's subject list. Unknown ones stay their raw id. */
export interface Directory {
  candidates: Subject[]
  agents: ReadonlySet<string>
  userName(id: string): string | null
  sysGroupName(id: string): string | null
}

const subjectLists = new WeakMap<WorkspaceSession, Promise<Subject[]>>()
let userNames: { at: number; promise: Promise<Map<string, string>> } | null = null

function zoneUserNames(): Promise<Map<string, string>> {
  if (userNames && Date.now() - userNames.at < 60_000) return userNames.promise
  const promise = fetchUserList().then(({ data }) => new Map((data?.users ?? []).filter((u) => u.show_name && u.show_name !== u.user_id).map((u) => [u.user_id, u.show_name])), () => new Map<string, string>())
  userNames = { at: Date.now(), promise }
  return promise
}

const EMPTY_DIRECTORY: Directory = { candidates: [], agents: new Set(), userName: () => null, sysGroupName: () => null }

export function useDirectory(): Directory {
  const { session } = useStore()
  const load = useCallback(async (): Promise<Directory> => {
    let subjects = subjectLists.get(session)
    if (!subjects) {
      subjects = session.listSubjects().catch(() => [] as Subject[])
      subjectLists.set(session, subjects)
    }
    const [list, names] = await Promise.all([subjects, zoneUserNames()])
    const groups = new Map(list.filter((s) => s.subject.startsWith('group:')).map((s) => [s.subject.slice(6), s.name ?? null]))
    return {
      candidates: list,
      agents: new Set(list.filter((s) => s.kind === 'agent').map((s) => s.subject)),
      userName: (id) => names.get(id) ?? null,
      sysGroupName: (id) => groups.get(id) ?? null,
    }
  }, [session])
  const loaded = useLoad(load)
  return loaded.data ?? EMPTY_DIRECTORY
}

/** The current value of `info()` re-read when the permissions move. */
export function useWorkspaceCaps(): { capabilities: string[]; owner: boolean; anonymous: boolean } {
  const { session } = useStore()
  useAclRev()
  const info = session.info()
  return { capabilities: info.capabilities, owner: info.owner === true, anonymous: info.anonymous === true }
}

/** Raw JSON helpers for audit records. */
export function field(value: Json | undefined, ...path: string[]): Json | undefined {
  let cur: Json | undefined = value
  for (const key of path) {
    if (!cur || typeof cur !== 'object' || Array.isArray(cur)) return undefined
    cur = (cur as Record<string, Json>)[key]
  }
  return cur
}
