/* UI improvement §16 (UI-M01…M02): the canvas on a phone. An emulated Pixel 7 (touch, small screen) opens a
 * workspace in its tab: view mode only, one toolbar with the canvas icon and switcher, and Miro-style touch
 * gestures driven through real multi-touch input (CDP), not synthetic DOM events. What the phone does to the
 * view is asserted on the camera transform; that it leaves the shared work state alone, through a desktop. */

import { devices, type CDPSession, type Page } from '@playwright/test'
import { expect, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
// eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
const { defaultBrowserType, ...pixel } = devices['Pixel 7']
test.use(pixel)

interface Point { x: number; y: number }

/** Fingers on the screen, in page coordinates (one touch event per call, like a real digitizer). */
class Fingers {
  private readonly cdp: CDPSession
  constructor(cdp: CDPSession) { this.cdp = cdp }
  static async of(page: Page) { return new Fingers(await page.context().newCDPSession(page)) }
  down(...points: Point[]) { return this.cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: points.map((p, id) => ({ x: p.x, y: p.y, id })) }) }
  move(...points: Point[]) { return this.cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: points.map((p, id) => ({ x: p.x, y: p.y, id })) }) }
  up() { return this.cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] }) }
  async tap(p: Point) { await this.down(p); await this.up() }
  /** Drag the given fingers from `from` to `to` in `steps` frames, then rest (no flick) unless `flick`. */
  async drag(page: Page, from: Point[], to: Point[], { steps = 8, flick = false } = {}) {
    await this.down(...from)
    for (let i = 1; i <= steps; i += 1) {
      await this.move(...from.map((f, n) => ({ x: f.x + ((to[n].x - f.x) * i) / steps, y: f.y + ((to[n].y - f.y) * i) / steps })))
      await page.waitForTimeout(16)
    }
    if (!flick) await page.waitForTimeout(150)
    await this.up()
  }
}

async function openOnPhone(page: Page, workspaceId: string) {
  await page.addInitScript((value) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ token: value })), ALICE)
  await page.goto(`/workspace/${workspaceId}`)
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', workspaceId, { timeout: 30_000 })
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await expect(page.getByTestId('aiws-canvas')).toBeVisible()
}

/** The camera as the world layer shows it: screen = origin + world × zoom (page coordinates). */
async function camera(page: Page) {
  const box = (await page.getByTestId('aiws-canvas').boundingBox())!
  const m = await page.getByTestId('aiws-world').evaluate((el) => new DOMMatrix(getComputedStyle(el).transform))
  return { zoom: m.a, toScreen: (w: Point) => ({ x: box.x + m.e + w.x * m.a, y: box.y + m.f + w.y * m.d }), toWorld: (s: Point) => ({ x: (s.x - box.x - m.e) / m.a, y: (s.y - box.y - m.f) / m.d }), box }
}

/** Wait until the camera stopped moving (animations and glides end), then read it. */
async function settled(page: Page) {
  let previous = ''
  await expect.poll(async () => {
    const now = await page.getByTestId('aiws-world').evaluate((el) => getComputedStyle(el).transform)
    const still = now === previous
    previous = now
    return still
  }, { intervals: [120] }).toBe(true)
  return camera(page)
}

type Camera = Awaited<ReturnType<typeof camera>>

/** The Blocks of the quarterly demo's first canvas (world rectangles). */
const BLOCKS = [{ x: 40, y: 40, w: 900, h: 120 }, { x: 40, y: 190, w: 900, h: 420 }, { x: 980, y: 190, w: 420, h: 300 }, { x: 980, y: 40, w: 420, h: 130 }, { x: 960, y: 520, w: 1100, h: 820 }]

/** A screen point over blank canvas, clear of the toolbar and the status area. */
function blankSpot(cam: Camera): Point {
  for (let y = cam.box.y + 120; y < cam.box.y + cam.box.height - 100; y += 16) {
    for (let x = cam.box.x + 40; x < cam.box.x + cam.box.width - 40; x += 16) {
      const w = cam.toWorld({ x, y })
      if (!BLOCKS.some((b) => w.x > b.x - 20 && w.x < b.x + b.w + 20 && w.y > b.y - 20 && w.y < b.y + b.h + 20)) return { x, y }
    }
  }
  throw new Error('no blank canvas on screen')
}

function quarterly(api: Api, title: string) {
  return api.demo(ALICE, 'quarterly', title)
}

test('UI-M01 a phone opens the canvas in view mode with one toolbar: canvas icon and switcher, zoom; it writes nothing a desktop would inherit except the canvas it chose', async ({ page, api, browser }) => {
  const ws = await quarterly(api, `phone ${Date.now()}`)
  await openOnPhone(page, ws.workspace_id)
  const viewport = page.viewportSize()!

  // view mode only, one toolbar: no main toolbar, no object toolbar, no data-source view
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', 'view')
  await expect(page.getByTestId('aiws-main-toolbar')).toHaveCount(0)
  await expect(page.getByTestId('aiws-object-toolbar')).toHaveCount(0)
  await expect(page.getByTestId('aiws-main-menu')).toHaveCount(0)
  const toolbar = page.getByTestId('aiws-presenter-toolbar')
  await expect(toolbar.getByTestId('aiws-surface-switch')).toContainText('经营分析')
  await expect(toolbar.getByTestId('aiws-surface-switch').locator('svg').first()).toBeVisible()
  await expect(toolbar.getByTestId('aiws-zoom-menu')).toBeVisible()
  const bar = (await toolbar.boundingBox())!
  expect(bar.x).toBeGreaterThanOrEqual(0)
  expect(bar.x + bar.width).toBeLessThanOrEqual(viewport.width + 0.5)
  for (const id of ['aiws-annotate-start', 'aiws-zoom-menu', 'aiws-principal', 'aiws-share']) {
    const b = (await toolbar.getByTestId(id).boundingBox())!
    expect(b.x + b.width, id).toBeLessThanOrEqual(viewport.width)
  }

  // a first visit on the phone shows the whole canvas (zoomed out to fit, never above 100%)
  const first = await settled(page)
  expect(first.zoom).toBeLessThan(1)
  for (const id of ['blk-intro', 'blk-frame', 'blk-kpi']) {
    const b = (await page.getByTestId(`aiws-canvas-block-${id}`).boundingBox())!
    expect(b.x, id).toBeGreaterThanOrEqual(0)
    expect(b.x + b.width, id).toBeLessThanOrEqual(viewport.width)
  }

  // the switcher: the workspace's canvases to pick from, nothing to rename, delete or create
  await toolbar.getByTestId('aiws-surface-switch').tap()
  const list = page.getByTestId('aiws-surface-list')
  await expect(list).toBeVisible()
  await expect(list.getByTestId('aiws-workspace-title')).toContainText('phone')
  await expect(list.getByTestId('aiws-phone-note')).toBeVisible()
  await expect(list.getByTestId('aiws-surface-new')).toHaveCount(0)
  await expect(list.getByTestId('aiws-surface-more-sf-detail')).toHaveCount(0)
  const listBox = (await list.boundingBox())!
  expect(listBox.x + listBox.width).toBeLessThanOrEqual(viewport.width)
  await list.getByTestId('aiws-surface-item-sf-detail').getByRole('menuitem').tap()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'sf-detail')
  await expect(toolbar.getByTestId('aiws-surface-switch')).toContainText('数据明细')
  await expect(list).toHaveCount(0)

  // the phone's own viewport: zoom in with the toolbar; a desktop afterwards still edits, at its own viewport
  await toolbar.getByTestId('aiws-zoom-menu').tap()
  await page.getByTestId('aiws-zoom-25').tap()
  await expect(page.getByTestId('aiws-zoom')).toHaveText('25%')
  await page.waitForTimeout(1500) // the user work state goes out after 0.8 s
  const desktop = await browser.newContext({ viewport: { width: 1600, height: 1000 } })
  const other = await desktop.newPage()
  await other.addInitScript((value) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ token: value })), ALICE)
  await other.goto(`/workspace/${ws.workspace_id}`)
  await expect(other.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', ws.workspace_id, { timeout: 30_000 })
  await expect(other.getByTestId('aiws-workspace')).not.toHaveAttribute('data-phone', 'true')
  await expect(other.getByTestId('aiws-canvas')).toHaveAttribute('data-surface-id', 'sf-detail')
  await expect(other.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', 'edit')
  await expect(other.getByTestId('aiws-main-toolbar')).toBeVisible()
  await expect(other.getByTestId('aiws-zoom')).toHaveText('100%')
  await desktop.close()

  // back to the workspace list from the switcher
  await toolbar.getByTestId('aiws-surface-switch').tap()
  await page.getByTestId('aiws-switch-workspace').tap()
  await expect(page.getByTestId('aiws-list')).toBeVisible({ timeout: 30_000 })
  await expect.poll(() => new URL(page.url()).pathname).toBe('/workspace')
})

test('UI-M02 touch gestures on the phone canvas: pan and glide, pinch, double tap, two-finger tap, tap to select, long press, scrolling a selected Block', async ({ page, api }) => {
  const ws = await quarterly(api, `gestures ${Date.now()}`)
  // enough rows that the sales table scrolls inside its Block
  const more = Array.from({ length: 40 }, (_, i) => ({ record_id: `x-${i}`, values: { region: '西南', product: `型号 ${i}`, revenue: 1000 + i, cost: 500, target: 1000, prev: 900 } }))
  expect((await api.commit(ALICE, ws, [{ op: 'table.insert_records', source_id: 'sales', records: more }])).status).toBe('accepted')
  await openOnPhone(page, ws.workspace_id)
  const seq = await api.headSeq(ALICE, ws.workspace_id)
  const fingers = await Fingers.of(page)

  // one finger pans; the world point under the finger follows it
  let cam = await settled(page)
  const start = blankSpot(cam)
  const grabbed = cam.toWorld(start)
  await fingers.drag(page, [start], [{ x: start.x - 60, y: start.y - 120 }])
  cam = await settled(page)
  const after = cam.toScreen(grabbed)
  expect(Math.abs(after.x - (start.x - 60))).toBeLessThan(3)
  expect(Math.abs(after.y - (start.y - 120))).toBeLessThan(3)

  // a flick glides on after the finger lifts
  const flickFrom = blankSpot(cam)
  const flicked = cam.toWorld(flickFrom)
  await fingers.drag(page, [flickFrom], [{ x: flickFrom.x, y: flickFrom.y + 90 }], { steps: 3, flick: true })
  const lifted = (await camera(page)).toScreen(flicked)
  cam = await settled(page)
  expect(cam.toScreen(flicked).y - lifted.y).toBeGreaterThan(20)

  // pinch: three times the finger distance is three times the zoom, about the midpoint
  cam = await settled(page)
  const zoom0 = cam.zoom
  const mid = { x: cam.box.x + cam.box.width / 2, y: cam.box.y + cam.box.height / 2 }
  const anchor = cam.toWorld(mid)
  await fingers.drag(page, [{ x: mid.x - 30, y: mid.y }, { x: mid.x + 30, y: mid.y }], [{ x: mid.x - 90, y: mid.y }, { x: mid.x + 90, y: mid.y }], { steps: 10 })
  cam = await settled(page)
  expect(cam.zoom / zoom0).toBeGreaterThan(2.8)
  expect(cam.zoom / zoom0).toBeLessThan(3.2)
  const kept = cam.toScreen(anchor)
  expect(Math.abs(kept.x - mid.x)).toBeLessThan(4)
  expect(Math.abs(kept.y - mid.y)).toBeLessThan(4)
  await expect(page.getByTestId('aiws-zoom')).toHaveText(`${Math.round(cam.zoom * 100)}%`)

  // two-finger tap zooms out ×2 about the fingers; double tap on a blank spot zooms in ×2 there
  const zoom1 = cam.zoom
  await fingers.down({ x: mid.x - 30, y: mid.y }, { x: mid.x + 30, y: mid.y })
  await fingers.up()
  cam = await settled(page)
  expect(cam.zoom / zoom1).toBeCloseTo(0.5, 2)
  const spot = blankSpot(cam)
  const spotWorld = cam.toWorld(spot)
  const zoom2 = cam.zoom
  await fingers.tap(spot)
  await fingers.tap(spot)
  cam = await settled(page)
  expect(cam.zoom / zoom2).toBeCloseTo(2, 2)
  const still = cam.toScreen(spotWorld)
  expect(Math.abs(still.x - spot.x)).toBeLessThan(2)
  expect(Math.abs(still.y - spot.y)).toBeLessThan(2)

  // double tap on a Block zooms to it (no editing in view mode); first the whole canvas again, from the toolbar
  await page.getByTestId('aiws-zoom-menu').tap()
  await page.getByTestId('aiws-fit-all').tap()
  cam = await settled(page)
  const sales = cam.toScreen({ x: 40 + 450, y: 190 + 60 })
  await fingers.tap(sales)
  await fingers.tap(sales)
  cam = await settled(page)
  const fitted = (await page.getByTestId('aiws-canvas-block-blk-sales').boundingBox())!
  expect(Math.abs(fitted.width - (cam.box.width - 24 - 48))).toBeLessThan(4)
  expect(fitted.x).toBeGreaterThanOrEqual(cam.box.x)
  await expect(page.locator('.aiws-frame-block.is-editing')).toHaveCount(0)

  // a tap selects the Block under the finger (the near toolbar shows); a tap on a blank spot clears it
  await page.waitForTimeout(400)
  const center = { x: fitted.x + fitted.width / 2, y: fitted.y + fitted.height / 2 }
  await fingers.tap(center)
  await expect(page.getByTestId('aiws-selection-blk-sales')).toBeVisible()
  await expect(page.getByTestId('aiws-near-toolbar')).toBeVisible()
  // the selected table scrolls under one finger; the camera stays
  const scroller = page.getByTestId('aiws-canvas-block-blk-sales').locator('.aiws-table-scroll')
  expect(await scroller.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true)
  const reference = { x: 490, y: 400 }
  const before = (await camera(page)).toScreen(reference)
  await fingers.drag(page, [{ x: center.x, y: center.y + 60 }], [{ x: center.x, y: center.y - 60 }])
  expect(await scroller.evaluate((el) => el.scrollTop)).toBeGreaterThan(50)
  const unmoved = (await settled(page)).toScreen(reference)
  expect(Math.abs(unmoved.y - before.y)).toBeLessThan(1)
  await page.waitForTimeout(400)
  const blank = (await camera(page)).toScreen({ x: 40 + 450, y: 190 + 420 + 40 })
  await fingers.tap(blank)
  await expect(page.getByTestId('aiws-selection-blk-sales')).toHaveCount(0)

  // a long press opens the Block's context menu (inside the screen), selecting the Block
  await page.waitForTimeout(400)
  await fingers.down(center)
  await page.waitForTimeout(700)
  await fingers.up()
  const menu = page.getByTestId('aiws-context-menu')
  await expect(menu).toBeVisible()
  await expect(menu.getByTestId('aiws-menu-relations')).toBeVisible()
  await expect(menu.getByTestId('aiws-menu-copy')).toHaveCount(0)
  await expect(page.getByTestId('aiws-selection-blk-sales')).toBeVisible()
  const menuBox = (await menu.boundingBox())!
  expect(menuBox.x + menuBox.width).toBeLessThanOrEqual(page.viewportSize()!.width)
  await menu.getByTestId('aiws-menu-relations').tap()
  await expect(page.locator('[data-testid="aiws-side-panel"][data-tab="relations"]')).toBeVisible()
  // no gesture wrote the document
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(seq)
})
