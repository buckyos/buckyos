import { expect, test, type Page } from '@playwright/test'
import { closeSessionList, openNewSession, openSessionDetails } from '../fixtures/messagehub'

/**
 * Group sender pictures and profiles, the conversation header (search and the
 * ⋮ session menu), the list's "New chat" and the session list's "New Session".
 */

declare global {
  interface Window { __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore }
}

const SELF = 'did:buckyos:user:self'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
const CODER = 'did:buckyos:agent:codeassistant'
const ALICE = 'did:buckyos:person:alice'
const BOB = 'did:buckyos:person:bob'
const TEAM = 'did:buckyos:group:product-team'
const TEAM_SESSION = 'session-team-1'

async function openHub(page: Page, entityId: string) {
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
  await page.goto(`/messagehub?entityId=${encodeURIComponent(entityId)}`)
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
}

const row = (page: Page, text: string) => page.locator('.mh-row', { hasText: text })

test('a group conversation shows each sender\'s picture at the start of their run; a direct one shows none', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 })
  await openHub(page, TEAM)
  const history = page.getByTestId('conversation-history')
  await expect(row(page, 'Sprint review at 3pm today').getByTestId('message-avatar')).toHaveAttribute('data-did', BOB)
  await expect(row(page, 'Shared the updated design tokens').getByTestId('message-avatar')).toHaveAttribute('data-did', 'did:buckyos:person:carol')
  // The viewer's own bubbles stay on the right without a picture.
  await expect(row(page, 'review the onboarding flow after lunch').getByTestId('message-avatar-slot')).toHaveCount(0)
  // The picture sits left of the bubble, and the bubble keeps its width limit.
  const avatar = await row(page, 'Sprint review at 3pm today').getByTestId('message-avatar').boundingBox()
  const bubble = await row(page, 'Sprint review at 3pm today').getByTestId('message-bubble').boundingBox()
  expect(avatar!.x + avatar!.width).toBeLessThanOrEqual(bubble!.x)
  expect(Math.round(avatar!.width)).toBe(32)

  // A second message of the same sender continues the run: its slot is empty, the bubbles stay aligned.
  for (const text of ['One more thing: bring the demo laptop.', 'And the spare charger.']) {
    await page.evaluate(({ session, bob, team, text }) => window.__messageHubMock.simulateIncoming(session, { from: bob, to: [team], kind: 'group_msg', ui_sender_name: 'Bob', content: { content: text } }), { session: TEAM_SESSION, bob: BOB, team: TEAM, text })
  }
  const started = row(page, 'bring the demo laptop'), continued = row(page, 'And the spare charger')
  await expect(started.getByTestId('message-avatar')).toHaveAttribute('data-did', BOB)
  await expect(continued.getByTestId('message-avatar-slot')).toHaveCount(1)
  await expect(continued.getByTestId('message-avatar')).toHaveCount(0)
  expect(Math.round((await continued.getByTestId('message-bubble').boundingBox())!.x)).toBe(Math.round((await started.getByTestId('message-bubble').boundingBox())!.x))
  await page.screenshot({ path: 'test-results/messagehub-group-avatars.png' })

  await openHub(page, ALICE)
  await expect(history.getByTestId('message-bubble').first()).toBeVisible()
  await expect(history.getByTestId('message-avatar-slot')).toHaveCount(0)
})

test('desktop: a sender\'s picture opens the entity info dialog, whose Send message opens the conversation with them', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 })
  await openHub(page, TEAM)
  const avatar = row(page, 'Sprint review at 3pm today').getByTestId('message-avatar')
  await expect(avatar).toHaveAccessibleName('Profile of Bob')
  await avatar.click()
  const dialog = page.getByRole('dialog', { name: 'Profile' })
  const info = dialog.getByTestId('entity-info')
  await expect(info).toHaveAttribute('data-did', BOB)
  await expect(info).toContainText('Bob Zhang')
  await expect(info).toContainText(BOB)
  await expect(info.getByTestId('entity-info-relationship')).toBeVisible()
  await page.screenshot({ path: 'test-results/messagehub-profile-dialog.png' })
  await page.keyboard.press('Escape')
  await dialog.getByRole('button', { name: 'Close dialog' }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Entity details: Product Team' })).toBeVisible()

  await avatar.click()
  await dialog.getByTestId('entity-info-message').click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Entity details: Bob Zhang' })).toBeVisible()
  await expect(page.locator('textarea')).toBeVisible()
})

test('phone: a sender\'s picture opens their profile as a details page', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 780 })
  await openHub(page, TEAM)
  await row(page, 'Sprint review at 3pm today').getByTestId('message-avatar').click()
  const profile = page.getByTestId('profile-details')
  await expect(profile).toBeVisible()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(profile.getByTestId('entity-info')).toHaveAttribute('data-did', BOB)
  const box = await profile.boundingBox()
  expect(Math.round(box!.width)).toBe(390)
  await page.screenshot({ path: 'test-results/messagehub-profile-phone.png' })
  await profile.getByRole('button', { name: 'Close', exact: true }).click()
  await expect(profile).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Entity details: Product Team' })).toBeVisible()

  await row(page, 'Sprint review at 3pm today').getByTestId('message-avatar').click()
  await profile.getByTestId('entity-info-message').click()
  await expect(profile).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Entity details: Bob Zhang' })).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
})

for (const width of [1280, 390]) {
  test(`the header has search and a ⋮ menu instead of New Session; the menu edits, mutes and clears at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 860 })
    await openHub(page, ALICE)
    const sessionId = await page.evaluate(({ context, alice }) => window.__messageHubMock.defaultSession(context, alice)!.id, { context: OWN, alice: ALICE })
    await expect(page.getByRole('button', { name: 'New Session', exact: true })).toHaveCount(0)
    await expect(page.getByRole('button', { name: 'Search messages', exact: true })).toBeVisible()

    await page.getByRole('button', { name: 'Session menu', exact: true }).click()
    const menu = page.getByRole('menu', { name: 'Session menu' })
    await expect(menu.getByRole('menuitem')).toHaveText(['Edit info', 'Mute', 'Clear history'])
    await page.screenshot({ path: `test-results/messagehub-session-menu-${width}.png` })
    await page.keyboard.press('Escape')
    await expect(menu).toHaveCount(0)
    await expect(page.getByRole('button', { name: 'Session menu', exact: true })).toBeFocused()

    const details = await openSessionDetails(page)
    await expect(details.getByLabel('Personal display title')).toBeVisible()
    await details.getByRole('button', { name: 'Close', exact: true }).click()

    await page.getByTestId('session-menu').click()
    await page.getByTestId('session-menu-mute').click()
    await expect(page.getByTestId('session-muted').filter({ visible: true })).toHaveCount(1)
    expect(await page.evaluate(({ context, id }) => window.__messageHubMock.preferences(context, id).muted, { context: OWN, id: sessionId })).toBe(true)
    await page.getByTestId('session-menu').click()
    await expect(page.getByTestId('session-menu-mute')).toHaveText('Unmute')
    await page.getByTestId('session-menu-mute').click()
    await expect(page.getByTestId('session-muted')).toHaveCount(0)
    expect(await page.evaluate(({ context, id }) => window.__messageHubMock.preferences(context, id).muted, { context: OWN, id: sessionId })).toBe(false)

    const history = page.getByTestId('conversation-history')
    const before = Number(await history.getAttribute('data-raw-count'))
    expect(before).toBeGreaterThan(0)
    await page.getByTestId('session-menu').click()
    await page.getByTestId('session-menu-clear-history').click()
    const confirm = page.getByRole('dialog', { name: 'Clear history' })
    await expect(confirm).toContainText('cannot be restored')
    await page.screenshot({ path: `test-results/messagehub-clear-history-${width}.png` })
    await confirm.getByRole('button', { name: 'Cancel' }).click()
    await expect(history).toHaveAttribute('data-raw-count', String(before))

    await page.getByTestId('session-menu').click()
    await page.getByTestId('session-menu-clear-history').click()
    await confirm.getByRole('button', { name: 'Clear history', exact: true }).click()
    await expect(confirm).toHaveCount(0)
    await expect(history).toHaveAttribute('data-raw-count', '0')
    // The session and its entity stay; only the history is gone, and the entry has nothing left to clear.
    await expect(page.getByRole('button', { name: 'Entity details: Alice Chen' })).toBeVisible()
    expect(await page.evaluate(({ context, id }) => window.__messageHubMock.sessions(context).some(session => session.id === id), { context: OWN, id: sessionId })).toBe(true)
    await page.getByTestId('session-menu').click()
    await expect(page.getByTestId('session-menu-clear-history')).toBeDisabled()
    await page.keyboard.press('Escape')

    await page.locator('textarea').fill('A fresh start')
    await page.locator('textarea').press('Enter')
    await expect(history).toHaveAttribute('data-raw-count', '1')
    await expect(history.getByTestId('delivery-status')).toHaveAttribute('data-status', 'sent')
    await page.reload()
    await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
    await expect(history).toHaveAttribute('data-raw-count', '1')
    await expect(history.getByText('A fresh start', { exact: true })).toBeVisible()
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  })
}

test('an observer sees the menu but cannot mute or clear; a notification session has no search and no clear', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 860 })
  await openHub(page, CODER)
  await page.goto(`/messagehub?entityId=${encodeURIComponent(ALICE)}&ownerDid=${encodeURIComponent(CODER)}&mode=observe`)
  await expect(page.getByTestId('owner-banner')).toContainText('Read only')
  await page.getByTestId('session-menu').click()
  await expect(page.getByRole('menu', { name: 'Session menu' }).getByRole('menuitem')).toHaveText(['Edit info'])
  await page.keyboard.press('Escape')

  await page.goto('/messagehub')
  await page.getByRole('button', { name: /^System notifications/ }).click()
  await expect(page.getByTestId('notify-filter')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Search messages', exact: true })).toHaveCount(0)
  await page.getByTestId('session-menu').click()
  await expect(page.getByRole('menu', { name: 'Session menu' }).getByRole('menuitem')).toHaveText(['Edit info', 'Mute'])
})

for (const width of [1280, 390]) {
  test(`search steps through the matches of the session from the newest and locates each at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 860 })
    await openHub(page, TEAM)
    await page.getByRole('button', { name: 'Search messages', exact: true }).click()
    const bar = page.getByTestId('conversation-search')
    const input = bar.getByRole('searchbox', { name: 'Search in this session' })
    await expect(input).toBeFocused()
    // The header gives way to the search bar.
    await expect(page.getByRole('button', { name: 'Session menu', exact: true })).toHaveCount(0)
    await expect(bar.getByTestId('conversation-search-older')).toBeDisabled()

    await input.fill('ONBOARDING')
    await expect(bar.getByTestId('conversation-search-status')).toHaveText('1 of 2')
    await expect(page.locator('.mh-row[data-highlight="true"]')).toContainText('review the onboarding flow after lunch')
    await expect(bar.getByTestId('conversation-search-newer')).toBeDisabled()
    await page.screenshot({ path: `test-results/messagehub-search-${width}.png` })

    await input.press('Enter')
    await expect(bar.getByTestId('conversation-search-status')).toHaveText('2 of 2')
    await expect(page.locator('.mh-row[data-highlight="true"]')).toContainText('pushed the new onboarding flow to staging')
    await expect(row(page, 'pushed the new onboarding flow to staging')).toBeInViewport()
    await expect(bar.getByTestId('conversation-search-older')).toBeDisabled()

    await bar.getByTestId('conversation-search-newer').click()
    await expect(bar.getByTestId('conversation-search-status')).toHaveText('1 of 2')
    await input.press('Enter')
    await input.press('Shift+Enter')
    await expect(bar.getByTestId('conversation-search-status')).toHaveText('1 of 2')

    await input.fill('onboarding staging')
    await expect(bar.getByTestId('conversation-search-status')).toHaveText('1 of 1')
    await input.fill('no such words')
    await expect(bar.getByTestId('conversation-search-status')).toHaveText('No results')
    await expect(bar.getByTestId('conversation-search-older')).toBeDisabled()

    // A message that arrives while searching joins the matches without moving the current one.
    await input.fill('onboarding')
    await expect(bar.getByTestId('conversation-search-status')).toHaveText('1 of 2')
    await input.press('Enter')
    await expect(bar.getByTestId('conversation-search-status')).toHaveText('2 of 2')
    await page.evaluate(({ session, bob, team }) => window.__messageHubMock.simulateIncoming(session, { from: bob, to: [team], kind: 'group_msg', ui_sender_name: 'Bob', content: { content: 'The onboarding copy is final.' } }), { session: TEAM_SESSION, bob: BOB, team: TEAM })
    await expect(bar.getByTestId('conversation-search-status')).toHaveText('3 of 3')

    await input.press('Escape')
    await expect(bar).toHaveCount(0)
    await expect(page.getByRole('button', { name: 'Search messages', exact: true })).toBeFocused()
    await expect(page.getByRole('button', { name: 'Session menu', exact: true })).toBeVisible()
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  })
}

test('New chat filters entities fuzzily as you type, keeps one selected and opens it on its latest session', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 860 })
  await openHub(page, ALICE)
  // The Agent's other session becomes its most recent one.
  await page.evaluate(({ agent, self }) => window.__messageHubMock.simulateIncoming('session-coder-2', { from: agent, to: [self], kind: 'chat', content: { content: 'Latest news in the second session' } }), { agent: CODER, self: SELF })
  await page.getByTestId('hub-create-menu').click()
  await page.getByRole('menuitem', { name: 'New chat', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'New chat' })
  const search = dialog.getByRole('combobox', { name: 'Search by name or DID' })
  await expect(search).toBeFocused()
  // No name, description or connection here: only the search and the entities.
  await expect(dialog.getByRole('textbox')).toHaveCount(0)
  await expect(dialog.locator('select')).toHaveCount(0)
  const options = dialog.getByRole('option')
  await expect(options.first()).toHaveAttribute('aria-selected', 'true')
  await expect(dialog.getByRole('option', { selected: true })).toHaveCount(1)
  // Each row: a picture and two lines.
  await expect(options.first().getByTestId('hub-avatar')).toBeVisible()
  expect(await options.first().locator('span.block').count()).toBe(2)

  await search.fill('cdast')
  await expect(dialog.getByRole('option', { selected: true })).toContainText('CodeAssistant')
  await search.fill('a')
  const first = await dialog.getByRole('option', { selected: true }).getAttribute('data-entity-id')
  await search.press('ArrowDown')
  const second = await dialog.getByRole('option', { selected: true }).getAttribute('data-entity-id')
  expect(second).not.toBe(first)
  expect(await options.nth(1).getAttribute('aria-selected')).toBe('true')
  await search.press('ArrowUp')
  expect(await dialog.getByRole('option', { selected: true }).getAttribute('data-entity-id')).toBe(first)
  await page.screenshot({ path: 'test-results/messagehub-new-chat.png' })

  await search.fill('zzzz-nobody')
  await expect(dialog.getByTestId('new-chat-empty')).toBeVisible()
  await expect(dialog.getByRole('button', { name: 'Open', exact: true })).toBeDisabled()

  await search.fill('codeass')
  await expect(dialog.getByRole('option', { selected: true })).toContainText('CodeAssistant')
  await search.press('Enter')
  await expect(dialog).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Entity details: CodeAssistant' })).toBeVisible()
  await expect(page.getByTestId('conversation-history').getByText('Latest news in the second session', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  await expect(page.locator('[data-session-id="session-coder-2"] button').first()).toHaveAttribute('aria-current', 'true')
})

test('phone: New chat picks by tap and opens the conversation', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 780 })
  await openHub(page, ALICE)
  await page.getByRole('button', { name: 'Back to conversation list', exact: true }).click()
  await page.getByTestId('hub-create-menu').click()
  await page.getByRole('menuitem', { name: 'New chat', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'New chat' })
  await dialog.getByTestId('new-chat-search').fill('team')
  await dialog.getByRole('option').filter({ hasText: 'Product Team' }).click()
  await expect(dialog.getByRole('option', { selected: true })).toContainText('Product Team')
  await page.screenshot({ path: 'test-results/messagehub-new-chat-phone.png' })
  await dialog.getByRole('button', { name: 'Open', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Entity details: Product Team' })).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
})

test('New Session in the session list asks for a name, an optional description and the connection', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 860 })
  await openHub(page, CODER)
  await openNewSession(page)
  const dialog = page.getByRole('dialog', { name: 'New Session: CodeAssistant' })
  await expect(dialog.getByLabel('Session name', { exact: true })).toBeFocused()
  // The entity is the one whose session list this is: nothing to pick.
  await expect(dialog.getByRole('combobox')).toHaveCount(1)
  const connection = dialog.getByRole('combobox', { name: 'Connection', exact: true })
  await expect(connection).toHaveValue('native')
  await expect(connection.locator('option:checked')).toHaveText('Named Message Protocol')
  await page.screenshot({ path: 'test-results/messagehub-new-session.png' })

  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(dialog.getByRole('alert')).toHaveText('Enter a name of up to 64 characters.')
  await dialog.getByLabel('Session name', { exact: true }).fill('Release plan')
  await dialog.getByLabel('Description (optional)', { exact: true }).fill('Everything for the 0.7 release')
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '0')

  const created = await page.evaluate(({ context, agent }) => {
    const session = window.__messageHubMock.sessions(context, agent).find(item => item.shared.title === 'Release plan')!
    return { id: session.id, description: session.shared.description, kind: session.binding.kind }
  }, { context: OWN, agent: CODER })
  expect(created).toMatchObject({ description: 'Everything for the 0.7 release', kind: 'native' })
  const listed = page.locator(`[data-session-id="${created.id}"]`)
  await expect(listed.getByRole('button').first()).toHaveAttribute('aria-current', 'true')
  await expect(listed.getByTestId('session-description')).toHaveText('Everything for the 0.7 release')
  await closeSessionList(page)

  const details = await openSessionDetails(page)
  await expect(details.getByRole('heading', { name: 'Release plan', exact: true })).toBeVisible()
  await expect(details.getByTestId('session-description')).toHaveText('Everything for the 0.7 release')
  // The owner's own description takes over where it is set.
  await details.getByLabel('Personal description').fill('My notes for the release')
  await details.locator('form').filter({ has: page.getByLabel('Personal description') }).getByRole('button', { name: 'Save', exact: true }).click()
  await expect(details.getByTestId('session-description')).toHaveText('My notes for the release')
})
