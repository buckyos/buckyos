import { expect, test, type Page } from '@playwright/test'

/**
 * Self-host Group application extensions in Message Hub (EXT-02..06): the
 * embedded panels of a group / Session, the origin check against the group's
 * host Zone, the display configuration, the Hook list and a Hook refusal, on
 * the mock store (`mock/store.ts`, Product Team) and the mock guest host.
 */

const TEAM = 'did:buckyos:group:product-team'
const PANEL_PAGE = '/messagehub/mock-panel.html'
const GROUP = 'did:buckyos:group:acme-support'
const CUSTOMER = 'did:bns:acme-c1'
const CP_KEY = 'buckyos.mock.control-panel.v1'
const HOST_KEY = 'buckyos.mock.messagehub.guest.v1'

async function openTeam(page: Page) {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(TEAM)}`)
  await expect(page.getByTestId('hub-create-menu')).toBeVisible()
}

async function openDesignReviewDetails(page: Page) {
  await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  await page.locator('[data-testid="session-row"]', { hasText: 'Design review' }).getByRole('button', { name: /Design review/ }).first().click()
  await page.getByRole('button', { name: 'Session details' }).last().click()
  return page.getByTestId('session-details')
}

test('group details show display, hooks and the default session panel with the session context in its URL', async ({ page }) => {
  await openTeam(page)
  await page.getByRole('button', { name: 'Entity details: Product Team' }).last().click()
  const extensions = page.getByTestId('group-panel').getByTestId('group-extensions')
  await expect(extensions.getByTestId('display-announcement')).toContainText('Welcome to the product team.')
  await expect(extensions.getByTestId('display-links').getByRole('link', { name: 'Roadmap' })).toHaveAttribute('href', 'https://roadmap.zone.example/')
  await expect(extensions.getByTestId('display-files')).toContainText('Product brief')
  await expect(extensions.getByTestId('display-featured')).toContainText('Alice Chen')
  const hooks = extensions.getByTestId('group-hooks')
  await expect(hooks).toContainText('This group has 2 business hook(s)')
  await expect(hooks.locator('li[data-point="join"]')).toContainText('crm.zone.example')
  await expect(hooks.locator('li[data-point="post"]')).toContainText('may grant')
  await expect(hooks.getByTestId('group-hooks-post-note')).toBeVisible()

  const panels = extensions.getByTestId('group-panels')
  await expect(panels.getByTestId('group-panels-toggle')).toContainText('Panels · 1')
  await expect(panels.getByTestId('group-panel-frame')).toHaveCount(0)
  await panels.getByTestId('group-panels-toggle').click()
  await expect(panels.getByRole('tab')).toHaveText(['CRM'])
  const frame = panels.getByTestId('group-panel-frame')
  await expect(frame).toHaveAttribute('data-state', 'ready')
  const src = await frame.locator('iframe').getAttribute('src')
  const url = new URL(src!)
  expect(url.pathname).toBe(PANEL_PAGE)
  expect(url.searchParams.get('group_did')).toBe(TEAM)
  expect(url.searchParams.get('session')).toBe(TEAM)
  expect(url.searchParams.get('session_id')).toBeNull()
  expect(url.searchParams.get('locale')).toBe('en')
  expect([...url.searchParams.keys()].sort()).toEqual(['group_did', 'locale', 'session'])
  await expect(page.frameLocator('iframe[title="CRM"]').locator('dd[data-key="group_did"]')).toHaveText(TEAM)
  await page.screenshot({ path: 'test-results/messagehub-ext-group-panel.png' })
})

test('a named session shows its own panels, refuses an untrusted origin and changes the frame URL when the session changes', async ({ page }) => {
  await openTeam(page)
  const details = await openDesignReviewDetails(page)
  await expect(details.getByTestId('session-display').getByRole('link', { name: 'Design system' })).toBeVisible()
  await expect(details.getByTestId('group-hooks').locator('li[data-point="post"]')).toBeVisible()
  const panels = details.getByTestId('group-panels')
  await expect(panels.getByTestId('group-panels-toggle')).toContainText('Panels · 2')
  await panels.getByTestId('group-panels-toggle').click()
  await expect(panels.getByRole('tab')).toHaveText(['CRM', 'Untrusted'])
  const frame = panels.getByTestId('group-panel-frame')
  await expect(frame).toHaveAttribute('data-state', 'ready')
  expect(new URL((await frame.locator('iframe').getAttribute('src'))!).searchParams.get('session_id')).toBe('design-review')
  await expect(page.frameLocator('iframe[title="CRM"]').locator('dd[data-key="session_id"]')).toHaveText('design-review')

  await panels.getByRole('tab', { name: 'Untrusted' }).click()
  const untrusted = panels.getByTestId('group-panel-untrusted')
  await expect(untrusted).toContainText('Untrusted source')
  await expect(untrusted).toContainText('https://evil.example/panel')
  await expect(panels.locator('iframe')).toHaveCount(0)
  await page.screenshot({ path: 'test-results/messagehub-ext-untrusted.png' })

  // The default session's panel (reached through the group details) is the same page without a session id.
  await details.getByRole('button', { name: 'Close' }).click()
  await page.getByRole('button', { name: 'Entity details: Product Team' }).last().click()
  const defaultPanels = page.getByTestId('group-panel').getByTestId('group-panels')
  await expect(defaultPanels.getByTestId('group-panels-toggle')).toContainText('Panels · 1')
  await defaultPanels.getByTestId('group-panels-toggle').click()
  const defaultSrc = new URL((await defaultPanels.getByTestId('group-panel-frame').locator('iframe').getAttribute('src'))!)
  expect(defaultSrc.searchParams.get('session_id')).toBeNull()
  expect(defaultSrc.searchParams.get('session')).toBe(TEAM)
})

test('a panel that does not load is shown as unavailable and can be retried; the conversation keeps working', async ({ page }) => {
  let hang = true
  await page.route(`**${PANEL_PAGE}**`, async route => {
    if (!hang) return route.continue()
    await new Promise(resolve => setTimeout(resolve, 12_000))
    return route.continue()
  })
  await openTeam(page)
  await page.getByRole('button', { name: 'Entity details: Product Team' }).last().click()
  const panels = page.getByTestId('group-panel').getByTestId('group-panels')
  await panels.getByTestId('group-panels-toggle').click()
  const frame = panels.getByTestId('group-panel-frame')
  await expect(frame).toHaveAttribute('data-state', 'loading')
  await expect(frame).toHaveAttribute('data-state', 'failed', { timeout: 15_000 })
  await expect(panels.getByTestId('group-panel-unavailable')).toContainText('This panel is unavailable.')
  // The details pane and the conversation are unaffected.
  await expect(page.getByTestId('group-members')).toBeVisible()
  await page.locator('textarea').fill('Still chatting')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('conversation-history').getByText('Still chatting')).toBeVisible()
  hang = false
  await panels.getByTestId('group-panel-unavailable').getByRole('button', { name: 'Retry' }).click()
  await expect(frame).toHaveAttribute('data-state', 'ready')
})

test('a session whose post hook may grant keeps the composer open; a hook refusal is shown on the bubble', async ({ page }) => {
  await openTeam(page)
  await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  await page.locator('[data-testid="session-row"]', { hasText: 'Design review' }).getByRole('button', { name: /Design review/ }).first().click()
  await expect(page.getByTestId('composer-readonly')).toHaveCount(0)
  const textarea = page.locator('textarea')
  await textarea.fill('#deny the launch date')
  await page.keyboard.press('Enter')
  const bubble = page.getByTestId('conversation-history').locator('.mh-bubble', { hasText: '#deny the launch date' })
  await expect(bubble.getByTestId('post-rejected')).toHaveAttribute('data-reason', 'hook-denied')
  await expect(bubble.getByTestId('post-rejected')).toContainText('The business service rejected this message.')
  await expect(bubble.getByTestId('delivery-status')).toHaveAttribute('data-status', 'failed')
  // The text stays on the bubble, not back in the composer.
  await expect(textarea).toHaveValue('')
  await expect(page.getByText('Rejected by the message service')).toHaveCount(0)
  await textarea.fill('Fine, Friday then')
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('conversation-history').getByText('Fine, Friday then')).toBeVisible()
  await page.screenshot({ path: 'test-results/messagehub-ext-hook-denied.png' })
})

const limitedCustomer = {
  account: { user_id: 'acme-c1', user_name: 'Alice Customer', user_type: 'limited', did: CUSTOMER },
  session: { kind: 'account', auth_method: 'exclusive_invite' },
  sso: { accounts: { 'acme-c1': { show_name: 'Alice Customer', password_set: false, state: 'active' } } },
}
const included = { participants: { 't-123': { [CUSTOMER]: 'included' }, 't-456': { 'did:bns:acme-c2': 'included' } } }

async function seedGuest(page: Page) {
  await page.addInitScript(({ controlPanel, host, cpKey, hostKey }) => {
    if (window.sessionStorage.getItem('ext-spec-seeded')) return
    window.sessionStorage.setItem('ext-spec-seeded', '1')
    window.localStorage.setItem(cpKey, JSON.stringify(controlPanel))
    window.localStorage.setItem(hostKey, JSON.stringify(host))
  }, { controlPanel: limitedCustomer, host: included, cpKey: CP_KEY, hostKey: HOST_KEY })
}

test('the guest page shows the session panels and display beside the conversation', async ({ page }) => {
  await seedGuest(page)
  await page.goto(`/messagehub/guest?${new URLSearchParams({ group: GROUP, session: 't-123' })}`)
  await expect(page.getByTestId('guest-conversation')).toBeVisible()
  const side = page.getByTestId('guest-side-panel')
  await expect(side.getByTestId('display-announcement')).toContainText('Support hours')
  await expect(side.getByTestId('display-links').getByRole('link', { name: 'Return policy' })).toBeVisible()
  const panels = side.getByTestId('group-panels')
  await expect(panels.getByRole('tab')).toHaveText(['Order status', 'Foreign widget'])
  const frame = panels.getByTestId('group-panel-frame')
  await expect(frame).toHaveAttribute('data-state', 'ready')
  const src = new URL((await frame.locator('iframe').getAttribute('src'))!)
  expect(src.searchParams.get('group_did')).toBe(GROUP)
  expect(src.searchParams.get('session_id')).toBe('t-123')
  expect(src.searchParams.get('session')).toBe(`${GROUP}/t-123`)
  await expect(page.frameLocator('iframe[title="Order status"]').locator('dd[data-key="session_id"]')).toHaveText('t-123')
  await panels.getByRole('tab', { name: 'Foreign widget' }).click()
  await expect(panels.getByTestId('group-panel-untrusted')).toContainText('https://widgets.example.net/acme')
  await page.screenshot({ path: 'test-results/messagehub-ext-guest.png' })
})

test('on a phone the guest page opens a panel as a full-screen page', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 })
  await seedGuest(page)
  await page.goto(`/messagehub/guest?${new URLSearchParams({ group: GROUP, session: 't-123' })}`)
  await expect(page.getByTestId('guest-conversation')).toBeVisible()
  await expect(page.getByTestId('guest-side-panel')).toHaveCount(0)
  const strip = page.getByTestId('guest-extensions')
  await expect(strip.getByTestId('group-panel-frame')).toHaveCount(0)
  await strip.getByRole('tab', { name: 'Order status' }).click()
  const sheet = page.getByTestId('group-panel-page')
  await expect(sheet.getByRole('heading', { name: 'Order status' })).toBeVisible()
  await expect(sheet.getByTestId('group-panel-frame')).toHaveAttribute('data-state', 'ready')
  await page.screenshot({ path: 'test-results/messagehub-ext-guest-phone.png' })
  await sheet.getByRole('button', { name: 'Close' }).click()
  await expect(sheet).toHaveCount(0)
  await expect(page.getByTestId('guest-conversation')).toBeVisible()
})
