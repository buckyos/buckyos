/* App mode's way back (UI 框架 PRD §4.6): a Workspace or canvas that hides and locks every tool zone leaves whoever may
 * change the layout (`structure` or `manage`) a collapsed ⋯ top left; from there the zones can be shown for a while —
 * in this window only, never stored, never for anyone else — and the toolbar customised. The customize dialog is
 * asked for from anywhere (the main menu, the ⋯) and drawn by `FrameworkHost`. */

import { useSyncExternalStore } from 'react'
import { useStore } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'

interface Flag { value: boolean; listeners: Set<() => void> }
const revealed = new WeakMap<WorkspaceStore, Flag>()

function flagOf(store: WorkspaceStore): Flag {
  let flag = revealed.get(store)
  if (!flag) { flag = { value: false, listeners: new Set() }; revealed.set(store, flag) }
  return flag
}

/** "临时显示工具区": the zones a shared layer hid are shown in this window (until it is closed or hidden again). */
export function setZonesRevealed(store: WorkspaceStore, value: boolean): void {
  const flag = flagOf(store)
  if (flag.value === value) return
  flag.value = value
  for (const listener of [...flag.listeners]) listener()
}

export function zonesRevealed(store: WorkspaceStore): boolean {
  return flagOf(store).value
}

export function useZonesRevealed(): boolean {
  const flag = flagOf(useStore())
  return useSyncExternalStore((listener) => { flag.listeners.add(listener); return () => { flag.listeners.delete(listener) } }, () => flag.value)
}

// ---- the customize dialog is asked for here and drawn by FrameworkHost

const customizeListeners = new Set<() => void>()

/** Open "自定义工具栏…" (§4.6). */
export function requestCustomize(): void {
  for (const listener of [...customizeListeners]) listener()
}

export function onCustomizeRequest(listener: () => void): () => void {
  customizeListeners.add(listener)
  return () => { customizeListeners.delete(listener) }
}

/** Whether the person may change a shared layer of the toolbar here: `manage` on the Workspace, or `structure` on the
 * current canvas (or the whole Workspace). */
export function mayCustomize(store: WorkspaceStore, surfaceId: string | null): boolean {
  const caps = store.session.info().capabilities
  if (caps.includes('manage') || caps.includes('structure')) return true
  const surface = surfaceId ? store.outline.get(surfaceId) : undefined
  return Boolean(surface?.capabilities.includes('structure'))
}
