/* Small hooks of the data mode. */

import { useSyncExternalStore } from 'react'
import { useEdits } from '../../state/hooks'
import { editNames } from './tabs'

/** Unsaved: an edit naming the object is not on the backend yet (a tab's dot). */
export function useDirty(id: string): boolean {
  const edits = useEdits()
  for (const entry of edits.values()) if (entry.state !== 'committed' && editNames(entry.id, id)) return true
  return false
}

/** A touch screen (coarse pointer): every row shows its ⋯. */
export function useCoarse(): boolean {
  return useSyncExternalStore((listener) => {
    const query = window.matchMedia('(pointer: coarse)')
    query.addEventListener('change', listener)
    return () => query.removeEventListener('change', listener)
  }, () => window.matchMedia('(pointer: coarse)').matches, () => false)
}
