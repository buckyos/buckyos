/**
 * Layout helper functions extracted from DesktopRoute.
 *
 * Pure functions for grid computation, layout resolution,
 * dead-zone migration, and position management.
 */
import type {
  DeadZone,
  DesktopPageState,
  FormFactor,
  LayoutItem,
  LayoutState,
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
 * Minimum row height on desktop -- more compact than the density value.
 * icon-padding-top(10) + icon(48) + label-padding(4) + 1 line(16) = 78
 */
export const DESKTOP_MIN_ROW_HEIGHT = 78

/**
 * Compute how many rows fit in the available height.
 */
export function rowsForHeight(
  height: number,
  density: GridDensity,
  isMobile: boolean,
): number {
  const slotH = isMobile ? densityRowHeight[density] : DESKTOP_MIN_ROW_HEIGHT
  return Math.max(1, Math.floor((height + GRID_GAP) / (slotH + GRID_GAP)))
}

/**
 * Compute actual row height so the grid fills the entire container height evenly.
 */
export function stretchedRowHeight(height: number, rows: number): number {
  if (rows <= 0) return densityRowHeight.medium
  return (height - (rows - 1) * GRID_GAP) / rows
}

// ---------------------------------------------------------------------------
// Grid slot scanning
// ---------------------------------------------------------------------------

export type ScanOrder = 'row-major' | 'col-major'

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

function findTailSlotColMajor(
  page: DesktopPageState,
  w: number,
  h: number,
  cols: number,
  rows: number,
): { x: number; y: number } | null {
  let maxLinearEnd = 0
  for (const item of page.items) {
    if (item.x === undefined || item.y === undefined) continue
    for (let col = item.x; col < item.x + item.w; col++) {
      const linearEnd = col * rows + (item.y + item.h)
      maxLinearEnd = Math.max(maxLinearEnd, linearEnd)
    }
  }
  const startCol = Math.floor(maxLinearEnd / rows)
  const startRow = maxLinearEnd % rows
  for (let x = startCol; x + w <= cols; x++) {
    const sy = x === startCol ? startRow : 0
    for (let y = sy; y + h <= rows; y++) {
      if (fits(page, x, y, w, h, cols, rows)) return { x, y }
    }
  }
  return null
}

/**
 * Find a slot at the tail of the page (after the last positioned content).
 */
export function findTailSlot(
  page: DesktopPageState,
  w: number,
  h: number,
  cols: number,
  rows: number,
  scanOrder: ScanOrder = 'row-major',
): { x: number; y: number } | null {
  if (scanOrder === 'col-major') {
    return findTailSlotColMajor(page, w, h, cols, rows)
  }
  // row-major (mobile)
  let maxLinearEnd = 0
  for (const item of page.items) {
    if (item.x === undefined || item.y === undefined) continue
    for (let row = item.y; row < item.y + item.h; row++) {
      const linearEnd = row * cols + (item.x + item.w)
      maxLinearEnd = Math.max(maxLinearEnd, linearEnd)
    }
  }
  const startRow = Math.floor(maxLinearEnd / cols)
  const startCol = maxLinearEnd % cols
  for (let y = startRow; y + h <= rows; y++) {
    const sx = y === startRow ? startCol : 0
    for (let x = sx; x + w <= cols; x++) {
      if (fits(page, x, y, w, h, cols, rows)) return { x, y }
    }
  }
  return null
}

// ---------------------------------------------------------------------------
// Layout resolution
// ---------------------------------------------------------------------------

/**
 * Mark positioned items as unpositioned when their position exceeds
 * the current grid bounds.
 */
export function invalidatePositions(
  layout: LayoutState,
  cols: number,
  rows: number,
): LayoutState {
  let anyChanged = false
  const pages = layout.pages.map((page) => {
    let pageChanged = false
    const items = page.items.map((item) => {
      if (item.x === undefined || item.y === undefined) return item
      if (item.x + item.w > cols || item.y + item.h > rows) {
        pageChanged = true
        return { ...item, x: undefined, y: undefined }
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
 */
export function resolveLayout(
  layout: LayoutState,
  cols: number,
  rows: number,
  scanOrder: ScanOrder = 'row-major',
): LayoutState {
  const unpositioned: LayoutItem[] = []
  const resolvedPages: DesktopPageState[] = layout.pages.map((page) => ({
    ...page,
    items: page.items.filter((item) => {
      if (item.x === undefined || item.y === undefined) {
        unpositioned.push(item)
        return false
      }
      return true
    }),
  }))

  if (unpositioned.length === 0) return layout

  for (const item of unpositioned) {
    let placed = false
    for (const page of resolvedPages) {
      const slot = findTailSlot(page, item.w, item.h, cols, rows, scanOrder)
      if (slot) {
        page.items.push({ ...item, x: slot.x, y: slot.y })
        placed = true
        break
      }
    }
    if (!placed) {
      resolvedPages.push({
        id: `${layout.formFactor}-page-${resolvedPages.length + 1}`,
        items: [{ ...item, x: 0, y: 0 }],
      })
    }
  }
  return { ...layout, pages: resolvedPages }
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
  defaultLayout.pages.forEach((defaultPage) => {
    defaultPage.items.forEach((item) => {
      if (
        item.type !== 'app' ||
        !launcherAppIds.has(item.appId) ||
        existingAppIds.has(item.appId)
      ) {
        return
      }
      newItems.push({ ...item, x: undefined, y: undefined })
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
