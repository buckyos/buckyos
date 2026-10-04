import { expect, test } from '@playwright/test'

const user = process.env.OPENDAN_ZONE_USER ?? 'devtest'
const password = process.env.OPENDAN_ZONE_PASSWORD ?? 'bucky2025'

test('the WebUI behind the zone gateway reads the Loader with the zone login', async ({ page }) => {
  const errors: string[] = []
  const denied: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  page.on('response', async (r) => {
    if (!r.url().endsWith('/kapi/opendan')) return
    const body = await r.text().catch(() => '')
    if (body.includes('No permission')) denied.push(`${r.request().postDataJSON()?.method}: ${body.slice(0, 160)}`)
  })

  await page.goto('/')
  await expect(page).toHaveURL(/\/login\?/)
  await page.locator('input[type="text"]').fill(user)
  await page.locator('input[type="password"]').fill(password)
  await page.getByRole('button', { name: 'Sign In' }).click()

  await expect(page.getByTestId('data-source')).toHaveText('data: krpc')
  await expect(page.getByTestId('agent-card')).toBeVisible()
  await expect(page.getByTestId('session-list').locator('a[data-target="messagehub"]').first()).toBeVisible()
  await page.screenshot({ path: 'test-results-zone/home.png', fullPage: true })
  await page.getByRole('link', { name: 'Advanced' }).click()
  await expect(page.getByTestId('sessions-panel')).toBeVisible()
  await expect(page.getByTestId('sessions-panel')).toContainText('ui-')
  await page.screenshot({ path: 'test-results-zone/sessions.png', fullPage: true })

  await page.getByTestId('sessions-panel').getByRole('link').filter({ hasText: 'ui-' }).first().click()
  await expect(page.locator('body')).toContainText('turn_ended')
  await page.screenshot({ path: 'test-results-zone/session.png', fullPage: true })

  await page.getByRole('link', { name: 'Loader' }).click()
  await expect(page.getByTestId('loader-info')).toContainText('did:')
  await expect(page.getByTestId('loader-modules')).toContainText('ui')
  await page.screenshot({ path: 'test-results-zone/loader.png', fullPage: true })

  expect(denied).toEqual([])
  expect(errors).toEqual([])
})
