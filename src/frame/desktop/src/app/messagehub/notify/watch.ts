import { isNotifyClosed, notifyTaskResult, TASK_ERR_ALREADY_COMPLETED, TASK_ERR_REVISION_CONFLICT, validateNotifyAnswer, type NotifyAnswer, type NotifyView } from './model'

/** What answering needs of a notification's TaskMgr task. */
export interface NotifyTaskState {
  revision: number
  phase: string
  result?: unknown
}

export type NotifyReadFailure = 'missing' | 'denied' | 'error'

/**
 * msg-center `notify.*` plus the TaskMgr side of answering an `input`. Two
 * implementations: the msg-center / TaskMgr client (`api.ts`) and the
 * scripted mock (`mock/notify.ts`).
 */
/** `notify.list` filters the status-bar bell uses; newest first. */
export interface NotifyListQuery {
  status?: NotifyView['status'][]
  limit?: number
}

export interface NotifyBackend {
  get(notifyId: string): Promise<NotifyView>
  /** The caller's own notifications, newest first. */
  list(query: NotifyListQuery): Promise<NotifyView[]>
  ack(notifyId: string): Promise<NotifyView>
  dismiss(notifyId: string): Promise<NotifyView>
  getTask(taskId: string): Promise<NotifyTaskState>
  commitResult(taskId: string, result: unknown, expectedRevision: number): Promise<void>
  /** TaskMgr error code of a failed call (`task_already_completed`, `revision_conflict`, …). */
  taskErrorCode(error: unknown): string | null
  failure(error: unknown): NotifyReadFailure
  /** Calls `onHint` when the notification may have changed (`/obj/taskmgr/<task_id>/changed` for an `input`): only a hint to read again. */
  subscribe(notifyId: string, taskId: string | undefined, onHint: () => void): () => void
  /** The logged-in user as TaskMgr names a submitter. */
  submitter(): Promise<string>
  now(): number
}

/** The state of one notification: every card of it reads this same entry. */
export interface NotifyEntry {
  notifyId: string
  status: 'loading' | 'ready' | NotifyReadFailure
  view?: NotifyView
  /** A read failed after a good one, so `view` may be outdated. */
  stale: boolean
}

/** `committed`: this answer is the result. `already_closed`: someone else was first, or the notification ended. */
export type NotifyCommit = 'committed' | 'already_closed'

/**
 * Commits an answer to the task of an `input` (`commit_notify_answer`): read
 * the latest snapshot, write with `expected_revision`; first wins. A lost
 * race is not an error: the caller re-reads and shows the final result.
 */
export async function commitNotifyAnswer(backend: Pick<NotifyBackend, 'getTask' | 'commitResult' | 'taskErrorCode'>, taskId: string, answer: NotifyAnswer, submittedBy: string, nowMs: number): Promise<NotifyCommit> {
  for (let attempt = 0; attempt < 3; attempt++) {
    const task = await backend.getTask(taskId)
    if (task.phase === 'Terminal' || (task.result !== undefined && task.result !== null)) return 'already_closed'
    try {
      await backend.commitResult(taskId, notifyTaskResult(answer, submittedBy, nowMs), task.revision)
      return 'committed'
    } catch (error) {
      const code = backend.taskErrorCode(error)
      if (code === TASK_ERR_ALREADY_COMPLETED) return 'already_closed'
      if (code === TASK_ERR_REVISION_CONFLICT) continue
      throw error
    }
  }
  if ((await backend.getTask(taskId)).phase === 'Terminal') return 'already_closed'
  throw Error('task_keeps_changing')
}

export interface NotifyWatchOptions {
  /** Delay before the n-th poll of an open notification; the last one repeats. */
  pollDelaysMs?: number[]
  /** Polling stops after this many polls; hints and `resync` still re-read. */
  maxPolls?: number
  maxConcurrentReads?: number
}

interface Watch {
  refs: number
  polls: number
  timer?: ReturnType<typeof setTimeout>
  close?: () => void
}

/**
 * Shared cache of notification states, keyed by `notify_id`. The home record
 * and every copy of a notification read one entry, so all of its cards show
 * the same state. A watched notification is read once, then again on a hint
 * (its task's kevent) or a bounded backoff poll until it is closed; a closed
 * one is never read again unless asked.
 */
export class NotifyWatch {
  private readonly backend: NotifyBackend
  private readonly pollDelaysMs: number[]
  private readonly maxPolls: number
  private readonly maxConcurrentReads: number
  private entries = new Map<string, NotifyEntry>()
  private watches = new Map<string, Watch>()
  private reading = new Map<string, { again: boolean; done: Promise<void> }>()
  private listeners = new Set<() => void>()
  private activeReads = 0
  private waitingReads: Array<() => void> = []
  private changes = 0

  constructor(backend: NotifyBackend, options: NotifyWatchOptions = {}) {
    this.backend = backend
    this.pollDelaysMs = options.pollDelaysMs ?? [3_000, 5_000, 10_000, 20_000, 30_000]
    this.maxPolls = options.maxPolls ?? 120
    this.maxConcurrentReads = options.maxConcurrentReads ?? 4
  }

  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  /** Changes so far: a snapshot for views that read many entries. */
  version = () => this.changes
  entry(notifyId: string): NotifyEntry | undefined { return this.entries.get(notifyId) }
  /** A fresh `notify.list`, outside the per-notification cache. */
  list(query: NotifyListQuery): Promise<NotifyView[]> { return this.backend.list(query) }

  private emit() {
    this.changes++
    this.listeners.forEach(listener => listener())
  }

  private set(entry: NotifyEntry) {
    this.entries.set(entry.notifyId, entry)
    this.emit()
  }

  private viewer?: string
  private viewerLoading = false
  /** The logged-in user as a submitter (`result.submitted_by`), once known; the first call starts loading it. */
  me(): string | undefined {
    if (this.viewer === undefined && !this.viewerLoading) {
      this.viewerLoading = true
      void this.backend.submitter().then(name => { this.viewer = name; this.emit() }, () => { this.viewerLoading = false })
    }
    return this.viewer
  }

  private setView(view: NotifyView) {
    this.set({ notifyId: view.notify_id, status: 'ready', view, stale: false })
    this.afterRead(view.notify_id)
  }

  /** Keeps the notification current until released. */
  watch(notifyId: string): () => void {
    let watch = this.watches.get(notifyId)
    if (!watch) { watch = { refs: 0, polls: 0 }; this.watches.set(notifyId, watch) }
    const entry = watch
    entry.refs++
    if (entry.refs === 1) {
      const known = this.entries.get(notifyId)
      if (!known) this.set({ notifyId, status: 'loading', stale: false })
      if (known?.view && isNotifyClosed(known.view.status)) this.afterRead(notifyId)
      else void this.refresh(notifyId)
    }
    let released = false
    return () => {
      if (released) return
      released = true
      entry.refs--
      if (entry.refs > 0) return
      if (entry.timer) clearTimeout(entry.timer)
      entry.close?.()
      this.watches.delete(notifyId)
    }
  }

  /** Reads the notification now and resolves to its entry (a link to it needs its `home` record). */
  async load(notifyId: string): Promise<NotifyEntry> {
    if (!this.entries.has(notifyId)) this.set({ notifyId, status: 'loading', stale: false })
    await this.refresh(notifyId)
    return this.entries.get(notifyId) ?? { notifyId, status: 'error', stale: false }
  }

  /** Reads the notifications not read yet (a filter needs every status), without watching them. */
  ensure(notifyIds: readonly string[]) {
    for (const notifyId of notifyIds) {
      if (this.entries.has(notifyId)) continue
      this.set({ notifyId, status: 'loading', stale: false })
      void this.refresh(notifyId)
    }
  }

  /** Reads again every watched notification that is still open (back online, page visible again). */
  resync() {
    for (const [notifyId, watch] of this.watches) {
      const view = this.entries.get(notifyId)?.view
      if (view && isNotifyClosed(view.status)) continue
      watch.polls = 0
      void this.refresh(notifyId)
    }
  }

  /** Reads the notification now; concurrent calls share one read and a call during a read reads once more. */
  refresh(notifyId: string): Promise<void> {
    const pending = this.reading.get(notifyId)
    if (pending) { pending.again = true; return pending.done }
    const state = { again: false, done: Promise.resolve() }
    state.done = (async () => {
      do {
        state.again = false
        await this.read(notifyId)
      } while (state.again)
    })().finally(() => { this.reading.delete(notifyId) })
    this.reading.set(notifyId, state)
    return state.done
  }

  private async slot<T>(work: () => Promise<T>): Promise<T> {
    if (this.activeReads >= this.maxConcurrentReads) await new Promise<void>(resolve => this.waitingReads.push(resolve))
    this.activeReads++
    try { return await work() } finally {
      this.activeReads--
      this.waitingReads.shift()?.()
    }
  }

  private async read(notifyId: string) {
    try {
      const view = await this.slot(() => this.backend.get(notifyId))
      const previous = this.entries.get(notifyId)
      if (previous?.status === 'ready' && previous.view && JSON.stringify(previous.view) === JSON.stringify(view)) this.afterRead(notifyId)
      else this.setView(view)
    } catch (error) {
      const failure = this.backend.failure(error)
      const previous = this.entries.get(notifyId)
      if (failure === 'error' && previous?.view) this.set({ ...previous, stale: true })
      else this.set({ notifyId, status: failure, stale: false })
      if (failure === 'error') this.schedulePoll(notifyId)
    }
  }

  /** Subscribes and polls an open watched notification; a closed one is left alone. */
  private afterRead(notifyId: string) {
    const watch = this.watches.get(notifyId)
    if (!watch) return
    const view = this.entries.get(notifyId)?.view
    if (view && isNotifyClosed(view.status)) {
      if (watch.timer) clearTimeout(watch.timer)
      watch.timer = undefined
      watch.close?.()
      watch.close = undefined
      return
    }
    if (view && !watch.close) watch.close = this.backend.subscribe(notifyId, view.task_id, () => { void this.refresh(notifyId) })
    this.schedulePoll(notifyId)
  }

  private schedulePoll(notifyId: string) {
    const watch = this.watches.get(notifyId)
    if (!watch || watch.polls >= this.maxPolls) return
    if (watch.timer) clearTimeout(watch.timer)
    const delay = this.pollDelaysMs[Math.min(watch.polls, this.pollDelaysMs.length - 1)]
    watch.timer = setTimeout(() => {
      watch.timer = undefined
      watch.polls++
      void this.refresh(notifyId)
    }, delay)
  }

  private async current(notifyId: string): Promise<NotifyView> {
    const view = this.entries.get(notifyId)?.view
    if (view) return view
    await this.refresh(notifyId)
    const loaded = this.entries.get(notifyId)?.view
    if (!loaded) throw Error(this.entries.get(notifyId)?.status ?? 'error')
    return loaded
  }

  /** `notice`: mark read. */
  async ack(notifyId: string) {
    this.setView(await this.backend.ack(notifyId))
  }

  /** `notice`: dismiss (archive every record). */
  async dismiss(notifyId: string) {
    this.setView(await this.backend.dismiss(notifyId))
  }

  /**
   * Answers (or dismisses) an `input` against its task, first one wins; a
   * `notice` is acknowledged or dismissed. Either way the notification is
   * read again, so every card shows the final state.
   */
  async answer(notifyId: string, answer: NotifyAnswer): Promise<NotifyCommit> {
    const view = await this.current(notifyId)
    if (view.kind === 'notice') {
      if (answer.dismiss) await this.dismiss(notifyId)
      else await this.ack(notifyId)
      return 'committed'
    }
    if (isNotifyClosed(view.status) || !view.task_id) {
      await this.refresh(notifyId)
      return 'already_closed'
    }
    const invalid = validateNotifyAnswer(answer, view.prompt)
    if (invalid) throw Error(invalid)
    const submittedBy = await this.backend.submitter()
    try {
      return await commitNotifyAnswer(this.backend, view.task_id, answer, submittedBy, this.backend.now())
    } finally {
      await this.refresh(notifyId)
    }
  }
}
