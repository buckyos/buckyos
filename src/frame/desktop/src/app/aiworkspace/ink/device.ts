/* Does this device have a pen (UI 框架 PRD §6.3, Q17)? Until a pen has touched the window, a phone's writing mode writes
 * with one finger and navigates with two, and the writing bar leaves out what only a pen uses (pressure, touch lock);
 * once one has, the pen writes and fingers move the canvas, as on a tablet. Seen once is remembered for the device. */

import { useSyncExternalStore } from 'react'
import { Emitter } from '../state/emitter'

const KEY = 'aiworkspace.ink.penSeen'
const emitter = new Emitter()
let seen = (() => { try { return window.localStorage.getItem(KEY) === '1' } catch { return false } })()
let watching = false

function mark(event: PointerEvent) {
  if (event.pointerType !== 'pen' || seen) return
  seen = true
  try { window.localStorage.setItem(KEY, '1') } catch { /* this window only */ }
  emitter.emit()
}

function watch() {
  if (watching || typeof window === 'undefined') return
  watching = true
  window.addEventListener('pointerdown', mark, true)
}

/** Has a pen been used on this device? */
export function penSeen(): boolean {
  watch()
  return seen
}

export function usePenSeen(): boolean {
  watch()
  return useSyncExternalStore(emitter.subscribe, () => seen, () => false)
}
