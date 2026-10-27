/* Handwriting on the free canvas (notepads/aiworkspace-handwriting-todo.md D4, H01–H09, H11): pen and mouse strokes
 * through the browser's real input path (CDP: a pen with pressure, fingers on a touch screen), synthetic pointer events
 * only where CDP has no second pen or no cancel; what is asserted is what the user sees (the drawn Blocks, the camera,
 * the selection) and what is stored (the backend's outline and configs), never the app's inner state alone.
 * Real devices (palm rejection of a system, pen latency) are not covered here: see the TODO's device matrix. */

import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { CDPSession, Page } from '@playwright/test'
import { expectAllCommitted, fitAll, hooks, openCanvas, setCanvasMode, test, expect, zoomIn, type Api } from './fixtures'

const ALICE = 'tok-alice'
const SHOTS = 'test-results/aiworkspace-handwriting'

type Json = Record<string, unknown>
interface Point { x: number; y: number }

/** A free canvas "笔记" with a shape (box) to write over, a group (frame) and a flow page. */
async function inkWorkspace(api: Api, title: string, extra: Json[] = []) {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const result = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'notes-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a1', name: '笔记', payload: { kind: 'folder', title: '笔记', system: 'surface_content', surface_id: 'notes' } },
    { op: 'entity.create', entity_id: 'notes', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a1', name: '笔记', payload: { kind: 'surface', layout: { mode: 'free' }, title: '笔记', content_folder_id: 'notes-content' } },
    { op: 'entity.create', entity_id: 'page-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a2', name: '页', payload: { kind: 'folder', title: '页', system: 'surface_content', surface_id: 'page' } },
    { op: 'entity.create', entity_id: 'page', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a2', name: '页', payload: { kind: 'surface', layout: { mode: 'flow' }, title: '页', content_folder_id: 'page-content' } },
    { op: 'entity.create', entity_id: 'box', type_id: 'buckyos.cell', parent_id: 'notes', order_key: 'kb', placement: { x: 300, y: 260, w: 220, h: 140 }, payload: { view: { type: 'shape' }, title: 'Box', config: { shape: 'rect', fill: '#e8f0fe', stroke: '#4f8df7' } } },
    { op: 'entity.create', entity_id: 'frame-g', type_id: 'buckyos.container', parent_id: 'notes', order_key: 'kg', placement: { x: 700, y: 120, w: 360, h: 300 }, payload: { kind: 'group', layout: { mode: 'free' }, title: '框' } },
    ...extra,
  ]) as { status: string }
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  return ws
}

/** A pen through DevTools input: real pointer events of type `pen`, with pressure (`force`) and tilt. */
class Pen {
  private readonly cdp: CDPSession
  constructor(cdp: CDPSession) { this.cdp = cdp }
  static async of(page: Page) { return new Pen(await page.context().newCDPSession(page)) }
  down(p: Point, force = 0.5) { return this.cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: p.x, y: p.y, button: 'left', buttons: 1, clickCount: 1, pointerType: 'pen', force }) }
  move(p: Point, force = 0.5, buttons = 1) { return this.cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: p.x, y: p.y, button: buttons ? 'left' : 'none', buttons, pointerType: 'pen', force }) }
  up(p: Point) { return this.cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: p.x, y: p.y, button: 'left', buttons: 0, clickCount: 1, pointerType: 'pen', force: 0 }) }
  /** Down at the first point, through the others (pressure per point), up at the last. */
  async stroke(page: Page, points: Point[], force: (i: number) => number = () => 0.5, { frame = 0 } = {}) {
    await this.down(points[0], force(0))
    for (let i = 1; i < points.length; i++) { await this.move(points[i], force(i)); if (frame) await page.waitForTimeout(frame) }
    await this.up(points.at(-1)!)
  }
}

/** Fingers on the touch screen (one touch event per call, like a digitizer). The touch screen is switched on once the
 * workspace is open: the Desktop shell treats a touch device as a tablet from the start. */
class Fingers {
  private readonly cdp: CDPSession
  constructor(cdp: CDPSession) { this.cdp = cdp }
  static async of(page: Page) {
    const cdp = await page.context().newCDPSession(page)
    await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 })
    return new Fingers(cdp)
  }
  private send(type: 'touchStart' | 'touchMove' | 'touchEnd', points: Point[]) { return this.cdp.send('Input.dispatchTouchEvent', { type, touchPoints: points.map((p, id) => ({ x: p.x, y: p.y, id, radiusX: 18, radiusY: 18 })) }) }
  down(...points: Point[]) { return this.send('touchStart', points) }
  move(...points: Point[]) { return this.send('touchMove', points) }
  up() { return this.send('touchEnd', []) }
}

/** The camera as the world layer shows it: screen = origin + world × zoom (page coordinates). */
async function camera(page: Page) {
  const box = (await page.getByTestId('aiws-canvas').boundingBox())!
  const m = await page.getByTestId('aiws-world').evaluate((el) => new DOMMatrix(getComputedStyle(el).transform))
  return { zoom: m.a, transform: `${m.a},${m.e},${m.f}`, toScreen: (w: Point) => ({ x: box.x + m.e + w.x * m.a, y: box.y + m.f + w.y * m.d }), toWorld: (s: Point) => ({ x: (s.x - box.x - m.e) / m.a, y: (s.y - box.y - m.f) / m.d }), box }
}

async function shot(page: Page, name: string) {
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/${name}.png` })
}

/** The ink Blocks of the canvas as the backend's outline projects them. */
async function inkCells(api: Api, workspaceId: string): Promise<Json[]> {
  return (await api.outline(ALICE, workspaceId)).filter((e: Json) => e.view_type === 'ink' && !e.deleted)
}
async function config(api: Api, workspaceId: string, id: string): Promise<Json> {
  return (await api.read(ALICE, workspaceId, id)).content.payload.config as Json
}
async function placementOf(api: Api, workspaceId: string, id: string) {
  return (await api.outline(ALICE, workspaceId)).find((e: Json) => e.entity_id === id)?.placement as Json
}

/** Nothing waits to be drawn by the live layer, nothing is being written. */
async function inkSettled(page: Page) {
  await expect.poll(async () => JSON.stringify((await hooks(page)).canvas?.['ink' as never] ?? null)).toBe(JSON.stringify({ busy: false, waiting: 0, erasing: 0 }))
}

/** A wavy line from `a` to `b` in screen space. */
function wave(a: Point, b: Point, n: number, amp = 12): Point[] {
  return Array.from({ length: n }, (_, i) => ({ x: a.x + ((b.x - a.x) * i) / (n - 1), y: a.y + ((b.y - a.y) * i) / (n - 1) + Math.sin(i / 3) * amp }))
}

async function openNotes(page: Page, workspaceId: string) {
  await openCanvas(page, ALICE, workspaceId)
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'notes')
}

async function pickPen(page: Page) {
  await page.getByTestId('aiws-tool-pen').click()
  await expect(page.getByTestId('aiws-ink-bar')).toBeVisible()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'pen')
}

test('H01 pen strokes: pressure widens the line, a tap is a dot, a fast stroke keeps its end, a lift outside the canvas saves once', async ({ page, api }) => {
  const ws = await inkWorkspace(api, `h01 ${Date.now()}`)
  await openNotes(page, ws.workspace_id)
  await pickPen(page)
  const pen = await Pen.of(page)
  const before = (await hooks(page)).commits
  let cam = await camera(page)
  // pressure rising along a line over the box: written, the box neither selected nor moved
  const line = wave(cam.toScreen({ x: 260, y: 300 }), cam.toScreen({ x: 600, y: 330 }), 40, 6)
  await pen.stroke(page, line, (i) => 0.05 + (0.9 * i) / 39)
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(1)
  // a tap
  await pen.stroke(page, [cam.toScreen({ x: 640, y: 300 })], () => 0.7)
  // a fast stroke: few samples far apart, its last point is where the pen lifted
  const fastEnd = cam.toScreen({ x: 600, y: 520 })
  await pen.stroke(page, [cam.toScreen({ x: 200, y: 520 }), cam.toScreen({ x: 400, y: 540 }), fastEnd], () => 0.6)
  // out of the canvas: the pen goes over the object toolbar and lifts there (pointer capture keeps the stroke)
  const tools = (await page.getByTestId('aiws-object-toolbar').boundingBox())!
  await pen.stroke(page, [cam.toScreen({ x: 200, y: 620 }), cam.toScreen({ x: 120, y: 640 }), { x: tools.x + tools.width / 2, y: tools.y + 40 }], () => 0.5)
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(4)
  expect((await hooks(page)).commits - before, 'one commit per stroke').toBe(4)
  await inkSettled(page)
  await expectAllCommitted(page)
  const cells = await inkCells(api, ws.workspace_id)
  const configs = await Promise.all(cells.map(async (c) => ({ cell: c, config: await config(api, ws.workspace_id, c.entity_id as string) })))
  for (const { cell, config: c } of configs) {
    expect(cell.ink_stroke).toBe(c.stroke)
    expect([c.seg, c.segs]).toEqual([0, 1])
    await expect(page.getByTestId(`aiws-ink-${cell.entity_id}`)).toBeVisible()
  }
  const points = (c: Json) => c.points as { x: number[]; y: number[]; t: number[]; p?: number[] }
  const pressured = configs.find(({ config: c }) => points(c).x.length > 10)!
  const p = points(pressured.config).p!
  expect(p[0]).toBeLessThan(0.2)
  expect(p.at(-1)!).toBeGreaterThan(0.85)
  expect(pressured.config.pressure).toBe(true)
  // drawn with several widths
  expect(Number(await page.getByTestId(`aiws-ink-${pressured.cell.entity_id}`).getAttribute('data-paths'))).toBeGreaterThan(3)
  const dot = configs.find(({ config: c }) => points(c).x.length === 1)!
  expect(points(dot.config).p).toEqual([0.7])
  const fast = configs.find(({ cell }) => (cell.placement as Json).y as number > 480 && (cell.placement as Json).y as number < 560)!
  const fc = fast.config
  const fp = fast.cell.placement as { x: number; y: number }
  const end = { x: fp.x + points(fc).x.at(-1)!, y: fp.y + points(fc).y.at(-1)! }
  expect(Math.hypot(end.x - 600, end.y - 520)).toBeLessThan(1.5)
  // time runs on, from 0
  for (const { config: c } of configs) { const t = points(c).t; expect(t[0]).toBe(0); for (let i = 1; i < t.length; i++) expect(t[i]).toBeGreaterThanOrEqual(t[i - 1]) }
  expect(await placementOf(api, ws.workspace_id, 'box')).toEqual({ x: 300, y: 260, w: 220, h: 140 })
  await expect(page.locator('[data-testid^="aiws-selection-"]')).toHaveCount(0)
  cam = await camera(page)
  await shot(page, 'h01-strokes')
})

test('H02 palm and pen: a palm before, during and after the pen never moves the canvas, writes or selects; two fingers navigate unless touch is locked', async ({ page, api }) => {
  const ws = await inkWorkspace(api, `h02 ${Date.now()}`)
  await openNotes(page, ws.workspace_id)
  await pickPen(page)
  const pen = await Pen.of(page)
  const fingers = await Fingers.of(page)
  let cam = await camera(page)
  const still = cam.transform
  const at = (x: number, y: number) => cam.toScreen({ x, y })
  // the palm lands first (on the box), the pen writes, the palm slides while it writes, the pen lifts, the palm slides on
  await fingers.down(at(420, 330))
  await pen.down(at(200, 150), 0.4)
  for (let i = 1; i <= 10; i++) { await pen.move(at(200 + i * 10, 150 + (i % 3) * 4), 0.5); if (i === 5) await fingers.move(at(450, 360)) }
  await pen.up(at(300, 158))
  await fingers.move(at(520, 420))
  await fingers.move(at(560, 460))
  await fingers.up()
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(1)
  // the pen first, then a palm reported as two contacts, the pen lifts, the palm goes on moving
  await pen.down(at(200, 200), 0.5)
  await pen.move(at(230, 205))
  await fingers.down(at(420, 330), at(470, 360))
  await fingers.move(at(440, 350), at(500, 390))
  await pen.move(at(260, 210))
  await pen.up(at(280, 212))
  await fingers.move(at(470, 380), at(540, 430))
  await fingers.move(at(520, 420), at(600, 470))
  await fingers.up()
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(2)
  // one finger alone: no pan, no stroke, no selection, no menu
  await fingers.down(at(420, 330))
  await page.waitForTimeout(650)
  await fingers.move(at(470, 380))
  await fingers.move(at(520, 430))
  await fingers.up()
  cam = await camera(page)
  expect(cam.transform, 'nothing moved the canvas').toBe(still)
  expect((await inkCells(api, ws.workspace_id)).length).toBe(2)
  expect(await placementOf(api, ws.workspace_id, 'box')).toEqual({ x: 300, y: 260, w: 220, h: 140 })
  await expect(page.locator('[data-testid^="aiws-selection-"]')).toHaveCount(0)
  await expect(page.locator('.aiws-menu')).toHaveCount(0)
  // two fingers (no stroke running) pan and zoom; when one lifts, the other moves nothing
  const a0 = at(400, 500), b0 = at(600, 500)
  await fingers.down(a0, b0)
  for (let i = 1; i <= 6; i++) await fingers.move({ x: a0.x - i * 10, y: a0.y + i * 8 }, { x: b0.x + i * 10, y: b0.y + i * 8 })
  await fingers.up()
  await page.waitForTimeout(100)
  const pinched = (await camera(page)).transform
  expect(pinched).not.toBe(still)
  // one of two fingers lifts and the other moves on: CDP lifts all fingers at once, so this one is synthetic touch
  await synthTouch(page, 'pointerdown', 101, a0)
  await synthTouch(page, 'pointerdown', 102, b0)
  for (let i = 1; i <= 4; i++) { await synthTouch(page, 'pointermove', 101, { x: a0.x - i * 12, y: a0.y }); await synthTouch(page, 'pointermove', 102, { x: b0.x + i * 12, y: b0.y }); await page.waitForTimeout(20) }
  await page.waitForTimeout(100)
  const twoFingers = (await camera(page)).transform
  expect(twoFingers, 'two synthetic fingers zoom').not.toBe(pinched)
  await synthTouch(page, 'pointerup', 102, { x: b0.x + 48, y: b0.y })
  for (let i = 1; i <= 4; i++) { await synthTouch(page, 'pointermove', 101, { x: a0.x - 48 - i * 30, y: a0.y + i * 30 }); await page.waitForTimeout(20) }
  await synthTouch(page, 'pointerup', 101, { x: a0.x - 168, y: a0.y + 120 })
  await page.waitForTimeout(100)
  expect((await camera(page)).transform, 'the finger left from a pinch does not pan').toBe(twoFingers)
  // a touch the system takes back at once (a cancel, no move) leaves nothing behind: two fingers pan again
  await synthTouch(page, 'pointerdown', 103, at(300, 300))
  await synthTouch(page, 'pointercancel', 103, at(300, 300))
  await fingers.down(a0, b0)
  for (let i = 1; i <= 6; i++) await fingers.move({ x: a0.x + i * 12, y: a0.y - i * 6 }, { x: b0.x + i * 12, y: b0.y - i * 6 })
  await fingers.up()
  await page.waitForTimeout(100)
  expect((await camera(page)).transform, 'a cancelled touch is forgotten').not.toBe(twoFingers)
  // the touch lock: two fingers move nothing
  await page.getByTestId('aiws-ink-touch-lock').click()
  await expect(page.getByTestId('aiws-ink-touch-lock')).toHaveAttribute('aria-pressed', 'true')
  cam = await camera(page)
  const locked = cam.transform
  await fingers.down(at(400, 500), at(600, 500))
  for (let i = 1; i <= 6; i++) await fingers.move(at(400 - i * 15, 500 + i * 10), at(600 + i * 15, 500 + i * 10))
  await fingers.up()
  await page.waitForTimeout(100)
  expect((await camera(page)).transform, 'locked').toBe(locked)
  // the lock is a personal setting
  await expect.poll(async () => (await api.rpc(ALICE, 'ws.get_user_state', { workspace_id: ws.workspace_id })).entries['ui:ink']?.touchLock).toBe(true)
  await shot(page, 'h02-palm')
})

test('H03 a pen landing stops a running pinch, an object drag and a glide; the stroke lands under the pen', async ({ page, api }) => {
  const ws = await inkWorkspace(api, `h03 ${Date.now()}`)
  await openNotes(page, ws.workspace_id)
  const pen = await Pen.of(page)
  const fingers = await Fingers.of(page)
  // a glide after a one-finger flick (select tool), then the pen: the camera stops where it is
  let cam = await camera(page)
  const from = cam.toScreen({ x: 900, y: 600 })
  await fingers.down(from)
  for (let i = 1; i <= 5; i++) await fingers.move({ x: from.x - i * 40, y: from.y - i * 25 })
  await fingers.up()
  await page.keyboard.press('p')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'pen')
  await pen.down({ x: 700, y: 500 }, 0.5)
  const atDown = (await camera(page)).transform
  await page.waitForTimeout(250)
  expect((await camera(page)).transform, 'the glide stopped').toBe(atDown)
  await pen.move({ x: 720, y: 505 })
  await pen.up({ x: 740, y: 510 })
  // a pinch under way when the pen lands: it stops and its fingers stay ignored
  cam = await camera(page)
  await fingers.down({ x: 500, y: 600 }, { x: 800, y: 600 })
  await fingers.move({ x: 480, y: 610 }, { x: 820, y: 610 })
  const pinch = (await camera(page)).transform
  await pen.down({ x: 900, y: 300 }, 0.5)
  const landed = await camera(page)
  await fingers.move({ x: 400, y: 650 }, { x: 900, y: 650 })
  await pen.move({ x: 950, y: 320 })
  await fingers.move({ x: 300, y: 700 }, { x: 1000, y: 700 })
  await pen.up({ x: 1000, y: 330 })
  await fingers.up()
  expect((await camera(page)).transform, 'no navigation after the pen came down').toBe(landed.transform)
  expect(pinch).not.toBe(cam.transform)
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(2)
  const cells = await Promise.all((await inkCells(api, ws.workspace_id)).map(async (c) => ({ c, config: await config(api, ws.workspace_id, c.entity_id as string) })))
  // where the second stroke starts, in world coordinates of the camera at the pen's landing
  const startWorld = landed.toWorld({ x: 900, y: 300 })
  const placed = cells.map(({ c, config: k }) => ({ x: (c.placement as { x: number }).x + (k.points as { x: number[] }).x[0], y: (c.placement as { y: number }).y + (k.points as { y: number[] }).y[0] }))
  expect(placed.some((p) => Math.hypot(p.x - startWorld.x, p.y - startWorld.y) < 1.5), JSON.stringify({ placed, startWorld })).toBe(true)
  // a mouse drag under way (the middle button pans while writing) is cut short by the pen: the camera stays where the
  // pen landed, the drag commits nothing (CDP's pen shares the mouse's buttons: the pen here is synthetic)
  const commits = (await hooks(page)).commits
  await page.mouse.move(800, 500)
  await page.mouse.down({ button: 'middle' })
  await page.mouse.move(760, 470, { steps: 4 })
  const root = (await page.getByTestId('aiws-canvas').boundingBox())!
  await synth(page, 'pointerdown', { pointerId: 31, pointerType: 'pen', clientX: root.x + 1000, clientY: root.y + 600 })
  const penLanded = (await camera(page)).transform
  await page.mouse.move(700, 420, { steps: 4 })
  await synth(page, 'pointermove', { pointerId: 31, pointerType: 'pen', clientX: root.x + 1020, clientY: root.y + 612 })
  await synth(page, 'pointerup', { pointerId: 31, pointerType: 'pen', buttons: 0, clientX: root.x + 1040, clientY: root.y + 620 })
  await page.mouse.move(650, 380, { steps: 2 })
  await page.mouse.up({ button: 'middle' })
  expect((await camera(page)).transform, 'the pan stopped when the pen landed').toBe(penLanded)
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(3)
  expect((await hooks(page)).commits - commits, 'the stroke only').toBe(1)
  expect(await placementOf(api, ws.workspace_id, 'box')).toEqual({ x: 300, y: 260, w: 220, h: 140 })
})

/** A synthetic touch: down on the canvas, moves and lifts on the window (where the canvas follows touches). */
async function synthTouch(page: Page, type: 'pointerdown' | 'pointermove' | 'pointerup' | 'pointercancel', pointerId: number, p: Point) {
  await page.getByTestId('aiws-canvas').evaluate((el, [t, id, x, y]) => {
    const event = new PointerEvent(t as string, { bubbles: true, cancelable: true, composed: true, pointerId: id as number, pointerType: 'touch', clientX: x as number, clientY: y as number, button: t === 'pointermove' || t === 'pointercancel' ? -1 : 0, buttons: t === 'pointerup' || t === 'pointercancel' ? 0 : 1, pressure: 0.5 })
    if (t === 'pointerdown') el.dispatchEvent(event); else window.dispatchEvent(event)
  }, [type, pointerId, p.x, p.y] as const)
}

/** Synthetic pointer events on the canvas: a second pen and cancels CDP cannot produce (they prove the app's dispatch only). */
async function synth(page: Page, type: string, init: Json) {
  await page.getByTestId('aiws-canvas').evaluate((el, [t, i]) => {
    el.dispatchEvent(new PointerEvent(t as string, { bubbles: true, cancelable: true, composed: true, isPrimary: false, button: 0, buttons: 1, pressure: 0.5, width: 1, height: 1, ...(i as PointerEventInit) }))
  }, [type, init] as const)
}

test('H04 the stroke belongs to its pointer: a second pen, other pointers\' moves, ups and cancels, and a reused id', async ({ page, api }) => {
  const ws = await inkWorkspace(api, `h04 ${Date.now()}`)
  await openNotes(page, ws.workspace_id)
  await pickPen(page)
  const box = (await page.getByTestId('aiws-canvas').boundingBox())!
  const p = (x: number, y: number) => ({ clientX: box.x + x, clientY: box.y + y })
  await synth(page, 'pointerdown', { pointerId: 11, pointerType: 'pen', ...p(200, 200) })
  await synth(page, 'pointermove', { pointerId: 11, pointerType: 'pen', ...p(220, 207) })
  // a second pen: does not start, take over, end or change the first one's stroke
  await synth(page, 'pointerdown', { pointerId: 12, pointerType: 'pen', ...p(600, 600) })
  await synth(page, 'pointermove', { pointerId: 12, pointerType: 'pen', ...p(650, 620) })
  await synth(page, 'pointerup', { pointerId: 12, pointerType: 'pen', buttons: 0, ...p(650, 620) })
  await synth(page, 'pointercancel', { pointerId: 12, pointerType: 'pen', ...p(650, 620) })
  await synth(page, 'pointerup', { pointerId: 13, pointerType: 'mouse', buttons: 0, ...p(10, 10) })
  await synth(page, 'pointermove', { pointerId: 11, pointerType: 'pen', ...p(240, 210) })
  expect((await hooks(page)).canvas?.['ink' as never]).toMatchObject({ busy: true })
  await synth(page, 'pointerup', { pointerId: 11, pointerType: 'pen', buttons: 0, pressure: 0, ...p(260, 212) })
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(1)
  const [cell] = await inkCells(api, ws.workspace_id)
  const c = await config(api, ws.workspace_id, cell.entity_id as string)
  const xs = (c.points as { x: number[] }).x.map((x) => x + (cell.placement as { x: number }).x)
  const cam = await camera(page)
  expect(xs.map((x) => Math.round(x - cam.toWorld({ x: box.x, y: 0 }).x)), JSON.stringify(c.points)).toEqual([200, 220, 240, 260])
  // the same pointer id writes the next stroke; synthetic events have no coalesced samples: the event itself counts
  await synth(page, 'pointerdown', { pointerId: 11, pointerType: 'pen', ...p(300, 300) })
  await synth(page, 'pointermove', { pointerId: 11, pointerType: 'pen', ...p(330, 300) })
  await synth(page, 'pointerup', { pointerId: 11, pointerType: 'pen', buttons: 0, ...p(330, 300) })
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(2)
  await inkSettled(page)
})

test('H05 a stroke is dropped by pointercancel, a lost capture, Esc, another tool or canvas, view mode, a hidden page; the wheel waits; the next stroke works', async ({ page, api }) => {
  const ws = await inkWorkspace(api, `h05 ${Date.now()}`)
  await openNotes(page, ws.workspace_id)
  await pickPen(page)
  const pen = await Pen.of(page)
  const commits = (await hooks(page)).commits
  const box = (await page.getByTestId('aiws-canvas').boundingBox())!
  const dropped = async () => { await expect.poll(async () => JSON.stringify((await hooks(page)).canvas?.['ink' as never])).toBe(JSON.stringify({ busy: false, waiting: 0, erasing: 0 })) }
  // Esc: the stroke goes, the pen stays the tool
  await pen.down({ x: 400, y: 300 }); await pen.move({ x: 450, y: 320 })
  await page.keyboard.press('Escape')
  await dropped()
  await pen.move({ x: 500, y: 340 }); await pen.up({ x: 520, y: 350 })
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'pen')
  // another tool (E)
  await pen.down({ x: 400, y: 400 }); await pen.move({ x: 450, y: 420 })
  await page.keyboard.press('e')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'eraser')
  await dropped()
  await pen.up({ x: 460, y: 430 })
  await page.keyboard.press('p')
  // pointercancel and a capture lost while writing (synthetic: the system's palm rejection or a stolen capture)
  for (const ending of ['pointercancel', 'lostpointercapture']) {
    await synth(page, 'pointerdown', { pointerId: 21, pointerType: 'pen', clientX: box.x + 300, clientY: box.y + 500 })
    await synth(page, 'pointermove', { pointerId: 21, pointerType: 'pen', clientX: box.x + 340, clientY: box.y + 520 })
    await synth(page, ending, { pointerId: 21, pointerType: 'pen', clientX: box.x + 340, clientY: box.y + 520 })
    await dropped()
    await synth(page, 'pointerup', { pointerId: 21, pointerType: 'pen', buttons: 0, clientX: box.x + 340, clientY: box.y + 520 })
  }
  // the page is hidden while writing
  await pen.down({ x: 600, y: 300 }); await pen.move({ x: 650, y: 320 })
  await page.evaluate(() => { Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true }); document.dispatchEvent(new Event('visibilitychange')) })
  await dropped()
  await page.evaluate(() => { Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true }); document.dispatchEvent(new Event('visibilitychange')) })
  await pen.up({ x: 660, y: 330 })
  // another canvas while writing (a synthetic pen: the mouse picks the canvas meanwhile): the stroke goes with it
  const writing = async () => { await expect.poll(async () => JSON.stringify((await hooks(page)).canvas?.['ink' as never])).toContain('"busy":true') }
  await synth(page, 'pointerdown', { pointerId: 22, pointerType: 'pen', clientX: box.x + 300, clientY: box.y + 300 })
  await synth(page, 'pointermove', { pointerId: 22, pointerType: 'pen', clientX: box.x + 340, clientY: box.y + 320 })
  await writing()
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-page').getByRole('menuitem').click()
  await expect(page.getByTestId('aiws-flow-host')).toBeVisible()
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-notes').getByRole('menuitem').click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'notes')
  await dropped()
  await pickPen(page)
  // view mode while writing (the way a lost write permission goes too): the stroke goes, the pen is put down
  await synth(page, 'pointerdown', { pointerId: 23, pointerType: 'pen', clientX: box.x + 300, clientY: box.y + 300 })
  await synth(page, 'pointermove', { pointerId: 23, pointerType: 'pen', clientX: box.x + 340, clientY: box.y + 320 })
  await writing()
  await setCanvasMode(page, 'view')
  await dropped()
  await expect(page.getByTestId('aiws-canvas')).not.toHaveAttribute('data-tool', 'pen')
  await synth(page, 'pointerup', { pointerId: 23, pointerType: 'pen', buttons: 0, clientX: box.x + 340, clientY: box.y + 320 })
  await setCanvasMode(page, 'edit')
  await pickPen(page)
  expect((await hooks(page)).commits, 'nothing saved').toBe(commits)
  expect(await inkCells(api, ws.workspace_id)).toHaveLength(0)
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'pen')
  // and the next stroke works; no capture is left behind
  await pen.stroke(page, wave({ x: 400, y: 600 }, { x: 600, y: 620 }, 12))
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(1)
  expect(await page.getByTestId('aiws-canvas').evaluate((el) => [1, 2, 3, 21].some((id) => { try { return el.hasPointerCapture(id) } catch { return false } }))).toBe(false)
  // the wheel while writing moves nothing; the stroke goes on under the pen and is saved
  const still = (await camera(page)).transform
  await pen.down({ x: 400, y: 700 }); await pen.move({ x: 450, y: 710 })
  await page.getByTestId('aiws-canvas').evaluate((el) => { for (let i = 0; i < 5; i++) el.dispatchEvent(new WheelEvent('wheel', { deltaX: 30, deltaY: 60, bubbles: true, cancelable: true, clientX: 800, clientY: 500 })) })
  await page.waitForTimeout(300)
  expect((await camera(page)).transform, 'the camera stays while a stroke runs').toBe(still)
  await pen.move({ x: 500, y: 720 }); await pen.up({ x: 520, y: 725 })
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(2)
  // Esc without a stroke puts the pen down
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'select')
  await expect(page.getByTestId('aiws-ink-bar')).toHaveCount(0)
})

test('H06 without real pressure the line is even: a mouse, a pen that only reports 0.5, synthetic events without samples', async ({ page, api }) => {
  const ws = await inkWorkspace(api, `h06 ${Date.now()}`)
  await openNotes(page, ws.workspace_id)
  await pickPen(page)
  const pen = await Pen.of(page)
  const cam = await camera(page)
  const m0 = cam.toScreen({ x: 200, y: 200 }), m1 = cam.toScreen({ x: 320, y: 240 })
  await page.mouse.move(m0.x, m0.y)
  await page.mouse.down()
  await page.mouse.move(m1.x, m1.y, { steps: 8 })
  await page.mouse.up()
  await pen.stroke(page, wave(cam.toScreen({ x: 200, y: 350 }), cam.toScreen({ x: 400, y: 370 }), 15), () => 0.5)
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(2)
  for (const cell of await inkCells(api, ws.workspace_id)) {
    const c = await config(api, ws.workspace_id, cell.entity_id as string)
    expect((c.points as Json).p, 'no pressure channel').toBeUndefined()
    expect(c.pressure).toBe(false)
    expect(Number(await page.getByTestId(`aiws-ink-${cell.entity_id}`).getAttribute('data-paths')), 'one even path').toBe(1)
  }
  // pressure switched off: real pressure is kept as data, drawn even
  await page.getByTestId('aiws-ink-pressure').uncheck()
  await pen.stroke(page, wave(cam.toScreen({ x: 200, y: 500 }), cam.toScreen({ x: 400, y: 520 }), 15), (i) => 0.1 + i * 0.05)
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(3)
  const third = (await inkCells(api, ws.workspace_id)).find((c) => (c.placement as { y: number }).y > 450)!
  const c3 = await config(api, ws.workspace_id, third.entity_id as string)
  expect(c3.pressure).toBe(false)
  expect(((c3.points as Json).p as number[]).length).toBeGreaterThan(2)
  expect(Number(await page.getByTestId(`aiws-ink-${third.entity_id}`).getAttribute('data-paths'))).toBe(1)
})

const FIXTURE = resolve('../aiworkspace/fixtures/ink/strokes.json')

/** The fixture's long stroke (several segments of one logical stroke), on the canvas "笔记". */
function longStroke(): Json[] {
  const fixture = JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commits: { message: string; operations: Json[] }[] }
  return fixture.commits.find((c) => c.message === 'long')!.operations.map((op) => ({ ...op, parent_id: 'notes', placement: { ...(op.placement as Json), x: ((op.placement as { x: number }).x) - 900, y: ((op.placement as { y: number }).y) + 900 } }))
}

test('H07 a long stroke is one: selected, copied, erased and undone as one; the copy is a stroke of its own', async ({ page, api }) => {
  const long = longStroke()
  const ws = await inkWorkspace(api, `h07 ${Date.now()}`, long)
  const ids = long.map((op) => op.entity_id as string)
  expect(ids.length).toBeGreaterThan(1)
  for (const op of long) expect(JSON.stringify((op.payload as Json).config).length).toBeLessThanOrEqual(64 * 1024)
  await openNotes(page, ws.workspace_id)
  await fitAll(page)
  for (const id of ids) await expect(page.getByTestId(`aiws-ink-${id}`)).toBeVisible()
  // a click on its line selects every segment
  const first = page.getByTestId(`aiws-ink-${ids[0]}`).locator('path').first()
  const b = (await first.boundingBox())!
  const cam = await camera(page)
  const seg0 = long[0]
  const cfg0 = (seg0.payload as Json).config as { points: { x: number[]; y: number[] } }
  const onLine = cam.toScreen({ x: (seg0.placement as { x: number }).x + cfg0.points.x[40], y: (seg0.placement as { y: number }).y + cfg0.points.y[40] })
  expect(b.width).toBeGreaterThan(0)
  await page.mouse.click(onLine.x, onLine.y)
  for (const id of ids) await expect(page.getByTestId(`aiws-selection-${id}`)).toBeVisible()
  // dragging it (pressed on its first segment, not selected) moves every segment by the same amount
  await page.keyboard.press('Escape')
  await expect(page.getByTestId(`aiws-selection-${ids[0]}`)).toHaveCount(0)
  const placed = async () => Promise.all(ids.map(async (id) => (await placementOf(api, ws.workspace_id, id)) as { x: number; y: number }))
  const before = await placed()
  await page.mouse.move(onLine.x, onLine.y)
  await page.mouse.down()
  await page.mouse.move(onLine.x + 30, onLine.y + 18, { steps: 6 })
  await page.mouse.up()
  await expect.poll(async () => (await placed())[0].x).not.toBe(before[0].x)
  const after = await placed()
  const shifts = after.map((p, i) => `${p.x - before[i].x},${p.y - before[i].y}`)
  expect(new Set(shifts).size, shifts.join(' ')).toBe(1)
  for (const id of ids) await expect(page.getByTestId(`aiws-selection-${id}`)).toBeVisible()
  const moved = cam.toScreen({ x: (seg0.placement as { x: number }).x + (after[0].x - before[0].x) + cfg0.points.x[40], y: (seg0.placement as { y: number }).y + (after[0].y - before[0].y) + cfg0.points.y[40] })
  onLine.x = moved.x
  onLine.y = moved.y
  // copy and paste: new Blocks, one new logical stroke shared by the copies, the original's untouched
  await page.keyboard.press('Control+c')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: `已复制 ${ids.length} 个对象` }).last()).toBeVisible()
  await page.keyboard.press('Control+v')
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(ids.length * 2)
  const cells = await inkCells(api, ws.workspace_id)
  const original = cells.filter((c) => ids.includes(c.entity_id as string))
  const copies = cells.filter((c) => !ids.includes(c.entity_id as string))
  expect(new Set(original.map((c) => c.ink_stroke)).size).toBe(1)
  expect(new Set(copies.map((c) => c.ink_stroke)).size).toBe(1)
  expect(copies[0].ink_stroke).not.toBe(original[0].ink_stroke)
  for (const c of copies) expect((await config(api, ws.workspace_id, c.entity_id as string)).stroke).toBe(copies[0].ink_stroke)
  await inkSettled(page)
  // the copy is dragged away from the original (it is selected after the paste), then the eraser on one of its
  // segments erases the whole copy, in one commit; the original stays
  const copyCfg = await Promise.all(copies.map(async (c) => ({ c, k: await config(api, ws.workspace_id, c.entity_id as string) as { seg: number; points: { x: number[]; y: number[] } } })))
  const copy0 = copyCfg.find(({ k }) => k.seg === 0)!
  const camCopy = await camera(page)
  const onCopy = camCopy.toScreen({ x: (copy0.c.placement as { x: number }).x + copy0.k.points.x[40], y: (copy0.c.placement as { y: number }).y + copy0.k.points.y[40] })
  await page.mouse.move(onCopy.x, onCopy.y)
  await page.mouse.down()
  await page.mouse.move(onCopy.x, onCopy.y + 260, { steps: 8 })
  await page.mouse.up()
  await expect.poll(async () => ((await placementOf(api, ws.workspace_id, copy0.c.entity_id as string)) as { y: number }).y).toBeGreaterThan((copy0.c.placement as { y: number }).y + 100)
  await inkSettled(page)
  await page.keyboard.press('e')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'eraser')
  const pen = await Pen.of(page)
  const movedCopy = (await placementOf(api, ws.workspace_id, copy0.c.entity_id as string)) as { x: number; y: number }
  const hit = camCopy.toScreen({ x: movedCopy.x + copy0.k.points.x[20], y: movedCopy.y + copy0.k.points.y[20] })
  const commits = (await hooks(page)).commits
  await pen.stroke(page, [{ x: hit.x - 6, y: hit.y - 6 }, { x: hit.x + 6, y: hit.y + 6 }])
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(ids.length)
  expect((await hooks(page)).commits - commits).toBe(1)
  expect((await inkCells(api, ws.workspace_id)).every((c) => ids.includes(c.entity_id as string))).toBe(true)
  await inkSettled(page)
  // one undo brings the whole copy back
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(ids.length * 2)
  // Delete on the selected original removes all its segments, one undo restores them
  await page.keyboard.press('Escape')
  await page.mouse.click(onLine.x, onLine.y)
  for (const id of ids) await expect(page.getByTestId(`aiws-selection-${id}`)).toBeVisible()
  await page.keyboard.press('Delete')
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).filter((c) => ids.includes(c.entity_id as string)).length).toBe(0)
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).filter((c) => ids.includes(c.entity_id as string)).length).toBe(ids.length)
  // a flow page takes no ink: the paste is refused with a reason
  await page.mouse.click(onLine.x, onLine.y)
  await page.keyboard.press('Control+c')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已复制' }).last()).toBeVisible()
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-page').getByRole('menuitem').click()
  await expect(page.getByTestId('aiws-flow-host')).toBeVisible()
  await page.getByTestId('aiws-flow-host').click()
  await page.keyboard.press('Control+v')
  await expect(page.getByText('手写只在自由画布上', { exact: false }).first()).toBeVisible()
  await expect(page.getByTestId('aiws-tool-pen')).toHaveCount(0)
})

test('H08 zoomed, turned, inside a group: what is stored is where the pen was, and the eraser hits the line, not its box', async ({ page, api }) => {
  const ws = await inkWorkspace(api, `h08 ${Date.now()}`)
  await openNotes(page, ws.workspace_id)
  // zoomed in: the stroke is stored in canvas coordinates at the canvas width (not as wide as it looks)
  await zoomIn(page)
  await zoomIn(page)
  await page.waitForTimeout(300)
  await pickPen(page)
  const pen = await Pen.of(page)
  let cam = await camera(page)
  expect(cam.zoom).toBeGreaterThan(1)
  const screenLine = [{ x: 600, y: 400 }, { x: 700, y: 430 }, { x: 800, y: 400 }]
  await pen.stroke(page, screenLine, () => 0.5)
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(1)
  const [zoomed] = await inkCells(api, ws.workspace_id)
  const zc = await config(api, ws.workspace_id, zoomed.entity_id as string) as { width: number; points: { x: number[]; y: number[] } }
  const start = cam.toWorld(screenLine[0])
  expect(Math.hypot((zoomed.placement as { x: number }).x + zc.points.x[0] - start.x, (zoomed.placement as { y: number }).y + zc.points.y[0] - start.y)).toBeLessThan(1)
  expect(zc.width).toBe(2)
  // inside the group "框": the stroke joins it, placed relative to it, and moves with it
  await page.keyboard.press('Escape')
  await fitAll(page)
  await pickPen(page)
  cam = await camera(page)
  const inGroup = [cam.toScreen({ x: 760, y: 200 }), cam.toScreen({ x: 820, y: 230 }), cam.toScreen({ x: 880, y: 210 })]
  await pen.stroke(page, inGroup, () => 0.5)
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(2)
  const grouped = (await inkCells(api, ws.workspace_id)).find((c) => c.parent_id === 'frame-g')!
  expect(grouped).toBeTruthy()
  const gc = await config(api, ws.workspace_id, grouped.entity_id as string) as { points: { x: number[]; y: number[] } }
  expect(Math.round(700 + (grouped.placement as { x: number }).x + gc.points.x[0])).toBe(760)
  // turned by 90° (another client): the eraser over where the line was before turning misses, over where it is now hits
  const turnedRect = grouped.placement as { x: number; y: number; w: number; h: number }
  await api.commit(ALICE, ws, [{ op: 'tree.place', entity_id: grouped.entity_id, placement: { ...turnedRect, rotation: 90 } }])
  await expect(page.getByTestId(`aiws-canvas-block-${grouped.entity_id}`)).toHaveAttribute('data-rotation', '90')
  await page.keyboard.press('e')
  await inkSettled(page)
  cam = await camera(page)
  const centre = { x: 700 + turnedRect.x + turnedRect.w / 2, y: 120 + turnedRect.y + turnedRect.h / 2 }
  const local = { x: 700 + turnedRect.x + gc.points.x[0], y: 120 + turnedRect.y + gc.points.y[0] }
  const turned = { x: centre.x - (local.y - centre.y), y: centre.y + (local.x - centre.x) }
  const before = cam.toScreen(local)
  await pen.stroke(page, [{ x: before.x - 3, y: before.y }, { x: before.x + 3, y: before.y }])
  await page.waitForTimeout(400)
  expect((await inkCells(api, ws.workspace_id)).some((c) => c.entity_id === grouped.entity_id), 'missed where the line no longer is').toBe(true)
  const after = cam.toScreen(turned)
  await pen.stroke(page, [{ x: after.x - 4, y: after.y - 4 }, { x: after.x + 4, y: after.y + 4 }])
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).some((c) => c.entity_id === grouped.entity_id)).toBe(false)
  // the zoomed stroke next to it was not touched
  expect((await inkCells(api, ws.workspace_id)).map((c) => c.entity_id)).toEqual([zoomed.entity_id])
  await shot(page, 'h08-erased')
})

test('H09 reload, export and import keep every stroke; two windows write at once; an unknown save result is resent without a duplicate', async ({ page, api, browser }) => {
  const ws = await inkWorkspace(api, `h09 ${Date.now()}`)
  await openNotes(page, ws.workspace_id)
  await pickPen(page)
  const pen = await Pen.of(page)
  // a second window of the same workspace writes at the same time
  const other = await browser.newPage()
  await openNotes(other, ws.workspace_id)
  await pickPen(other)
  const pen2 = await Pen.of(other)
  await Promise.all([
    pen.stroke(page, wave({ x: 300, y: 300 }, { x: 600, y: 320 }, 20), (i) => 0.3 + i * 0.02),
    pen2.stroke(other, wave({ x: 300, y: 500 }, { x: 600, y: 520 }, 20), () => 0.5),
  ])
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(2)
  for (const p of [page, other]) for (const cell of await inkCells(api, ws.workspace_id)) await expect(p.getByTestId(`aiws-ink-${cell.entity_id}`)).toBeVisible()
  await other.close()
  // the save's result never comes back (commits and their status checks fail): the stroke stays drawn and listed,
  // then is resent with its key — one stroke, not two
  let blocked = true
  await page.route('**/kapi/aiworkspace', async (route) => {
    const body = route.request().postData() ?? ''
    if (blocked && /"method":"doc\.(commit|get_submission)"/.test(body)) { await route.abort('connectionreset'); return }
    await route.continue()
  })
  await pen.stroke(page, wave({ x: 300, y: 650 }, { x: 600, y: 670 }, 20), () => 0.6)
  await expect(page.getByTestId('aiws-save-summary')).toHaveAttribute('data-unsaved', '1', { timeout: 30_000 })
  await expect.poll(async () => (await hooks(page)).canvas?.['ink' as never]).toMatchObject({ waiting: 1 })
  await shot(page, 'h09-unknown')
  blocked = false
  const retry = page.getByTestId('aiws-notice-action').last()
  if (await retry.isVisible()) await retry.click()
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length, { timeout: 30_000 }).toBe(3)
  await inkSettled(page)
  await page.unroute('**/kapi/aiworkspace')
  await page.waitForTimeout(500)
  expect((await inkCells(api, ws.workspace_id)).length, 'no duplicate').toBe(3)
  const saved = new Map<string, Json>()
  for (const cell of await inkCells(api, ws.workspace_id)) saved.set(cell.entity_id as string, await config(api, ws.workspace_id, cell.entity_id as string))
  // opened again (a new page load) it draws the same strokes from the same data
  await openNotes(page, ws.workspace_id)
  for (const id of saved.keys()) await expect(page.getByTestId(`aiws-ink-${id}`)).toBeVisible()
  // export and import into a new workspace: the same configs, the same logical strokes
  const exported = await api.rpc(ALICE, 'doc.export', { workspace_id: ws.workspace_id, mode: 'share' })
  const imported = await api.importPackage(ALICE, await api.download(exported.download), 'new')
  const copies = await inkCells(api, imported.workspace_id)
  expect(copies.length).toBe(3)
  for (const c of copies) expect(await (async () => (await api.read(ALICE, imported.workspace_id, c.entity_id as string)).content.payload.config)()).toEqual(saved.get(c.entity_id as string))
})

test('H11 putting the pen down gives the canvas back: select, move and wheel as before; view mode and flow pages offer no pen', async ({ page, api }) => {
  const ws = await inkWorkspace(api, `h11 ${Date.now()}`)
  await openNotes(page, ws.workspace_id)
  await pickPen(page)
  // the wheel still pans while no stroke runs
  const before = (await camera(page)).transform
  await page.mouse.move(800, 500)
  await page.mouse.wheel(0, 120)
  await expect.poll(async () => (await camera(page)).transform).not.toBe(before)
  await page.getByTestId('aiws-ink-done').click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-tool', 'select')
  const cam = await camera(page)
  const box = cam.toScreen({ x: 410, y: 330 })
  await page.mouse.click(box.x, box.y)
  await expect(page.getByTestId('aiws-selection-box')).toBeVisible()
  await page.mouse.move(box.x, box.y)
  await page.mouse.down()
  await page.mouse.move(box.x + 50, box.y + 30, { steps: 5 })
  await page.mouse.up()
  await expect.poll(async () => (await placementOf(api, ws.workspace_id, 'box'))?.x).toBe(350)
  // view mode: no writing tools
  await setCanvasMode(page, 'view')
  await expect(page.getByTestId('aiws-tool-pen')).toHaveCount(0)
  await page.keyboard.press('p')
  await expect(page.getByTestId('aiws-canvas')).not.toHaveAttribute('data-tool', 'pen')
  await setCanvasMode(page, 'edit')
  await expect(page.getByTestId('aiws-tool-pen')).toBeVisible()
})

test('H-scale a page of 1000 strokes (200 000 points): drawn without placeholders, panned, written on; the numbers go to a report', async ({ page, api }) => {
  test.setTimeout(300_000)
  const { encodeStroke } = await import('../../src/app/aiworkspace/ink/model')
  const ws = await inkWorkspace(api, `h-scale ${Date.now()}`)
  // handwriting-like strokes on a 40 × 25 grid, 200 samples each, pressure rising and falling
  const strokes: Json[] = []
  let bytes = 0
  for (let n = 0; n < 1000; n++) {
    const ox = (n % 40) * 140, oy = Math.floor(n / 40) * 90
    const points = Array.from({ length: 200 }, (_, i) => ({ x: ox + i * 0.55 + Math.sin(i / 4 + n) * 9, y: oy + 30 + Math.cos(i / 5 + n) * 18, t: i * 4, p: Math.round(Math.min(0.6, i / 30, (199 - i) / 30, 1) * 1000) / 1000 }))
    const [segment] = encodeStroke(points, { color: '#111827', width: 2, pressure: true }, `kscale${n}`)
    bytes += JSON.stringify(segment.config).length
    strokes.push({ op: 'entity.create', entity_id: `cs${n}`, type_id: 'buckyos.cell', parent_id: 'notes', order_key: `s${String(n).padStart(4, '0')}1`, placement: segment.rect, payload: { view: { type: 'ink', version: 1 }, config: segment.config } })
  }
  for (let i = 0; i < strokes.length; i += 100) {
    const result = await api.commit(ALICE, ws, strokes.slice(i, i + 100))
    expect(result.status, JSON.stringify(result).slice(0, 300)).toBe('accepted')
  }
  const opened = Date.now()
  await openNotes(page, ws.workspace_id)
  await expect(page.locator('svg.aiws-ink').first()).toBeVisible({ timeout: 60_000 })
  const firstView = Date.now() - opened
  // the whole page: every stroke drawn, none a placeholder or held back by the mount budget
  const fitStarted = Date.now()
  await fitAll(page)
  await expect.poll(async () => page.locator('svg.aiws-ink').count(), { timeout: 120_000 }).toBe(1000)
  const allDrawn = Date.now() - fitStarted
  const stats = (await hooks(page)).canvas!
  expect(stats.placeholders).toBe(0)
  const paths = await page.locator('svg.aiws-ink path').count()
  // panning: frame intervals while the wheel moves the canvas
  const pan = await page.evaluate(async () => {
    const root = document.querySelector('[data-testid="aiws-canvas"]')!
    const gaps: number[] = []
    let last = performance.now()
    let running = true
    const tick = (now: number) => { gaps.push(now - last); last = now; if (running) requestAnimationFrame(tick) }
    requestAnimationFrame(tick)
    for (let i = 0; i < 90; i++) {
      root.dispatchEvent(new WheelEvent('wheel', { deltaX: 12, deltaY: 8, bubbles: true, cancelable: true, clientX: 800, clientY: 500 }))
      await new Promise((r) => requestAnimationFrame(r))
    }
    running = false
    gaps.sort((a, b) => a - b)
    return { frames: gaps.length, median: gaps[Math.floor(gaps.length / 2)], p95: gaps[Math.floor(gaps.length * 0.95)], max: gaps.at(-1) }
  })
  // writing over it all at 100 %: input acknowledgements per sample (not pen-to-glass latency), save until drawn
  await page.keyboard.press('Escape')
  await zoomIn(page)
  await pickPen(page)
  const pen = await Pen.of(page)
  const acks: number[] = []
  const path = wave({ x: 300, y: 300 }, { x: 1100, y: 600 }, 300, 30)
  await pen.down(path[0], 0.3)
  for (let i = 1; i < path.length; i++) { const t = performance.now(); await pen.move(path[i], 0.3 + (i % 50) / 100); acks.push(performance.now() - t) }
  const lifted = performance.now()
  await pen.up(path.at(-1)!)
  await expect.poll(async () => (await inkCells(api, ws.workspace_id)).length).toBe(1001)
  await inkSettled(page)
  const saveToDrawn = performance.now() - lifted
  acks.sort((a, b) => a - b)
  // a continuous long stroke (4000 samples, every one a corner): segments within the limit, one commit
  const commits = (await hooks(page)).commits
  const zig = Array.from({ length: 4000 }, (_, i) => ({ x: 200 + (i % 400) * 2.2, y: 250 + Math.floor(i / 400) * 30 + (i % 2) * 6 }))
  await pen.stroke(page, zig, (i) => 0.2 + (i % 7) / 10)
  await expect.poll(async () => (await hooks(page)).commits - commits, { timeout: 60_000 }).toBe(1)
  await inkSettled(page)
  const longCells = (await inkCells(api, ws.workspace_id)).filter((c) => !(c.entity_id as string).startsWith('cs'))
  const longest = longCells.filter((c) => c.ink_stroke !== longCells[0].ink_stroke || longCells.filter((x) => x.ink_stroke === c.ink_stroke).length > 1)
  const segConfigs = await Promise.all(longest.map(async (c) => JSON.stringify(await config(api, ws.workspace_id, c.entity_id as string)).length))
  for (const size of segConfigs) expect(size).toBeLessThanOrEqual(64 * 1024)
  const env = await page.evaluate(() => ({ userAgent: navigator.userAgent, cores: navigator.hardwareConcurrency, dpr: window.devicePixelRatio, heap: (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory?.usedJSHeapSize ?? null }))
  const report = {
    note: 'Headless Chromium on the test host, synthetic CDP pen; not a device measurement (see the TODO device matrix).',
    env, data: { strokes: 1000, points: 200_000, configBytes: bytes },
    timings: { firstViewMs: firstView, allDrawnAfterFitMs: allDrawn, saveToDrawnMs: Math.round(saveToDrawn), inputAckMs: { median: acks[Math.floor(acks.length / 2)], p95: acks[Math.floor(acks.length * 0.95)], max: acks.at(-1) } },
    dom: { inkBlocks: 1000, paths, mounted: stats.mounted, hidden: stats.hidden, placeholders: stats.placeholders },
    panFrames: pan,
    longStroke: { samples: zig.length, segments: longest.length, configBytes: segConfigs },
  }
  writeFileSync('test-results/aiworkspace-ink-scale.json', `${JSON.stringify(report, null, 2)}\n`)
  console.log(JSON.stringify(report))
  await shot(page, 'h-scale')
})
