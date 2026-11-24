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
    const period = page.getByTestId('home-usage-summary').getByLabel('用量明细', { exact: true }).first()
    await period.tap()
    const periodDetails = period.locator('..').locator('dl')
    await expect(periodDetails).toBeVisible()
    const periodBox = await periodDetails.boundingBox()
    expect(periodBox!.x).toBeGreaterThanOrEqual(0)
    expect(periodBox!.x + periodBox!.width).toBeLessThanOrEqual(375)
    await period.tap()
    const recent = page.getByTestId('recent-usage-row').first()
    await recent.getByLabel('用量明细', { exact: true }).tap()
    await expect(recent.locator('dl')).toBeVisible()
    const unitsBox = await recent.locator('dl').boundingBox()
    expect(unitsBox!.x).toBeGreaterThanOrEqual(0)
    expect(unitsBox!.x + unitsBox!.width).toBeLessThanOrEqual(375)
    await recent.getByLabel('用量明细', { exact: true }).tap()
    const money = page.getByTestId('usage-currencies').filter({ visible: true }).first()
    await expect(money.locator('summary')).toContainText('USD')
    await money.locator('summary').tap()
    await expect(money.locator('dl')).toBeVisible()
    await page.screenshot({ path: testInfo.outputPath('usage-mobile-currencies.png'), fullPage: true })
  })
})

test('filtered consumption totals cover all pages and token details expose only present units', async ({ page }, testInfo) => {
  await openAiCenter(page)
  await page.getByRole('button', { name: 'Usage', exact: true }).click()
  const consumption = page.getByTestId('usage-consumption')
  await expect(consumption).toContainText('140 matching records, all pages')
  await expect(consumption).toContainText('USD')
  await expect(consumption).toContainText('EUR')
  const initial = await consumption.innerText()
  await page.getByRole('button', { name: 'Next', exact: true }).click()
  await expect(page.getByText('Page 2', { exact: true })).toBeVisible()
  await expect(consumption).toHaveText(initial, { useInnerText: true })
  const details = consumption.locator('details').first()
  await details.locator('summary').click()
  await expect(details).toHaveAttribute('open', '')
  await expect(details).toContainText('Generated images')
  await expect(details).not.toContainText('Compute duration')
  await page.screenshot({ path: testInfo.outputPath('usage-units.png') })
  await details.locator('summary').press('Escape')
  await expect(details).not.toHaveAttribute('open', '')
  await page.getByRole('button', { name: 'Filters', exact: true }).click()
  await page.getByPlaceholder('All models', { exact: true }).fill('sn-router-image')
  await expect(consumption).toContainText('28 matching records, all pages')
  await expect(consumption.locator('summary').first()).toContainText('0 tokens')
  await consumption.locator('summary').first().click()
  await expect(consumption.locator('details').first()).toContainText('Generated images')
  await expect(consumption.locator('details').first()).not.toContainText('Input tokens')
  await consumption.screenshot({ path: testInfo.outputPath('usage-filtered-media.png') })
  await page.getByPlaceholder('All models', { exact: true }).fill('no-matching-model')
  await expect(consumption).toContainText('0 matching records, all pages')
})

test('home and usage lists expose unit details and multi-currency totals default to USD', async ({ page }, testInfo) => {
  await openAiCenter(page)
  const recent = page.getByTestId('recent-usage-row').first()
  await recent.getByLabel('Usage details', { exact: true }).click()
  await expect(recent.locator('details')).toHaveAttribute('open', '')
  await expect(recent.locator('dl')).toBeVisible()
  await recent.getByLabel('Usage details', { exact: true }).press('Escape')
  await expect(recent.locator('details')).not.toHaveAttribute('open', '')
  const homeMoney = page.getByTestId('usage-currencies').filter({ visible: true }).first()
  await expect(homeMoney.locator('summary')).toContainText('USD')
  await expect(homeMoney.locator('dl')).not.toBeVisible()
  await homeMoney.locator('summary').click()
  await expect(homeMoney.locator('dl')).toBeVisible()
  await expect(homeMoney.locator('dl')).toContainText('EUR')
  await expect(homeMoney.locator('dl')).toContainText('CNY')
  await page.screenshot({ path: testInfo.outputPath('home-currencies.png') })
  await page.getByRole('button', { name: 'Usage', exact: true }).click()
  const totalMoney = page.getByTestId('usage-consumption').getByTestId('usage-currencies').first()
  await expect(totalMoney.locator('summary')).toContainText('USD')
  await expect(totalMoney.locator('dl')).not.toBeVisible()
  await totalMoney.locator('summary').click()
  await expect(totalMoney.locator('dl')).toContainText('EUR')
  await expect(totalMoney.locator('dl')).toBeVisible()
  const row = page.getByTestId('usage-event-row').filter({ visible: true }).first()
  await row.getByLabel('Usage details', { exact: true }).click()
  await expect(row.locator('dl')).toBeVisible()
  await page.screenshot({ path: testInfo.outputPath('usage-row-units-and-currencies.png') })
})

test('home summary exposes independent today, month and thirty-day token details', async ({ page }, testInfo) => {
  await openAiCenter(page)
  const summary = page.getByTestId('home-usage-summary')
  const units = summary.getByLabel('Usage details', { exact: true })
  await expect(units).toHaveCount(3)
  for (let index = 0; index < 3; index += 1) {
    await units.nth(index).click()
    const details = units.nth(index).locator('..')
    await expect(details).toHaveAttribute('open', '')
    await expect(details.locator('dl')).toBeVisible()
    await expect(details).toContainText('Input tokens')
    await units.nth(index).press('Escape')
    await expect(details).not.toHaveAttribute('open', '')
  }
  await summary.screenshot({ path: testInfo.outputPath('home-summary-token-details.png') })
})

async function installBillingFixture(page: Page) {
  await expect(page.getByTestId('home-usage-summary')).toBeVisible()
  await page.evaluate(async () => {
    const dashboardSource = await fetch('/src/app/ai-center/components/home/UsageDashboard.tsx').then((response) => response.text())
    const modulePath = dashboardSource.match(/from "([^" ]*\/api\/aicc_mgr\.ts[^" ]*)"/)?.[1] ?? '/src/api/aicc_mgr.ts'
    const { AICCModelStore } = await import(modulePath)
    const original = AICCModelStore.prototype.queryUsageEvents
    const state = { settled: false, hold: false, releases: [] as Array<() => void>, cursors: [] as Array<string | undefined> }
    Reflect.set(window, 'usageBillingFixture', state)
    AICCModelStore.prototype.queryUsageEvents = async function (params: { summaryOnly?: boolean; groupBy?: string; groupLimit?: number; groupOffset?: number; cursor?: string }) {
      const result = await original.call(this, params)
      if (params.summaryOnly) {
        if (state.hold) await new Promise<void>((resolve) => state.releases.push(resolve))
        if (params.groupBy === 'provider_model') {
          const aggregate = result.grouped[0]?.aggregate ?? (await original.call(this, { ...params, groupOffset: 0 })).grouped[0].aggregate
          result.grouped = Array.from({ length: 40 }, (_, index) => ({ group: { provider_model: `billing-model-${String(index).padStart(2, '0')}` }, aggregate }))
            .slice(params.groupOffset ?? 0, (params.groupOffset ?? 0) + (params.groupLimit ?? 20))
        }
      } else {
        state.cursors.push(params.cursor)
        result.events = result.events.map((event: { usage?: Record<string, unknown> }) => ({ ...event, usage: { ...event.usage, cost_source: state.settled ? 'provider_billing' : 'static_pricing_pending_billing' } }))
      }
      return result
    }
  })
}

test('billing refresh retains consumption totals and all loaded groups until replacements arrive', async ({ page }) => {
  await page.clock.install()
  await openAiCenter(page)
  await installBillingFixture(page)
  await page.getByRole('button', { name: 'Usage', exact: true }).click()
  const consumption = page.getByTestId('usage-consumption')
  const models = page.getByTestId('usage-consumption-provider_model')
  await expect(models).toContainText('Models · 20+')
  await models.getByRole('button', { name: 'Show more', exact: true }).click()
  await models.getByRole('button', { name: 'Load more', exact: true }).click()
  await expect(models).toContainText('billing-model-39')
  await page.evaluate(() => { const state = Reflect.get(window, 'usageBillingFixture'); state.settled = true; state.hold = true })
  await page.clock.fastForward(30_000)
  await expect.poll(() => page.evaluate(() => Reflect.get(window, 'usageBillingFixture').releases.length)).toBe(5)
  await expect(consumption).toContainText('140 matching records, all pages')
  await expect(models).toContainText('billing-model-39')
  await expect(consumption).not.toContainText('Loading')
  await page.evaluate(() => { const state = Reflect.get(window, 'usageBillingFixture'); state.hold = false; state.releases.forEach((release: () => void) => release()) })
  await expect(page.getByTestId('usage-event-row').first()).not.toContainText('Awaiting billing')
  await expect(models).toContainText('billing-model-39')
})

test('mobile billing refresh replaces records on earlier pages and stops polling after settlement', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 })
  await page.clock.install()
  await openAiCenter(page)
  await installBillingFixture(page)
  await page.getByRole('button', { name: 'Usage', exact: true }).click()
  const rows = page.getByTestId('usage-event-row').filter({ visible: true })
  await expect(rows).toHaveCount(10)
  await page.getByRole('button', { name: 'Load more', exact: true }).click()
  await expect(rows).toHaveCount(20)
  await page.evaluate(() => { const state = Reflect.get(window, 'usageBillingFixture'); state.settled = true; state.cursors = [] })
  await page.clock.fastForward(30_000)
  await expect.poll(() => page.evaluate(() => Reflect.get(window, 'usageBillingFixture').cursors.length)).toBe(2)
  await expect(rows).toHaveCount(20)
  await expect(page.getByText('Awaiting billing', { exact: true })).toHaveCount(0)
  await page.clock.fastForward(30_000)
  expect(await page.evaluate(() => Reflect.get(window, 'usageBillingFixture').cursors.length)).toBe(2)
})
