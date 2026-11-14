/* The work log's actions outside its component (UI 框架 PRD §4.5): locating an entry, and what the collapsed status bar
 * counts as under way. */

import { useSyncExternalStore } from 'react'
import type { WorkLogEntry } from '../../api/types'
import { useStore } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { workLogLocation } from '../../state/worklog'
import { uploadQueue } from '../directory/uploads'
import type { ShellApi } from '../shell/shellContext'

/** Go to what an entry names: a Block (or the canvas) on its canvas; a data object through a Block of it on its canvas,
 * else in data mode. False when there is nothing to go to. */
export function locateWorkLogEntry(entry: WorkLogEntry, store: WorkspaceStore, shell: ShellApi, openEntity: (entityId: string) => void): boolean {
  const at = workLogLocation(entry)
  if (!at) return false
  const entity = store.outline.get(at.entityId)
  const toSurface = (surfaceId: string) => {
    if (!store.outline.get(surfaceId)) return false
    shell.selectSurface(surfaceId)
    if (shell.topMode === 'sources') shell.setTopMode('canvas')
    return true
  }
  if (!entity || entity.deleted) return at.surfaceId ? toSurface(at.surfaceId) : false
  if (entity.kind === 'surface' || entity.type_id === 'buckyos.cell' || !at.surfaceId) { openEntity(at.entityId); return true }
  const surfaceId = at.surfaceId
  const view = store.outline.all().find((e) => e.type_id === 'buckyos.cell' && e.source_id === at.entityId && !e.deleted && store.outline.ancestors(e.entity_id).includes(surfaceId))
  openEntity(view ? view.entity_id : at.entityId)
  return true
}

/** For the collapsed status bar: uploads still running in this window. */
export function useActiveUploads(): number {
  const store = useStore()
  const queue = uploadQueue(store)
  useSyncExternalStore(queue.subscribe, queue.snapshot)
  return queue.active().length
}
