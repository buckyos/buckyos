/* The Block-control constants the core reads from core.json (`include_str!`): which container kinds are controls and
 * which views size themselves from their title. Kept here as code for the browser and Deno; contract_test.ts checks
 * the two agree. */

/** Container kinds that are controls (脑图方案 §6.2, R2): the kind is the control id. */
export const CONTROL_KINDS: readonly string[] = ['mindmap', 'composite', 'todo', 'calendar']

/** Data controls (控件机制 TODO G2, Q19): they show the records of the tables they bind as projected Blocks (computed
 * by the reader, never stored), hold no Blocks and have the numeric size their user gives them. */
export const DATA_CONTROLS: readonly string[] = ['todo', 'calendar']

/** The host kind of composite extension controls (扩展控件 P1, K4): which control it is, is the container's `def_ref`. */
export const COMPOSITE = 'composite'

/** Views whose `w` / `h` may be `"auto"` inside a control (R17): their size comes from the title. */
export const TEXT_VIEWS: readonly string[] = ['mindnode', 'shape', 'richtext', 'label']

export const isControlKind = (kind: string | null | undefined): boolean => !!kind && CONTROL_KINDS.includes(kind)
export const isTextView = (view: string | null | undefined): boolean => !!view && TEXT_VIEWS.includes(view)
export const isDataControl = (kind: string | null | undefined): boolean => !!kind && DATA_CONTROLS.includes(kind)

/** A slot (R22): a Block's role name inside its control, unique among its siblings. */
export const SLOT_PATTERN = /^[a-z0-9][a-z0-9_-]{0,31}$/
