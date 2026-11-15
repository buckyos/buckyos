import { readFileSync } from 'node:fs'
import { crc32, deflateRawSync } from 'node:zlib'
import { expect, test, type Page } from '@playwright/test'
import { openFromAllApps } from '../fixtures/desktop'
import { pickLocal, pickZone, pickZoneFolder, putZoneFile, selectDialog } from '../fixtures/selectDialog'

/**
 * Reflow mode and the reflow library of the Agent browser, on the mock runtime (src/app/agent-browser/mock/reflow.ts):
 * the library is kept in the page and read and written by the same code as on a Zone, `browser.reflow_*` answers
 * over it, and the frame shows pages built from it that post the `reflow.*` messages of the protocol. The specs play
 * the Agent's side through `window.__agentBrowserMock.reflow` and replace `askAgent` to see what the Agent is asked.
 */

const AGENT = 'codeassistant'
const STORY = 'https://news.example.com/story/1'

interface Ask { group: string; text: string }
interface ReflowControl {
  seededAt: number
  extensions: { askAgent?: (request: Ask) => Promise<void> }
  read(key: string): string | null
  contentType(key: string): string | null
  keys(prefix?: string): string[]
  agentFallback(url: string, title?: string): Promise<void>
  failRefresh(url: string, error: string): void
  address(url: string): Promise<{ url: string; site: string; page: string; view: string }>
  matches(rules: Array<{ host: string; path?: string }>, url: string): boolean
}
interface MockControl {
  rpc: Array<{ method: string; params: Record<string, unknown> }>
  reflow: ReflowControl
  setAvailability(agentId: string, patch: { enabled?: boolean; available?: boolean }): void
}
type MockWindow = { __agentBrowserMock: MockControl; __asked?: Ask[] }

test.use({ viewport: { width: 1280, height: 900 } })

const rpc = (page: Page, method: string) =>
  page.evaluate((m) => (window as unknown as MockWindow).__agentBrowserMock.rpc.filter((call) => call.method === m).map((call) => call.params), method)
const asks = (page: Page) => page.evaluate(() => (window as unknown as MockWindow).__asked ?? [])
const keys = (page: Page, prefix: string) => page.evaluate((p) => (window as unknown as MockWindow).__agentBrowserMock.reflow.keys(p), prefix)
const stored = (page: Page, key: string) => page.evaluate((k) => (window as unknown as MockWindow).__agentBrowserMock.reflow.read(k), key)
const agentFallback = (page: Page, url: string, title: string) =>
  page.evaluate(([u, t]) => (window as unknown as MockWindow).__agentBrowserMock.reflow.agentFallback(u, t), [url, title] as const)
const frame = (page: Page) => page.frameLocator('[data-testid="ab-reflow-frame"]')
const panel = (page: Page) => page.getByTestId('ab-reflow-panel')
const madeAt = async (page: Page) => Number(await page.getByTestId('ab-reflow-made').getAttribute('data-made-at'))

/** `askAgent` of the conversation side replaced by a recorder (or taken away), whatever the App wired. */
async function recordAsks(page: Page, available = true) {
  await page.evaluate((on) => {
    const host = window as unknown as MockWindow
    host.__asked = []
    if (on) host.__agentBrowserMock.reflow.extensions.askAgent = async (request) => { host.__asked!.push({ group: request.group, text: request.text }) }
    else delete host.__agentBrowserMock.reflow.extensions.askAgent
  }, available)
}

/** The App on the mock Agent's browser, in reflow mode, with nothing open yet; what the Agent is asked is recorded unless `record` is off. */
async function openReflow(page: Page, { tap = false, record = true } = {}) {
  await page.goto('/?scenario=normal')
  if (tap) await expect(page.getByTestId('mobile-taskbar')).toBeAttached()
  await openFromAllApps(page, 'agent-browser')
  const press = (testId: string) => tap ? page.getByTestId(testId).tap() : page.getByTestId(testId).click()
  await press(`ab-agent-${AGENT}`)
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
  await press('ab-mode-reflow')
  await expect(page.getByTestId('ab-reflow-blank')).toBeVisible()
  if (record) await recordAsks(page)
}

async function go(page: Page, address: string) {
  const input = page.getByTestId('ab-address-input')
  await input.click()
  await input.fill(address)
  await input.press('Enter')
}

async function openLibrary(page: Page) {
  await page.getByTestId('ab-menu-button').click()
  await page.getByTestId('ab-reflow-library-open').click()
  await expect(page.getByTestId('ab-reflow-sites')).toBeVisible()
}

/** A zip archive of these files (deflated, or stored). */
function zip(files: Record<string, string>, stored = false): Buffer {
  const parts: Buffer[] = []
  const directory: Buffer[] = []
  let offset = 0
  for (const [name, content] of Object.entries(files)) {
    const data = Buffer.from(content)
    const body = stored ? data : deflateRawSync(data)
    const file = Buffer.from(name)
    const local = Buffer.alloc(30)
    local.writeUInt32LE(0x04034b50, 0)
    local.writeUInt16LE(20, 4)
    local.writeUInt16LE(stored ? 0 : 8, 8)
    local.writeUInt32LE(crc32(data), 14)
    local.writeUInt32LE(body.length, 18)
    local.writeUInt32LE(data.length, 22)
    local.writeUInt16LE(file.length, 26)
    const entry = Buffer.alloc(46)
    entry.writeUInt32LE(0x02014b50, 0)
    entry.writeUInt16LE(20, 4)
    entry.writeUInt16LE(20, 6)
    entry.writeUInt16LE(stored ? 0 : 8, 10)
    entry.writeUInt32LE(crc32(data), 16)
    entry.writeUInt32LE(body.length, 20)
    entry.writeUInt32LE(data.length, 24)
    entry.writeUInt16LE(file.length, 28)
    entry.writeUInt32LE(offset, 42)
    parts.push(local, file, body)
    directory.push(entry, file)
    offset += local.length + file.length + body.length
  }
  const size = directory.reduce((sum, part) => sum + part.length, 0)
  const end = Buffer.alloc(22)
  end.writeUInt32LE(0x06054b50, 0)
  end.writeUInt16LE(directory.length / 2, 8)
  end.writeUInt16LE(directory.length / 2, 10)
  end.writeUInt32LE(size, 12)
  end.writeUInt32LE(offset, 16)
  return Buffer.concat([...parts, ...directory, end])
}

const manifestOf = (site: string, host: string, extra: Record<string, unknown> = {}) =>
  JSON.stringify({ schema: 'buckyos.reflow/1', site, match: [{ host, path: '/*' }], ...extra })
const PAGE = '<!doctype html><html><head><meta charset="utf-8"><title>Reflow</title></head><body><main id="main"></main></body></html>'

test('a page that was made shows at once, with when and by whom; making it again updates both', async ({ page }) => {
  await openReflow(page)
  await go(page, 'news.example.com/story/1')
  await expect(panel(page)).toHaveAttribute('data-phase', 'view')
  const { address, seededAt, when } = await page.evaluate(async (url) => {
    const reflow = (window as unknown as MockWindow).__agentBrowserMock.reflow
    return { address: await reflow.address(url), seededAt: reflow.seededAt, when: new Intl.DateTimeFormat('en', { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(reflow.seededAt)) }
  }, STORY)
  await expect(page.getByTestId('ab-reflow-frame')).toHaveAttribute('data-view', address.view)
  await expect(frame(page).locator('#title')).toHaveText('Story /story/1')
  // The bar keeps the address of the original page; beside it, the time the page was made from the site and by whom.
  await expect(page.getByTestId('ab-address-input')).toHaveValue('news.example.com/story/1')
  await expect(page.getByTestId('ab-reflow-made')).toHaveText(`Made from the site as of ${when}`)
  await expect(page.getByTestId('ab-reflow-made')).toHaveAttribute('data-kind', 'package')
  expect(await madeAt(page)).toBe(seededAt)
  await expect(page.getByTestId('ab-reflow-maker')).toContainText('CodeAssistant')
  // Nothing was made for it and nobody was asked: the library had it. The screen stays closed in this mode.
  expect(await rpc(page, 'browser.reflow_refresh')).toEqual([])
  expect(await asks(page)).toEqual([])
  await expect(page.getByTestId('ab-screen')).toHaveCount(0)

  await page.getByTestId('ab-reflow-refresh').click()
  await expect(page.getByTestId('ab-reflow-busy')).toHaveAttribute('data-job', 'refresh')
  await expect.poll(() => madeAt(page), { timeout: 10_000 }).toBeGreaterThan(seededAt)
  await expect(page.getByTestId('ab-reflow-busy')).toHaveCount(0)
  expect(await rpc(page, 'browser.reflow_refresh')).toEqual([{ url: STORY }])
  // The page was told (`reflow.updated`) and read its result again; it was not loaded anew.
  await expect(frame(page).locator('#made')).toHaveText(String(await madeAt(page)))
  await expect(frame(page).locator('body')).toHaveAttribute('data-draws', '2')
  await expect(page.getByTestId('ab-reflow-made')).not.toHaveText(`Made from the site as of ${when}`)
})

test('a page of a site with a package and no result is made by a refresh; the original page stays one press away', async ({ page }) => {
  await openReflow(page)
  await go(page, 'news.example.com/story/2')
  await expect(page.getByTestId('ab-reflow-making')).toHaveAttribute('data-by', 'refresh')
  await expect(page.getByTestId('ab-reflow-status-source')).toBeVisible()
  await expect(panel(page)).toHaveAttribute('data-phase', 'view', { timeout: 10_000 })
  await expect(frame(page).locator('#title')).toHaveText('Story /story/2')
  await expect(page.getByTestId('ab-reflow-made')).toHaveAttribute('data-kind', 'package')
  expect(await rpc(page, 'browser.reflow_refresh')).toEqual([{ url: 'https://news.example.com/story/2' }])
  expect(await asks(page)).toEqual([])

  // While another page is being made, the original page is shown instead: the browser opens the address.
  await go(page, 'news.example.com/story/3')
  await expect(page.getByTestId('ab-reflow-making')).toBeVisible()
  await page.getByTestId('ab-reflow-status-source').click()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
  await expect(page.getByTestId('ab-address-input')).toHaveValue('news.example.com/story/3')
})

test('a refresh that fails goes to the Agent with the reason, once; its fallback page shows when it is there', async ({ page }) => {
  await openReflow(page)
  await go(page, 'news.example.com/broken/1')
  await expect(page.getByTestId('ab-reflow-making')).toHaveAttribute('data-by', 'refresh')
  await expect(page.getByTestId('ab-reflow-making')).toHaveAttribute('data-by', 'agent', { timeout: 10_000 })
  await expect(page.getByTestId('ab-reflow-failure')).toContainText('no element matches')
  const sent = await asks(page)
  expect(sent).toHaveLength(1)
  expect(sent[0].group).toBe('default')
  for (const part of ['https://news.example.com/broken/1', 'no element matches', 'news-example-com', 'Headline and text only, one column']) expect(sent[0].text).toContain(part)

  await agentFallback(page, 'https://news.example.com/broken/1', 'Repaired by hand')
  await expect(panel(page)).toHaveAttribute('data-phase', 'view', { timeout: 10_000 })
  await expect(page.getByTestId('ab-reflow-made')).toHaveAttribute('data-kind', 'fallback')
  await expect(frame(page).locator('#title')).toHaveText('Repaired by hand')
  expect(await asks(page)).toHaveLength(1)
})

test('a page the library has nothing for is asked of the Agent once, and shows when the fallback page arrives', async ({ page }) => {
  await openReflow(page)
  await go(page, 'old.example.org/news')
  const making = page.getByTestId('ab-reflow-making')
  await expect(making).toHaveAttribute('data-by', 'agent')
  let sent = await asks(page)
  expect(sent).toHaveLength(1)
  expect(sent[0].group).toBe('default')
  expect(sent[0].text).toContain('https://old.example.org/news')
  expect(sent[0].text).toContain('fallback page')
  expect(await rpc(page, 'browser.reflow_refresh')).toEqual([])

  // The same address is not asked for again: coming back to the mode, entering it again.
  await page.getByTestId('ab-mode-chat').click()
  await page.getByTestId('ab-mode-reflow').click()
  await expect(making).toHaveAttribute('data-by', 'agent')
  await go(page, 'https://old.example.org/news#top')
  await expect(making).toHaveAttribute('data-by', 'agent')
  expect(await asks(page)).toHaveLength(1)
  // Unless the user asks once more.
  await page.getByTestId('ab-reflow-ask-again').click()
  await expect.poll(async () => (await asks(page)).length).toBe(2)

  await agentFallback(page, 'https://old.example.org/news', 'News of the old site')
  await expect(panel(page)).toHaveAttribute('data-phase', 'view', { timeout: 10_000 })
  await expect(frame(page).locator('#title')).toHaveText('News of the old site')
  await expect(page.getByTestId('ab-reflow-made')).toHaveAttribute('data-kind', 'fallback')
  await expect(page.getByTestId('ab-reflow-maker')).toHaveText('By CodeAssistant, as a one-off page')
  sent = await asks(page)
  expect(sent).toHaveLength(2)
})

test('the request reaches the Agent as a message in the default conversation', async ({ page }) => {
  await openReflow(page, { record: false })
  await go(page, 'old.example.org/integrated')
  await expect(page.getByTestId('ab-reflow-making')).toHaveAttribute('data-by', 'agent', { timeout: 15_000 })
  await page.getByTestId('ab-mode-chat').click()
  await expect(page.getByTestId('ab-stage')).toContainText('Please make a reflow page for this address (a fallback page): https://old.example.org/integrated')
})

test('without a way to the Agent the page says so, and asks when there is one', async ({ page }) => {
  await openReflow(page)
  await recordAsks(page, false)
  await go(page, 'old.example.org/missing')
  await expect(page.getByTestId('ab-reflow-unmade')).toHaveAttribute('data-reason', 'agent_unreachable')
  await expect(page.getByTestId('ab-reflow-unmade')).toContainText('could not be asked')
  await recordAsks(page)
  await page.getByTestId('ab-reflow-retry').click()
  await expect(page.getByTestId('ab-reflow-making')).toHaveAttribute('data-by', 'agent')
  expect((await asks(page)).map((ask) => ask.text.includes('https://old.example.org/missing'))).toEqual([true])
})

test('a fallback page that is there shows; its links open reflowed, and Back returns', async ({ page }) => {
  await openReflow(page)
  await go(page, 'old.example.org/about')
  await expect(panel(page)).toHaveAttribute('data-phase', 'view')
  await expect(page.getByTestId('ab-reflow-made')).toHaveAttribute('data-kind', 'fallback')
  expect(await madeAt(page)).toBe(await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.reflow.seededAt))
  await expect(frame(page).locator('#title')).toHaveText('About the old site')
  expect(await asks(page)).toEqual([])
  await expect(page.getByTestId('ab-back')).toBeDisabled()

  await frame(page).locator('a').click()
  await expect(page.getByTestId('ab-address-input')).toHaveValue('old.example.org/contact')
  await expect(page.getByTestId('ab-reflow-making')).toHaveAttribute('data-by', 'agent')
  await page.getByTestId('ab-back').click()
  await expect(page.getByTestId('ab-address-input')).toHaveValue('old.example.org/about')
  await expect(frame(page).locator('#title')).toHaveText('About the old site')
})

test('making a fallback page again is the Agent\'s: it is asked, and the page is loaded anew when the new one is there', async ({ page }) => {
  await openReflow(page)
  await go(page, 'old.example.org/about')
  await expect(frame(page).locator('#title')).toHaveText('About the old site')
  const before = await madeAt(page)
  await page.getByTestId('ab-reflow-refresh').click()
  await expect(page.getByTestId('ab-reflow-notice')).toHaveAttribute('data-kind', 'agent_asked')
  await expect(page.getByTestId('ab-reflow-busy')).toHaveAttribute('data-job', 'agent')
  expect(await rpc(page, 'browser.reflow_refresh')).toEqual([{ url: 'https://old.example.org/about' }])
  const sent = await asks(page)
  expect(sent).toHaveLength(1)
  expect(sent[0].text).toContain('again')
  expect(sent[0].text).toContain('https://old.example.org/about')
  // Asking twice for the same page sends nothing more.
  await page.getByTestId('ab-reflow-refresh').click()
  await expect.poll(async () => (await rpc(page, 'browser.reflow_refresh')).length).toBe(2)
  expect(await asks(page)).toHaveLength(1)

  await agentFallback(page, 'https://old.example.org/about', 'About, made again')
  await expect(frame(page).locator('#title')).toHaveText('About, made again', { timeout: 10_000 })
  expect(await madeAt(page)).toBeGreaterThan(before)
  await expect(page.getByTestId('ab-reflow-busy')).toHaveCount(0)
})

test('the page opens a link, has an action run on the original page, asks to be made again and goes back to the original', async ({ page }) => {
  await openReflow(page)
  await go(page, 'news.example.com/story/1')
  await expect(frame(page).locator('#title')).toHaveText('Story /story/1')
  const first = await madeAt(page)

  // `reflow.action`: the Agent's browser presses the control on the original page and makes the page again.
  await frame(page).locator('#more').click()
  await expect(page.getByTestId('ab-reflow-busy')).toHaveAttribute('data-job', 'action')
  await expect(frame(page).locator('#comments')).toHaveText('#load-comments', { timeout: 10_000 })
  await expect(page.getByTestId('ab-reflow-busy')).toHaveCount(0)
  expect(await rpc(page, 'browser.reflow_action')).toEqual([{ url: STORY, selector: '#load-comments' }])
  expect(await madeAt(page)).toBeGreaterThan(first)
  await expect(frame(page).locator('body')).toHaveAttribute('data-draws', '2')

  // `reflow.refresh` is the refresh button.
  await frame(page).locator('#again').click()
  await expect(frame(page).locator('body')).toHaveAttribute('data-draws', '3', { timeout: 10_000 })
  expect(await rpc(page, 'browser.reflow_refresh')).toEqual([{ url: STORY }])

  // `reflow.open`: the link is the new address, looked up like one typed into the bar.
  await frame(page).locator('#next').click()
  await expect(page.getByTestId('ab-address-input')).toHaveValue('news.example.com/story/2')
  await expect(frame(page).locator('#title')).toHaveText('Story /story/2', { timeout: 10_000 })
  await page.getByTestId('ab-back').click()
  await expect(frame(page).locator('#title')).toHaveText('Story /story/1')

  // `reflow.source`: original page mode, the browser on what the page was made from.
  await frame(page).locator('#original').click()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
  await expect(page.getByTestId('ab-address-input')).toHaveValue('news.example.com/story/1')
  // Back in reflow the page is still the one that was read.
  await page.getByTestId('ab-mode-reflow').click()
  await expect(frame(page).locator('#title')).toHaveText('Story /story/1')
  expect(await asks(page)).toEqual([])
})

test('an action that fails is handed to the Agent with the element and the reason', async ({ page }) => {
  await openReflow(page)
  await go(page, 'news.example.com/story/1')
  await expect(frame(page).locator('#title')).toHaveText('Story /story/1')
  await page.evaluate((url) => (window as unknown as MockWindow).__agentBrowserMock.reflow.failRefresh(url, 'no element matches `#load-comments`'), STORY)
  await frame(page).locator('#more').click()
  const notice = page.getByTestId('ab-reflow-notice')
  await expect(notice).toHaveAttribute('data-kind', 'action_failed', { timeout: 10_000 })
  await expect(notice).toHaveAttribute('data-asked', 'true')
  await expect(notice).toContainText('no element matches `#load-comments`')
  const sent = await asks(page)
  expect(sent).toHaveLength(1)
  for (const part of [STORY, '#load-comments', 'no element matches']) expect(sent[0].text).toContain(part)
  // What was made before is still shown.
  await expect(frame(page).locator('#title')).toHaveText('Story /story/1')
})

test('messages that do not come from the reflow page are ignored', async ({ page }) => {
  await openReflow(page)
  await go(page, 'news.example.com/story/1')
  await expect(frame(page).locator('#title')).toHaveText('Story /story/1')
  const other = 'https://news.example.com/story/9'
  // From the App's own window, and from another frame: not the reflow frame's window.
  await page.evaluate((url) => {
    window.postMessage({ type: 'reflow.open', url }, '*')
    const stranger = document.createElement('iframe')
    stranger.srcdoc = `<script>parent.postMessage({ type: 'reflow.open', url: ${JSON.stringify(url)} }, '*')</script>`
    document.body.append(stranger)
  }, other)
  // From the reflow frame's window, but a document of another origin in it.
  await page.evaluate((url) => {
    const reflow = document.querySelector<HTMLIFrameElement>('[data-testid="ab-reflow-frame"]')!
    reflow.removeAttribute('srcdoc')
    reflow.src = `data:text/html,<script>parent.postMessage({ type: 'reflow.open', url: ${JSON.stringify(url)} }, '*'); parent.postMessage({ type: 'reflow.source' }, '*')</script><p id="stranger">stranger</p>`
  }, other)
  await expect(frame(page).locator('#stranger')).toBeVisible()
  await page.waitForTimeout(500)
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'reflow')
  await expect(panel(page)).toHaveAttribute('data-url', STORY)
  await expect(page.getByTestId('ab-address-input')).toHaveValue('news.example.com/story/1')
  expect((await rpc(page, 'browser.reflow_state')).some((params) => params.url === other)).toBe(false)
})

test('with the browser unavailable what was made still shows, and making says why it cannot', async ({ page }) => {
  await openReflow(page)
  await page.evaluate((agent) => (window as unknown as MockWindow).__agentBrowserMock.setAvailability(agent, { available: false }), AGENT)
  await go(page, 'news.example.com/story/1')
  await expect(frame(page).locator('#title')).toHaveText('Story /story/1')
  const before = await madeAt(page)
  await page.getByTestId('ab-reflow-refresh').click()
  const notice = page.getByTestId('ab-reflow-notice')
  await expect(notice).toHaveAttribute('data-kind', 'browser_unavailable')
  await expect(notice).toContainText('browser is not available')
  await expect(notice).toContainText('can still be read')
  expect(await madeAt(page)).toBe(before)
  await page.getByTestId('ab-reflow-notice-dismiss').click()
  await expect(notice).toHaveCount(0)

  // A browser that is turned off does not even answer what the library has: the App reads the library itself.
  await page.evaluate((agent) => (window as unknown as MockWindow).__agentBrowserMock.setAvailability(agent, { enabled: false }), AGENT)
  await go(page, 'old.example.org/about')
  await expect(frame(page).locator('#title')).toHaveText('About the old site')
  await expect(page.getByTestId('ab-reflow-made')).toHaveAttribute('data-kind', 'fallback')
  await go(page, 'old.example.org/nothing')
  await expect(page.getByTestId('ab-reflow-unmade')).toHaveAttribute('data-reason', 'browser_unavailable')
  expect(await asks(page)).toEqual([])
})

test('an address that is not a web page is not reflowed', async ({ page }) => {
  await openReflow(page)
  await go(page, 'about:blank')
  await expect(page.getByTestId('ab-reflow-unsupported')).toBeVisible()
  expect(await asks(page)).toEqual([])
})

test('the preference of a site is shown and changed: in the package, or beside a site that has none; the Agent is told', async ({ page }) => {
  await openReflow(page)
  await go(page, 'news.example.com/story/1')
  await expect(frame(page).locator('#title')).toHaveText('Story /story/1')
  await page.getByTestId('ab-reflow-preference').click()
  const input = page.getByTestId('ab-reflow-preference-input')
  await expect(input).toHaveValue('Headline and text only, one column')
  await input.fill('Large text, no pictures')
  await page.getByTestId('ab-reflow-preference-save').click()
  await expect(page.getByTestId('ab-reflow-preference-sheet')).toHaveCount(0)
  await expect(page.getByTestId('ab-reflow-notice')).toHaveAttribute('data-kind', 'agent_asked')
  const manifest = JSON.parse((await stored(page, 'news-example-com/manifest.json'))!)
  expect(manifest).toMatchObject({ schema: 'buckyos.reflow/1', site: 'news-example-com', preference: 'Large text, no pictures', refresh: { script: 'refresh.mjs', timeout_s: 60 } })
  let sent = await asks(page)
  expect(sent).toHaveLength(1)
  for (const part of ['news.example.com', 'Large text, no pictures', STORY]) expect(sent[0].text).toContain(part)

  // A site without a package: the preference is kept beside it and goes into the request for a fallback page.
  await recordAsks(page)
  await go(page, 'old.example.org/about')
  await expect(frame(page).locator('#title')).toHaveText('About the old site')
  await page.getByTestId('ab-reflow-preference').click()
  await expect(input).toHaveValue('')
  await input.fill('Text only')
  await page.getByTestId('ab-reflow-preference-save').click()
  await expect.poll(() => stored(page, 'old-example-org/preference.json')).not.toBeNull()
  expect(JSON.parse((await stored(page, 'old-example-org/preference.json'))!)).toMatchObject({ schema: 'buckyos.reflow.preference/1', preference: 'Text only' })
  await expect.poll(async () => (await asks(page)).length).toBe(1)
  await go(page, 'old.example.org/history')
  await expect(page.getByTestId('ab-reflow-making')).toHaveAttribute('data-by', 'agent')
  sent = await asks(page)
  expect(sent).toHaveLength(2)
  expect(sent[1].text).toContain('https://old.example.org/history')
  expect(sent[1].text).toContain('My preference for this site: Text only')
})

test('the library lists the sites, shows one and deletes it with everything made for it', async ({ page }) => {
  await openReflow(page)
  await openLibrary(page)
  const news = page.getByTestId('ab-reflow-site-news-example-com')
  await expect(news).toHaveAttribute('data-package', 'true')
  await expect(news).toHaveAttribute('data-results', '1')
  await expect(news).toContainText('news.example.com')
  await expect(news).toContainText('Package by CodeAssistant')
  await expect(news).toContainText('Preference: Headline and text only, one column')
  const old = page.getByTestId('ab-reflow-site-old-example-org')
  await expect(old).toHaveAttribute('data-package', 'false')
  await expect(old).toHaveAttribute('data-fallbacks', '1')
  await expect(old).toContainText('No package')

  await news.click()
  await expect(page.getByTestId('ab-reflow-site-facts')).toContainText('CodeAssistant')
  await expect(page.getByTestId('ab-reflow-manifest')).toContainText('"schema": "buckyos.reflow/1"')
  await expect(page.getByTestId('ab-reflow-manifest')).toContainText('"host": "news.example.com"')
  expect(await page.getByTestId('ab-reflow-file').evaluateAll((items) => items.map((item) => item.getAttribute('data-name')))).toEqual(['index.html', 'manifest.json', 'refresh.mjs', 'style.css'])

  expect((await keys(page, 'news-example-com/')).some((key) => key.startsWith('news-example-com/results/'))).toBe(true)
  await page.getByTestId('ab-reflow-site-delete').click()
  await page.getByTestId('ab-reflow-delete-accept').click()
  await expect(page.getByTestId('ab-reflow-sites')).toBeVisible()
  await expect(news).toHaveCount(0)
  expect(await keys(page, 'news-example-com/')).toEqual([])

  await old.click()
  await page.getByTestId('ab-reflow-site-delete').click()
  await page.getByTestId('ab-reflow-delete-accept').click()
  await expect(page.getByTestId('ab-reflow-library-empty')).toBeVisible()
  expect(await keys(page, '')).toEqual([])
})

test('a package is imported from a zip in the Zone: checked, stored with its types, and used for the page being read', async ({ page }) => {
  await openReflow(page)
  // The site has no package yet; its page waits for the Agent, and the user's preference is kept beside the site.
  await go(page, 'blog.example.net/post/1')
  await expect(page.getByTestId('ab-reflow-making')).toHaveAttribute('data-by', 'agent')
  await page.getByTestId('ab-reflow-preference').click()
  await page.getByTestId('ab-reflow-preference-input').fill('Text only')
  await page.getByTestId('ab-reflow-preference-save').click()
  await expect.poll(() => stored(page, 'blog-example-net/preference.json')).not.toBeNull()

  // A zipped directory, with what a desktop leaves in one.
  const archive = zip({
    'blog-example-net/manifest.json': manifestOf('blog-example-net', 'blog.example.net', { refresh: { script: 'refresh.mjs', timeout_s: 30 } }),
    'blog-example-net/index.html': await stored(page, 'news-example-com/index.html') ?? '',
    'blog-example-net/refresh.mjs': 'export default async () => ({})\n',
    'blog-example-net/assets/app.css': 'body { margin: 0 }\n',
    'blog-example-net/.DS_Store': 'x',
  })
  await putZoneFile(page, '/home/Documents/blog-package.zip', archive, 'application/zip')
  const started = Date.now()
  await openLibrary(page)
  await page.getByTestId('ab-reflow-import-zip').click()
  await pickZone(selectDialog(page), '/home/Documents/blog-package.zip')
  const report = page.getByTestId('ab-reflow-report')
  await expect(report).toHaveAttribute('data-kind', 'imported')
  await expect(report).toContainText('blog-example-net')
  await expect(page.getByTestId('ab-reflow-site-blog-example-net')).toHaveAttribute('data-package', 'true')

  expect((await keys(page, 'blog-example-net/')).filter((key) => !key.includes('/results/'))).toEqual([
    'blog-example-net/assets/app.css', 'blog-example-net/index.html', 'blog-example-net/manifest.json', 'blog-example-net/refresh.mjs',
  ])
  const types = await page.evaluate(() => {
    const reflow = (window as unknown as MockWindow).__agentBrowserMock.reflow
    return ['assets/app.css', 'index.html', 'manifest.json', 'refresh.mjs'].map((name) => reflow.contentType(`blog-example-net/${name}`))
  })
  expect(types).toEqual(['text/css; charset=utf-8', 'text/html; charset=utf-8', 'application/json', 'text/javascript; charset=utf-8'])
  // Stamped like `reflow put`: when it went in, who put it in; the preference kept for the site went into the manifest.
  const manifest = JSON.parse((await stored(page, 'blog-example-net/manifest.json'))!)
  expect(manifest.updated_at).toBeGreaterThanOrEqual(started)
  expect(manifest.author).toMatchObject({ kind: 'user' })
  expect(manifest.author.id).toBeTruthy()
  expect(manifest.preference).toBe('Text only')

  // The page that was waiting is now made with the package.
  await page.getByTestId('ab-reflow-library-close').click()
  await expect(panel(page)).toHaveAttribute('data-phase', 'view', { timeout: 10_000 })
  await expect(page.getByTestId('ab-reflow-made')).toHaveAttribute('data-kind', 'package')
  await expect(frame(page).locator('#title')).toHaveText('Story /post/1')
  expect(await rpc(page, 'browser.reflow_refresh')).toEqual([{ url: 'https://blog.example.net/post/1' }])
})

test('what is not a valid package is refused with what is wrong, and nothing is stored', async ({ page }) => {
  await openReflow(page)
  await openLibrary(page)
  const before = await keys(page, '')
  const report = page.getByTestId('ab-reflow-report')

  // A package for another host than its site, without any code, bringing a result along.
  const bad = zip({
    'manifest.json': manifestOf('shop-example-com', 'store.example.com'),
    'results/0123456789abcdef.json': '{}',
  }, true)
  await page.getByTestId('ab-reflow-import-zip').click()
  await pickLocal(page, { name: 'bad.zip', mimeType: 'application/zip', buffer: bad })
  await expect(report).toHaveAttribute('data-kind', 'invalid')
  for (const problem of ['match[0].host "store.example.com" is not the lower-case host name of site shop-example-com', 'a package has refresh.mjs, index.html or both', 'results/ holds the library\'s data and is not part of a package']) {
    await expect(report).toContainText(problem)
  }

  await page.getByTestId('ab-reflow-import-zip').click()
  await pickLocal(page, { name: 'notes.zip', mimeType: 'application/zip', buffer: Buffer.from('not an archive') })
  await expect(report).toContainText('the file is not a zip archive')

  await page.getByTestId('ab-reflow-import-zip').click()
  await pickLocal(page, { name: 'nameless.zip', mimeType: 'application/zip', buffer: zip({ 'manifest.json': manifestOf('shop', 'shop'), 'index.html': PAGE }) })
  await expect(report).toContainText('site "shop" is not a site name')

  expect(await keys(page, '')).toEqual(before)
  await expect(page.getByTestId('ab-reflow-site-shop-example-com')).toHaveCount(0)
})

test('a package is imported from a folder of the Zone, where an Agent that cannot write the library leaves it', async ({ page }) => {
  await openReflow(page)
  const folder = '/home/Documents/shop-example-com'
  await putZoneFile(page, `${folder}/manifest.json`, Buffer.from(manifestOf('shop-example-com', 'shop.example.com', { author: { kind: 'agent', id: 'did:example:writer' }, preference: 'Prices first' })), 'application/json')
  await putZoneFile(page, `${folder}/index.html`, Buffer.from(PAGE), 'text/html')
  await putZoneFile(page, `${folder}/assets/app.js`, Buffer.from('export {}\n'), 'text/javascript')
  await putZoneFile(page, `${folder}/.notes.txt`, Buffer.from('mine'), 'text/plain')
  await openLibrary(page)
  await page.getByTestId('ab-reflow-import-folder').click()
  const picker = selectDialog(page)
  await expect(picker).toHaveAttribute('data-mode', 'folder')
  await expect(picker.getByTestId('select-local')).toHaveCount(0)
  await pickZoneFolder(picker, folder)
  await expect(page.getByTestId('ab-reflow-report')).toHaveAttribute('data-kind', 'imported')
  const row = page.getByTestId('ab-reflow-site-shop-example-com')
  await expect(row).toContainText('shop.example.com')
  await expect(row).toContainText('Package by did:example:writer')
  await expect(row).toContainText('Preference: Prices first')
  expect(await keys(page, 'shop-example-com/')).toEqual(['shop-example-com/assets/app.js', 'shop-example-com/index.html', 'shop-example-com/manifest.json'])
  expect(await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.reflow.contentType('shop-example-com/assets/app.js'))).toBe('text/javascript; charset=utf-8')

  // A folder that is no package is refused the same way.
  await putZoneFile(page, '/home/Documents/just-notes/readme.txt', Buffer.from('hello'), 'text/plain')
  await page.getByTestId('ab-reflow-import-folder').click()
  await pickZoneFolder(picker, '/home/Documents/just-notes')
  await expect(page.getByTestId('ab-reflow-report')).toHaveAttribute('data-kind', 'invalid')
  await expect(page.getByTestId('ab-reflow-report')).toContainText('manifest.json is missing')
})

test('the App computes sites, pages and matches as the shared vectors say', async ({ page }) => {
  const vectors = JSON.parse(readFileSync(new URL('../../../../../../doc/opendan/protocol/fixtures/reflow_vectors.json', import.meta.url), 'utf8')) as {
    normalize: Array<{ url: string; normalized: string; site: string; page: string; view: string }>
    invalid: string[]
    match: Array<{ manifest_match: Array<{ host: string; path?: string }>; url: string; matches: boolean }>
  }
  await openReflow(page)
  const computed = await page.evaluate(async (given) => {
    const reflow = (window as unknown as MockWindow).__agentBrowserMock.reflow
    return {
      normalize: await Promise.all(given.normalize.map(async (vector) => {
        const address = await reflow.address(vector.url)
        return { url: vector.url, normalized: address.url, site: address.site, page: address.page, view: address.view }
      })),
      invalid: await Promise.all(given.invalid.map((url) => reflow.address(url).then(() => false, () => true))),
      match: given.match.map((vector) => reflow.matches(vector.manifest_match, vector.url)),
    }
  }, vectors)
  expect(vectors.normalize.length).toBeGreaterThan(0)
  expect(computed.normalize).toEqual(vectors.normalize.map(({ url, normalized, site, page: id, view }) => ({ url, normalized, site, page: id, view })))
  expect(computed.invalid).toEqual(vectors.invalid.map(() => true))
  expect(computed.match).toEqual(vectors.match.map((vector) => vector.matches))
})

test.describe('on a phone', () => {
  test.use({ viewport: { width: 375, height: 812 }, hasTouch: true, isMobile: true })

  test('the reflow view fits: the page, when it was made, the preference, a link, the way back to the original', async ({ page }) => {
    await openReflow(page, { tap: true })
    const input = page.getByTestId('ab-address-input')
    await input.tap()
    await input.fill('news.example.com/story/1')
    await input.press('Enter')
    await expect(frame(page).locator('#title')).toHaveText('Story /story/1')
    await expect(page.getByTestId('ab-reflow-made')).toContainText('Made from the site as of')
    await expect(page.getByTestId('ab-reflow-maker')).toContainText('CodeAssistant')
    // Everything of the strip and the page is inside the screen's width.
    for (const testId of ['ab-reflow-strip', 'ab-reflow-made', 'ab-reflow-preference', 'ab-reflow-source', 'ab-reflow-frame', 'ab-address']) {
      const box = await page.getByTestId(testId).boundingBox()
      expect(box, testId).not.toBeNull()
      expect(box!.x, testId).toBeGreaterThanOrEqual(0)
      expect(box!.x + box!.width, testId).toBeLessThanOrEqual(375)
    }
    expect((await page.getByTestId('ab-reflow-frame').boundingBox())!.height).toBeGreaterThan(400)

    await page.getByTestId('ab-reflow-preference').tap()
    await expect(page.getByTestId('ab-reflow-preference-input')).toHaveValue('Headline and text only, one column')
    await page.getByTestId('ab-reflow-preference-cancel').tap()

    await frame(page).locator('#next').tap()
    await expect(input).toHaveValue('news.example.com/story/2')
    await expect(frame(page).locator('#title')).toHaveText('Story /story/2', { timeout: 10_000 })

    await page.getByTestId('ab-reflow-source').tap()
    await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
    await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
    await expect(input).toHaveValue('news.example.com/story/2')
  })
})
