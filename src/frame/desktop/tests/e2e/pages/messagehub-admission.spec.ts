import { expect, test } from '@playwright/test'

declare global {
  interface Window { __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore }
}

const SELF = 'did:buckyos:user:self'
const AGENT = 'did:buckyos:agent:codeassistant'
const SESSION = 'session-coder-1'

for (const width of [1440, 375]) {
  test(`friend conversations hide historical request admission at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 })
    await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
    await page.goto(`/messagehub?entityId=${encodeURIComponent(AGENT)}&sessionId=${SESSION}`)
    await expect(page.getByTestId('message-composer')).toBeVisible()
    await page.evaluate(async ({ owner, sessionId }) => {
      const store = window.__messageHubMock
      const session = Object.values(store.getSnapshot().sessions).find(item => item.ownerDid === owner && item.id === sessionId)!
      session.requestCount = 7
      store.admission = () => ({ accessLevel: 'friend', canChange: true })
      await store.injectState(owner, sessionId, {})
    }, { owner: SELF, sessionId: SESSION })

    const banner = page.getByTestId('request-banner')
    await expect(banner).toHaveCount(0)
    await expect(page.getByTestId('conversation-history')).toBeVisible()
    await expect(page.getByTestId('message-composer')).toBeVisible()

    await page.evaluate(async ({ owner, sessionId }) => {
      const store = window.__messageHubMock
      store.admission = () => ({ accessLevel: 'stranger', canChange: true })
      store.setAdmission = async () => {
        store.admission = () => ({ accessLevel: 'friend', canChange: true })
        await store.injectState(owner, sessionId, {})
      }
      await store.injectState(owner, sessionId, {})
    }, { owner: SELF, sessionId: SESSION })
    await expect(banner).toContainText('7')
    await expect(banner.getByRole('button', { name: 'Block', exact: true })).toBeVisible()
    await banner.getByRole('button', { name: 'Accept contact', exact: true }).click()
    await expect(banner).toHaveCount(0)
    expect(await page.evaluate(({ owner, sessionId }) => Object.values(window.__messageHubMock.getSnapshot().sessions).find(item => item.ownerDid === owner && item.id === sessionId)?.requestCount, { owner: SELF, sessionId: SESSION })).toBe(7)

    await page.evaluate(async ({ owner, sessionId }) => {
      const store = window.__messageHubMock
      store.admission = () => ({ accessLevel: 'block', canChange: true })
      await store.injectState(owner, sessionId, {})
    }, { owner: SELF, sessionId: SESSION })
    await expect(banner).toContainText('Blocked')
    await expect(banner.getByRole('button', { name: 'Accept contact', exact: true })).toBeVisible()
    await expect(banner.getByRole('button', { name: 'Block', exact: true })).toHaveCount(0)
  })
}
