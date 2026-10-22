import { expect, test, type Page } from '@playwright/test'

const STORAGE_KEY = 'buckyos.mock.control-panel.v1'
const TARGET_DID = 'did:bns:bob.other'
const INVITE_PATH = '/users/invite/invite-bob?from=shared-link#details'

async function seed(page: Page, session: Record<string, unknown>, invite: Record<string, unknown> = {}) {
  await page.addInitScript(({ key, session, invite, did }) => {
    if (sessionStorage.getItem('user-invite-seeded')) return
    localStorage.setItem(key, JSON.stringify({
      session,
      account: { user_id: 'bob', user_name: 'Bob', user_type: 'user', did },
      user_invites: {
        'invite-bob': {
          invite_id: 'invite-bob', created_by: 'alice', created_at: 1,
          state: 'pending', target_user_id: 'bob-guest', target_did: did,
          default_user_type: 'user', ...invite,
        },
      },
    }))
    sessionStorage.setItem('user-invite-seeded', '1')
  }, { key: STORAGE_KEY, session, invite, did: TARGET_DID })
}

test('anonymous visitors verify the target DID and return with the complete invitation URL', async ({ page }) => {
  await seed(page, { kind: 'anonymous' })
  await page.route('https://bob.other.web3.buckyos.ai/**', route => route.fulfill({
    status: 200, contentType: 'text/html', body: '<p>Home Zone sign-in</p>',
  }))
  await page.goto(INVITE_PATH)
  const returnUrl = page.url()
  await expect(page.getByTestId('user-invite-page')).toBeVisible()
  await expect(page.getByTestId('sso-did-input')).toHaveValue(TARGET_DID)
  await expect(page.getByTestId('sso-did-input')).toBeDisabled()
  await expect(page.getByTestId('user-invite-form')).toHaveCount(0)
  await page.getByTestId('sso-did-submit').click()
  await page.waitForURL('https://bob.other.web3.buckyos.ai/**')
  expect(new URL(page.url()).searchParams.get('did')).toBe(TARGET_DID)
  const flowState = new URL(page.url()).searchParams.get('state')!

  await page.goto(returnUrl)
  const login = await page.evaluate(({ key, flowState }) => {
    return JSON.parse(localStorage.getItem(key)!).sso.did_logins[flowState]
  }, { key: STORAGE_KEY, flowState })
  expect(login).toEqual({ did: TARGET_DID, redirect_url: returnUrl })
  await page.evaluate(({ key, did }) => {
    const state = JSON.parse(localStorage.getItem(key)!)
    state.session = { kind: 'external', did, show_name: 'Bob' }
    localStorage.setItem(key, JSON.stringify(state))
  }, { key: STORAGE_KEY, did: TARGET_DID })
  await page.reload()
  await expect(page.getByTestId('user-invite-form')).toBeVisible()
  expect(page.url()).toBe(returnUrl)
})

test('a password session for the same DID must sign out and verify through federation', async ({ page }) => {
  await seed(page, { kind: 'account', auth_method: 'password' })
  await page.goto(INVITE_PATH)
  await expect(page.getByTestId('user-invite-switch')).toBeVisible()
  await expect(page.getByTestId('user-invite-accept')).toHaveCount(0)
  await page.getByRole('button', { name: 'Sign out and verify DID' }).click()
  await expect(page.getByTestId('sso-did-input')).toHaveValue(TARGET_DID)
  await expect(page.getByTestId('user-invite-form')).toHaveCount(0)
})

test('a different federated DID cannot accept the invitation', async ({ page }) => {
  await seed(page, { kind: 'external', did: 'did:bns:mallory', show_name: 'Mallory' })
  await page.goto(INVITE_PATH)
  await expect(page.getByTestId('user-invite-switch')).toBeVisible()
  await expect(page.getByTestId('user-invite-form')).toHaveCount(0)
})

test('the verified DID accepts explicitly after providing its matching document', async ({ page }) => {
  await seed(page, { kind: 'external', did: TARGET_DID, show_name: 'Bob' })
  await page.goto(INVITE_PATH)
  const input = page.getByLabel('OwnerDocument JSON')
  await expect(input).toBeEmpty()
  await expect(page.getByTestId('user-invite-accept')).toBeDisabled()
  await input.fill(JSON.stringify({ id: 'did:bns:mallory', name: 'mallory' }))
  await page.getByTestId('user-invite-accept').click()
  await expect(page.getByTestId('user-invite-error')).toHaveText('The document DID must match the DID you used to sign in.')
  await input.fill('[]')
  await page.getByTestId('user-invite-accept').click()
  await expect(page.getByTestId('user-invite-error')).toHaveText('Enter a valid OwnerDocument JSON object.')
  await input.fill(JSON.stringify({ id: TARGET_DID, name: 'bob', binded_zone_list: ['did:bns:alice'] }))
  await page.getByTestId('user-invite-accept').click()
  await expect(page.getByTestId('user-invite-accepted')).toBeVisible()
  const invitation = await page.evaluate(key => JSON.parse(localStorage.getItem(key)!).user_invites['invite-bob'], STORAGE_KEY)
  expect(invitation).toMatchObject({ state: 'accepted', accepted_user_id: 'bob-guest' })
})

test('a session changed after loading is rejected when accepting', async ({ page }) => {
  await seed(page, { kind: 'external', did: TARGET_DID, show_name: 'Bob' })
  await page.goto(INVITE_PATH)
  await page.getByLabel('OwnerDocument JSON').fill(JSON.stringify({ id: TARGET_DID, name: 'bob' }))
  await page.evaluate(key => {
    const state = JSON.parse(localStorage.getItem(key)!)
    state.session = { kind: 'anonymous' }
    localStorage.setItem(key, JSON.stringify(state))
  }, STORAGE_KEY)
  await page.getByTestId('user-invite-accept').click()
  await expect(page.getByTestId('user-invite-error')).toBeVisible()
  await expect(page.getByTestId('user-invite-accepted')).toHaveCount(0)
})

test('an expired invitation offers neither acceptance nor DID sign-in', async ({ page }) => {
  await seed(page, { kind: 'anonymous' }, { expires_at: 1 })
  await page.goto(INVITE_PATH)
  await expect(page.getByTestId('user-invite-unavailable')).toBeVisible()
  await expect(page.getByTestId('sso-did-form')).toHaveCount(0)
  await expect(page.getByTestId('user-invite-form')).toHaveCount(0)
})
