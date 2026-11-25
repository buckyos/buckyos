import { readFileSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { openFromAllApps } from '../fixtures/desktop'

/**
 * Pages tidied by themselves ("this page only") in home mode, on the mock runtime (src/app/agent-browser/mock/reflow.ts):
 * the bucket is kept in the page and read by the same code as on a Zone, `browser.reflow_state` answers over it, and
 * the frame shows fallback pages built from it that post the `reflow.*` messages of the protocol. The specs play the
 * Agent's side through `window.__agentBrowserMock.reflow` and replace `askAgent` to see what the Agent is asked.
 * Nothing is made, and the Agent is not written to, unless the owner asks.
 */

const AGENT = 'codeassistant'
const STORY = 'https://news.example.com/story/1'

interface Ask { text: string }
interface ReflowControl {
  seededAt: number
  extensions: { askAgent?: (request: Ask) => Promise<void> }
  read(key: string): string | null
  keys(prefix?: string): string[]
  agentFallback(url: string, title?: string): Promise<void>
  silence(on: boolean): void
  address(url: string): Promise<{ url: string; site: string; page: string }>
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
const asks = (page: Page) => page.evaluate(() => ((window as unknown as MockWindow).__asked ?? []).map((ask) => ask.text))
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
    if (on) host.__agentBrowserMock.reflow.extensions.askAgent = async (request) => { host.__asked!.push({ text: request.text }) }
    else delete host.__agentBrowserMock.reflow.extensions.askAgent
  }, available)
}

/** The App on the mock Agent's browser, at home with nothing collected; what the Agent is asked is recorded unless `record` is off. */
async function openHome(page: Page, { tap = false, record = true } = {}) {
  await page.goto('/?scenario=normal')
  if (tap) await expect(page.getByTestId('mobile-taskbar')).toBeAttached()
  await openFromAllApps(page, 'agent-browser')
  const press = (testId: string) => tap ? page.getByTestId(testId).tap() : page.getByTestId(testId).click()
  await press(`ab-agent-${AGENT}`)
  await expect(page.getByTestId('ab-site-start')).toBeVisible()
  if (record) await recordAsks(page)
}

/** An address entered in the bar: at home, what was tidied for it shows there; anything else is the browser's. */
async function go(page: Page, address: string) {
  const input = page.getByTestId('ab-address-input')
  await input.click()
  await input.fill(address)
  await input.press('Enter')
}

/** The original page of an address the owner has nothing for, "Reflow" beside it, and "Tidy this page only", maybe with a line for the Agent. */
async function tidy(page: Page, address: string, note = '') {
  await go(page, address)
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
  await expect(page.getByTestId('ab-address-input')).toHaveValue(address.replace(/^https:\/\//, ''))
  await page.getByTestId('ab-reflow-button').click()
  await page.getByTestId('ab-reflow-choice-tidy').click()
  await expect(page.getByTestId('ab-tidy-page')).toContainText(address.replace(/^https:\/\//, ''))
  if (note) await page.getByTestId('ab-tidy-note').fill(note)
  await page.getByTestId('ab-tidy-confirm').click()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'home')
}

test('a page that was tidied shows at home at once, with when and by whom; asking for it again is a message to the Agent', async ({ page }) => {
  await openHome(page)
  await go(page, 'news.example.com/story/1')
  await expect(panel(page)).toHaveAttribute('data-phase', 'view')
  const { address, seededAt, when } = await page.evaluate(async (url) => {
    const reflow = (window as unknown as MockWindow).__agentBrowserMock.reflow
    return { address: await reflow.address(url), seededAt: reflow.seededAt, when: new Intl.DateTimeFormat('en', { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(reflow.seededAt)) }
  }, STORY)
  await expect(page.getByTestId('ab-reflow-frame')).toHaveAttribute('data-view', `${address.site}/fallback/${address.page}.html`)
  await expect(frame(page).locator('#title')).toHaveText('Story one')
  // The bar keeps the address of the original page; beside it, the time the page was made from the site and by whom.
  await expect(page.getByTestId('ab-address-input')).toHaveValue('news.example.com/story/1')
  await expect(page.getByTestId('ab-reflow-made')).toHaveText(`Made from the site as of ${when}`)
  expect(await madeAt(page)).toBe(seededAt)
  await expect(page.getByTestId('ab-reflow-maker')).toHaveText('By CodeAssistant, as a one-off page')
  // Nobody was asked: the bucket had it. The browser was not started and has no screen here.
  expect(await rpc(page, 'browser.reflow_state')).toContainEqual({ url: STORY })
  expect(await rpc(page, 'browser.ensure')).toEqual([])
  expect(await asks(page)).toEqual([])
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'home')
  await expect(page.getByTestId('ab-screen')).toHaveCount(0)

  // "Make this page again" is the Agent's: it is asked once, and the page is loaded anew when the new one is there.
  await page.getByTestId('ab-reflow-refresh').click()
  await expect(page.getByTestId('ab-reflow-notice')).toHaveAttribute('data-kind', 'agent_asked')
  await expect(page.getByTestId('ab-reflow-busy')).toBeVisible()
  const sent = await asks(page)
  expect(sent).toHaveLength(1)
  expect(sent[0]).toContain('again')
  expect(sent[0]).toContain(STORY)
  await page.getByTestId('ab-reflow-refresh').click()
  await page.waitForTimeout(300)
  expect(await asks(page)).toHaveLength(1)
  await agentFallback(page, STORY, 'Story one, made again')
  await expect(frame(page).locator('#title')).toHaveText('Story one, made again', { timeout: 10_000 })
  expect(await madeAt(page)).toBeGreaterThan(seededAt)
  await expect(page.getByTestId('ab-reflow-busy')).toHaveCount(0)
  await expect(page.getByTestId('ab-reflow-made')).not.toHaveText(`Made from the site as of ${when}`)
})

test('"Tidy this page only" asks the Agent once in the fixed format, with the line the owner added; the page shows when it arrives', async ({ page }) => {
  await openHome(page)
  await tidy(page, 'old.example.org/news', 'Only the headlines,\nplease')
  const making = page.getByTestId('ab-reflow-making')
  await expect(making).toBeVisible()
  expect(await asks(page)).toEqual([[
    'Please tidy this page only: make a one-off reflow page for it (reflow result put), without adding the website to my reading site.',
    'URL: https://old.example.org/news',
    'Title: old.example.org',
    'Note: Only the headlines, please',
  ].join('\n')])

  // The same address is not asked for again: coming back to home, or pressing Reflow on the original page once more.
  await page.getByTestId('ab-mode-chat').click()
  await page.getByTestId('ab-mode-home').click()
  await expect(making).toBeVisible()
  await go(page, 'https://old.example.org/news#top')
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-address-input')).toHaveValue('old.example.org/news')
  await page.getByTestId('ab-reflow-button').click()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'home')
  await expect(page.getByTestId('ab-reflow-sheet')).toHaveCount(0)
  await expect(making).toBeVisible()
  expect(await asks(page)).toHaveLength(1)
  // Unless the owner asks once more.
  await page.getByTestId('ab-reflow-ask-again').click()
  await expect.poll(async () => (await asks(page)).length).toBe(2)

  await agentFallback(page, 'https://old.example.org/news', 'News of the old site')
  await expect(panel(page)).toHaveAttribute('data-phase', 'view', { timeout: 10_000 })
  await expect(frame(page).locator('#title')).toHaveText('News of the old site')
  await expect(page.getByTestId('ab-reflow-maker')).toHaveText('By CodeAssistant, as a one-off page')
  // What the owner has now opens at once, from Reflow as from the address bar.
  await page.getByTestId('ab-mode-page').click()
  await page.getByTestId('ab-reflow-button').click()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'home')
  await expect(frame(page).locator('#title')).toHaveText('News of the old site')
  expect(await asks(page)).toHaveLength(2)
  // Nothing of it started anything in the Agent's browser beyond the owner's own browsing.
  expect((await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.rpc.map((call) => call.method))).filter((method) => /reflow_(refresh|action)/.test(method))).toEqual([])
})

test('the request reaches the Agent as a message in the browser conversation', async ({ page }) => {
  await openHome(page, { record: false })
  await tidy(page, 'old.example.org/integrated')
  await expect(page.getByTestId('ab-reflow-making')).toBeVisible({ timeout: 15_000 })
  await page.getByTestId('ab-mode-chat').click()
  await expect(page.getByTestId('ab-stage')).toContainText('Please tidy this page only')
  await expect(page.getByTestId('ab-stage')).toContainText('URL: https://old.example.org/integrated')
  const bound = await rpc(page, 'browser.bind_default')
  expect(bound).toHaveLength(1)
  await expect(page.getByTestId('ab-chat-panel').getByTestId('ab-conversation')).toHaveAttribute('data-session', String(bound[0].session_id))
})

test('without a way to the Agent the page says so, and asks when there is one', async ({ page }) => {
  await openHome(page)
  await recordAsks(page, false)
  await tidy(page, 'old.example.org/missing')
  await expect(page.getByTestId('ab-reflow-unmade')).toHaveAttribute('data-reason', 'agent_unreachable')
  await expect(page.getByTestId('ab-reflow-unmade')).toContainText('could not be asked')
  await recordAsks(page)
  await page.getByTestId('ab-reflow-ask').click()
  await expect(page.getByTestId('ab-reflow-making')).toBeVisible()
  expect((await asks(page)).map((text) => text.includes('https://old.example.org/missing'))).toEqual([true])
})

test('a link of a tidied page opens what the owner has for it, else the original site; the Agent is not asked, and Back returns', async ({ page }) => {
  await openHome(page)
  await go(page, 'old.example.org/about')
  await expect(panel(page)).toHaveAttribute('data-phase', 'view')
  expect(await madeAt(page)).toBe(await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.reflow.seededAt))
  await expect(frame(page).locator('#title')).toHaveText('About the old site')

  // Nothing was tidied for the link: the browser opens the original page.
  await frame(page).locator('a').click()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-address-input')).toHaveValue('old.example.org/contact')
  await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
  expect(await asks(page)).toEqual([])
  // Home still shows what was being read.
  await page.getByTestId('ab-mode-home').click()
  await expect(frame(page).locator('#title')).toHaveText('About the old site')

  // Once a page was tidied for the link, it opens at home; Back returns, and then leaves the tidied pages.
  await agentFallback(page, 'https://old.example.org/contact', 'Contact the old site')
  await frame(page).locator('a').click()
  await expect(page.getByTestId('ab-address-input')).toHaveValue('old.example.org/contact')
  await expect(frame(page).locator('#title')).toHaveText('Contact the old site')
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'home')
  await page.getByTestId('ab-back').click()
  await expect(page.getByTestId('ab-address-input')).toHaveValue('old.example.org/about')
  await expect(frame(page).locator('#title')).toHaveText('About the old site')
  // `reflow.source` of the page, like "Original page" of the strip: the browser on what the page was made from.
  await frame(page).locator('#original').click()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-address-input')).toHaveValue('old.example.org/about')
  await page.getByTestId('ab-mode-home').click()
  await page.getByTestId('ab-back').click()
  await expect(page.getByTestId('ab-site-start')).toBeVisible()
  await expect(page.getByTestId('ab-back')).toBeDisabled()
  expect(await asks(page)).toEqual([])
})

test('messages that do not come from the tidied page are ignored', async ({ page }) => {
  await openHome(page)
  await go(page, 'news.example.com/story/1')
  await expect(frame(page).locator('#title')).toHaveText('Story one')
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
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'home')
  await expect(panel(page)).toHaveAttribute('data-url', STORY)
  await expect(page.getByTestId('ab-address-input')).toHaveValue('news.example.com/story/1')
  expect((await rpc(page, 'browser.reflow_state')).some((params) => params.url === other)).toBe(false)
})

test('what was tidied shows with the browser unavailable, and when the Agent’s service does not answer at all', async ({ page }) => {
  await openHome(page)
  await page.evaluate((agent) => (window as unknown as MockWindow).__agentBrowserMock.setAvailability(agent, { available: false, enabled: false }), AGENT)
  await go(page, 'news.example.com/story/1')
  await expect(frame(page).locator('#title')).toHaveText('Story one')
  // The Agent is stopped: the App reads the bucket itself.
  await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.reflow.silence(true))
  await go(page, 'old.example.org/about')
  await expect(frame(page).locator('#title')).toHaveText('About the old site')
  await expect(page.getByTestId('ab-reflow-maker')).toHaveText('By CodeAssistant, as a one-off page')
  expect(await rpc(page, 'browser.ensure')).toEqual([])
  expect(await asks(page)).toEqual([])
})

test('an address that is not a web page is the browser\'s, and Reflow is not offered for it', async ({ page }) => {
  await openHome(page)
  await go(page, 'about:blank')
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
  await expect(page.getByTestId('ab-reflow-button')).toBeDisabled()
  expect(await asks(page)).toEqual([])
})

test('the menu has no reflow library any more; the bucket keeps only what was tidied', async ({ page }) => {
  await openHome(page)
  await page.getByTestId('ab-menu-button').click()
  await expect(page.getByTestId('ab-menu')).toBeVisible()
  await expect(page.getByTestId('ab-reflow-library-open')).toHaveCount(0)
  await expect(page.getByTestId('ab-menu-manage')).toBeVisible()
  const keys = await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.reflow.keys(''))
  expect(keys.every((key) => /^[a-z0-9-]+\/fallback\/[0-9a-f]{16}\.(html|json)$/.test(key))).toBe(true)
})

test('the App computes sites, pages and matches as the shared vectors say', async ({ page }) => {
  const vectors = JSON.parse(readFileSync(new URL('../../../../../../doc/opendan/protocol/fixtures/reflow_vectors.json', import.meta.url), 'utf8')) as {
    normalize: Array<{ url: string; normalized: string; site: string; page: string }>
    invalid: string[]
    match: Array<{ manifest_match: Array<{ host: string; path?: string }>; url: string; matches: boolean }>
  }
  await openHome(page)
  const computed = await page.evaluate(async (given) => {
    const reflow = (window as unknown as MockWindow).__agentBrowserMock.reflow
    return {
      normalize: await Promise.all(given.normalize.map(async (vector) => {
        const address = await reflow.address(vector.url)
        return { url: vector.url, normalized: address.url, site: address.site, page: address.page }
      })),
      invalid: await Promise.all(given.invalid.map((url) => reflow.address(url).then(() => false, () => true))),
      match: given.match.map((vector) => reflow.matches(vector.manifest_match, vector.url)),
    }
  }, vectors)
  expect(vectors.normalize.length).toBeGreaterThan(0)
  expect(computed.normalize).toEqual(vectors.normalize.map(({ url, normalized, site, page: id }) => ({ url, normalized, site, page: id })))
  expect(computed.invalid).toEqual(vectors.invalid.map(() => true))
  expect(computed.match).toEqual(vectors.match.map((vector) => vector.matches))
})

test.describe('on a phone', () => {
  test.use({ viewport: { width: 375, height: 812 }, hasTouch: true, isMobile: true })

  test('a tidied page fits: the page, when it was made, the way back to the original', async ({ page }) => {
    await openHome(page, { tap: true })
    const input = page.getByTestId('ab-address-input')
    await input.tap()
    await input.fill('news.example.com/story/1')
    await input.press('Enter')
    await expect(frame(page).locator('#title')).toHaveText('Story one')
    await expect(page.getByTestId('ab-reflow-made')).toContainText('Made from the site as of')
    await expect(page.getByTestId('ab-reflow-maker')).toContainText('CodeAssistant')
    // Everything of the strip and the page is inside the screen's width.
    for (const testId of ['ab-reflow-strip', 'ab-reflow-made', 'ab-reflow-source', 'ab-reflow-frame', 'ab-address']) {
      const box = await page.getByTestId(testId).boundingBox()
      expect(box, testId).not.toBeNull()
      expect(box!.x, testId).toBeGreaterThanOrEqual(0)
      expect(box!.x + box!.width, testId).toBeLessThanOrEqual(375)
    }
    expect((await page.getByTestId('ab-reflow-frame').boundingBox())!.height).toBeGreaterThan(400)

    await page.getByTestId('ab-reflow-source').tap()
    await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
    await expect(page.getByTestId('ab-screen-canvas')).toBeVisible()
    await expect(input).toHaveValue('news.example.com/story/1')
  })
})
