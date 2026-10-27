/* The pen's settings (手写笔 TODO D3): a personal preference in the user work state (`ui:ink`), not in the document —
 * what a stroke is drawn with is written into the stroke itself. */

import { useCallback } from 'react'
import { useStore, useUserState } from '../state/hooks'
import type { InkStyle } from './model'

export type InkPrefs = { color: string; width: number; pressure: boolean; touchLock: boolean }

export const INK_PREFS_KEY = 'ui:ink'
export const INK_COLORS = ['#111827', '#2563eb', '#dc2626', '#16a34a', '#f59e0b', '#7c3aed']
/** Base widths in canvas units: fine, medium, bold. */
export const INK_WIDTHS = [2, 4, 8]
export const DEFAULT_INK: InkPrefs = { color: INK_COLORS[0], width: INK_WIDTHS[0], pressure: true, touchLock: false }

export function inkStyleOf(prefs: InkPrefs): InkStyle {
  return { color: prefs.color, width: prefs.width, pressure: prefs.pressure }
}

export function useInkPrefs(): [InkPrefs, (patch: Partial<InkPrefs>) => void] {
  const store = useStore()
  const saved = useUserState<Record<string, string | number | boolean>>(INK_PREFS_KEY)
  const prefs: InkPrefs = { ...DEFAULT_INK, ...(saved as Partial<InkPrefs> | undefined) }
  const set = useCallback((patch: Partial<InkPrefs>) => {
    const current = { ...DEFAULT_INK, ...(store.userState.get(INK_PREFS_KEY) as Partial<InkPrefs> | undefined) }
    store.userState.set(INK_PREFS_KEY, { ...current, ...patch })
  }, [store])
  return [prefs, set]
}
