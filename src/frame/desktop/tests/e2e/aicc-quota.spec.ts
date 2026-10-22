import { expect, test, type Page } from '@playwright/test'

// AI Center token quota and model visibility against the mock quota world (src/app/ai-center/mock/quota.ts):
// the viewer alice is an administrator with an exhausted own limit on aiworkspace; bob has a provider
// limit at 86 %, carol a total account with 30K left.

async function openAiCenter(page: Page, options: { chinese?: boolean; dark?: boolean } = {}) {
  await page.addInitScript(({ locale, theme }) => {
    localStorage.setItem('buckyos.prototype.locale.v1', locale)
    if (theme) localStorage.setItem('buckyos.prototype.theme.v1', theme)
  }, { locale: options.chinese ? 'zh-CN' : 'en', theme: options.dark ? 'dark' : '' })
  await page.goto('/?aiccScenario=populated')
  await page.getByTestId('desktop-app-ai-center').click()
}

async function openPage(page: Page, name: string) {
  await page.getByRole('button', { name, exact: true }).click()
}

function trackErrors(page: Page) {
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()) })
  return errors
}

test('own limits, administrator views, rule creation and recharge', async ({ page }, testInfo) => {
  const errors = trackErrors(page)
  await openAiCenter(page)
  await expect(page.getByTestId('quota-home-card')).toBeVisible()

  await openPage(page, 'Quota')
  const own = page.getByTestId('quota-own').getByTestId('quota-rule')
  await expect(own).toHaveCount(1)
  await expect(own.getByTestId('quota-status')).toHaveAttribute('data-status', 'exhausted')
  await expect(page.getByTestId('quota-warning')).toBeVisible()
  await page.screenshot({ path: testInfo.outputPath('quota-mine.png'), fullPage: true })

  // Detail shows every constraint of the same call, including the administrator's App limit.
  await own.click()
  const detail = page.getByTestId('quota-detail')
  await expect(detail).toBeVisible()
  await expect(detail.getByTestId('quota-constraints')).toBeVisible()
  await detail.getByRole('button', { name: 'Close' }).first().click()

  await page.getByRole('radio', { name: 'User quotas', exact: true }).click()
  const bob = page.locator('[data-testid="quota-rule"]').filter({ hasText: 'User bob' })
  await expect(bob.getByTestId('quota-status')).toHaveAttribute('data-status', 'near_limit')
  const carol = page.locator('[data-testid="quota-rule"][data-kind="total"]').filter({ hasText: 'User carol' })
  await expect(carol).toBeVisible()

  // Recharge carol's total account: 30K available → 80K, credited once.
  await carol.click()
  await expect(detail.getByTestId('quota-available')).toHaveText('30K')
  await detail.getByTestId('quota-recharge-open').click()
  await page.getByTestId('quota-recharge-amount').fill('50K')
  await expect(page.getByTestId('quota-recharge')).toContainText('= 50,000 tokens')
  await page.getByTestId('quota-recharge-submit').click()
  await expect(page.getByTestId('quota-recharge-done')).toBeVisible()
  await page.screenshot({ path: testInfo.outputPath('quota-recharge.png'), fullPage: true })
  await page.getByTestId('quota-recharge').getByRole('button', { name: 'Close' }).click()
  await expect(detail.getByTestId('quota-available')).toHaveText('80K')
  await detail.getByRole('button', { name: 'Close' }).first().click()

  // A new periodic limit for dave, with a live preview.
  await page.getByTestId('quota-new').click()
  const editor = page.getByTestId('quota-editor')
  await editor.getByTestId('quota-user').selectOption('dave')
  await editor.getByTestId('quota-limit').fill('50k')
  await expect(editor).toContainText('= 50,000 tokens')
  await editor.getByTestId('quota-period').selectOption('4h')
  await expect(editor.getByTestId('quota-preview').getByTestId('quota-status')).toHaveAttribute('data-status', 'normal')
  await page.screenshot({ path: testInfo.outputPath('quota-editor.png'), fullPage: true })
  await editor.getByTestId('quota-save').click()
  await expect(editor).toHaveCount(0)
  await expect(page.locator('[data-testid="quota-rule"]').filter({ hasText: 'User dave' })).toBeVisible()

  // The same object, resource and period again is a conflict.
  await page.getByTestId('quota-new').click()
  await editor.getByTestId('quota-user').selectOption('dave')
  await editor.getByTestId('quota-limit').fill('10K')
  await editor.getByTestId('quota-period').selectOption('4h')
  await expect(editor.getByTestId('quota-conflict')).toBeVisible()
  await expect(editor.getByTestId('quota-save')).toBeDisabled()
  await editor.getByRole('button', { name: 'Close' }).first().click()

  // Usage breakdown rows on Home open the editor prefilled for the row.
  await openPage(page, 'Home')
  await page.getByTestId('usage-set-quota').first().click()
  await expect(page.getByTestId('quota-editor')).toBeVisible()
  await page.getByTestId('quota-editor').getByRole('button', { name: 'Close' }).first().click()
  await expect(page.getByTestId('quota-editor')).toHaveCount(0)

  expect(errors).toEqual([])
})

test('group visibility: per-Provider choice, preview before saving, per-user grants', async ({ page }, testInfo) => {
  const errors = trackErrors(page)
  await openAiCenter(page)
  await openPage(page, 'Visibility')
  const users = page.locator('[data-testid="visibility-subject"][data-subject="type:user"]')
  await expect(users).toContainText('All Providers')
  await users.click()

  await page.getByTestId('visibility-all').uncheck()
  const provider = page.getByTestId('visibility-provider').first()
  await provider.getByRole('radio', { name: 'All models', exact: true }).click()
  await page.getByTestId('visibility-review').click()
  const confirm = page.getByTestId('visibility-confirm')
  await expect(confirm).toContainText('Removed')
  await page.screenshot({ path: testInfo.outputPath('visibility-confirm.png'), fullPage: true })
  await page.getByTestId('visibility-save').click()
  await expect(confirm).toHaveCount(0)
  await expect(users).toContainText('Providers: 1')

  await page.getByTestId('visibility-user').selectOption('bob')
  // bob is a plain user: only the one Provider opened to Users remains, granted by that group.
  const grants = page.getByTestId('visibility-user-grants')
  await expect(grants).toContainText('Users')
  await expect(grants).not.toContainText('Administrators')
  await page.screenshot({ path: testInfo.outputPath('visibility.png'), fullPage: true })

  expect(errors).toEqual([])
})

test('dark theme quota page', async ({ page }, testInfo) => {
  await openAiCenter(page, { dark: true })
  await openPage(page, 'Quota')
  await expect(page.getByTestId('quota-own').getByTestId('quota-rule')).toHaveCount(1)
  await page.screenshot({ path: testInfo.outputPath('quota-dark.png'), fullPage: true })
  await page.getByRole('radio', { name: 'User quotas', exact: true }).click()
  await page.locator('[data-testid="quota-rule"][data-kind="total"]').click()
  await expect(page.getByTestId('quota-detail').getByTestId('quota-recharge-open')).toBeVisible()
  await page.screenshot({ path: testInfo.outputPath('quota-detail-dark.png'), fullPage: true })
})

test.describe('touch viewport', () => {
  test.use({ hasTouch: true })

  test('mobile Chinese quota page fits the viewport', async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 375, height: 812 })
    await page.addInitScript(() => localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
    await page.goto('/?aiccScenario=populated')
    await page.getByTestId('desktop-app-ai-center').tap()
    await page.getByRole('button', { name: '额度', exact: true }).tap()
    const quota = page.getByTestId('quota-page')
    await expect(quota.getByRole('heading', { name: 'Token 额度' })).toBeVisible()
    await expect(quota.getByTestId('quota-own').getByTestId('quota-rule')).toHaveCount(1)
    expect(await quota.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
    await page.screenshot({ path: testInfo.outputPath('quota-mobile.png') })
    await quota.getByTestId('quota-own').getByTestId('quota-rule').tap()
    await expect(page.getByTestId('quota-detail')).toBeVisible()
    await page.screenshot({ path: testInfo.outputPath('quota-detail-mobile.png') })
  })
})
