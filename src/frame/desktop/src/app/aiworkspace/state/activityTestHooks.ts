/* The e2e hooks of presence and the work log (api/testHooks.ts `activity`): what `usePresence` and `useWorkLog` do,
 * without a component — the shell does not show them yet. */

import type { WorkLogFilter } from '../api/types'
import { presenceGroupLabel, presenceName, presenceOf } from './presence'
import type { WorkspaceStore } from './store'
import { WorkLogFeed, describeWorkLogEntry } from './worklog'

export function activityTestHooks(store: WorkspaceStore) {
  const presence = presenceOf(store)
  let release: (() => void) | null = null
  const feeds = new Map<string, WorkLogFeed>()
  return {
    presence: {
      retain() { release ??= presence.retain() },
      release() { release?.(); release = null },
      beat: () => presence.beat(),
      snapshot: () => presence.snapshot(),
      surface: () => presence.currentSurface(),
      /** What the canvas selector does (WorkspaceShell `selectSurface`). */
      select: (surfaceId: string) => store.userState.set('surface:active', surfaceId),
      labels: () => presence.snapshot().members.map((m) => ({ name: presenceName(m), group: presenceGroupLabel(m) })),
    },
    workLog(filter: WorkLogFilter, limit?: number) {
      const key = `${filter}/${limit ?? ''}`
      let feed = feeds.get(key)
      if (!feed) { feed = new WorkLogFeed(store, filter, limit); feeds.set(key, feed) }
      const f = feed
      return { refresh: () => f.refresh(), loadMore: () => f.loadMore(), snapshot: () => f.snapshot(), describe: () => f.snapshot().entries.map(describeWorkLogEntry) }
    },
  }
}
