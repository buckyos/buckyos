/* What a macro is (UI 框架 PRD Q27, after the AI Native 内核升级 PRD §3.1): the runnable product of a wish — a program
 * version with its declaration manifest — made by the wish's coding stage and, for now, run in the browser. A data
 * source's sync program is not one. Whether a wish holds one is the core's answer: `doc.freshness` says
 * `needs_coding: no_macro` when it has no program with a manifest (`macro_check`). The wish card's run button, the
 * collaboration zone's "宏", the data mode's "宏" group and the toolbar customization all ask here, so they agree. */

import type { EntityEnvelope, FreshnessInfo, WishProgram } from '../../api/types'

/** The wish holds a macro (it may still need coding again: its inputs changed, an older program API). */
export function holdsMacro(wish: EntityEnvelope | undefined, fresh: FreshnessInfo | undefined): boolean {
  if (!wish || wish.type_id !== 'buckyos.wish' || wish.deleted) return false
  if (!fresh || fresh.error || fresh.wish !== true) return false
  return !(fresh.needs_coding?.reasons ?? []).some((r) => r.code === 'no_macro')
}

/** The macro can run now: the analysis is done and current, the macro fits its inputs (no coding needed), and the user
 * may run it — a run writes its results as whoever starts it, so it takes `update` on the wish (权限需求 Q31). */
export function macroRunnable(wish: EntityEnvelope | undefined, fresh: FreshnessInfo | undefined): boolean {
  if (!wish || !fresh || !holdsMacro(wish, fresh)) return false
  return wish.capabilities.includes('update') && !fresh.needs_analysis && fresh.analysis_status === 'ready' && !fresh.needs_coding
}

/** The macro calls AI: its manifest declares `ai` (a call it does not declare is refused by the host, `check_manifest`).
 * A hand-edited macro keeps the manifest of its last coding until the next trial run (许愿格交互升级 §3.6). */
export function macroUsesAi(program: WishProgram | undefined): boolean {
  const ai = (program?.manifest as { ai?: unknown } | null | undefined)?.ai
  if (!ai) return false
  return typeof ai !== 'object' || Array.isArray(ai) ? Boolean(ai) : Object.keys(ai).length > 0
}
