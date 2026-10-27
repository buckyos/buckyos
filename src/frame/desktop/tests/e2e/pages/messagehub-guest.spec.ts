import { expect, test, type Page } from '@playwright/test'

/**
 * Session Only guest page (`/messagehub/guest`, SSO-12) on the mock host
 * group (`src/app/messagehub/mock/guest.ts`) and the mock control panel's
 * session; plus the host side "invite external customer" dialog.
 */

interface GuestMockControls {
  objectRequests: Array<{ objId: string; context?: string }>
  remove(sessionId: string, did: string): void
  post(sessionId: string, content: string, from?: string): void
  messages(sessionId: string): Array<{ seq: number; objId: string; msg: { from: string; to: string[]; kind: string; to_session?: string; content: { content?: string } } }>
}

declare global {
  interface Window { __messageHubGuestMock: GuestMockControls }
}

const GROUP = 'did:buckyos:group:acme-support'
const CUSTOMER = 'did:bns:acme-c1'
const CP_KEY = 'buckyos.mock.control-panel.v1'
const HOST_KEY = 'buckyos.mock.messagehub.guest.v1'
const TEAM = 'did:buckyos:group:product-team'

const limitedCustomer = {
  account: { user_id: 'acme-c1', user_name: 'Alice Customer', user_type: 'limited', did: CUSTOMER },
  session: { kind: 'account', auth_method: 'exclusive_invite' },
  sso: { accounts: { 'acme-c1': { show_name: 'Alice Customer', password_set: false, state: 'active' } } },
}
const included = { participants: { 't-123': { [CUSTOMER]: 'included' }, 't-456': { 'did:bns:acme-c2': 'included' } } }

/** Seeds the mock control panel session (and the host group) once per tab; later loads keep what the page changed. */
async function seed(page: Page, controlPanel: object, host?: object) {
  await page.addInitScript(({ controlPanel, host, cpKey, hostKey }) => {
    if (window.sessionStorage.getItem('guest-spec-seeded')) return
    window.sessionStorage.setItem('guest-spec-seeded', '1')
    window.localStorage.setItem(cpKey, JSON.stringify(controlPanel))
    if (host) window.localStorage.setItem(hostKey, JSON.stringify(host))
  }, { controlPanel, host: host ?? null, cpKey: CP_KEY, hostKey: HOST_KEY })
}

const guestPage = (session?: string) => `/messagehub/guest?${new URLSearchParams({ group: GROUP, ...(session ? { session } : {}) })}`

test('an invited customer joins the session, reads it with its attachment and replies without upload', async ({ page }) => {
  await seed(page, limitedCustomer)
  await page.goto(guestPage('t-123'))
  await expect(page.getByTestId('guest-group-name')).toHaveText('Acme Support')
  const invitation = page.getByTestId('guest-invitation')
  await expect(invitation).toContainText('Acme Support invited you to a conversation')
  // Signing in alone shows nothing of the Session.
  await expect(page.getByTestId('guest-conversation')).toHaveCount(0)
  await expect(page.getByTestId('sso-first-password')).toBeVisible()
  await page.screenshot({ path: 'test-results/messagehub-guest-invited.png' })

  await invitation.getByTestId('guest-accept').click()
  const conversation = page.getByTestId('guest-conversation')
  await expect(conversation.getByText('Hello, this is Acme support. We are looking into order #123.')).toBeVisible()
  await expect(page.getByTestId('guest-session-title')).toHaveText('Order #123')
  await expect(conversation.getByTestId('attachment-ready')).toContainText('design-brief.pdf')
  const requests = await page.evaluate(() => window.__messageHubGuestMock.objectRequests)
  expect(requests).toContainEqual({ objId: 'cyfile:mock-doc-brief', context: `${GROUP}/t-123/cymsg:mock-t123-2` })
  expect(requests.every(request => request.context !== undefined)).toBe(true)
  // A file of the Session's display is read through that display.
  const opened = page.waitForEvent('popup')
  await page.getByTestId('session-display').getByRole('button', { name: 'Return label', exact: true }).click()
  await (await opened).close()
  expect(await page.evaluate(() => window.__messageHubGuestMock.objectRequests)).toContainEqual({ objId: 'cyfile:mock-doc-brief', context: `${GROUP}/t-123/_display` })

  // No owner surfaces: entity list, personal sessions, group management or uploads.
  await expect(page.getByTestId('hub-create-menu')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toHaveCount(0)
  await expect(conversation.getByRole('button', { name: 'Add attachment' })).toHaveCount(0)

  await conversation.locator('textarea').fill('The parcel arrived damaged.')
  await page.keyboard.press('Enter')
  await expect(conversation.getByTestId('conversation-history').getByText('The parcel arrived damaged.')).toBeVisible()
  // The bubble shows at once; the host has it once the send returns.
  await expect.poll(async () => (await page.evaluate(() => window.__messageHubGuestMock.messages('t-123'))).at(-1)?.msg).toMatchObject({ from: CUSTOMER, to: [GROUP], kind: 'group_msg', to_session: 't-123', content: { content: 'The parcel arrived damaged.' } })

  await page.evaluate(() => window.__messageHubGuestMock.post('t-123', 'We will send a replacement today.'))
  await expect(conversation.getByText('We will send a replacement today.')).toBeVisible({ timeout: 10_000 })
  await page.screenshot({ path: 'test-results/messagehub-guest-session.png' })

  // Accepted stays accepted: a reload opens the Session directly.
  await page.reload()
  await expect(page.getByTestId('guest-conversation').getByText('Hello, this is Acme support. We are looking into order #123.')).toBeVisible()
  await expect(page.getByTestId('guest-invitation')).toHaveCount(0)
})

test('the identity menu shows who is signed in to which Zone and signs out to the group entry', async ({ page }) => {
  await seed(page, limitedCustomer, included)
  await page.goto(guestPage('t-123'))
  await expect(page.getByTestId('guest-conversation')).toBeVisible()
  await page.getByTestId('guest-identity').click()
  const panel = page.getByTestId('guest-identity-panel')
  await expect(panel.getByTestId('sso-identity-name')).toHaveText('Alice Customer')
  await expect(panel.getByTestId('sso-identity-kind')).toHaveText('Restricted account')
  await expect(panel.getByTestId('sso-identity-did')).toHaveText(CUSTOMER)
  await panel.getByTestId('guest-sign-out').click()
  await page.waitForURL(url => url.pathname === '/sso/entry' && url.searchParams.get('service') === 'msg-center' && url.searchParams.get('entry') === GROUP)
  const session = await page.evaluate(key => JSON.parse(window.localStorage.getItem(key) ?? '{}').session, CP_KEY)
  expect(session).toEqual({ kind: 'anonymous' })
})

test('a guest removed from the session loses it on the next read and sees the denial', async ({ page }) => {
  await seed(page, limitedCustomer, included)
  await page.goto(guestPage('t-123'))
  const conversation = page.getByTestId('guest-conversation')
  await expect(conversation.getByText('Hello, this is Acme support. We are looking into order #123.')).toBeVisible()
  await page.evaluate(did => window.__messageHubGuestMock.remove('t-123', did), CUSTOMER)
  const denied = page.getByTestId('guest-denied')
  await expect(denied).toBeVisible({ timeout: 10_000 })
  await expect(denied.getByRole('alert')).toHaveAttribute('data-reason', 'resource_grant_missing')
  await expect(page.getByTestId('guest-conversation')).toHaveCount(0)
  await expect(page.getByText('Hello, this is Acme support. We are looking into order #123.')).toHaveCount(0)
  await page.screenshot({ path: 'test-results/messagehub-guest-denied.png' })

  // Another visit is refused the same way.
  await page.reload()
  await expect(page.getByTestId('guest-denied')).toBeVisible()
})

test('someone not invited to the session cannot join it', async ({ page }) => {
  await seed(page, limitedCustomer, { participants: { 't-123': {}, 't-456': { 'did:bns:acme-c2': 'included' } } })
  await page.goto(guestPage('t-456'))
  await page.getByTestId('guest-accept').click()
  await expect(page.getByTestId('guest-accept-failed')).toBeVisible()
  await expect(page.getByTestId('guest-conversation')).toHaveCount(0)
  await expect(page.getByText('Another customer’s ticket.')).toHaveCount(0)
})

test('without a session the visitor picks one of theirs or starts a conversation through the public entry', async ({ page }) => {
  await seed(page, limitedCustomer, included)
  await page.goto(guestPage())
  const lobby = page.getByTestId('guest-lobby')
  await expect(lobby.getByTestId('guest-session-link')).toHaveText(['Order #123'])
  await lobby.getByTestId('guest-start').click()
  await page.waitForURL(url => url.searchParams.get('group') === GROUP && /^g-/.test(url.searchParams.get('session') ?? ''))
  const conversation = page.getByTestId('guest-conversation')
  await expect(page.getByTestId('guest-session-title')).toHaveText('New conversation')
  await conversation.locator('textarea').fill('I have a question about my bill.')
  await page.keyboard.press('Enter')
  await expect(conversation.getByTestId('conversation-history').getByText('I have a question about my bill.')).toBeVisible()
})

test('an External identity sees its badge and no password prompt; without a session the page goes to the group entry', async ({ page, context }) => {
  await seed(page, { session: { kind: 'external', did: 'did:bns:carol', show_name: 'Carol' } }, { participants: { 't-123': { 'did:bns:carol': 'included' } } })
  await page.goto(guestPage('t-123'))
  await expect(page.getByTestId('guest-conversation')).toBeVisible()
  await page.getByTestId('guest-identity').click()
  await expect(page.getByTestId('guest-identity-panel').getByTestId('sso-identity-kind')).toHaveText('External identity')
  await expect(page.getByTestId('sso-first-password')).toHaveCount(0)

  const anonymous = await context.newPage()
  await seed(anonymous, { session: { kind: 'anonymous' } })
  await anonymous.goto(guestPage('t-123'))
  await anonymous.waitForURL(url => url.pathname === '/sso/entry' && url.searchParams.get('entry') === GROUP)
})

test('a Zone admin invites an external customer to a group session with a private link shown once', async ({ page }) => {
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
  await page.goto(`/messagehub?entityId=${encodeURIComponent(TEAM)}`)
  await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  await page.locator('[data-testid="session-row"]', { hasText: 'Design review' }).getByRole('button', { name: /Design review/ }).first().click()
  await page.getByRole('button', { name: 'Session details' }).last().click()
  await page.getByTestId('session-details').getByTestId('invite-external').click()
  const dialog = page.getByRole('dialog', { name: 'Invite external customer (private link)' })
  const form = dialog.getByTestId('exclusive-invite-form')
  await form.getByLabel('Customer name').fill('Dana Rivera')
  await expect(form.getByLabel('Account ID')).toHaveValue('dana-rivera')
  await form.getByLabel('Account ID').fill('dana-r')
  await form.getByLabel('Channel').selectOption('sms')
  await form.getByTestId('exclusive-invite-submit').click()
  await expect(form.getByRole('alert')).toHaveText('Enter the masked contact.')
  await form.getByLabel('Masked contact (for your records)').fill('+1 *** 0199')
  await form.getByTestId('exclusive-invite-submit').click()

  const link = dialog.getByTestId('exclusive-invite-link-value')
  await expect(link).toHaveText(/\/sso\/invite\/ei-[^#]+#s=.+/)
  await expect(dialog.getByTestId('exclusive-invite-link')).toContainText('shown only now')
  const invites = await page.evaluate(key => JSON.parse(window.localStorage.getItem(key) ?? '{}').sso?.invites ?? [], CP_KEY)
  expect(invites.at(-1)).toMatchObject({ user_id: 'dana-r', channel: { kind: 'sms', target_hint: '+1 *** 0199' }, resource: { kind: 'group_session', group_did: TEAM, session_id: 'design-review', title: 'Design review' } })
  await page.screenshot({ path: 'test-results/messagehub-guest-invite-link.png' })

  await dialog.getByTestId('exclusive-invite-link-done').click()
  await expect(dialog).toHaveCount(0)
  // The link is gone with the dialog: opening it again starts a new invitation.
  await page.getByTestId('session-details').getByTestId('invite-external').click()
  await expect(page.getByTestId('exclusive-invite-form')).toBeVisible()
  await expect(page.getByTestId('exclusive-invite-link-value')).toHaveCount(0)
})

test('only a Zone admin is offered the external invitation', async ({ page }) => {
  await page.addInitScript(key => window.localStorage.setItem(key, JSON.stringify({ account: { user_id: 'bob', user_name: 'Bob', user_type: 'user', did: 'did:bns:bob' } })), CP_KEY)
  await page.goto(`/messagehub?entityId=${encodeURIComponent(TEAM)}`)
  await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  await page.locator('[data-testid="session-row"]', { hasText: 'Design review' }).getByRole('button', { name: /Design review/ }).first().click()
  await page.getByRole('button', { name: 'Session details' }).last().click()
  const actions = page.getByTestId('session-details').getByTestId('group-session-actions')
  await expect(actions.getByRole('button', { name: 'Invite guest' })).toBeVisible()
  await expect(actions.getByTestId('invite-external')).toHaveCount(0)
})

test('the guest page fits a phone in the dark theme', async ({ page }) => {
  await page.addInitScript(() => { try { window.localStorage.setItem('buckyos.prototype.theme.v1', 'dark') } catch { /* ignored */ } })
  await seed(page, limitedCustomer, included)
  await page.setViewportSize({ width: 375, height: 760 })
  await page.goto(guestPage('t-123'))
  const conversation = page.getByTestId('guest-conversation')
  await expect(conversation.getByTestId('attachment-ready')).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  for (const id of ['guest-header', 'guest-identity', 'message-composer']) {
    const box = await page.getByTestId(id).boundingBox()
    expect(box && box.x >= 0 && box.x + box.width <= 375).toBe(true)
  }
  await page.screenshot({ path: 'test-results/messagehub-guest-phone-dark.png' })
  await page.getByTestId('guest-identity').click()
  await expect(page.getByTestId('guest-identity-panel')).toBeVisible()
  const panel = await page.getByTestId('guest-identity-panel').boundingBox()
  expect(panel && panel.x >= 0 && panel.x + panel.width <= 375).toBe(true)
  await page.screenshot({ path: 'test-results/messagehub-guest-phone-dark-identity.png' })
})

test('a restricted account opening the full Message Hub gets no mailbox, only the way to its links', async ({ page }) => {
  await seed(page, limitedCustomer)
  await page.goto('/messagehub')
  const notice = page.getByTestId('messagehub-restricted')
  await expect(notice).toContainText('Open a conversation from the link you were given.')
  await expect(page.getByTestId('hub-create-menu')).toHaveCount(0)
})
