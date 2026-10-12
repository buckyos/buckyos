/* What a Block on a flow page can ask of the page (流式页交互改进 §2): insert the next Block right after it
 * (the `/` menu of rich text), and hand the selection back to the page (Esc from an editor). Absent on a free
 * canvas and in the data-source view. */

import { createContext, useContext } from 'react'
import type { CatalogEntry } from '../canvas/catalog'

export interface FlowBlockApi {
  cellId: string
  /** Insert a Block of `entry` right after this one; an entry that needs a file asks for it first. */
  insertAfter: (entry: CatalogEntry) => void
  /** Leave the editor and select this Block on the page. */
  select: () => void
}

export const FlowBlockContext = createContext<FlowBlockApi | null>(null)

export function useFlowBlock(): FlowBlockApi | null {
  return useContext(FlowBlockContext)
}
