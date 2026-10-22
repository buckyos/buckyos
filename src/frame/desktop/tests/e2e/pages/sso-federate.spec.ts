import { expect, test, type Page } from '@playwright/test'

const MOCK_CONTROL_PANEL = 'buckyos.mock.control-panel.v1'
const TARGET = 'https://bob.web3.buckyos.ai'

async function seedControlPanel(page: Page, state: Record<string, unknown>) {
  await page.addInitScript(([key, value]) => {
    if (!window.sessionStorage.getItem('sso-seeded')) {
      window.localStorage.setItem(key, value)
      window.sessionStorage.setItem('sso-seeded', '1')
    }
  }, [MOCK_CONTROL_PANEL, JSON.stringify(state)] as const)
}

/** The consent request the target Zone sends; the mock account is alice (did:bns:alice). */
function consentUrl(did = 'did:bns:alice') {
  const params = new URLSearchParams({
    did,
    client_zone: 'did:bns:bob',
    redirect_uri: `${TARGET}/sso/did_callback`,
    state: 'flow-1',
    nonce: 'n-1',
    code_challenge: 'c-1',
  })
  return `/sso/federate?${params}`
}

async function routeTarget(page: Page) {
  await page.route(`${TARGET}/**`, (route) => route.fulfill({ status: 200, contentType: 'text/html', body: '<p>target zone</p>' }))
}

test.describe('DID consent page', () => {
  test('shows what is shared and returns a code on approval; a remembered Zone gets one-click continue', async ({ page }) => {
    await routeTarget(page)
    await page.goto(consentUrl())
    const consent = page.getByTestId('sso-federate-consent')
    await expect(consent.getByTestId('sso-federate-did')).toHaveText('did:bns:alice')
    await expect(consent.getByTestId('sso-federate-target')).toHaveText('bob.web3.buckyos.ai')
    await expect(consent.getByTestId('sso-federate-name')).toHaveText('Alice')
    await expect(consent).toHaveAttribute('data-already-granted', 'false')
    await expect(consent.getByTestId('sso-federate-remember').getByRole('checkbox')).toBeChecked()

    await consent.getByTestId('sso-federate-approve').click()
    await page.waitForURL(`${TARGET}/sso/did_callback?**`)
    const back = new URL(page.url())
    expect(back.searchParams.get('code')).toBe('fc-mock')
    expect(back.searchParams.get('state')).toBe('flow-1')

    await page.goto(consentUrl())
    await expect(page.getByTestId('sso-federate-consent')).toHaveAttribute('data-already-granted', 'true')
    await expect(page.getByTestId('sso-federate-approve')).toHaveText('Continue')
  })

  test('cancel goes back with access_denied', async ({ page }) => {
    await routeTarget(page)
    await page.goto(consentUrl())
    await page.getByTestId('sso-federate-deny').click()
    await page.waitForURL(`${TARGET}/sso/did_callback?**`)
    const back = new URL(page.url())
    expect(back.searchParams.get('error')).toBe('access_denied')
    expect(back.searchParams.get('state')).toBe('flow-1')
    expect(back.searchParams.get('code')).toBeNull()
  })

  test('another DID than the signed-in account is refused', async ({ page }) => {
    await page.goto(consentUrl('did:bns:mallory'))
    await expect(page.getByTestId('sso-federate-denied')).toHaveAttribute('data-reason', 'session_ceiling_exceeded')
  })

  test('a restricted account cannot sign in elsewhere', async ({ page }) => {
    await seedControlPanel(page, { account: { user_id: 'dave', user_name: 'Dave', user_type: 'limited', did: 'did:bns:dave' } })
    await page.goto(consentUrl('did:bns:dave'))
    await expect(page.getByTestId('sso-federate-denied')).toHaveAttribute('data-reason', 'session_kind_not_allowed')
  })

  test('without a session it asks to sign in first and keeps the request', async ({ page }) => {
    await seedControlPanel(page, { session: { kind: 'anonymous' } })
    await page.goto(consentUrl())
    await page.waitForURL('**/login?**')
    const redirect = new URL(new URL(page.url()).searchParams.get('redirect_url') ?? '')
    expect(redirect.pathname).toBe('/sso/federate')
    expect(redirect.searchParams.get('state')).toBe('flow-1')
  })
})
