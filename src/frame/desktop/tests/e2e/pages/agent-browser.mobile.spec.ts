import { expect, test, type CDPSession, type Page } from '@playwright/test'
import { openFromAllApps } from '../fixtures/desktop'

/** The Agent browser on a phone (the viewport of tests/e2e/flows/mobile.spec.ts), on the mock Agent Service. */

test.use({
  viewport: { width: 375, height: 812 },
  hasTouch: true,
  isMobile: true,
})

const AGENT = 'codeassistant'

type MockElement = 'input' | 'select' | 'alert' | 'newTab'
/** A message the mock service received, as far as the specs look at it. */
interface Message {
  [field: string]: unknown
  type: string
  kind?: string
  action?: string
  phase?: string
  x?: number
  y?: number
  button?: string
  url?: string
  viewport?: { width: number; height: number; mobile: boolean }
}
interface Received { groupId: string; dropped: boolean; message: Message }
interface MockControl {
  log: Received[]
  rects(width: number): Record<MockElement, { x: number; y: number; width: number; height: number }>
  page(agentId: string): { text: string; scrollY: number; dialog: string } | null
  agentAcquire(agentId: string): boolean
}
type MockWindow = { __agentBrowserMock: MockControl }

const received = (page: Page, type: string) =>
  page.evaluate((t) => (window as unknown as MockWindow).__agentBrowserMock.log.filter((entry) => entry.message.type === t), type)
const inputs = async (page: Page, kind: string) => (await received(page, 'input')).filter((entry) => entry.message.kind === kind)
const mockPage = (page: Page) => page.evaluate((agent) => (window as unknown as MockWindow).__agentBrowserMock.page(agent), AGENT)

async function openBrowser(page: Page) {
  await page.goto('/?scenario=normal')
  await expect(page.getByTestId('mobile-taskbar')).toBeAttached()
  await openFromAllApps(page, 'agent-browser')
  await page.getByTestId(`ab-agent-${AGENT}`).tap()
  await page.getByTestId('ab-mode-page').tap()
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
  const address = page.getByTestId('ab-address-input')
  await address.tap()
  await address.fill('example.com')
  await address.press('Enter')
  await expect(page.getByTestId('ab-screen-canvas')).toHaveAttribute('data-seq', /\d+/)
  await expect(address).toHaveValue('example.com')
}

async function elementAt(page: Page, element: MockElement) {
  const viewport = await page.getByTestId('ab-screen-surface').getAttribute('data-viewport')
  const width = Number(viewport?.split('x')[0])
  const box = await page.getByTestId('ab-screen-canvas').boundingBox()
  if (!box || !width) throw new Error('the screen shows no frame')
  const rect = await page.evaluate(([w, name]) => (window as unknown as MockWindow).__agentBrowserMock.rects(w)[name], [width, element] as const)
  const at = { x: Math.round(rect.x + rect.width / 2), y: Math.round(rect.y + rect.height / 2) }
  const scale = box.width / width
  return { screen: { x: box.x + at.x * scale, y: box.y + at.y * scale }, page: at }
}

/** One finger pressed at `from`, dragged to `to`, lifted. */
async function drag(cdp: CDPSession, from: { x: number; y: number }, to: { x: number; y: number }) {
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ ...from, id: 1 }] })
  for (let step = 1; step <= 6; step++) {
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: from.x + ((to.x - from.x) * step) / 6, y: from.y + ((to.y - from.y) * step) / 6, id: 1 }] })
  }
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
}

test('on a phone the page is laid out for the phone, touches reach it, and the on-screen keyboard types into it', async ({ page }) => {
  const errors: string[] = []
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()) })
  await openBrowser(page)

  // The App fills the screen under the status bar; the service is asked for a phone viewport of the screen's width.
  const viewport = (await received(page, 'hello'))[0].message.viewport
  expect(viewport).toMatchObject({ width: 375, mobile: true })
  const surface = page.getByTestId('ab-screen-surface')
  await expect(surface).toHaveAttribute('data-mobile', 'true')
  await expect(surface).toHaveAttribute('data-viewport', `375x${viewport?.height}`)
  const canvas = await page.getByTestId('ab-screen-canvas').boundingBox()
  expect(canvas).toMatchObject({ x: 0, width: 375, height: viewport?.height })
  const app = await page.getByTestId('agent-browser').boundingBox()
  expect(app).toMatchObject({ x: 0, width: 375 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(375)

  // A tap is a touch on the page, at the page's own position.
  const field = await elementAt(page, 'input')
  await page.touchscreen.tap(field.screen.x, field.screen.y)
  await expect.poll(async () => (await inputs(page, 'touch')).map((entry) => entry.message)).toEqual([
    { type: 'input', kind: 'touch', phase: 'start', points: [{ id: 0, ...field.page }] },
    { type: 'input', kind: 'touch', phase: 'end', points: [] },
  ])
  expect(await inputs(page, 'mouse')).toHaveLength(0)

  // The page says a text field was pressed: the hidden input takes the focus, and what the on-screen keyboard produces.
  await expect(page.getByTestId('ab-screen-input')).toBeFocused()
  await expect(page.getByTestId('ab-keyboard')).toHaveAttribute('aria-pressed', 'true')
  await page.keyboard.insertText('手机上')
  await expect.poll(async () => (await mockPage(page))?.text).toBe('手机上')
  // Backspace and Enter of an on-screen keyboard come without key events.
  await page.evaluate(() => document.execCommand('delete'))
  await expect.poll(async () => (await mockPage(page))?.text).toBe('手机')
  await page.keyboard.insertText('\n')
  await expect.poll(async () => (await mockPage(page))?.dialog).toBe('submitted:手机')
  expect((await inputs(page, 'key')).map((entry) => `${entry.message.action}:${entry.message.key}`)).toEqual(['down:Backspace', 'up:Backspace', 'down:Enter', 'up:Enter'])
  await expect(page.getByTestId('ab-screen-input')).toBeFocused()
  // The keyboard button puts the keyboard away and brings it back.
  await page.getByTestId('ab-keyboard').tap()
  await expect(page.getByTestId('ab-screen-input')).not.toBeFocused()
  await page.getByTestId('ab-keyboard').tap()
  await expect(page.getByTestId('ab-screen-input')).toBeFocused()
  await page.getByTestId('ab-keyboard').tap()
  await expect(page.getByTestId('ab-screen-input')).not.toBeFocused()

  // A swipe scrolls the page.
  const cdp = await page.context().newCDPSession(page)
  await drag(cdp, { x: 180, y: 600 }, { x: 180, y: 360 })
  await expect.poll(async () => (await mockPage(page))?.scrollY).toBe(240)
  expect((await inputs(page, 'touch')).some((entry) => entry.message.phase === 'move')).toBe(true)

  // While the Agent operates a touch goes nowhere and the App says why.
  const sent = (await received(page, 'input')).length
  await page.evaluate((agent) => { (window as unknown as MockWindow).__agentBrowserMock.agentAcquire(agent) }, AGENT)
  await expect(page.getByTestId('ab-screen-readonly')).toBeVisible()
  await expect(page.getByTestId('ab-keyboard')).toHaveCount(0)
  await page.touchscreen.tap(field.screen.x, field.screen.y)
  await expect(page.getByTestId('ab-toast')).toContainText('Take over')
  await page.waitForTimeout(200)
  expect((await received(page, 'input')).length).toBe(sent)
  await page.getByTestId('ab-take-over').tap()
  await expect(page.getByTestId('ab-status')).toHaveAttribute('data-user-hold', 'true')
  await expect(page.getByTestId('ab-hand-back')).toBeInViewport()
  expect(errors).toEqual([])
})

test('on a phone panels come up from the bottom and the Back key closes them', async ({ page }) => {
  await openBrowser(page)
  const app = await page.getByTestId('agent-browser').boundingBox()
  await page.getByTestId('ab-menu-button').tap()
  const sheet = page.getByTestId('ab-menu')
  await expect(sheet).toBeVisible()
  await expect.poll(async () => {
    const box = await sheet.boundingBox()
    return box ? Math.round(box.y + box.height) : 0
  }).toBe(Math.round((app?.y ?? 0) + (app?.height ?? 0)))
  expect((await sheet.boundingBox())?.width).toBe(375)
  await page.getByLabel('Status bar').getByRole('button', { name: 'Back' }).tap()
  await expect(sheet).toHaveCount(0)

  // A select's options and a page dialog are shown the same way.
  const select = await elementAt(page, 'select')
  await page.touchscreen.tap(select.screen.x, select.screen.y)
  await page.getByTestId('ab-select').getByRole('option', { name: 'Banana' }).tap()
  await expect(page.getByTestId('ab-select')).toHaveCount(0)
  const alert = await elementAt(page, 'alert')
  await page.touchscreen.tap(alert.screen.x, alert.screen.y)
  await expect(page.getByTestId('ab-dialog-message')).toBeVisible()
  await page.getByTestId('ab-dialog-accept').tap()
  await expect.poll(async () => (await mockPage(page))?.dialog).toBe('alert:ok')

  await page.getByTestId('ab-tabs-button').tap()
  await page.getByTestId('ab-tab-new').tap()
  await expect(page.getByTestId('ab-tabs-count')).toHaveText('2')
})

test('desktop site on a phone: scaled to the width, a tap clicks, a drag scrolls, two fingers zoom the view', async ({ page }) => {
  await openBrowser(page)
  await page.getByTestId('ab-menu-button').tap()
  await page.getByTestId('ab-desktop-site').tap()
  await page.getByTestId('ab-menu-close').tap()
  const surface = page.getByTestId('ab-screen-surface')
  await expect(surface).toHaveAttribute('data-viewport', /^1280x\d+$/)
  await expect(surface).toHaveAttribute('data-mobile', 'false')
  expect((await received(page, 'viewport')).at(-1)?.message).toMatchObject({ width: 1280, mobile: false })
  const canvas = page.getByTestId('ab-screen-canvas')
  expect(Math.round((await canvas.boundingBox())?.width ?? 0)).toBe(375)

  // The desktop page has no touch: a tap is a click at that place, a drag turns the wheel.
  const field = await elementAt(page, 'input')
  await page.touchscreen.tap(field.screen.x, field.screen.y)
  await expect.poll(async () => (await inputs(page, 'mouse')).map((entry) => entry.message.action)).toEqual(['move', 'down', 'up'])
  const down = (await inputs(page, 'mouse'))[1].message
  expect(down.button).toBe('left')
  expect(Math.abs((down.x ?? 0) - field.page.x)).toBeLessThanOrEqual(3)
  expect(Math.abs((down.y ?? 0) - field.page.y)).toBeLessThanOrEqual(3)
  expect(await inputs(page, 'touch')).toHaveLength(0)

  const cdp = await page.context().newCDPSession(page)
  await drag(cdp, { x: 180, y: 600 }, { x: 180, y: 480 })
  await expect.poll(async () => (await mockPage(page))?.scrollY ?? 0).toBeGreaterThan(300)
  expect((await inputs(page, 'wheel')).length).toBeGreaterThan(0)
  expect((await inputs(page, 'mouse')).map((entry) => entry.message.action)).toEqual(['move', 'down', 'up'])

  // Two fingers zoom and move the view on this side; nothing is sent to the page.
  const before = (await received(page, 'input')).length
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: 150, y: 400, id: 1 }, { x: 220, y: 400, id: 2 }] })
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: 110, y: 400, id: 1 }, { x: 260, y: 400, id: 2 }] })
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: 80, y: 400, id: 1 }, { x: 290, y: 400, id: 2 }] })
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
  await expect.poll(async () => Math.round((await canvas.boundingBox())?.width ?? 0)).toBe(1125)
  await page.waitForTimeout(200)
  expect((await received(page, 'input')).length).toBe(before)
  // Pressing a place of the zoomed view still reaches that place of the page.
  const shown = await canvas.boundingBox()
  const host = await surface.boundingBox()
  if (!shown || !host) throw new Error('the screen is not shown')
  const tap = { x: host.x + 200, y: host.y + 300 }
  await page.touchscreen.tap(tap.x, tap.y)
  await expect.poll(async () => (await inputs(page, 'mouse')).filter((entry) => entry.message.action === 'down')).toHaveLength(2)
  const pressed = (await inputs(page, 'mouse')).filter((entry) => entry.message.action === 'down')[1].message
  expect(Math.abs((pressed.x ?? 0) - ((tap.x - shown.x) / shown.width) * 1280)).toBeLessThanOrEqual(2)
  expect(Math.abs((pressed.y ?? 0) - ((tap.y - shown.y) / shown.width) * 1280)).toBeLessThanOrEqual(2)
})
