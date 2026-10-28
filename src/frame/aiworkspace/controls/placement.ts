/* `auto` placement components (脑图方案 §5.1, R5/R19): each of x, y, w, h is a number (the user's, pinned) or the
 * literal "auto" (computed when shown by the control laying the Block out, never stored). */

export type Axis = number | 'auto'
export interface StoredPlacement { x: Axis; y: Axis; w: Axis; h: Axis; rotation?: number }

export const AUTO = 'auto' as const
export const ALL_AUTO: StoredPlacement = { x: AUTO, y: AUTO, w: AUTO, h: AUTO }

export const isAuto = (v: unknown): v is 'auto' => v === AUTO
export const num = (v: Axis | undefined): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined)

/** Any component computed by a layout. */
export function hasAuto(p: Partial<StoredPlacement> | null | undefined): boolean {
  return !!p && (isAuto(p.x) || isAuto(p.y) || isAuto(p.w) || isAuto(p.h))
}

/** Is the position pinned (a free topic, R16)? Either coordinate written as a number. */
export function isPinned(p: Partial<StoredPlacement> | null | undefined): boolean {
  return !!p && (num(p.x) !== undefined || num(p.y) !== undefined)
}

/** The placement when every component is a number (a Block outside any control, or pinned all over), else undefined. */
export function fixed(p: Partial<StoredPlacement> | null | undefined): { x: number; y: number; w: number; h: number; rotation?: number } | undefined {
  const x = num(p?.x), y = num(p?.y), w = num(p?.w), h = num(p?.h)
  if (x === undefined || y === undefined || w === undefined || h === undefined) return undefined
  return { x, y, w, h, ...(p?.rotation ? { rotation: p.rotation } : {}) }
}
