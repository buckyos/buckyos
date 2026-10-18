import { expect, test, type Page } from '@playwright/test'

declare global {
  interface Window { __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore }
}

const SELF = 'did:buckyos:user:self'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
const AGENT = 'did:buckyos:agent:codeassistant'
const BOB = 'did:buckyos:person:bob'
const NINA = 'did:bns:nina'

async function open(page: Page, entityId: string, sessionId?: string) {
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
  await page.goto(`/messagehub?entityId=${encodeURIComponent(entityId)}${sessionId ? `&sessionId=${sessionId}` : ''}`)
  await expect(page.getByTestId('message-composer')).toBeVisible()
}

async function setRequestCount(page: Page, entityId: string, count: number) {
  await page.evaluate(async ({ context, entityId, count }) => {
    const store = window.__messageHubMock
    const session = store.sessions(context, entityId)[0]
    const stored = Object.values(store.getSnapshot().sessions).find(item => item.ownerDid === context.ownerDid && item.id === session.id)!
    stored.requestCount = count
    await store.injectState(context.ownerDid, session.id, {})
  }, { context: OWN, entityId, count })
}

for (const width of [1440, 375]) {
  test(`request banner: friends never see it; others get add friend / allow messages / block at ${width}px`, async ({ page }) => {
    const consoleErrors: string[] = []
    page.on('console', message => { if (message.type() === 'error') consoleErrors.push(message.text()) })
    await page.setViewportSize({ width, height: 900 })
    await open(page, AGENT, 'session-coder-1')
    await setRequestCount(page, AGENT, 7)
    const banner = page.getByTestId('request-banner')
    await expect(page.getByTestId('conversation-history')).toBeVisible()
    await expect(banner).toHaveCount(0)

    await open(page, BOB)
    await setRequestCount(page, BOB, 3)
    await expect(banner).toContainText('3')
    await expect(banner.getByRole('button', { name: 'Add friend' })).toBeVisible()
    await expect(banner.getByRole('button', { name: 'Block' })).toBeVisible()
    await banner.getByRole('button', { name: 'Allow messages' }).click()
    await expect(banner).toHaveCount(0)
    expect(await page.evaluate(({ context }) => window.__messageHubMock.sessions(context, 'did:buckyos:person:bob')[0].requestCount, { context: OWN })).toBe(3)
    expect(consoleErrors).toEqual([])
  })
}

test('blocking from the banner asks first and leaves an unblock action', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await open(page, BOB)
  await setRequestCount(page, BOB, 2)
  const banner = page.getByTestId('request-banner')
  await banner.getByRole('button', { name: 'Block' }).click()
  const confirm = page.getByRole('dialog', { name: 'Block Bob Zhang?' })
  await expect(confirm.getByTestId('network-confirm')).toHaveAttribute('data-strength', 'explicit')
  await confirm.getByTestId('network-confirm-submit').click()
  await expect(banner.getByTestId('relation-blocked')).toBeVisible()
  await expect(banner.getByRole('button', { name: 'Unblock' })).toBeVisible()

  await page.getByRole('button', { name: 'Entity details: Bob Zhang' }).last().click()
  const relation = page.getByTestId('entity-relation')
  await expect(relation.getByTestId('relation-blocked')).toBeVisible()
  await relation.getByRole('button', { name: 'Unblock' }).click()
  await expect(relation.getByRole('button', { name: 'Add friend' })).toBeVisible()
  await expect(relation.getByTestId('entity-open-my-network')).toBeVisible()
})

test('an unknown DID opens a new conversation and its friend request card is answered in place', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await open(page, NINA)
  await expect(page.getByRole('button', { name: /Entity details: nina/i }).last()).toBeVisible()
  const sessionId = await page.evaluate(({ context, did }) => window.__messageHubMock.sessions(context, did)[0]?.id, { context: OWN, did: NINA })
  expect(sessionId).toBeTruthy()
  await page.evaluate(({ owner, id, from }) => window.__messageHubMock.injectMessage(owner, id, {
    from, to: [owner], kind: 'chat', created_at_ms: Date.now(), ui_message_id: 'msg-fr-nina', ui_sender_name: 'Nina',
    content: { format: 'text/plain', content: 'Hi Alice, we met at the BuckyOS meetup last week.', machine: { intent: 'buckyos.friend', data: { action: 'request', request_id: 'fr-nina', message: 'Hi Alice, we met at the BuckyOS meetup last week.', expires_at_ms: Date.now() + 86_400_000 } } },
  } as never), { owner: SELF, id: sessionId!, from: NINA })
  const card = page.getByTestId('friend-notice')
  await expect(card).toHaveAttribute('data-state', 'pending')
  await expect(card).toContainText('we met at the BuckyOS meetup')
  await card.getByRole('button', { name: 'Accept' }).click()
  await expect(card).toHaveAttribute('data-state', 'accepted')
  await expect(card.getByTestId('friend-notice-state')).toHaveText('You are friends')
})
