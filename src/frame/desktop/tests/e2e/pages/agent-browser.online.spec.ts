import { expect, test, type Page } from '@playwright/test'
import { openFromAllApps } from '../fixtures/desktop'

/**
 * An Agent whose browser is not there (protocol §2.1 `unavailable_code`), on the mock Agent Service: the browser is
 * ext-core's online item `chromium`, which the mock control panel (api/settings_mock.ts) downloads in a few seconds.
 */

const AGENT = 'codeassistant'
const MOCK_CONTROL_PANEL = 'buckyos.mock.control-panel.v1'

interface Availability { enabled?: boolean; available?: boolean; unavailable?: string | null; download?: 'missing' | 'failed' | 'ready' }
interface MockWindow {
  __agentBrowserMock: {
    log: Array<{ message: { type: string } }>
    setAvailability(agentId: string, patch: Availability): void
  }
  __extToolsMock: { failNextDownload(item: string, error: string): void }
}

const setAvailability = (page: Page, patch: Availability) =>
  page.evaluate(([agent, value]) => (window as unknown as MockWindow).__agentBrowserMock.setAvailability(agent, value), [AGENT, patch] as const)
const hellos = (page: Page) => page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.log.filter((entry) => entry.message.type === 'hello').length)

/** Signs the mock in as Alice, who is not an administrator. */
async function asMember(page: Page) {
  await page.addInitScript(([key]) => {
    if (!window.sessionStorage.getItem('ab-online-seeded')) {
      window.localStorage.setItem(key, JSON.stringify({ account: { user_id: 'alice', user_name: 'Alice', user_type: 'user', did: 'did:bns:alice' } }))
      window.sessionStorage.setItem('ab-online-seeded', '1')
    }
  }, [MOCK_CONTROL_PANEL] as const)
}

async function openUnavailable(page: Page, patch: Availability, tap = false) {
  await page.goto('/?scenario=normal')
  if (tap) await expect(page.getByTestId('mobile-taskbar')).toBeAttached()
  await openFromAllApps(page, 'agent-browser')
  await expect(page.getByTestId('agent-browser')).toBeVisible()
  await setAvailability(page, patch)
  const agent = page.getByTestId(`ab-agent-${AGENT}`)
  await (tap ? agent.tap() : agent.click())
  const notice = page.getByTestId('ab-unavailable')
  await expect(notice).toBeVisible()
  return notice
}

async function browse(page: Page) {
  const input = page.getByTestId('ab-address-input')
  await input.click()
  await input.fill('example.com')
  await input.press('Enter')
  await expect(page.getByTestId('ab-screen-canvas')).toHaveAttribute('data-seq', /\d+/)
}

test.use({ viewport: { width: 1280, height: 900 } })

test('a browser not downloaded yet is downloaded from the App by an administrator, and then it connects by itself', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(String(error)))
  const notice = await openUnavailable(page, { download: 'missing' })
  await expect(notice).toHaveAttribute('data-reason', 'not_downloaded')
  await expect(notice).toContainText('The browser is not downloaded to this node yet.')
  await expect(page.getByTestId('ab-unavailable-detail')).toContainText('the browser is not downloaded on this node yet')
  const node = page.getByTestId('ab-download-ood1')
  await expect(node).toHaveAttribute('data-state', 'missing')
  await expect(node).toContainText('Not downloaded')
  await expect(page.getByTestId('ab-download')).toContainText('199 MB')
  await expect(page.getByTestId('ab-ask-admin')).toHaveCount(0)
  expect(await hellos(page)).toBe(0)

  await page.getByTestId('ab-download-browser').click()
  await expect(page.getByTestId('ab-download-browser')).toHaveCount(0)
  await expect(node).toHaveAttribute('data-state', 'downloading')
  await expect(node).toContainText(/Downloading \d+%/)
  await expect(node.getByRole('progressbar')).toBeVisible()
  await expect(page.getByTestId('ab-unavailable-detail')).toContainText('the browser is being downloaded to this node')

  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open', { timeout: 15_000 })
  await expect(page.getByTestId('ab-unavailable')).toHaveCount(0)
  await browse(page)
  expect(errors).toEqual([])
})

test('a failed download says why, and the administrator tries again', async ({ page }) => {
  const notice = await openUnavailable(page, { download: 'failed' })
  await expect(notice).toHaveAttribute('data-reason', 'not_downloaded')
  await expect(page.getByTestId('ab-unavailable-detail')).toContainText('the browser\'s download to this node failed')
  const node = page.getByTestId('ab-download-ood1')
  await expect(node).toHaveAttribute('data-state', 'failed')
  await expect(node).toContainText('Download failed')
  await expect(node).toContainText('connection to storage.googleapis.com timed out')

  await page.evaluate(() => (window as unknown as MockWindow).__extToolsMock.failNextDownload('chromium', 'sha256 of the archive does not match'))
  await page.getByTestId('ab-download-browser').filter({ hasText: 'Retry the download' }).click()
  await expect(node).toContainText('sha256 of the archive does not match', { timeout: 10_000 })
  await expect(node).toHaveAttribute('data-state', 'failed')

  await page.getByTestId('ab-download-browser').click()
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open', { timeout: 15_000 })
})

test('someone who is not an administrator is told who can fix it, and the browser connects once it is there', async ({ page }) => {
  await asMember(page)
  const notice = await openUnavailable(page, { available: false })
  await expect(notice).toHaveAttribute('data-reason', 'no_package')
  await expect(page.getByTestId('ab-ask-admin')).toHaveText('Ask an administrator to enable ext-core in Settings › Extension tools.')
  await expect(page.getByTestId('ab-open-ext-tools')).toHaveCount(0)

  await setAvailability(page, { available: true, download: 'missing' })
  await page.getByTestId('ab-status-retry').click()
  await expect(notice).toHaveAttribute('data-reason', 'not_downloaded')
  await expect(page.getByTestId('ab-ask-admin')).toContainText('An administrator can download it in Settings › Extension tools.')
  await expect(page.getByTestId('ab-download')).toHaveCount(0)
  await expect(page.getByTestId('ab-download-browser')).toHaveCount(0)

  // An administrator downloads it elsewhere: the App notices on its own.
  await setAvailability(page, { download: 'ready' })
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open', { timeout: 10_000 })
})

test('without ext-core an administrator is taken to Extension tools', async ({ page }) => {
  const notice = await openUnavailable(page, { available: false })
  await expect(notice).toHaveAttribute('data-reason', 'no_package')
  await expect(notice).toContainText('does not have the “ext-core” extension tools')
  await expect(page.getByTestId('ab-unavailable-detail')).toHaveText('no `chromium` on this node (it comes with the ext-core extension tools)')
  await expect(page.getByTestId('ab-download')).toHaveCount(0)
  await page.getByTestId('ab-open-ext-tools').click()
  await expect(page.getByTestId('window-settings').getByTestId('ext-tools-page')).toBeVisible()
})

test('a browser its owner turned off says so, with nothing to download', async ({ page }) => {
  const notice = await openUnavailable(page, { enabled: false })
  await expect(notice).toHaveAttribute('data-reason', 'disabled')
  await expect(notice).toContainText('[loader.browser] enabled = false')
  await expect(page.getByTestId('ab-download')).toHaveCount(0)
  await expect(page.getByTestId('ab-open-ext-tools')).toHaveCount(0)
  await expect(page.getByTestId('ab-ask-admin')).toHaveCount(0)
})

test('reflow is read while the browser is not downloaded', async ({ page }) => {
  await openUnavailable(page, { download: 'missing' })
  await page.getByTestId('ab-mode-reflow').click()
  await expect(page.getByTestId('ab-reflow-blank')).toBeVisible()
  const input = page.getByTestId('ab-address-input')
  await input.click()
  await input.fill('news.example.com/story/1')
  await input.press('Enter')
  await expect(page.frameLocator('[data-testid="ab-reflow-frame"]').locator('#title')).toHaveText('Story /story/1')
  await page.getByTestId('ab-mode-page').click()
  await expect(page.getByTestId('ab-unavailable')).toHaveAttribute('data-reason', 'not_downloaded')
})

test.describe('on a phone', () => {
  test.use({ viewport: { width: 375, height: 812 }, hasTouch: true, isMobile: true })

  test('the not-downloaded screen fits, and the download is started with a tap', async ({ page }, testInfo) => {
    const notice = await openUnavailable(page, { download: 'missing' }, true)
    await expect(notice).toHaveAttribute('data-reason', 'not_downloaded')
    const button = page.getByTestId('ab-download-browser')
    await expect(button).toBeInViewport()
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375)
    const box = await notice.boundingBox()
    expect(box && box.x >= 0 && box.x + box.width <= 375).toBe(true)
    await page.screenshot({ path: testInfo.outputPath('not-downloaded.png') })
    await button.tap()
    await expect(page.getByTestId('ab-download-ood1')).toHaveAttribute('data-state', 'downloading')
    await page.screenshot({ path: testInfo.outputPath('downloading.png') })
    await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open', { timeout: 15_000 })
  })
})
