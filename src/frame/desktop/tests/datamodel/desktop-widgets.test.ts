/**
 * Widget sizes and resizing on the desktop grid.
 *   deno test --no-check --allow-env --allow-read --node-modules-dir=manual --unstable-sloppy-imports tests/datamodel/desktop-widgets.test.ts
 */
import { clampWidgetSize, defaultWidgetSize, isResizableWidget, widgetSizeLimits } from '../../src/desktop/widgets/catalog.ts'
import { applyDragCollision, resizeGridRect } from '../../src/models/layout.ts'
import type { DesktopPageState } from '../../src/models/ui.ts'

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
}

const grid = { cols: 10, rows: 6 }
const clockLimits = widgetSizeLimits('clock', grid)

Deno.test('a widget declares its sizes; one the desktop does not know keeps its own', () => {
  assertEquals(defaultWidgetSize('clock'), { w: 2, h: 1 }, 'clock default')
  assertEquals(clockLimits, { min: { w: 2, h: 1 }, max: { w: 4, h: 2 } }, 'clock limits')
  assertEquals(isResizableWidget('clock'), true, 'clock resizes')
  assertEquals(isResizableWidget('weather'), false, 'unknown type does not')
  assertEquals(clampWidgetSize('notepad', { w: 9, h: 1 }), { w: 4, h: 2 }, 'saved size kept in the limits')
  assertEquals(clampWidgetSize('weather', { w: 3, h: 5 }), { w: 3, h: 5 }, 'unknown type keeps its size')
  assertEquals(widgetSizeLimits('notepad', { cols: 4, rows: 3 }).max, { w: 4, h: 3 }, 'never more than the grid')
})

Deno.test('a right or bottom handle grows and shrinks within the limits', () => {
  const start = { x: 1, y: 1, w: 2, h: 1 }
  assertEquals(resizeGridRect(start, { dx: 1, dy: 1 }, { cols: 1, rows: 1 }, clockLimits, grid), { x: 1, y: 1, w: 3, h: 2 }, 'one cell each way')
  assertEquals(resizeGridRect(start, { dx: 1, dy: 1 }, { cols: 9, rows: 9 }, clockLimits, grid), { x: 1, y: 1, w: 4, h: 2 }, 'stops at the largest size')
  assertEquals(resizeGridRect(start, { dx: 1, dy: 0 }, { cols: -5, rows: 3 }, clockLimits, grid), { x: 1, y: 1, w: 2, h: 1 }, 'stops at the smallest; the other axis is not moved')
  assertEquals(resizeGridRect({ x: 8, y: 5, w: 2, h: 1 }, { dx: 1, dy: 1 }, { cols: 2, rows: 1 }, clockLimits, grid), { x: 8, y: 5, w: 2, h: 1 }, 'stops at the grid edge')
})

Deno.test('a left or top handle moves that edge and keeps the opposite one', () => {
  const start = { x: 3, y: 2, w: 2, h: 1 }
  assertEquals(resizeGridRect(start, { dx: -1, dy: -1 }, { cols: -2, rows: -1 }, clockLimits, grid), { x: 1, y: 1, w: 4, h: 2 }, 'grows to the top left')
  assertEquals(resizeGridRect({ x: 1, y: 0, w: 2, h: 1 }, { dx: -1, dy: -1 }, { cols: -5, rows: -5 }, clockLimits, grid), { x: 0, y: 0, w: 3, h: 1 }, 'stops at the grid edge')
  assertEquals(resizeGridRect({ x: 0, y: 0, w: 4, h: 2 }, { dx: -1, dy: -1 }, { cols: 3, rows: 2 }, clockLimits, grid), { x: 2, y: 1, w: 2, h: 1 }, 'shrinks toward the bottom right')
})

Deno.test('a resized widget takes its new size and moves what is in the way', () => {
  const page: DesktopPageState = {
    id: 'p',
    items: [
      { id: 'clock', type: 'widget', widgetType: 'clock', config: {}, x: 0, y: 0, w: 2, h: 1 },
      { id: 'files', type: 'app', appId: 'files', x: 2, y: 0, w: 1, h: 1 },
    ],
  }
  const next = applyDragCollision(page, { i: 'clock', x: 0, y: 0, w: 2, h: 1 }, { i: 'clock', x: 0, y: 0, w: 3, h: 1 }, 6, 4, 'col-major')
  const at = Object.fromEntries(next.items.map((item) => [item.id, [item.x, item.y, item.w, item.h]]))
  assertEquals(at.clock, [0, 0, 3, 1], 'new size')
  assertEquals(at.files, [2, 1, 1, 1], 'files moved to the nearest free cell')
})
