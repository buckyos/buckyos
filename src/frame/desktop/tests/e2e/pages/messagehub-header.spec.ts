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
  await expect(more.getByRole('menuitem')).toHaveText(['Back to desktop', 'Media settings', 'Collapse list'])
  await more.getByRole('menuitem', { name: 'Collapse list' }).click()
  await expect(page.getByTestId('entity-list-header')).toHaveCount(0)
  await page.getByRole('button', { name: 'Expand list' }).click()
  await expect(page.getByTestId('entity-list-header')).toBeVisible()
  await page.getByTestId('hub-create-menu').click()
  await page.getByRole('menuitem', { name: 'New Session', exact: true }).click()
  await expect(page.getByRole('dialog')).toBeVisible()
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
