import { useEffect, useState, useSyncExternalStore } from 'react'
import { createTaskWatchSource, type TaskWatchSource } from '../../../../api/task_mgr'
import { MockTaskSource } from '../../mock/tasks'
import { isMessageHubMock } from '../../store'
import { TaskWatch, type TaskChildren, type TaskEvents, type TaskSnapshot } from './taskWatch'

let instance: TaskWatch | null = null
let source: TaskWatchSource | null = null

/** TaskMgr reads and `/obj/taskmgr/<id>/changed` hints of MessageHub: one kevent stream for every follower (task areas, notification cards). */
export function getTaskWatchSource(): TaskWatchSource {
  return source ??= isMessageHubMock() ? new MockTaskSource() : createTaskWatchSource()
}

/**
 * One task cache for every MessageHub view. Reads use the logged-in user's
 * own TaskMgr permissions whatever mailbox is shown: observing an Agent's
 * mailbox does not borrow the Agent's (or its owner's) identity.
 */
export function getTaskWatch(): TaskWatch {
  if (instance) return instance
  const tasks = getTaskWatchSource()
  const watch = new TaskWatch(tasks)
  instance = watch
  if (typeof window !== 'undefined') {
    window.addEventListener('online', () => watch.resync())
    document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') watch.resync() })
    if (import.meta.env.DEV) Object.assign(window, { __messageHubTaskWatch: watch, ...(tasks instanceof MockTaskSource ? { __messageHubMockTasks: tasks } : {}) })
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

/** Open / closed choices made by hand, remembered per viewer in this browser under one localStorage key; the oldest are forgotten first. */
export class ChoiceMemory {
  private choices: Record<string, Record<string, boolean>> | null = null
  private readonly storageKey: string
  private readonly limit: number
  constructor(storageKey: string, limit = 200) { this.storageKey = storageKey; this.limit = limit }

  read(viewerDid: string, key: string): boolean | undefined {
    if (!this.choices) {
      try {
        const parsed: unknown = JSON.parse(localStorage.getItem(this.storageKey) ?? '{}')
        this.choices = parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Record<string, Record<string, boolean>> : {}
      } catch {
        this.choices = {}
      }
    }
    const stored = this.choices[viewerDid]?.[key]
    return typeof stored === 'boolean' ? stored : undefined
  }

  store(viewerDid: string, key: string, expanded: boolean) {
    this.read(viewerDid, key)
    const all = this.choices ?? {}
    const own = { ...(typeof all[viewerDid] === 'object' ? all[viewerDid] : {}) }
    delete own[key]
    own[key] = expanded
    const keys = Object.keys(own)
    for (const old of keys.slice(0, Math.max(0, keys.length - this.limit))) delete own[old]
    this.choices = { ...all, [viewerDid]: own }
    try { localStorage.setItem(this.storageKey, JSON.stringify(this.choices)) } catch { /* kept for this page only */ }
  }
}

/** `fallback` until the viewer sets the choice by hand. */
export function useRememberedChoice(memory: ChoiceMemory, viewerDid: string, key: string, fallback: boolean): [boolean, (value: boolean) => void] {
  const [choice, setChoice] = useState<boolean | undefined>(() => memory.read(viewerDid, key))
  return [choice ?? fallback, value => {
    memory.store(viewerDid, key, value)
    setChoice(value)
  }]
}

const qaExpanded = new ChoiceMemory('buckyos.messagehub.qaExpanded.v1')

/**
 * Whether the Q/A list under an Agent bubble is open: `fallback` (open while
 * the Turn waits, closed once its reply arrived) until the viewer opens or
 * closes it by hand, which is remembered per viewer in this browser.
 */
export function useQaExpanded(viewerDid: string, anchorId: string, fallback: boolean): [boolean, (value: boolean) => void] {
  return useRememberedChoice(qaExpanded, viewerDid, anchorId, fallback)
}
