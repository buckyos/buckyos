/* The work log (UI 框架 PRD §4.5, §12, K3): what happened in this Workspace, newest first — Commits (people, Agents and
 * macros marked apart), wish and macro runs (start, end), imports, shows and public classes, and, for managers, the
 * permission changes — as the service reads it for this caller (`ws.work_log`): only what it may read now.
 *
 * `WorkLogFeed` holds one filtered view: the newest page (50 by default), older pages added by `loadMore`, refreshed when
 * a Commit arrives or this window's runs move, and every 15 s while the page is visible (another window's run starting
 * makes no Commit). `useWorkLog` mounts one. `describeWorkLogEntry` says what an entry is in the reader's language; a
 * Commit's own message is the author's text, kept as it was written. */

import { useEffect, useMemo, useSyncExternalStore } from 'react'
import { describeError } from '../api/session'
import type { WorkLogEntry, WorkLogFilter, WorkLogPage } from '../api/types'
import { isGuestPrincipal } from '../api/transport'
import { aiwsT } from '../i18n'
import { Emitter } from './emitter'
import { guestLabel } from './guest'
import { useStore } from './hooks'
import type { WorkspaceStore } from './store'

export const WORK_LOG_PAGE = 50
const REFRESH_MS = 15_000
const CHANGE_DEBOUNCE_MS = 800

export interface WorkLogState {
  entries: readonly WorkLogEntry[]
  /** The first page is being read (and nothing was read yet). */
  loading: boolean
  /** Older entries are being read. */
  loadingMore: boolean
  /** There are older entries (`loadMore`). */
  more: boolean
  /** Why the latest read failed (offline: the service is needed), or null. */
  error: string | null
}

const keyOf = (e: WorkLogEntry) => `${e.at}|${e.id}`

export class WorkLogFeed {
  private readonly store: WorkspaceStore
  readonly filter: WorkLogFilter
  private readonly limit: number
  private readonly emitter = new Emitter()
  readonly subscribe = this.emitter.subscribe
  private state: WorkLogState = { entries: [], loading: true, loadingMore: false, more: false, error: null }
  /** Where `loadMore` goes on from. */
  private next: string | null = null
  private generation = 0
  private offs: (() => void)[] = []
  private timer: number | null = null
  private debounce: number | null = null

  constructor(store: WorkspaceStore, filter: WorkLogFilter = 'all', limit = WORK_LOG_PAGE) {
    this.store = store
    this.filter = filter
    this.limit = limit
  }

  snapshot = (): WorkLogState => this.state

  private set(patch: Partial<WorkLogState>) {
    this.state = { ...this.state, ...patch }
    this.emitter.emit()
  }

  private read(before: string | null): Promise<WorkLogPage> {
    return this.store.session.workLog({ before, limit: this.limit, filter: this.filter })
  }

  /** Read the newest page again; older pages already loaded stay below it. */
  async refresh(): Promise<void> {
    const generation = ++this.generation
    try {
      const page = await this.read(null)
      if (generation !== this.generation) return
      const oldest = page.entries.length > 0 ? keyOf(page.entries[page.entries.length - 1]) : null
      const loadedMore = this.state.entries.length > this.limit
      if (loadedMore && oldest && page.more) {
        // keep what the reader paged to: the entries older than the new first page, and where they go on from
        const older = this.state.entries.filter((e) => keyOf(e) < oldest)
        this.set({ entries: [...page.entries, ...older], loading: false, error: null })
      } else {
        this.next = page.next_before
        this.set({ entries: page.entries, loading: false, more: page.more, error: null })
      }
    } catch (error) {
      if (generation === this.generation) this.set({ loading: false, error: describeError(error) })
    }
  }

  /** Add the next older page. */
  async loadMore(): Promise<void> {
    if (!this.state.more || this.state.loadingMore || !this.next) return
    const generation = this.generation
    this.set({ loadingMore: true })
    try {
      const page = await this.read(this.next)
      if (generation !== this.generation) { this.set({ loadingMore: false }); return }
      const seen = new Set(this.state.entries.map((e) => e.id))
      this.next = page.next_before
      this.set({ entries: [...this.state.entries, ...page.entries.filter((e) => !seen.has(e.id))], more: page.more, loadingMore: false, error: null })
    } catch (error) {
      this.set({ loadingMore: false, error: describeError(error) })
    }
  }

  /** Follow the Workspace: refresh on Commits, on this window's run changes and every 15 s while visible. */
  start(): () => void {
    const soon = () => {
      if (this.debounce !== null) window.clearTimeout(this.debounce)
      this.debounce = window.setTimeout(() => { this.debounce = null; void this.refresh() }, CHANGE_DEBOUNCE_MS)
    }
    const tick = () => { if (document.visibilityState === 'visible') void this.refresh() }
    this.offs = [this.store.session.subscribeChanges(soon), this.store.wish.subscribe(soon), this.store.session.subscribeStatus(() => { if (this.store.session.status().kind === 'live') soon() })]
    this.timer = window.setInterval(tick, REFRESH_MS)
    void this.refresh()
    return () => this.stop()
  }

  stop() {
    for (const off of this.offs.splice(0)) off()
    if (this.timer !== null) { window.clearInterval(this.timer); this.timer = null }
    if (this.debounce !== null) { window.clearTimeout(this.debounce); this.debounce = null }
    this.generation += 1
  }
}

export interface WorkLogView extends WorkLogState {
  loadMore: () => void
  refresh: () => void
}

/** The work log of the open Workspace with `filter` (全部 / 我的 / Agent 与宏), newest first, kept current while mounted. */
export function useWorkLog({ filter = 'all', limit = WORK_LOG_PAGE, enabled = true }: { filter?: WorkLogFilter; limit?: number; enabled?: boolean } = {}): WorkLogView {
  const store = useStore()
  const feed = useMemo(() => new WorkLogFeed(store, filter, limit), [store, filter, limit])
  useEffect(() => (enabled ? feed.start() : undefined), [feed, enabled])
  const state = useSyncExternalStore(feed.subscribe, feed.snapshot)
  return { ...state, loadMore: () => void feed.loadMore(), refresh: () => void feed.refresh() }
}

const targetText = (e: WorkLogEntry) => e.target?.title?.trim() || e.target?.name || (e.target ? e.target.entity_id : aiwsT('aiworkspace.activity.untitled'))

/** Who did it, as the log shows it: a guest's short label, an Agent or a macro marked as such, the service as 系统. */
export function workLogActor(entry: WorkLogEntry): string {
  const name = isGuestPrincipal(entry.actor) ? guestLabel(entry.actor) : entry.actor
  switch (entry.actor_kind) {
    case 'agent': return aiwsT('aiworkspace.activity.actor.agent', { name })
    case 'macro': return aiwsT('aiworkspace.activity.actor.macro', { name })
    case 'system': return aiwsT('aiworkspace.activity.actor.system')
    default: return name
  }
}

const RUN_END: Record<string, string> = {
  succeeded: 'aiworkspace.activity.run.succeeded', waiting_confirmation: 'aiworkspace.activity.run.waiting', applying: 'aiworkspace.activity.run.applying',
  cancelled: 'aiworkspace.activity.run.cancelled',
}

const ACL: Record<string, string> = {
  'acl.set': 'aiworkspace.activity.acl.set', 'acl.set_anonymous': 'aiworkspace.activity.acl.anonymous', 'acl.members': 'aiworkspace.activity.acl.members',
  'acl.set_group': 'aiworkspace.activity.acl.group', 'acl.delete_group': 'aiworkspace.activity.acl.deleteGroup', 'acl.bind_macro': 'aiworkspace.activity.acl.bindMacro',
  'acl.load_design': 'aiworkspace.activity.acl.set', 'macro.changed': 'aiworkspace.activity.acl.macroChanged', 'visibility.widened': 'aiworkspace.activity.acl.widened',
  'hosted_exec.set': 'aiworkspace.activity.acl.hostedExec', 'entry.create': 'aiworkspace.activity.acl.entry', 'entry.close': 'aiworkspace.activity.acl.entry',
  'entry.join': 'aiworkspace.activity.acl.members', publish: 'aiworkspace.activity.acl.publish', unpublish: 'aiworkspace.activity.acl.publish',
}

/** The service's own Commit messages (English constants), said in the reader's language. */
function serviceMessage(message: string): string | null {
  if (message === 'Write back wish block analysis') return aiwsT('aiworkspace.activity.commit.wishAnalysis')
  if (message.startsWith('Apply wish block results') || / \(simulated result\)$/.test(message)) return aiwsT('aiworkspace.activity.commit.wishResults')
  if (message.startsWith('refresh from source')) return aiwsT('aiworkspace.activity.commit.sourceRefresh')
  if (message === 'purge deleted entities') return aiwsT('aiworkspace.activity.commit.purge')
  if (message === 'create from template') return aiwsT('aiworkspace.activity.commit.fromTemplate')
  return null
}

/** What an entry says happened, in the reader's language (a person's Commit message as written). */
export function describeWorkLogEntry(entry: WorkLogEntry): string {
  switch (entry.kind) {
    case 'commit': {
      if (entry.undoes) return aiwsT('aiworkspace.activity.commit.undo')
      const message = entry.message?.trim()
      if (message) return serviceMessage(message) ?? message
      return entry.target ? aiwsT('aiworkspace.activity.commit.changed', { target: targetText(entry) }) : aiwsT('aiworkspace.activity.commit.changedSome')
    }
    case 'import':
      return entry.message?.trim() || aiwsT('aiworkspace.activity.import.package')
    case 'run': {
      const what = aiwsT(`aiworkspace.activity.run.${entry.stage === 'analyze' || entry.stage === 'code' ? entry.stage : 'run'}`, { target: targetText(entry) })
      if (entry.phase === 'start') return aiwsT('aiworkspace.activity.run.started', { what })
      return aiwsT(RUN_END[entry.state ?? ''] ?? 'aiworkspace.activity.run.failed', { what })
    }
    case 'show':
      return aiwsT(entry.phase === 'end' ? 'aiworkspace.activity.show.end' : 'aiworkspace.activity.show.start')
    case 'session':
      return aiwsT(entry.phase === 'end' ? 'aiworkspace.activity.session.end' : 'aiworkspace.activity.session.start', { title: entry.title ?? targetText(entry) })
    case 'acl':
      return ACL[entry.action ?? ''] ? aiwsT(ACL[entry.action ?? '']) : aiwsT('aiworkspace.activity.acl.other', { action: entry.action ?? '' })
    default:
      return entry.message ?? ''
  }
}

/** Where “locate” goes: the canvas (with the Block or object to focus), or the object in data mode; null when the
 * entry names nothing the reader can open. */
export function workLogLocation(entry: WorkLogEntry): { surfaceId: string | null; entityId: string } | null {
  const target = entry.target
  if (!target || target.deleted) return target?.surface_id ? { surfaceId: target.surface_id, entityId: target.surface_id } : null
  return { surfaceId: target.surface_id ?? null, entityId: target.entity_id }
}
