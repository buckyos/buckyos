import type { TaskWatchDetail, TaskWatchEvent, TaskWatchNode, TaskWatchSource } from '../../../../api/task_mgr'

/**
 * `denied` / `missing`: the reader may not see the task, or it was cleaned
 * up. `error`: no read has succeeded yet. `stale`: a read failed after a good
 * one, so `task` may be outdated.
 */
export interface TaskSnapshot {
  taskId: string
  status: 'loading' | 'ready' | 'denied' | 'missing' | 'error'
  task?: TaskWatchDetail
  stale: boolean
}

/** The loaded direct children of one task; `nextCursor` means more exist. */
export interface TaskChildren {
  status: 'loading' | 'ready' | 'error'
  items: TaskWatchNode[]
  nextCursor?: string
}

export interface TaskEvents {
  status: 'loading' | 'ready' | 'error'
  items: TaskWatchEvent[]
}

export interface TaskWatchOptions {
  /** Delay before the n-th unchanged (or failed) poll; the last one repeats. */
  pollDelaysMs?: number[]
  /** Polling stops after this many reads without a new revision; events and `resync` still re-read. */
  maxUnchangedPolls?: number
  maxFailures?: number
  readTimeoutMs?: number
  pageSize?: number
  maxCached?: number
  maxConcurrentReads?: number
}

interface Watch {
  refs: number
  unchanged: number
  failures: number
  timer?: ReturnType<typeof setTimeout>
  close?: () => void
}

interface TreeWatch {
  refs: number
  /** Nodes below the root whose children were loaded by expanding them. */
  parents: Set<string>
  timer?: ReturnType<typeof setTimeout>
  close?: () => void
}

const TREE_HINT_DEBOUNCE_MS = 250

/**
 * Shared task cache of the visible bubbles, keyed by task id and
 * deduplicated by revision. A watched task is read once, then re-read on a
 * `/task_mgr/<id>` event or a bounded backoff poll until its terminal
 * snapshot (and its direct children's) is cached; after that nothing is
 * subscribed or polled. Deeper levels and events are only read while a tree
 * is watched (an expanded bubble or the message details) and are dropped
 * when the last watcher leaves.
 */
export class TaskWatch {
  private readonly source: TaskWatchSource
  private readonly pollDelaysMs: number[]
  private readonly maxUnchangedPolls: number
  private readonly maxFailures: number
  private readonly readTimeoutMs: number
  private readonly pageSize: number
  private readonly maxCached: number
  private readonly maxConcurrentReads: number
  private snapshots = new Map<string, TaskSnapshot>()
  private childPages = new Map<string, TaskChildren>()
  private eventLists = new Map<string, TaskEvents>()
  private watches = new Map<string, Watch>()
  private trees = new Map<string, TreeWatch>()
  private reading = new Map<string, { again: boolean; done: Promise<void> }>()
  private listeners = new Set<() => void>()
  private activeReads = 0
  private waitingReads: Array<() => void> = []

  constructor(source: TaskWatchSource, options: TaskWatchOptions = {}) {
    this.source = source
    this.pollDelaysMs = options.pollDelaysMs ?? [2_000, 3_000, 5_000, 8_000, 13_000, 20_000, 30_000]
    this.maxUnchangedPolls = options.maxUnchangedPolls ?? 120
    this.maxFailures = options.maxFailures ?? 6
    this.readTimeoutMs = options.readTimeoutMs ?? 10_000
    this.pageSize = options.pageSize ?? 20
    this.maxCached = options.maxCached ?? 300
    this.maxConcurrentReads = options.maxConcurrentReads ?? 4
  }

  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  snapshot(taskId: string): TaskSnapshot | undefined { return this.snapshots.get(taskId) }
  children(taskId: string): TaskChildren | undefined { return this.childPages.get(taskId) }
  events(rootId: string): TaskEvents | undefined { return this.eventLists.get(rootId) }
  /** Whether the task is still subscribed or polled (false once its terminal snapshot is cached). */
  isLive(taskId: string): boolean { const watch = this.watches.get(taskId); return !!watch && (!!watch.close || watch.timer !== undefined) }

  /** Keeps the task's summary (the task and its first page of direct children) current until released. */
  watch(taskId: string): () => void {
    let watch = this.watches.get(taskId)
    if (!watch) { watch = { refs: 0, unchanged: 0, failures: 0 }; this.watches.set(taskId, watch) }
    const entry = watch
    entry.refs++
    if (entry.refs === 1) {
      if (!this.snapshots.has(taskId)) this.setSnapshot({ taskId, status: 'loading', stale: false })
      if (!this.isClosed(taskId)) {
        entry.close = this.source.subscribe('task', taskId, () => { void this.read(taskId) })
        void this.read(taskId)
      }
    }
    let released = false
    return () => {
      if (released) return
      released = true
      entry.refs--
      if (entry.refs > 0) return
      this.stop(entry)
      this.watches.delete(taskId)
      this.evict()
    }
  }

  /** Follows the whole tree under `rootId` (`/task_mgr/tree/<root_id>`) until released. */
  watchTree(rootId: string): () => void {
    let tree = this.trees.get(rootId)
    if (!tree) { tree = { refs: 0, parents: new Set() }; this.trees.set(rootId, tree) }
    const entry = tree
    entry.refs++
    if (entry.refs === 1) {
      if (!this.treeSettled(rootId)) entry.close = this.source.subscribe('tree', rootId, () => this.hintTree(rootId))
      if (!this.childPages.has(rootId)) void this.loadChildren(rootId)
    }
    let released = false
    return () => {
      if (released) return
      released = true
      entry.refs--
      if (entry.refs > 0) return
      entry.close?.()
      if (entry.timer !== undefined) clearTimeout(entry.timer)
      for (const parent of entry.parents) this.childPages.delete(parent)
      this.eventLists.delete(rootId)
      this.trees.delete(rootId)
      this.notify()
    }
  }

  /** Loads the children of a node of a watched tree (one level, first page). */
  expandNode(rootId: string, taskId: string): Promise<void> {
    const tree = this.trees.get(rootId)
    if (!tree) return Promise.resolve()
    if (taskId !== rootId) tree.parents.add(taskId)
    return this.loadChildren(taskId)
  }

  /** Drops the loaded children of a collapsed node. */
  collapseNode(rootId: string, taskId: string) {
    const tree = this.trees.get(rootId)
    if (!tree || taskId === rootId || !tree.parents.delete(taskId)) return
    this.childPages.delete(taskId)
    this.notify()
  }

  /** `more`: appends the next page; otherwise re-reads what is loaded. */
  async loadChildren(taskId: string, more = false): Promise<void> {
    const current = this.childPages.get(taskId)
    if (more && (!current?.nextCursor || current.status === 'loading')) return
    if (!current || more) this.setChildren(taskId, { status: 'loading', items: current?.items ?? [], nextCursor: current?.nextCursor })
    try {
      const page = await this.timed(this.source.getSubtasks(taskId, more ? current?.nextCursor : undefined, more ? this.pageSize : Math.max(this.pageSize, current?.items.length ?? 0)))
      const known = new Map((this.childPages.get(taskId)?.items ?? []).map(item => [item.taskId, item]))
      const fresh = page.tasks.map(task => { const previous = known.get(task.taskId); return previous && previous.revision > task.revision ? previous : task })
      const ids = new Set(fresh.map(task => task.taskId))
      const items = more ? [...(this.childPages.get(taskId)?.items ?? []).filter(item => !ids.has(item.taskId)), ...fresh] : fresh
      this.setChildren(taskId, { status: 'ready', items, nextCursor: page.nextCursor })
    } catch {
      const latest = this.childPages.get(taskId)
      this.setChildren(taskId, { status: latest?.items.length ? 'ready' : 'error', items: latest?.items ?? [], nextCursor: latest?.nextCursor })
    }
  }

  async loadEvents(rootId: string): Promise<void> {
    if (!this.trees.has(rootId)) return
    if (!this.eventLists.has(rootId)) this.setEvents(rootId, { status: 'loading', items: [] })
    try {
      const items = await this.timed(this.source.listEvents(rootId, 100))
      if (this.trees.has(rootId)) this.setEvents(rootId, { status: 'ready', items })
    } catch {
      const latest = this.eventLists.get(rootId)
      if (this.trees.has(rootId)) this.setEvents(rootId, { status: latest?.items.length ? 'ready' : 'error', items: latest?.items ?? [] })
    }
  }

  /** Reads a task again now, whatever its poll state (a manual retry). */
  refresh(taskId: string): Promise<void> {
    const watch = this.watches.get(taskId)
    if (watch) {
      watch.unchanged = 0
      watch.failures = 0
      if (!watch.close && !this.isSettled(taskId)) watch.close = this.source.subscribe('task', taskId, () => { void this.read(taskId) })
    }
    return this.read(taskId)
  }

  /** Catch-up after a reconnect (or the page becoming visible): every watched task that is not settled is read again. */
  resync() {
    for (const taskId of this.watches.keys()) if (!this.isSettled(taskId)) void this.refresh(taskId)
  }

  dispose() {
    for (const watch of this.watches.values()) this.stop(watch)
    for (const tree of this.trees.values()) { tree.close?.(); if (tree.timer !== undefined) clearTimeout(tree.timer) }
    this.watches.clear()
    this.trees.clear()
    this.listeners.clear()
  }

  private read(taskId: string): Promise<void> {
    const current = this.reading.get(taskId)
    if (current) { current.again = true; return current.done }
    const entry = { again: false, done: Promise.resolve() }
    this.reading.set(taskId, entry)
    entry.done = this.readOnce(taskId).finally(() => {
      this.reading.delete(taskId)
      if (entry.again && this.watches.has(taskId)) void this.read(taskId)
      else this.schedule(taskId)
    })
    return entry.done
  }

  private async readOnce(taskId: string): Promise<void> {
    const watch = this.watches.get(taskId)
    try {
      const task = await this.limited(() => this.timed(this.source.getTask(taskId)))
      const previous = this.snapshots.get(taskId)
      // An answer older than the cached revision lost a race with a newer read.
      if (previous?.task && task.revision < previous.task.revision) return
      const changed = !previous?.task || previous.task.revision !== task.revision
      if (changed || previous?.status !== 'ready' || previous.stale) this.setSnapshot({ taskId, status: 'ready', task, stale: false })
      if (watch) { watch.failures = 0; watch.unchanged = changed ? 0 : watch.unchanged + 1 }
      if (changed || !this.childrenSettled(taskId)) await this.loadChildren(taskId)
      if (this.trees.has(taskId)) await this.reloadTree(taskId)
    } catch (error) {
      const failure = this.source.failure(error)
      const previous = this.snapshots.get(taskId)
      if (failure !== 'error') this.setSnapshot({ taskId, status: failure, stale: false })
      else {
        if (watch) watch.failures++
        if (!previous?.task) this.setSnapshot({ taskId, status: 'error', stale: false })
        else if (!previous.stale) this.setSnapshot({ ...previous, stale: true })
      }
    }
  }

  private schedule(taskId: string) {
    const watch = this.watches.get(taskId)
    if (!watch) return
    if (watch.timer !== undefined) { clearTimeout(watch.timer); watch.timer = undefined }
    if (this.isClosed(taskId)) { this.stop(watch); return }
    if (watch.failures >= this.maxFailures || watch.unchanged >= this.maxUnchangedPolls) return
    const step = Math.max(watch.unchanged, watch.failures)
    watch.timer = setTimeout(() => { watch.timer = undefined; void this.read(taskId) }, this.pollDelaysMs[Math.min(step, this.pollDelaysMs.length - 1)])
  }

  private stop(watch: Watch) {
    if (watch.timer !== undefined) { clearTimeout(watch.timer); watch.timer = undefined }
    watch.close?.()
    watch.close = undefined
  }

  private hintTree(rootId: string) {
    const tree = this.trees.get(rootId)
    if (!tree || tree.timer !== undefined) return
    tree.timer = setTimeout(() => {
      tree.timer = undefined
      void this.read(rootId)
    }, TREE_HINT_DEBOUNCE_MS)
  }

  private async reloadTree(rootId: string) {
    const tree = this.trees.get(rootId)
    if (!tree) return
    await Promise.all([...tree.parents].map(parent => this.loadChildren(parent)))
    if (this.eventLists.has(rootId)) await this.loadEvents(rootId)
    if (this.treeSettled(rootId)) { tree.close?.(); tree.close = undefined }
  }

  private childrenSettled(taskId: string): boolean {
    const page = this.childPages.get(taskId)
    return page?.status === 'ready' && page.items.every(item => item.phase === 'Terminal')
  }

  /** The terminal snapshot of the task and of its loaded direct children is cached. */
  private isSettled(taskId: string): boolean {
    const snapshot = this.snapshots.get(taskId)
    return snapshot?.status === 'ready' && !snapshot.stale && snapshot.task?.phase === 'Terminal' && this.childrenSettled(taskId)
  }

  /** Nothing more to follow: settled, or not readable at all. */
  private isClosed(taskId: string): boolean {
    const status = this.snapshots.get(taskId)?.status
    return status === 'denied' || status === 'missing' || this.isSettled(taskId)
  }

  private treeSettled(rootId: string): boolean {
    const tree = this.trees.get(rootId)
    return this.isSettled(rootId) && [...(tree?.parents ?? [])].every(parent => this.childrenSettled(parent))
  }

  private setSnapshot(snapshot: TaskSnapshot) {
    this.snapshots.delete(snapshot.taskId)
    this.snapshots.set(snapshot.taskId, snapshot)
    this.notify()
  }

  private setChildren(taskId: string, page: TaskChildren) {
    const previous = this.childPages.get(taskId)
    if (previous && JSON.stringify(previous) === JSON.stringify(page)) return
    this.childPages.set(taskId, page)
    this.notify()
  }

  private setEvents(rootId: string, events: TaskEvents) {
    this.eventLists.set(rootId, events)
    this.notify()
  }

  private notify() { this.listeners.forEach(listener => listener()) }

  private evict() {
    for (const taskId of this.snapshots.keys()) {
      if (this.snapshots.size <= this.maxCached) return
      if (this.watches.has(taskId)) continue
      this.snapshots.delete(taskId)
      this.childPages.delete(taskId)
    }
  }

  private async limited<T>(operation: () => Promise<T>): Promise<T> {
    if (this.activeReads >= this.maxConcurrentReads) await new Promise<void>(resolve => this.waitingReads.push(resolve))
    this.activeReads++
    try { return await operation() } finally {
      this.activeReads--
      this.waitingReads.shift()?.()
    }
  }

  private timed<T>(promise: Promise<T>): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => reject(Error('timeout')), this.readTimeoutMs)
      promise.then(value => { clearTimeout(timer); resolve(value) }, error => { clearTimeout(timer); reject(error) })
    })
  }
}
