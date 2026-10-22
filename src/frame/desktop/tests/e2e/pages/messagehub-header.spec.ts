import { expect, test, type Page } from '@playwright/test'

/**
 * The entity list's title bar: edit on the left, the signed-in user with the
 * unread total (or the sync status) in the middle, + and ⋮ menus on the right.
 */

declare global {
  interface Window { __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore }
}

const SELF = 'did:buckyos:user:self'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
const CODER = 'did:buckyos:agent:codeassistant'
const ALICE = 'did:buckyos:person:alice'

async function openHub(page: Page, phone = false) {
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
  await page.goto('/messagehub')
  // On a phone the most recent conversation opens first.
  if (phone) await page.getByRole('button', { name: 'Back to conversation list', exact: true }).click()
  await expect(page.getByTestId('hub-create-menu')).toBeVisible()
}

const unreadTotal = (page: Page) => page.evaluate(context => window.__messageHubMock.entities(context).reduce((total, entity) => total + (entity.isMuted ? 0 : entity.unreadCount), 0), OWN)

test('the title shows the signed-in user and the unread total; a long name is cut with an ellipsis', async ({ page }) => {
  const longName = 'Alexandria Montgomery-Wellington of the Northern Reaches'
  await page.addInitScript(name => {
    window.localStorage.setItem('buckyos.mock.control-panel.v1', JSON.stringify({ account: { user_id: 'alice', user_name: name, user_type: 'admin', did: 'did:bns:alice' } }))
  }, longName)
  await page.setViewportSize({ width: 375, height: 760 })
  await openHub(page, true)
  const name = page.getByTestId('hub-identity-name')
  await expect(name).toHaveText(longName)
  await expect(name).toHaveAttribute('title', longName)
  expect(await name.evaluate(element => getComputedStyle(element).textOverflow)).toBe('ellipsis')
  expect(await name.evaluate(element => element.scrollWidth > element.clientWidth)).toBe(true)
  await expect(page.getByTestId('hub-unread-total')).toHaveText(String(await unreadTotal(page)))
  for (const id of ['entity-list-edit', 'hub-identity', 'hub-create-menu', 'hub-more-menu']) {
    const box = await page.getByTestId(id).boundingBox()
    expect(box && box.x >= 0 && box.x + box.width <= 375).toBe(true)
  }
  await page.screenshot({ path: 'test-results/messagehub-header-375.png' })
})

test('the sync status takes the identity\'s place while offline or reconnecting', async ({ page, context }) => {
  await openHub(page)
  const status = page.getByTestId('hub-sync-status')
  await expect(status).toHaveCount(0)
  await context.setOffline(true)
  await expect(status).toHaveText('Waiting for network…')
  await expect(page.getByTestId('hub-identity-name')).toHaveCount(0)
  await expect(page.getByRole('status').filter({ hasText: 'Waiting for network…' })).toHaveCount(1)
  await context.setOffline(false)
  await expect(status).toHaveCount(0)
  await page.evaluate(() => window.__messageHubMock.configure({ sync: 'connecting' }))
  await expect(status).toHaveText('Connecting…')
  await page.evaluate(() => window.__messageHubMock.configure({ sync: 'online' }))
  await expect(page.getByTestId('hub-identity-name')).toBeVisible()
})

test('+ holds the new entries and ⋮ the rest; Escape closes a menu back to its button', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await openHub(page)
  await page.getByTestId('hub-create-menu').click()
  const create = page.getByRole('menu', { name: 'New' })
  await expect(create.getByRole('menuitem')).toHaveText(['New Session', 'New group', 'Join group'])
  await page.keyboard.press('Escape')
  await expect(create).toHaveCount(0)
  await expect(page.getByTestId('hub-create-menu')).toBeFocused()
  await page.getByTestId('hub-more-menu').click()
  const more = page.getByRole('menu', { name: 'More' })
  await expect(more.getByRole('menuitem')).toHaveText(['Mark all as read', 'Back to desktop', 'Media settings', 'Collapse list'])
  await page.screenshot({ path: 'test-results/messagehub-mark-all-read-menu.png', clip: { x: 0, y: 0, width: 420, height: 300 } })
  await more.getByRole('menuitem', { name: 'Collapse list' }).click()
  await expect(page.getByTestId('entity-list-header')).toHaveCount(0)
  await page.getByRole('button', { name: 'Expand list' }).click()
  await expect(page.getByTestId('entity-list-header')).toBeVisible()
  await page.getByTestId('hub-create-menu').click()
  await page.getByRole('menuitem', { name: 'New Session', exact: true }).click()
  await expect(page.getByRole('dialog')).toBeVisible()
})

test('mark all as read clears every badge and persists, including muted and archived chats; new messages become unread', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await openHub(page)
  await page.getByRole('button', { name: /^CodeAssistant/ }).click()
  const before = await page.evaluate(async ({ context, agent, alice }) => {
    const store = window.__messageHubMock
    await store.manage(context, 'session-coder-ask', 'archive')
    for (const session of store.sessions(context, alice)) await store.updatePreferences(context, session.id, { muted: true })
    return {
      sessions: store.sessions(context).map(session => ({ id: session.id, lifecycle: session.lifecycle, unreadCount: session.unreadCount })).sort((a, b) => a.id.localeCompare(b.id)),
      coderUnread: store.entities(context).find(entity => entity.id === agent)!.unreadCount,
      mutedUnread: store.entities(context).find(entity => entity.id === alice)!.unreadCount,
      observed: store.sessions({ ...context, ownerDid: agent, mode: 'observe' }).map(session => session.unreadCount),
    }
  }, { context: OWN, agent: CODER, alice: ALICE })
  expect(before.mutedUnread).toBeGreaterThan(0)
  const cleared = before.sessions.map(session => ({ ...session, unreadCount: 0 }))
  const sessions = () => page.evaluate(context => window.__messageHubMock.sessions(context).map(session => ({ id: session.id, lifecycle: session.lifecycle, unreadCount: session.unreadCount })).sort((a, b) => a.id.localeCompare(b.id)), OWN)
  const coder = page.getByRole('button', { name: /^CodeAssistant/ })
  await expect(coder.getByText(String(before.coderUnread), { exact: true })).toBeVisible()
  await expect(page.getByTestId('hub-unread-total')).toBeVisible()
  await expect(page.getByTestId('other-sessions-unread')).toBeVisible()
  await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  await expect(page.getByTestId('archived-unread')).toHaveText('1')
  const currentSession = page.locator('[data-session-id="session-coder-1"]')
  await expect(currentSession.getByText('3', { exact: true })).toBeVisible()

  await page.getByTestId('hub-more-menu').click()
  await page.getByTestId('mark-all-read').click()
  await expect(page.getByTestId('hub-unread-total')).toHaveCount(0)
  await expect(page.getByTestId('other-sessions-unread')).toHaveCount(0)
  await expect(page.getByTestId('archived-unread')).toHaveCount(0)
  await expect(coder.getByText(/^\d+$/)).toHaveCount(0)
  await expect(currentSession.getByText(/^\d+$/)).toHaveCount(0)
  await expect.poll(sessions).toEqual(cleared)
  expect(await page.evaluate(({ context, agent }) => window.__messageHubMock.sessions({ ...context, ownerDid: agent, mode: 'observe' }).map(session => session.unreadCount), { context: OWN, agent: CODER })).toEqual(before.observed)
  await page.getByTestId('hub-more-menu').click()
  await expect(page.getByTestId('mark-all-read')).toBeDisabled()

  await page.reload()
  await expect(page.getByTestId('hub-identity-name')).toBeVisible()
  await expect(page.getByTestId('hub-unread-total')).toHaveCount(0)
  await expect.poll(sessions).toEqual(cleared)
  await page.evaluate(() => window.__messageHubMock.simulateIncoming('session-coder-2', { content: { format: 'text/plain', content: 'New message after clearing unread' } }))
  await expect(page.getByTestId('hub-unread-total')).toHaveText('1')
  await expect(coder.getByText('1', { exact: true })).toBeVisible()
  await page.getByTestId('hub-more-menu').click()
  await expect(page.getByTestId('mark-all-read')).toBeEnabled()
})

test('mark all as read prevents duplicate requests and keeps unread counts after a failed attempt', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await openHub(page)
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  const before = await unreadTotal(page)
  await page.evaluate(() => window.__messageHubMock.configure({ failNext: 'mock_failure', delayMs: 1200 }))
  await page.getByTestId('hub-more-menu').click()
  await page.getByTestId('mark-all-read').click()
  await page.locator('[data-session-id="session-coder-2"] button').first().click()
  await page.getByTestId('hub-more-menu').click()
  await expect(page.getByTestId('mark-all-read')).toBeDisabled()
  await page.keyboard.press('Escape')
  const error = page.getByRole('dialog')
  await expect(error).toContainText('Some messages could not be marked as read. Please try again.')
  await expect(page.getByTestId('hub-unread-total')).toHaveText(String(before))
  expect(await unreadTotal(page)).toBe(before)
  await error.getByRole('button', { name: 'Close', exact: true }).click()
  await page.evaluate(() => window.__messageHubMock.configure({ delayMs: 0 }))
  await page.getByTestId('hub-more-menu').click()
  await expect(page.getByTestId('mark-all-read')).toBeEnabled()
  await page.getByTestId('mark-all-read').click()
  await expect(page.getByTestId('hub-unread-total')).toHaveCount(0)
})

test('edit mode pins and mutes the selected chats; chats without a session cannot be picked', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await openHub(page)
  const picks = await page.evaluate(context => {
    const store = window.__messageHubMock
    const entities = store.entities(context)
    return {
      names: entities.filter(entity => !entity.isPinned && !entity.isMuted && store.sessions(context, entity.id).length > 0).slice(0, 2).map(entity => entity.name),
      empty: entities.find(entity => store.sessions(context, entity.id).length === 0)?.name,
    }
  }, OWN)
  expect(picks.names).toHaveLength(2)
  const state = () => page.evaluate(({ context, names }) => {
    const store = window.__messageHubMock
    return store.entities(context).filter(entity => names.includes(entity.name)).map(entity => ({ pinned: !!entity.isPinned, muted: !!entity.isMuted }))
  }, { context: OWN, names: picks.names })
  const edit = page.getByTestId('entity-list-edit')
  const row = (name: string) => page.getByTestId('entity-select-row').filter({ has: page.getByText(name, { exact: true }) })

  await edit.click()
  await expect(edit).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByTestId('hub-identity')).toContainText('Select chats')
  if (picks.empty) await expect(row(picks.empty)).toBeDisabled()
  const actions = page.getByTestId('entity-edit-actions')
  await expect(actions.getByRole('button', { name: 'Pin' })).toBeDisabled()
  for (const name of picks.names) await row(name).click()
  await expect(row(picks.names[0])).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByTestId('hub-identity')).toContainText('2 selected')
  await page.screenshot({ path: 'test-results/messagehub-header-edit.png' })
  await actions.getByRole('button', { name: 'Pin' }).click()
  await expect(edit).toHaveAttribute('aria-pressed', 'false')
  await expect.poll(state).toEqual([{ pinned: true, muted: false }, { pinned: true, muted: false }])

  await edit.click()
  for (const name of picks.names) await row(name).click()
  await expect(actions.getByRole('button', { name: 'Unpin' })).toBeVisible()
  await actions.getByRole('button', { name: 'Mute' }).click()
  await expect.poll(state).toEqual([{ pinned: true, muted: true }, { pinned: true, muted: true }])
  await expect(page.getByTestId('hub-unread-total')).toHaveText(String(await unreadTotal(page)))

  await edit.click()
  await row(picks.names[0]).click()
  await page.keyboard.press('Escape')
  await expect(edit).toHaveAttribute('aria-pressed', 'false')
  await expect(page.getByTestId('entity-select-row')).toHaveCount(0)
})

test('an observer sees the observed owner and can neither edit nor create', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto(`/messagehub?entityId=${encodeURIComponent(ALICE)}&ownerDid=${encodeURIComponent(CODER)}&mode=observe`)
  await expect(page.getByTestId('owner-banner')).toBeVisible()
  await expect(page.getByTestId('hub-identity-name')).toHaveText('CodeAssistant')
  await expect(page.getByTestId('entity-list-edit')).toBeDisabled()
  await expect(page.getByTestId('hub-create-menu')).toBeDisabled()
  await expect(page.getByTestId('hub-more-menu')).toBeEnabled()
  await page.getByTestId('hub-more-menu').click()
  await expect(page.getByTestId('mark-all-read')).toHaveCount(0)
  const unread = await page.getByTestId('hub-unread-total').innerText()
  expect(await page.evaluate(async ({ context, agent }) => {
    try { await window.__messageHubMock.markAllRead({ ...context, ownerDid: agent, mode: 'observe' }); return false } catch { return true }
  }, { context: OWN, agent: CODER })).toBe(true)
  await expect(page.getByTestId('hub-unread-total')).toHaveText(unread)
})

for (const theme of ['light', 'dark'] as const) {
  test(`the title bar, its menu and edit mode in the ${theme} theme`, async ({ page }) => {
    await page.addInitScript(value => window.localStorage.setItem('buckyos.prototype.theme.v1', value), theme)
    await page.setViewportSize({ width: 1400, height: 900 })
    await openHub(page)
    await page.getByTestId('hub-create-menu').click()
    await page.getByRole('menuitem', { name: 'New group' }).hover()
    await page.screenshot({ path: `test-results/messagehub-header-menu-${theme}.png`, clip: { x: 0, y: 0, width: 420, height: 260 } })
    await page.keyboard.press('Escape')
    await page.getByTestId('entity-list-edit').click()
    await page.getByTestId('entity-select-row').first().click()
    await page.screenshot({ path: `test-results/messagehub-header-edit-${theme}.png`, clip: { x: 0, y: 0, width: 420, height: 900 } })
  })
}
