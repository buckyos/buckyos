import { expect, test, type Page } from '@playwright/test'

const adminUser = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const adminPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025'

async function loginThroughUi(page: Page): Promise<void> {
  await page.goto('/')
  await expect(page.getByLabel('Username')).toBeVisible()
  await page.getByLabel('Username').fill(adminUser)
  await page.getByLabel('Password').fill(adminPassword)
  await page.getByRole('button', { name: 'Sign In' }).click()
  await expect(page.getByRole('button', { name: 'BuckyOS' })).toBeVisible()
}

test.describe('HomeStation on the real zone', () => {
  test.use({ viewport: { width: 1440, height: 900 } })

  test('the app loads from the homestation service and publishes a post', async ({ page }) => {
    await loginThroughUi(page)
    await page.goto('/homestation')
    await expect(page.getByTestId('homestation')).toBeVisible({ timeout: 30_000 })
    await expect(page.getByTestId('hs-store-status')).toHaveCount(0)

    const text = `HomeStation DV UI ${Date.now().toString(36)}`
    await page.getByTestId('hs-quick-text').fill(text)
    await page.getByTestId('hs-quick-submit').click()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Published to your feed' })).toBeVisible()
    await page.getByTestId('hs-nav-published').click()
    const mine = page.getByTestId('hs-card').filter({ hasText: text })
    await expect(mine).toHaveCount(1)
    await expect(mine.getByTestId('hs-publish-status')).toHaveAttribute('data-stage', 'published')
  })
})
