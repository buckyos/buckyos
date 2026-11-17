/* What a Block on a printed page learns from the page it is on (打印 TODO T2, T9): on a flow page a table prints the rows
 * of its fragment, measured beforehand; `measure` is the pass that measures. */

import { createContext } from 'react'

export interface PrintFragment {
  /** The width of the flow column (CSS px). */
  width: number
  /** The table rows `[from, to)` of this fragment (null: all of them). */
  rows: [number, number] | null
  /** The pass that measures the flow: HTML Blocks keep their snapshot, nothing waits for them. */
  measure: boolean
}

export const PrintFragmentContext = createContext<PrintFragment | null>(null)
