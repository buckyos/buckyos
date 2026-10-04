import { expect, test } from '@playwright/test'

// Against a running opendan (`./debug_jarvis.sh`, OPENDAN_URL defaults to its port).
test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true })

test('home reads the profile, usage and sessions of the running agent', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.goto('/')
  await expect(page.getByTestId('data-source')).toHaveText('data: krpc')
  await expect(page.getByTestId('agent-did')).toContainText('did:')
  await expect(page.getByTestId('stat-total')).not.toContainText('–')
  const conversation = page.getByTestId('session-list').locator('a[data-target="messagehub"]').first()
  await expect(conversation).toHaveAttribute('href', /\/messagehub\?.*sessionId=/)
  await expect(page.getByTestId('open-default-session')).toHaveAttribute('href', /\/messagehub\?entityId=did/)
  expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(0)
  await page.screenshot({ path: 'test-results/home-real.png', fullPage: true })
  expect(errors).toEqual([])
})
