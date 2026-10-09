import { useEffect, useState, useSyncExternalStore } from 'react'
import { createTaskWatchSource } from '../../../../api/task_mgr'
import { MockTaskSource } from '../../mock/tasks'
import { isMessageHubMock } from '../../store'
import { TaskWatch, type TaskChildren, type TaskEvents, type TaskSnapshot } from './taskWatch'

let instance: TaskWatch | null = null

/**
 * One task cache for every MessageHub view. Reads use the logged-in user's
 * own TaskMgr permissions whatever mailbox is shown: observing an Agent's
 * mailbox does not borrow the Agent's (or its owner's) identity.
 */
export function getTaskWatch(): TaskWatch {
  if (instance) return instance
  const mock = isMessageHubMock() ? new MockTaskSource() : null
  const watch = new TaskWatch(mock ?? createTaskWatchSource())
  instance = watch
  if (typeof window !== 'undefined') {
    window.addEventListener('online', () => watch.resync())
    document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') watch.resync() })
    if (import.meta.env.DEV) Object.assign(window, { __messageHubTaskWatch: watch, ...(mock ? { __messageHubMockTasks: mock } : {}) })
  }
  return watch
}

/** Watches a task while the caller is mounted: the task and its first page of direct children. */
export function useTaskSummary(taskId: string | undefined): { snapshot?: TaskSnapshot; children?: TaskChildren } {
  const watch = getTaskWatch()
  useEffect(() => taskId ? watch.watch(taskId) : undefined, [watch, taskId])
  const snapshot = useSyncExternalStore(watch.subscribe, () => taskId ? watch.snapshot(taskId) : undefined)
  const children = useSyncExternalStore(watch.subscribe, () => taskId ? watch.children(taskId) : undefined)
  return { snapshot, children }
}

/** Follows the tree under `rootId` while mounted; `events` also keeps its event list current. */
export function useTaskTree(rootId: string, events = false) {
  const watch = getTaskWatch()
  useEffect(() => {
    const release = watch.watchTree(rootId)
    if (events) void watch.loadEvents(rootId)
    return release
  }, [watch, rootId, events])
}

export function useTaskChildren(taskId: string): TaskChildren | undefined {
  const watch = getTaskWatch()
  return useSyncExternalStore(watch.subscribe, () => watch.children(taskId))
}

export function useTaskEvents(rootId: string): TaskEvents | undefined {
  const watch = getTaskWatch()
  return useSyncExternalStore(watch.subscribe, () => watch.events(rootId))
}

const expandedAnchors = new Set<string>()

/** Whether a bubble's task tree is open; kept per anchor so a row scrolled out and back stays open. */
export function useTaskExpanded(anchorId: string): [boolean, (value: boolean) => void] {
  const [expanded, setExpanded] = useState(() => expandedAnchors.has(anchorId))
  return [expanded, value => {
    if (value) expandedAnchors.add(anchorId); else expandedAnchors.delete(anchorId)
    setExpanded(value)
  }]
}
