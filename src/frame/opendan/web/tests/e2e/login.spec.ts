import { expect, test } from '@playwright/test'

test('the WebUI loads its app API when the zone has no opendan service', async ({ page }) => {
  const appId = 'xiaobai.test.buckyos.io'
  const token = `header.${Buffer.from(JSON.stringify({
    appid: appId, sub: 'devtest', iss: 'verify-hub', exp: Math.floor(Date.now() / 1000) + 3600,
  })).toString('base64url')}.signature`
  const legacyRequests: string[] = []
  const apiMethods: string[] = []
  await page.route('**/sso_refresh', (route) => route.fulfill({ json: {
    session_token: token,
    identity: {
      kind: 'account', user_id: 'devtest', did: 'did:bns:devtest', user_type: 'admin',
      show_name: 'devtest', zone_host: 'test.buckyos.io', restricted: false, password_set: true,
    },
  } }))
  await page.route('**/kapi/**', (route) => {
    legacyRequests.push(route.request().url())
    return route.fulfill({ status: 503, body: 'service_not_ready:opendan' })
  })
  await page.route('**/api/opendan', (route) => {
    const request = route.request()
    apiMethods.push(request.method())
    if (request.method() === 'GET') return route.fulfill({ json: { app_id: appId } })
    const { sys, method } = request.postDataJSON()
    expect(method).toBe('loader.status')
    expect(sys[1]).toBe(token)
    return route.fulfill({ json: { sys: [sys[0]], result: {
      agent_id: appId,
      agent_did: `did:web:${appId}`,
      who: 'app:xiaobai.test.buckyos.io@devtest',
      agent_root: '/agent',
      started_at_ms: 1,
      modules: [], hosted: [], errors: [],
    } } })
  })
  await page.goto('/?data=krpc&lang=en#/loader')
  await expect(page.getByTestId('loader-info')).toContainText('xiaobai.test.buckyos.io')
  await expect(page.getByRole('alert')).toHaveCount(0)
  expect(apiMethods).toEqual(expect.arrayContaining(['GET', 'POST']))
  expect(legacyRequests).toEqual([])
})

// The kRPC data model behind a zone gateway, with the gateway's answers stubbed.

test('a page the zone turns away asks to sign in again', async ({ page }) => {
  // What the gateway does once the SSO cookies are gone: every request is sent
  // to the zone's login page, on another origin.
  await page.route('**/api/opendan', (route) =>
    route.fulfill({ status: 302, headers: { location: 'https://sys.zone.invalid/login?redirect_url=x' } }),
  )
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.goto('/?data=krpc&lang=en')

  const card = page.getByTestId('login-lost')
  await expect(card).toContainText('Your sign-in has expired')
  await expect(page.locator('body')).not.toContainText('transport')

  const reload = page.waitForRequest((r) => r.isNavigationRequest() && r.frame() === page.mainFrame())
  await card.getByRole('button', { name: 'Sign in again' }).click()
  expect(new URL((await reload).url()).search).toBe('?data=krpc&lang=en')
  expect(errors).toEqual([])
})

test('a service that cannot be reached stays a transport error', async ({ page }) => {
  await page.route('**/api/opendan', (route) =>
    route.request().method() === 'GET'
      ? route.fulfill({ contentType: 'application/json', body: '{"app_id":null}' })
      : route.abort('connectionrefused'),
  )
  await page.goto('/?data=krpc&lang=en')

  await expect(page.getByRole('alert')).toContainText('transport')
  await expect(page.getByTestId('login-lost')).toHaveCount(0)
})
