import { expect, test } from '@playwright/test'

// The kRPC data model behind a zone gateway, with the gateway's answers stubbed.

test('a page the zone turns away asks to sign in again', async ({ page }) => {
  // What the gateway does once the SSO cookies are gone: every request is sent
  // to the zone's login page, on another origin.
  await page.route('**/kapi/opendan', (route) =>
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
  await page.route('**/kapi/opendan', (route) =>
    route.request().method() === 'GET'
      ? route.fulfill({ contentType: 'application/json', body: '{"app_id":null}' })
      : route.abort('connectionrefused'),
  )
  await page.goto('/?data=krpc&lang=en')

  await expect(page.getByRole('alert')).toContainText('transport')
  await expect(page.getByTestId('login-lost')).toHaveCount(0)
})
