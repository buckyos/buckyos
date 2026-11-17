/**
 * Icon-view size (global, like `files.view`): three steps. Desktop sizes the
 * grid by cell width, phones by column count.
 */

import { useSyncExternalStore } from 'react'

export type IconSize = 'small' | 'medium' | 'large'
export const ICON_SIZES: IconSize[] = ['small', 'medium', 'large']

const KEY = 'files.iconSize'
const listeners = new Set<() => void>()

function read(): IconSize {
  try {
    const value = localStorage.getItem(KEY)
    return value === 'small' || value === 'large' ? value : 'medium'
  } catch {
    return 'medium'
  }
}

let current: IconSize = read()

export function setIconSize(size: IconSize) {
  current = size
  try { localStorage.setItem(KEY, size) } catch { /* no storage */ }
  listeners.forEach((listener) => listener())
}

export function useIconSize(): IconSize {
  return useSyncExternalStore(
    (listener) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    () => current,
  )
}

/** Grid metrics per step: cell width (desktop), columns (phone), icon box edge. */
export const ICON_GRID = {
  small: { cell: 120, columns: 4, box: 72 },
  medium: { cell: 168, columns: 3, box: 112 },
  large: { cell: 232, columns: 2, box: 176 },
} as const satisfies Record<IconSize, { cell: number; columns: number; box: number }>
