/**
 * Layout helper functions extracted from DesktopRoute.
 *
 * Pure functions for grid computation, layout resolution,
 * dead-zone migration, and position management.
 *
 * ## Design (proposal: desktop-grid)
 *
 * - **Slot-first**: the logical slot index (column-major on desktop,
 *   row-major on mobile) is the single source of truth.
 *   Pixel coordinates are derived from it at render time.
 * - **Page + Slot**: layout is `pages[pageIndex].slots[slotIndex]`,
 *   not a flat global array.
 * - **Tail-append, no hole-filling**: unpositioned items are appended
 *   after `maxUsedSlot + 1`; user-created gaps are preserved.
 * - **Fixed desktop cell size**: determined by density tier, not by
 *   stretching to fill the viewport.
 * - **Resize / drag separation**: resize reflow and drag collision
 *   use different rule sets (handled in the store layer).
 */
import type {
  DeadZone,
  DesktopPageState,
  FormFactor,
  LayoutItem,
  LayoutState,
  PlacementType,
  WindowAppearancePreferences,
  WindowGeometry,
} from './ui'
import {
  defaultWindowAppearancePreferences,
  isLauncherApp,
  windowAppearancePreferencesSchema,
} from './ui'
import type { DesktopAppItem } from '../app/types'
import { defaultDeadZone } from '../mock/data'

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

export const runtimeStorageKey = 'buckyos.prototype.runtime.v1'
export const windowGeometryStorageKey = 'buckyos.window-geometry.desktop.v1'
export const windowAppearanceStorageKey = 'buckyos.window-appearance.v1'

export function layoutStorageKey(formFactor: FormFactor) {
  return `buckyos.layout.${formFactor}.v1`
}

// ---------------------------------------------------------------------------
// JSON helpers (localStorage)
// ---------------------------------------------------------------------------

export function readJson<T>(key: string): T | null {
  const raw = window.localStorage.getItem(key)
  if (!raw) return null
  try {
    return JSON.parse(raw) as T
  } catch {
    return null
  }
}

export function writeJson<T>(key: string, value: T) {
  window.localStorage.setItem(key, JSON.stringify(value))
}

// ---------------------------------------------------------------------------
// Window geometry helpers
// ---------------------------------------------------------------------------

// Re-export WindowGeometry from ui.ts (canonical definition)
export type { WindowGeometry } from './ui'
export type WindowGeometryMap = Record<string, WindowGeometry>

export function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
}

export function sanitizeWindowGeometryMap(input: unknown): WindowGeometryMap {
  if (!input || typeof input !== 'object') return {}

  return Object.fromEntries(
    Object.entries(input).flatMap(([appId, geometry]) => {
      if (
        !geometry ||
        typeof geometry !== 'object' ||
        !isFiniteNumber((geometry as WindowGeometry).x) ||
        !isFiniteNumber((geometry as WindowGeometry).y) ||
        !isFiniteNumber((geometry as WindowGeometry).width) ||
        !isFiniteNumber((geometry as WindowGeometry).height)
      ) {
        return []
      }
      return [[appId, geometry as WindowGeometry]]
    }),
  )
}

export function sameWindowGeometry(
  left: WindowGeometry | undefined,
  right: WindowGeometry,
) {
  return (
    left?.x === right.x &&
    left?.y === right.y &&
    left?.width === right.width &&
    left?.height === right.height
  )
}

export function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max)
}

// ---------------------------------------------------------------------------
// Window appearance
// ---------------------------------------------------------------------------

export function readWindowAppearancePreferences(): WindowAppearancePreferences {
  const parsed = windowAppearancePreferencesSchema.safeParse(
    readJson(windowAppearanceStorageKey),
  )
  return parsed.success ? parsed.data : { ...defaultWindowAppearancePreferences }
}

// ---------------------------------------------------------------------------
// Dead-zone migration
// ---------------------------------------------------------------------------

function legacyDeadZone(formFactor: FormFactor): DeadZone {
  return formFactor === 'desktop'
    ? { top: 64, bottom: 24, left: 20, right: 20 }
    : { top: 52, bottom: 20, left: 12, right: 12 }
}

function matchesDeadZone(
  target: DeadZone | undefined,
  expected: DeadZone,
) {
  return (
    target?.top === expected.top &&
    target?.bottom === expected.bottom &&
    target?.left === expected.left &&
    target?.right === expected.right
  )
}

export function migrateDeadZone(
  layout: LayoutState,
  formFactor: FormFactor,
): LayoutState {
  if (!matchesDeadZone(layout.deadZone, legacyDeadZone(formFactor))) {
    return layout
  }
  return { ...layout, deadZone: { ...defaultDeadZone } }
}

// ---------------------------------------------------------------------------
// Grid spec helpers
// ---------------------------------------------------------------------------

/** Density tier for the grid slot system. */
export type GridDensity = 'small' | 'medium' | 'large'

export const densityRowHeight: Record<GridDensity, number> = {
  small: 92,
  medium: 108,
  large: 124,
}

export const GRID_GAP = 2

/**
 * Maximum width (px) a single grid cell may occupy on desktop.
 * When the container is wide enough that cells would exceed this,
 * more columns are added to keep cells compact.
 */
export const MAX_CELL_WIDTH = 110

/** Minimum columns on desktop so the grid never looks too sparse. */
export const MIN_DESKTOP_COLS = 6

/** Compute column count so that each cell stays within MAX_CELL_WIDTH. */
export function columnsForWidth(width: number): number {
  const cols = Math.ceil((width + GRID_GAP) / (MAX_CELL_WIDTH + GRID_GAP))
  return Math.max(MIN_DESKTOP_COLS, cols)
}

/**
 * Fixed row height for desktop by density tier.
 * Desktop cells do NOT stretch — extra space becomes margin.
 */
export const desktopFixedRowHeight: Record<GridDensity, number> = {
  small: 78,
  medium: 92,
  large: 108,
}

/**
 * Minimum row height on desktop -- more compact than the density value.
 * icon-padding-top(10) + icon(48) + label-padding(4) + 1 line(16) = 78
 */
export const DESKTOP_MIN_ROW_HEIGHT = 78

/**
 * Compute how many rows fit in the available height.
 * Desktop uses fixed cell height (by density); mobile uses density row height.
 */
export function rowsForHeight(
  height: number,
  density: GridDensity,
  isMobile: boolean,
): number {
  const slotH = isMobile ? densityRowHeight[density] : desktopFixedRowHeight[density]
  return Math.max(1, Math.floor((height + GRID_GAP) / (slotH + GRID_GAP)))
}

/**
 * Compute actual row height so the grid fills the entire container height evenly.
 * Used for mobile only. Desktop uses fixed row height.
 */
export function stretchedRowHeight(height: number, rows: number): number {
  if (rows <= 0) return densityRowHeight.medium
  return (height - (rows - 1) * GRID_GAP) / rows
}

/**
 * Return the fixed row height for desktop, or stretched height for mobile.
 */
export function effectiveRowHeight(
  height: number,
  rows: number,
  density: GridDensity,
  isMobile: boolean,
): number {
  if (isMobile) {
    return stretchedRowHeight(height, rows)
  }
  return desktopFixedRowHeight[density]
}

// ---------------------------------------------------------------------------
// Slot ↔ grid-coordinate conversion
// ---------------------------------------------------------------------------

export type ScanOrder = 'row-major' | 'col-major'

/**
 * Convert a linear slot index to (x, y) grid coordinates.
 *
 * - column-major (desktop): slot = col * rows + row → x=col, y=row
 * - row-major (mobile):     slot = row * cols + col → x=col, y=row
 */
export function slotToCoord(
  slot: number,
  cols: number,
  rows: number,
  order: ScanOrder,
): { x: number; y: number } {
  if (order === 'col-major') {
    const col = Math.floor(slot / rows)
    const row = slot % rows
    return { x: col, y: row }
  }
  // row-major
  const row = Math.floor(slot / cols)
  const col = slot % cols
  return { x: col, y: row }
}

/**
 * Convert (x, y) grid coordinates to a linear slot index.
 */
export function coordToSlot(
  x: number,
  y: number,
  cols: number,
  rows: number,
  order: ScanOrder,
): number {
  if (order === 'col-major') {
    return x * rows + y
  }
  return y * cols + x
}

/**
 * Page capacity = rows * cols (total number of 1×1 slots per page).
 */
export function pageCapacity(cols: number, rows: number): number {
  return cols * rows
}

// ---------------------------------------------------------------------------
// Max-used-slot & tail-append helpers
// ---------------------------------------------------------------------------

/**
 * Return the highest slot index occupied on a page, or -1 if the page is empty.
 * Uses column-major or row-major depending on scanOrder.
 */
export function getMaxUsedSlot(
  page: DesktopPageState,
  cols: number,
  rows: number,
  order: ScanOrder,
): number {
  let maxSlot = -1
  for (const item of page.items) {
    if (item.slotIndex !== undefined) {
      // Use slotIndex directly (new model)
      const endSlot = item.slotIndex + (item.w - 1) * (order === 'col-major' ? rows : 1)
        + (item.h - 1) * (order === 'col-major' ? 1 : cols)
      maxSlot = Math.max(maxSlot, endSlot)
    } else if (item.x !== undefined && item.y !== undefined) {
      // Fallback to x/y (legacy or derived)
      for (let dx = 0; dx < item.w; dx++) {
        for (let dy = 0; dy < item.h; dy++) {
          const slot = coordToSlot(item.x + dx, item.y + dy, cols, rows, order)
          maxSlot = Math.max(maxSlot, slot)
        }
      }
    }
  }
  return maxSlot
}

/**
 * Build a set of occupied slot indices for a page.
 */
function getOccupiedSlots(
  page: DesktopPageState,
  cols: number,
  rows: number,
  order: ScanOrder,
  excludeId?: string,
): Set<number> {
  const occupied = new Set<number>()
  const cap = pageCapacity(cols, rows)
  for (const item of page.items) {
    if (item.id === excludeId) continue
    if (item.x === undefined || item.y === undefined) continue
    for (let dx = 0; dx < item.w; dx++) {
      for (let dy = 0; dy < item.h; dy++) {
        const slot = coordToSlot(item.x + dx, item.y + dy, cols, rows, order)
        if (slot < cap) occupied.add(slot)
      }
    }
  }
  return occupied
}

// ---------------------------------------------------------------------------
// Grid slot scanning (tail-append — no hole filling)
// ---------------------------------------------------------------------------

function fits(
  page: DesktopPageState,
  x: number,
  y: number,
  w: number,
  h: number,
  cols: number,
  rows: number,
  excludeId?: string,
) {
  if (x + w > cols || y + h > rows) return false
  return !page.items.some((item) => {
    if (item.id === excludeId) return false
    if (item.x === undefined || item.y === undefined) return false
    return !(
      x + w <= item.x ||
      item.x + item.w <= x ||
      y + h <= item.y ||
      item.y + item.h <= y
    )
  })
}

/**
 * Find a slot at the tail of the page (after maxUsedSlot).
 * Implements the "tail-append, no hole-filling" policy from the proposal.
 *
 * For 1×1 items this is simply `maxUsedSlot + 1` if it's within capacity.
 * For multi-cell items we scan from that point forward.
 */
export function findTailSlot(
  page: DesktopPageState,
  w: number,
  h: number,
  cols: number,
  rows: number,
  scanOrder: ScanOrder = 'row-major',
): { x: number; y: number } | null {
  const maxSlot = getMaxUsedSlot(page, cols, rows, scanOrder)
  const cap = pageCapacity(cols, rows)
  const startSlot = maxSlot + 1

  if (startSlot >= cap && w === 1 && h === 1) return null

  // For 1×1 items, just convert startSlot
  if (w === 1 && h === 1) {
    if (startSlot < cap) {
      return slotToCoord(startSlot, cols, rows, scanOrder)
    }
    return null
  }

  // For multi-cell items, scan from startSlot
  if (scanOrder === 'col-major') {
    const startCol = Math.floor(startSlot / rows)
    const startRow = startSlot % rows
    for (let x = startCol; x + w <= cols; x++) {
      const sy = x === startCol ? startRow : 0
      for (let y = sy; y + h <= rows; y++) {
        if (fits(page, x, y, w, h, cols, rows)) return { x, y }
      }
    }
  } else {
    const startRow = Math.floor(startSlot / cols)
    const startCol = startSlot % cols
    for (let y = startRow; y + h <= rows; y++) {
      const sx = y === startRow ? startCol : 0
      for (let x = sx; x + w <= cols; x++) {
        if (fits(page, x, y, w, h, cols, rows)) return { x, y }
      }
    }
  }
  return null
}

// ---------------------------------------------------------------------------
// Layout resolution
// ---------------------------------------------------------------------------

/**
 * Mark positioned items as unpositioned when their position exceeds
 * the current grid bounds (slot >= pageCapacity).
 * Also marks their slotIndex as undefined and records preferredPage.
 */
export function invalidatePositions(
  layout: LayoutState,
  cols: number,
  rows: number,
): LayoutState {
  let anyChanged = false
  const cap = pageCapacity(cols, rows)
  const order: ScanOrder = layout.formFactor === 'mobile' ? 'row-major' : 'col-major'

  const pages = layout.pages.map((page, pageIdx) => {
    let pageChanged = false
    const items = page.items.map((item) => {
      if (item.x === undefined || item.y === undefined) return item
      if (item.x + item.w > cols || item.y + item.h > rows) {
        pageChanged = true
        return {
          ...item,
          x: undefined,
          y: undefined,
          slotIndex: undefined,
          preferredPage: item.preferredPage ?? pageIdx,
          placementType: 'reflow' as PlacementType,
        }
      }
      // Also check slotIndex against capacity
      const slot = coordToSlot(item.x, item.y, cols, rows, order)
      if (slot >= cap) {
        pageChanged = true
        return {
          ...item,
          x: undefined,
          y: undefined,
          slotIndex: undefined,
          preferredPage: item.preferredPage ?? pageIdx,
          placementType: 'reflow' as PlacementType,
        }
      }
      return item
    })
    if (pageChanged) anyChanged = true
    return pageChanged ? { ...page, items } : page
  })
  return anyChanged ? { ...layout, pages } : layout
}

/**
 * Resolve all unpositioned items by placing them at the tail of each page.
 * Returns a fully-positioned layout suitable for rendering.
 *
 * Unpositioned items are sorted by:
 *   1. preferredPage (try to stay on preferred page)
 *   2. original slotIndex (if available)
 *   3. seq (install order)
 */
export function resolveLayout(
  layout: LayoutState,
  cols: number,
  rows: number,
  scanOrder: ScanOrder = 'row-major',
): LayoutState {
  const unpositioned: Array<{ item: LayoutItem; sourcePageIdx: number }> = []
  const resolvedPages: DesktopPageState[] = layout.pages.map((page, pageIdx) => ({
    ...page,
    items: page.items.filter((item) => {
      if (item.x === undefined || item.y === undefined) {
        unpositioned.push({ item, sourcePageIdx: pageIdx })
        return false
      }
      return true
    }),
  }))

  if (unpositioned.length === 0) return layout

  // Sort unpositioned items for stable placement
  unpositioned.sort((a, b) => {
    const prefA = a.item.preferredPage ?? a.sourcePageIdx
    const prefB = b.item.preferredPage ?? b.sourcePageIdx
    if (prefA !== prefB) return prefA - prefB
    const slotA = a.item.slotIndex ?? Infinity
    const slotB = b.item.slotIndex ?? Infinity
    if (slotA !== slotB) return slotA - slotB
    const seqA = a.item.seq ?? Infinity
    const seqB = b.item.seq ?? Infinity
    return seqA - seqB
  })

  for (const { item } of unpositioned) {
    const preferredPage = item.preferredPage ?? 0
    let placed = false

    // Try from preferredPage onward
    for (let pi = preferredPage; pi < resolvedPages.length; pi++) {
      const slot = findTailSlot(resolvedPages[pi], item.w, item.h, cols, rows, scanOrder)
      if (slot) {
        const slotIdx = coordToSlot(slot.x, slot.y, cols, rows, scanOrder)
        resolvedPages[pi].items.push({
          ...item,
          x: slot.x,
          y: slot.y,
          slotIndex: slotIdx,
          placementType: item.placementType ?? 'auto',
        })
        placed = true
        break
      }
    }
    // Also try pages before preferredPage if not placed
    if (!placed) {
      for (let pi = 0; pi < preferredPage && pi < resolvedPages.length; pi++) {
        const slot = findTailSlot(resolvedPages[pi], item.w, item.h, cols, rows, scanOrder)
        if (slot) {
          const slotIdx = coordToSlot(slot.x, slot.y, cols, rows, scanOrder)
          resolvedPages[pi].items.push({
            ...item,
            x: slot.x,
            y: slot.y,
            slotIndex: slotIdx,
            placementType: item.placementType ?? 'auto',
          })
          placed = true
          break
        }
      }
    }
    if (!placed) {
      resolvedPages.push({
        id: `${layout.formFactor}-page-${resolvedPages.length + 1}`,
        items: [{ ...item, x: 0, y: 0, slotIndex: 0, placementType: item.placementType ?? 'auto' }],
      })
    }
  }
  return { ...layout, pages: resolvedPages }
}

// ---------------------------------------------------------------------------
// Drag collision helpers
// ---------------------------------------------------------------------------

/**
 * Find the nearest empty slot to `targetSlot` on the same page.
 * Search by Manhattan distance, with tie-break favoring the scan-order direction
 * (column-major: prefer below; row-major: prefer right).
 *
 * Returns the (x, y) of the nearest empty slot, or null if page is full.
 */
export function findNearestEmptySlot(
  page: DesktopPageState,
  targetX: number,
  targetY: number,
  cols: number,
  rows: number,
  order: ScanOrder,
  excludeId?: string,
): { x: number; y: number } | null {
  const occupied = getOccupiedSlots(page, cols, rows, order, excludeId)
  const cap = pageCapacity(cols, rows)

  let best: { x: number; y: number; dist: number; slot: number } | null = null

  for (let slot = 0; slot < cap; slot++) {
    if (occupied.has(slot)) continue
    const coord = slotToCoord(slot, cols, rows, order)
    const dist = Math.abs(coord.x - targetX) + Math.abs(coord.y - targetY)
    if (!best || dist < best.dist || (dist === best.dist && slot < best.slot)) {
      best = { x: coord.x, y: coord.y, dist, slot }
    }
  }

  return best ? { x: best.x, y: best.y } : null
}

/**
 * Reorder items within a full page: shift items between source and target
 * slots to make room. Returns a new items array with updated positions.
 *
 * - If source < target: items in (source+1..target) shift back by 1
 * - If source > target: items in (target..source-1) shift forward by 1
 */
export function reorderWithinPage(
  page: DesktopPageState,
  sourceSlot: number,
  targetSlot: number,
  cols: number,
  rows: number,
  order: ScanOrder,
): DesktopPageState {
  if (sourceSlot === targetSlot) return page

  // Build slot→item map
  const slotMap = new Map<number, LayoutItem>()
  let draggedItem: LayoutItem | undefined
  for (const item of page.items) {
    if (item.x === undefined || item.y === undefined) continue
    const slot = coordToSlot(item.x, item.y, cols, rows, order)
    if (slot === sourceSlot) {
      draggedItem = item
    } else {
      slotMap.set(slot, item)
    }
  }

  if (!draggedItem) return page

  // Shift items
  const lo = Math.min(sourceSlot, targetSlot)
  const hi = Math.max(sourceSlot, targetSlot)
  const newItems: LayoutItem[] = []

  // Items outside the range stay where they are
  for (const item of page.items) {
    if (item.x === undefined || item.y === undefined) {
      newItems.push(item)
      continue
    }
    const slot = coordToSlot(item.x, item.y, cols, rows, order)
    if (slot === sourceSlot) continue // handle separately
    if (slot < lo || slot > hi) {
      newItems.push(item)
      continue
    }
    // Item is in the shift range
    const newSlot = sourceSlot < targetSlot ? slot - 1 : slot + 1
    const coord = slotToCoord(newSlot, cols, rows, order)
    newItems.push({
      ...item,
      x: coord.x,
      y: coord.y,
      slotIndex: newSlot,
      placementType: 'manual',
    })
  }

  // Place dragged item at target
  const targetCoord = slotToCoord(targetSlot, cols, rows, order)
  newItems.push({
    ...draggedItem,
    x: targetCoord.x,
    y: targetCoord.y,
    slotIndex: targetSlot,
    placementType: 'manual',
  })

  return { ...page, items: newItems }
}

/**
 * Check if a page is full (all slots occupied).
 */
export function isPageFull(
  page: DesktopPageState,
  cols: number,
  rows: number,
  order: ScanOrder,
): boolean {
  const occupied = getOccupiedSlots(page, cols, rows, order)
  return occupied.size >= pageCapacity(cols, rows)
}

// ---------------------------------------------------------------------------
// Layout sanitisation & reconciliation
// ---------------------------------------------------------------------------

export function sanitizeLayoutForApps(
  layout: LayoutState,
  apps: DesktopAppItem[],
): LayoutState {
  const launcherAppIds = new Set(
    apps.filter((app) => isLauncherApp(app)).map((app) => app.id),
  )
  return {
    ...layout,
    pages: layout.pages.map((page) => ({
      ...page,
      items: page.items.filter(
        (item) => item.type === 'widget' || launcherAppIds.has(item.appId),
      ),
    })),
  }
}

export function reconcileLayoutWithDefaultApps(
  layout: LayoutState,
  defaultLayout: LayoutState,
  apps: DesktopAppItem[],
  formFactor: FormFactor,
): LayoutState {
  const sanitizedLayout = sanitizeLayoutForApps(layout, apps)
  const launcherAppIds = new Set(
    apps.filter((app) => isLauncherApp(app)).map((app) => app.id),
  )
  const existingAppIds = new Set(
    sanitizedLayout.pages.flatMap((page) =>
      page.items.flatMap((item) => (item.type === 'app' ? [item.appId] : [])),
    ),
  )

  const newItems: LayoutItem[] = []
  let nextSeq = sanitizedLayout.pages.reduce(
    (max, page) => Math.max(max, ...page.items.map((i) => i.seq ?? 0)),
    0,
  ) + 1

  defaultLayout.pages.forEach((defaultPage) => {
    defaultPage.items.forEach((item) => {
      if (
        item.type !== 'app' ||
        !launcherAppIds.has(item.appId) ||
        existingAppIds.has(item.appId)
      ) {
        return
      }
      newItems.push({
        ...item,
        x: undefined,
        y: undefined,
        slotIndex: undefined,
        placementType: 'auto',
        seq: nextSeq++,
      })
      existingAppIds.add(item.appId)
    })
  })

  if (newItems.length === 0) return sanitizedLayout

  const pages = sanitizedLayout.pages.map((page) => ({
    ...page,
    items: [...page.items],
  }))
  if (pages.length === 0) {
    pages.push({ id: `${formFactor}-page-1`, items: [] })
  }
  pages[pages.length - 1].items.push(...newItems)
  return { ...sanitizedLayout, pages }
}

export function getPageIndex(layoutState: LayoutState, itemId: string) {
  return layoutState.pages.findIndex((page) =>
    page.items.some((item) => item.id === itemId),
  )
}

// ---------------------------------------------------------------------------
// Grid ↔ layout interop
// ---------------------------------------------------------------------------

export interface GridLayoutItem {
  i: string
  x: number
  y: number
  w: number
  h: number
  static: boolean
}

export function mapPageToGrid(page: DesktopPageState): GridLayoutItem[] {
  return page.items
    .filter(
      (item): item is LayoutItem & { x: number; y: number } =>
        item.x !== undefined && item.y !== undefined,
    )
    .map((item) => ({
      i: item.id,
      x: item.x,
      y: item.y,
      w: item.w,
      h: item.h,
      static: false,
    }))
}

// ---------------------------------------------------------------------------
// Viewport progress
// ---------------------------------------------------------------------------

export function normalizeViewportProgress(
  progress: number,
  pageCount: number,
) {
  if (!Number.isFinite(progress) || pageCount <= 1) return 0
  return Math.min(Math.max(progress, 0), 1)
}

export const desktopMinCanvasSize = { width: 960, height: 720 }

// ---------------------------------------------------------------------------
// Slot sync: ensure slotIndex is populated from x/y (migration helper)
// ---------------------------------------------------------------------------

/**
 * Migrate a layout so that every positioned item has a slotIndex and seq.
 * Used when loading layouts that predate the slot-based model.
 */
export function migrateToSlotModel(
  layout: LayoutState,
  cols: number,
  rows: number,
): LayoutState {
  const order: ScanOrder = layout.formFactor === 'mobile' ? 'row-major' : 'col-major'
  let globalSeq = 0

  const pages = layout.pages.map((page, pageIdx) => ({
    ...page,
    items: page.items.map((item) => {
      const needsSlot = item.slotIndex === undefined && item.x !== undefined && item.y !== undefined
      const needsSeq = item.seq === undefined
      if (!needsSlot && !needsSeq) return item
      return {
        ...item,
        slotIndex: needsSlot
          ? coordToSlot(item.x!, item.y!, cols, rows, order)
          : item.slotIndex,
        preferredPage: item.preferredPage ?? pageIdx,
        placementType: item.placementType ?? ('manual' as PlacementType),
        seq: needsSeq ? globalSeq++ : item.seq,
      }
    }),
  }))

  return { ...layout, pages }
}
