import { expect, test, type Locator, type Page } from '@playwright/test'

async function openAiCenter(page: Page, options: { user?: string; chinese?: boolean } = {}) {
  await page.addInitScript(({ user, locale }) => {
    localStorage.setItem('buckyos.prototype.locale.v1', locale)
    if (user) {
      localStorage.setItem('buckyos.mock.control-panel.v1', JSON.stringify({
        account: { user_id: user, user_name: user, user_type: 'user', did: `did:bns:${user}` },
      }))
    }
  }, { user: options.user, locale: options.chinese ? 'zh-CN' : 'en' })
  await page.goto('/?aiccScenario=populated')
  await page.getByTestId('desktop-app-ai-center').click()
}

async function expectOwner(rows: Locator, userId: string) {
  await expect(rows.first()).toBeVisible()
  await expect.poll(async () => rows.evaluateAll((elements) => [...new Set(elements.map((element) => element.getAttribute('data-user-id')))])).toEqual([userId])
}

test('administrator can compare users, drill into one user and return to own or system usage', async ({ page }, testInfo) => {
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  await openAiCenter(page)
  const scope = page.getByTestId('usage-scope')
  const content = page.getByTestId('usage-scope-content')
  const users = page.getByTestId('usage-by-user')
  await expect(scope).toHaveValue('system')
  await expect(content).toHaveAttribute('data-scope', 'system')
  await expect(users.getByTestId('user-usage-row')).toHaveCount(3)
  await expect(page.getByTestId('usage-set-quota')).toHaveCount(0)
  const alice = users.locator('[data-user-id="alice"]')
  await expect(alice).toContainText('USD')
  await expect(alice).toContainText('EUR')
  await expect(alice).toContainText('CNY')
  await users.screenshot({ path: testInfo.outputPath('usage-by-user.png') })

  await users.getByRole('button', { name: 'bob', exact: true }).click()
  await expect(scope).toHaveValue('user:bob')
  await expect(content).toHaveAttribute('data-scope', 'user:bob')
  await expectOwner(page.getByTestId('recent-usage-row'), 'bob')
  await expect(page.getByTestId('usage-set-quota')).toHaveCount(0)

  await scope.selectOption('mine')
  await expect(content).toHaveAttribute('data-scope', 'mine')
  await expectOwner(page.getByTestId('recent-usage-row'), 'alice')
  await expect(page.getByTestId('usage-set-quota').first()).toBeVisible()

  await scope.selectOption('user:carol')
  await expect(content).toHaveAttribute('data-scope', 'user:carol')
  await expect(page.getByTestId('recent-usage-row')).toHaveCount(0)
  await expect(page.getByText('No usage events match the current filters.', { exact: true })).toBeVisible()

  await scope.selectOption('system')
  await expect(content).toHaveAttribute('data-scope', 'system')
  await expect(users.getByTestId('user-usage-row')).toHaveCount(3)
  expect(errors).toEqual([])
})

test('administrator usage detail keeps every event within the selected user', async ({ page }) => {
  await openAiCenter(page)
  await page.getByRole('button', { name: 'Usage', exact: true }).click()
  await page.getByTestId('usage-scope').selectOption('user:bob')
  await expect(page.getByTestId('usage-scope-content')).toHaveAttribute('data-scope', 'user:bob')
  await expectOwner(page.getByTestId('usage-event-row').filter({ visible: true }), 'bob')
  await page.getByRole('button', { name: 'Next', exact: true }).click()
  await expect(page.getByText('Page 2', { exact: true })).toBeVisible()
  await expectOwner(page.getByTestId('usage-event-row').filter({ visible: true }), 'bob')
  await page.getByTestId('usage-scope').selectOption('mine')
  await expect(page.getByTestId('usage-scope-content')).toHaveAttribute('data-scope', 'mine')
  await expectOwner(page.getByTestId('usage-event-row').filter({ visible: true }), 'alice')
  await expect(page.getByText('Page 1', { exact: true })).toBeVisible()
})

test('ordinary user only sees their own usage and no administrator scope controls', async ({ page }) => {
  await openAiCenter(page, { user: 'bob' })
  await expect(page.getByTestId('usage-scope-content')).toHaveAttribute('data-scope', 'mine')
  await expect(page.getByTestId('usage-scope')).toHaveCount(0)
  await expect(page.getByTestId('usage-by-user')).toHaveCount(0)
  await expect(page.getByText('My usage', { exact: true })).toBeVisible()
  await expectOwner(page.getByTestId('recent-usage-row'), 'bob')
  await page.getByRole('button', { name: 'Usage', exact: true }).click()
  await expect(page.getByTestId('usage-scope')).toHaveCount(0)
  await expectOwner(page.getByTestId('usage-event-row').filter({ visible: true }), 'bob')
})

test.describe('touch viewport', () => {
  test.use({ hasTouch: true })

  test('mobile Chinese usage scope and user breakdown fit the viewport', async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 375, height: 812 })
    await openAiCenter(page, { chinese: true })
    const scope = page.getByTestId('usage-scope')
    const content = page.getByTestId('usage-scope-content')
    await expect(scope).toHaveValue('system')
    await expect(page.getByTestId('usage-by-user').getByTestId('user-usage-row')).toHaveCount(3)
    const scopeBox = await scope.boundingBox()
    expect(scopeBox!.x).toBeGreaterThanOrEqual(0)
    expect(scopeBox!.x + scopeBox!.width).toBeLessThanOrEqual(375)
    expect(await content.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
    await page.screenshot({ path: testInfo.outputPath('usage-mobile.png'), fullPage: true })
    await page.getByTestId('usage-by-user').screenshot({ path: testInfo.outputPath('usage-by-user-mobile.png') })
    await page.getByTestId('usage-by-user').getByRole('button', { name: 'bob', exact: true }).tap()
    await expect(scope).toHaveValue('user:bob')
    await expectOwner(page.getByTestId('recent-usage-row'), 'bob')
  })
})
