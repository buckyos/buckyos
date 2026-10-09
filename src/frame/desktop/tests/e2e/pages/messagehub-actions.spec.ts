import { expect, test, type Page } from '@playwright/test'

/**
 * Message actions as icons (reply / copy / forward / delete), media without a
 * bubble, and the floating session panel (pinned message, host-defined state).
 */

declare global {
  interface Window { __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore }
}

const SELF = 'did:buckyos:user:self'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
const BOB = 'did:buckyos:person:bob'
const ALICE = 'did:buckyos:person:alice'
const RELEASE_HUB = 'did:buckyos:entity:service-release-hub'

async function openHub(page: Page, entityId: string) {
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
  await page.goto(`/messagehub?entityId=${encodeURIComponent(entityId)}`)
  await expect(page.getByTestId('conversation-history')).toBeVisible()
}

const sessionTexts = (page: Page, entityId: string) => page.evaluate(async ({ context, entity }) => {
  const mock = window.__messageHubMock
  const session = mock.defaultSession(context, entity)
  return session ? (await mock.reader(context, session.id).readRange(0, 500)).map(message => message.content.content) : []
}, { context: OWN, entity: entityId })

test('hover bar offers reply, copy, forward and delete as icons', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  await openHub(page, BOB)
  const history = page.getByTestId('conversation-history')
  const bubble = history.locator('[data-testid="message-bubble"]', { hasText: 'Ah makes sense' })
  await bubble.hover()
  const bar = bubble.getByTestId('message-hover-bar')
  await expect(bar).toBeVisible()
  for (const name of ['Reply', 'Copy', 'Forward', 'Delete']) await expect(bar.getByRole('button', { name, exact: true })).toBeVisible()
  await page.screenshot({ path: 'test-results/messagehub-actions-hover.png' })

  await bar.getByTestId('message-copy').click()
  await expect(bar.getByTestId('message-copy')).toHaveAttribute('data-copied', 'true')
  expect(await page.evaluate(() => navigator.clipboard.readText())).toContain('Ah makes sense')

  await bar.getByTestId('message-reply').click()
  await expect(page.getByTestId('composer-relation')).toContainText('Replying to')
  await page.getByTestId('composer-relation').getByRole('button').click()

  await bubble.hover()
  await bar.getByTestId('message-forward').click()
  const dialog = page.getByTestId('forward-dialog')
  await expect(dialog.getByTestId('forward-preview')).toContainText('Ah makes sense')
  await dialog.getByPlaceholder('Search').fill('alice')
  await expect(dialog.getByTestId('forward-targets').getByRole('button')).toHaveCount(1)
  await dialog.getByRole('button', { name: /Alice/ }).click()
  await expect(dialog).toHaveCount(0)
  expect((await sessionTexts(page, ALICE)).some(text => text?.includes('Ah makes sense'))).toBe(true)

  // Delete asks the peer to delete too (a redact request); the placeholder can then be dropped locally.
  const before = await history.locator('[data-testid="message-bubble"]').count()
  await bubble.hover()
  await bar.getByTestId('message-delete').click()
  await expect(page.getByTestId('message-delete-menu').getByRole('menuitem')).toHaveCount(1)
  await page.getByTestId('message-delete-menu').getByRole('menuitem', { name: 'Delete for everyone' }).click()
  await expect(history.locator('[data-testid="message-bubble"]')).toHaveCount(before - 1)
  const placeholder = history.getByTestId('message-redacted').filter({ hasText: 'deleted by You' })
  await expect(placeholder).toBeVisible()
  const redact = await page.evaluate(({ owner }) => Object.values(window.__messageHubMock.getSnapshot().messages).flat().find(message => message.from === owner && message.relates_to?.rel === 'redact'), { owner: SELF })
  expect(redact).toBeTruthy()
  await placeholder.hover()
  await placeholder.getByTestId('message-delete').click()
  await page.getByTestId('message-delete-menu').getByRole('menuitem', { name: 'Delete for me' }).click()
  await expect(history.getByTestId('message-redacted')).toHaveCount(0)
})

test('pictures and videos render without a bubble; files keep theirs', async ({ page }) => {
  await openHub(page, ALICE)
  const history = page.getByTestId('conversation-history')
  const gif = history.getByTestId('attachment-image').filter({ has: page.getByRole('img', { name: 'orbit-loader.gif' }) })
  await expect(gif).toBeVisible({ timeout: 15_000 })
  const media = history.getByTestId('message-media').filter({ has: page.getByRole('img', { name: 'orbit-loader.gif' }) })
  await expect(media).toHaveCount(1)
  expect(await media.evaluate(element => getComputedStyle(element).backgroundColor)).toBe('rgba(0, 0, 0, 0)')
  await expect(history.getByTestId('message-media').filter({ has: page.getByTestId('attachment-video') }).first()).toBeVisible({ timeout: 15_000 })
  await expect(history.getByTestId('message-media').filter({ has: page.getByTestId('attachment-file') })).toHaveCount(0)
  await page.screenshot({ path: 'test-results/messagehub-media-frameless.png' })
})

test('a pinned message floats over the history in at most three lines', async ({ page }) => {
  await openHub(page, BOB)
  const history = page.getByTestId('conversation-history')
  await expect(page.getByTestId('session-panel')).toHaveCount(0)
  const long = Array.from({ length: 30 }, (_, index) => `Checklist item ${index + 1} for the release.`).join(' ')
  await page.locator('textarea').fill(long)
  await page.keyboard.press('Enter')
  const bubble = history.locator('[data-testid="message-bubble"]', { hasText: 'Checklist item 30' })
  await bubble.hover()
  await bubble.getByTestId('message-actions').click()
  await page.getByRole('menuitem', { name: 'Pin', exact: true }).click()

  const panel = page.getByTestId('session-panel')
  await expect(panel.getByTestId('session-panel-title')).toHaveText('Pinned · You')
  await expect(panel.getByTestId('session-panel-text')).toContainText('Checklist item 1 ')
  // Three 20px lines plus the panel's padding and border.
  expect((await panel.boundingBox())!.height).toBeLessThanOrEqual(3 * 20 + 18)
  await page.screenshot({ path: 'test-results/messagehub-panel-pinned.png' })
  await panel.getByTestId('session-panel-toggle').click()
  expect((await panel.boundingBox())!.height).toBeGreaterThan(3 * 20 + 18)
  await panel.getByTestId('session-panel-toggle').click()

  await page.reload()
  await expect(page.getByTestId('session-panel').getByTestId('session-panel-text')).toContainText('Checklist item 1 ')
  await page.getByTestId('session-panel-dismiss').click()
  await expect(page.getByTestId('session-panel')).toHaveCount(0)
})

test('a session can define its own panel content', async ({ page }) => {
  await openHub(page, RELEASE_HUB)
  const panel = page.getByTestId('session-panel')
  await expect(panel.getByTestId('session-panel-title')).toHaveText('Ticket #1042 · Release 2.4 rollout')
  await expect(panel.getByTestId('session-panel-fields')).toContainText('In review')
  expect((await panel.boundingBox())!.height).toBeLessThanOrEqual(3 * 20 + 18)
  await expect(panel.getByTestId('session-panel-dismiss')).toHaveCount(0)
  await panel.getByTestId('session-panel-toggle').click()
  await expect(panel.getByTestId('session-panel-text')).toContainText('sign-off from QA')
  await page.screenshot({ path: 'test-results/messagehub-panel-custom.png' })
})
