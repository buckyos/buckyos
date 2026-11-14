/* What a macro is (UI 框架 PRD Q27, after the AI Native 内核升级 PRD §3.1): the runnable product of a wish — a program
 * version with its declaration manifest — made by the wish's coding stage and, for now, run in the browser. The Mock
 * executor makes none (its extension produces the results directly), and a data source's sync program is not one.
 * Whether a wish holds one is the core's answer: `doc.freshness` says `needs_coding: no_macro` when it has no program
 * with a manifest (`macro_check`). The collaboration zone's "宏", the data mode's "宏" group and the toolbar
 * customization all ask here, so they agree. */

import type { EntityEnvelope, FreshnessInfo } from '../../api/types'

/** The wish holds a macro (it may still need coding again: its inputs changed, an older program API). */
export function holdsMacro(wish: EntityEnvelope | undefined, fresh: FreshnessInfo | undefined): boolean {
  if (!wish || wish.type_id !== 'buckyos.wish' || wish.deleted || wish.executor === 'mock') return false
  if (!fresh || fresh.error || fresh.wish !== true) return false
  return !(fresh.needs_coding?.reasons ?? []).some((r) => r.code === 'no_macro')
}

/** The macro can run now: the analysis is done and current, the macro fits its inputs (no coding needed), and the user
 * may run it — a run writes its results as whoever starts it, so it takes `update` on the wish (the wish panel's rule). */
export function macroRunnable(wish: EntityEnvelope | undefined, fresh: FreshnessInfo | undefined): boolean {
  if (!wish || !fresh || !holdsMacro(wish, fresh)) return false
  return wish.capabilities.includes('update') && !fresh.needs_analysis && fresh.analysis_status === 'ready' && !fresh.needs_coding
}
