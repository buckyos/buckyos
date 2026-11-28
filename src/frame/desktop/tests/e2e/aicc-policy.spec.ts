import { expect, test, type Page } from '@playwright/test'

// AI Center user policies, price list and AI Credit against the mock policy world (src/app/ai-center/mock/policy.ts):
// alice is an administrator (free: `type:admin` coefficient 0); `type:user` pays the price list (×1) except the SN
// Provider (coefficient 0); bob has a 24 h limit at 17 / 20 AI Credit and 3.25 AI Credit; dave is at -2.5.

async function openAiCenter(page: Page, options: { user?: string; chinese?: boolean; dark?: boolean } = {}) {
  await page.addInitScript(({ user, locale, theme }) => {
    localStorage.setItem('buckyos.prototype.locale.v1', locale)
    if (theme) localStorage.setItem('buckyos.prototype.theme.v1', theme)
    if (user) {
      localStorage.setItem('buckyos.mock.control-panel.v1', JSON.stringify({
        account: { user_id: user, user_name: user, user_type: 'user', did: `did:bns:${user}` },
      }))
    }
  }, { user: options.user, locale: options.chinese ? 'zh-CN' : 'en', theme: options.dark ? 'dark' : '' })
  await page.goto('/?aiccScenario=populated')
  await page.getByTestId('desktop-app-ai-center').click()
}

async function openPage(page: Page, name: string) {
  await page.getByRole('navigation').getByRole('button', { name, exact: true }).click()
}

function trackErrors(page: Page) {
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()) })
  return errors
}

test('administrator policies: layer editor, preview before saving, own loss, start of charging and per-user result', async ({ page }, testInfo) => {
  const errors = trackErrors(page)
  await openAiCenter(page)
  await expect(page.getByTestId('credit-home-card')).toContainText('150.00')
  await openPage(page, 'Policies')

  const subjects = page.getByTestId('policy-subject')
  await expect(subjects.filter({ hasText: 'Administrators' })).toContainText('Free')
  await expect(page.locator('[data-testid="policy-subject"][data-subject="type:user"]')).toContainText('×1')
  await page.screenshot({ path: testInfo.outputPath('policies.png'), fullPage: true })

  // Charging administrators: unpriced models stop working, for the viewer too.
  await page.locator('[data-testid="policy-subject"][data-subject="type:admin"]').click()
  const detail = page.getByTestId('policy-detail')
  await expect(detail).toHaveAttribute('data-subject', 'type:admin')
  await detail.getByTestId('policy-coefficient').getByRole('radio', { name: 'Charged', exact: true }).click()
  await detail.getByTestId('policy-review').click()
  const confirm = page.getByTestId('policy-confirm')
  await expect(confirm.getByTestId('policy-unpriced-warning')).toContainText('gpt-5.1-mini@openai-main')
  await expect(confirm.getByTestId('policy-self-loss')).toContainText('gpt-5.1-mini@openai-main')
  await page.screenshot({ path: testInfo.outputPath('policy-confirm-charging.png'), fullPage: true })
  await confirm.getByRole('button', { name: 'Cancel', exact: true }).click()
  await detail.getByRole('button', { name: 'Discard', exact: true }).click()

  // Users: hide one Provider; the preview lists what bob loses, then it is saved.
  await page.locator('[data-testid="policy-subject"][data-subject="type:user"]').click()
  const anthropic = detail.locator('[data-testid="policy-provider"][data-provider="anthropic-work"]')
  await anthropic.getByTestId('policy-provider-visibility').getByRole('radio', { name: 'Hidden', exact: true }).click()
  await detail.getByTestId('policy-review').click()
  await expect(confirm.getByTestId('policy-diff')).toContainText('claude-opus-4.7@anthropic-work')
  await expect(confirm.getByTestId('policy-self-loss')).toHaveCount(0)
  await confirm.getByTestId('policy-save').click()
  await expect(confirm).toHaveCount(0)
  await expect(page.locator('[data-testid="policy-subject"][data-subject="type:user"]')).toContainText('Hidden: 1')

  // The per-user result says where each value comes from.
  const effective = page.getByTestId('policy-effective')
  await effective.getByTestId('policy-effective-user').selectOption('bob')
  const result = effective.getByTestId('policy-effective-result')
  await expect(result.locator('[data-testid="policy-effective-model"][data-model="gpt-5.1@openai-main"]')).toContainText('Users')
  await expect(result.locator('[data-testid="policy-effective-model"][data-model="gpt-5.1-mini@openai-main"]')).toHaveAttribute('data-usable', 'false')
  await expect(result.locator('[data-testid="policy-effective-model"][data-model="claude-opus-4.7@anthropic-work"]')).toHaveCount(0)
  await expect(result.getByTestId('effective-limit').first()).toHaveAttribute('data-status', 'near_limit')
  await page.screenshot({ path: testInfo.outputPath('policy-effective.png'), fullPage: true })

  // A new user policy with a limit.
  await page.getByTestId('policy-add-user-open').click()
  await page.getByTestId('policy-add-user').selectOption('carol')
  await page.getByTestId('policy-add-confirm').click()
  await expect(detail).toHaveAttribute('data-subject', 'user:carol')
  await detail.getByTestId('policy-limits-add').click()
  await detail.getByTestId('policy-limits').getByTestId('limit-amount').fill('25')
  await detail.getByTestId('policy-review').click()
  await expect(confirm).toContainText('Applies to: carol')
  await confirm.getByTestId('policy-save').click()
  await expect(page.locator('[data-testid="policy-subject"][data-subject="user:carol"]')).toContainText('Limits: 1')

  expect(errors).toEqual([])
})

test('price list: unpriced notice and filter, model price, Credit rate', async ({ page }, testInfo) => {
  const errors = trackErrors(page)
  await openAiCenter(page)
  await openPage(page, 'Price list')
  await expect(page.getByTestId('prices-unpriced')).toContainText('3 models')
  await page.getByTestId('prices-show-unpriced').click()
  await expect(page.getByTestId('prices-filter')).toHaveValue('unpriced')
  const mini = page.locator('[data-testid="price-model"][data-key="model:openai:gpt-5.1-mini"]')
  await expect(mini).toBeVisible()
  await expect(page.locator('[data-testid="price-model"][data-key="model:openai:gpt-5.1"]')).toHaveCount(0)
  await expect(page.getByTestId('price-unmapped')).toHaveCount(2)

  await mini.getByTestId('price-edit').click()
  const dialog = page.getByTestId('price-dialog')
  await dialog.getByTestId('price-input').fill('2')
  await dialog.getByTestId('price-output').fill('8')
  await page.screenshot({ path: testInfo.outputPath('price-dialog.png'), fullPage: true })
  await dialog.getByTestId('price-save').click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByTestId('prices-unpriced')).toContainText('2 models')
  await page.getByTestId('prices-filter').selectOption('in_use')
  await expect(page.locator('[data-testid="price-model"][data-key="model:openai:gpt-5.1-mini"]').getByTestId('price-value')).toContainText('In 2 · Out 8')

  await page.getByTestId('price-rates-edit').click()
  const rates = page.getByTestId('price-rates-dialog')
  await rates.getByTestId('price-rate-add').click()
  await rates.getByTestId('price-rate-currency').fill('usd')
  await rates.getByTestId('price-rate-value').fill('100')
  await rates.getByTestId('price-rates-save').click()
  await expect(page.getByTestId('price-rate')).toHaveText('1 USD = 100 AI Credit')
  // The models left report a USD cost: the rate prices them now.
  await expect(page.getByTestId('prices-unpriced')).toHaveCount(0)
  await page.screenshot({ path: testInfo.outputPath('prices.png'), fullPage: true })
  expect(errors).toEqual([])
})

test('AI Credit: balances, recharge once with a preview, adjustment with a reason, ledger', async ({ page }, testInfo) => {
  const errors = trackErrors(page)
  await openAiCenter(page)
  await openPage(page, 'AI Credit')
  const bob = page.locator('[data-testid="credit-account"][data-user-id="bob"]')
  await expect(bob.getByTestId('credit-balance')).toHaveText('3.25')
  await expect(page.locator('[data-testid="credit-account"][data-user-id="dave"]').getByTestId('credit-balance')).toHaveText('-2.50')
  await expect(page.getByTestId('credit-negative')).toBeVisible()

  await bob.getByTestId('credit-recharge-open').click()
  const recharge = page.getByTestId('credit-recharge')
  await expect(recharge.getByTestId('credit-entry-id')).toContainText('rc-')
  await recharge.getByTestId('credit-amount').fill('10')
  await expect(recharge.getByTestId('credit-after')).toContainText('13.25')
  await page.screenshot({ path: testInfo.outputPath('credit-recharge.png'), fullPage: true })
  await recharge.getByTestId('credit-submit').click()
  await expect(recharge.getByTestId('credit-entry-done')).toBeVisible()
  await recharge.getByRole('button', { name: 'Close', exact: true }).click()
  await expect(bob.getByTestId('credit-balance')).toHaveText('13.25')

  const dave = page.locator('[data-testid="credit-account"][data-user-id="dave"]')
  await dave.getByTestId('credit-adjust-open').click()
  const adjust = page.getByTestId('credit-adjust')
  await adjust.getByTestId('credit-amount').fill('-1')
  await expect(adjust.getByTestId('credit-submit')).toBeDisabled()
  await adjust.getByTestId('credit-note').fill('Correction')
  await adjust.getByTestId('credit-submit').click()
  await expect(adjust.getByTestId('credit-entry-done')).toBeVisible()
  await adjust.getByRole('button', { name: 'Close', exact: true }).click()
  await expect(dave.getByTestId('credit-balance')).toHaveText('-3.50')

  await bob.getByRole('button', { name: 'bob', exact: true }).click()
  const ledger = page.getByTestId('credit-ledger-panel')
  await expect(ledger.getByTestId('ledger-item').first()).toHaveAttribute('data-type', 'recharge')
  await expect(ledger.locator('[data-testid="ledger-item"][data-type="charge"]')).toHaveCount(3)
  await page.screenshot({ path: testInfo.outputPath('credit-ledger.png'), fullPage: true })
  expect(errors).toEqual([])
})

test('a user: own balance and limits on Home, prices on Models, own App policy, no price list', async ({ page }, testInfo) => {
  const errors = trackErrors(page)
  await openAiCenter(page, { user: 'bob' })
  const card = page.getByTestId('credit-home-card')
  await expect(card.getByTestId('credit-home-balance')).toHaveText('3.25')
  await expect(card).toHaveAttribute('data-warnings', '1')
  await expect(page.getByRole('navigation').getByRole('button', { name: 'Price list', exact: true })).toHaveCount(0)

  await openPage(page, 'Models')
  await expect(page.locator('[data-testid="model-card"][data-model="gpt-5.1-mini"]').getByTestId('credit-price')).toHaveAttribute('data-status', 'unpriced')
  await expect(page.locator('[data-testid="model-card"][data-model="gpt-5.1"]').getByTestId('credit-price')).toContainText('In 10 · Out 40')
  await expect(page.getByTestId('models-unpriced')).toHaveCount(0)

  await openPage(page, 'Policies')
  await expect(page.locator('[data-testid="policy-subject"][data-subject="user:bob"]')).toContainText('Limits: 1')
  await page.locator('[data-testid="policy-subject"][data-subject="type:user"]').click()
  await expect(page.getByTestId('policy-body-summary')).toBeVisible()
  await expect(page.getByTestId('policy-review')).toHaveCount(0)
  await page.getByTestId('policy-new-own').click()
  await page.getByTestId('policy-add-app').fill('studio')
  await page.getByTestId('policy-add-confirm').click()
  const detail = page.getByTestId('policy-detail')
  await expect(detail).toHaveAttribute('data-subject', 'user_app:bob/studio')
  await expect(detail.getByTestId('policy-editor')).toHaveAttribute('data-mode', 'own_app')
  await expect(detail.getByTestId('policy-coefficient')).toHaveCount(0)
  await expect(detail.getByTestId('policy-provider-visibility').first().getByRole('radio', { name: 'All models', exact: true })).toHaveCount(0)
  await detail.getByTestId('policy-limits-add').click()
  await detail.getByTestId('policy-limits').getByTestId('limit-period').selectOption('5h')
  await detail.getByTestId('policy-limits').getByTestId('limit-amount').fill('2')
  await detail.getByTestId('policy-review').click()
  await page.getByTestId('policy-confirm').getByTestId('policy-save').click()
  await expect(page.locator('[data-testid="policy-subject"][data-subject="user_app:bob/studio"][data-source="user"]')).toContainText('Limits: 1')
  await page.screenshot({ path: testInfo.outputPath('policies-user.png'), fullPage: true })

  await openPage(page, 'AI Credit')
  await expect(page.getByTestId('credit-own-balance')).toHaveText('3.25')
  await expect(page.getByTestId('ledger-item').first()).toBeVisible()
  expect(errors).toEqual([])
})

test('Usage: AI Credit per event and user, usage by user group, a breakdown row opens its policy', async ({ page }) => {
  const errors = trackErrors(page)
  await openAiCenter(page)
  await expect(page.getByTestId('usage-by-user').locator('[data-user-id="bob"]').getByTestId('user-usage-credit')).not.toHaveText('0.00')
  const groups = page.getByTestId('usage-by-group')
  await expect(groups.locator('[data-subject="type:user"]')).toBeVisible()
  await expect(groups.locator('[data-subject="type:admin"]')).toBeVisible()

  await page.getByTestId('usage-scope').selectOption('user:bob')
  await expect(page.getByTestId('usage-by-group')).toHaveCount(0)
  await page.getByTestId('usage-set-policy').first().click()
  const detail = page.getByTestId('policy-detail')
  await expect(detail).toHaveAttribute('data-subject', 'user:bob')

  await openPage(page, 'Usage')
  await page.getByTestId('usage-scope').selectOption('user:bob')
  await expect(page.getByTestId('usage-event-credit').filter({ visible: true }).first()).toBeVisible()
  expect(errors).toEqual([])
})

test('Playground explains an AI Credit refusal apart from a Provider balance', async ({ page }) => {
  await page.route('**/src/api/aicc_playground.ts*', (route) => route.fulfill({ contentType: 'application/javascript', body: `
    export async function listPlaygroundModels() { return [{ exact_model: 'gpt-5.1@openai-main', provider: 'openai-main', api_types: ['llm'] }] }
    export async function invokePlayground() {
      throw Object.assign(new Error('credit_insufficient'), { code: 'credit_insufficient', data: { code: 'credit_insufficient', details: { balance: '-2.5', provider_instance_name: 'openai-main', exact_model: 'gpt-5.1@openai-main' } } })
    }
    export async function getPlaygroundTask() { return {} }
    export async function cancelPlaygroundTask() { return {} }
  ` }))
  await openAiCenter(page, { user: 'bob' })
  await openPage(page, 'Playground')
  const model = page.getByRole('combobox', { name: 'Exact model', exact: true })
  await expect(model.locator('option[value="gpt-5.1@openai-main"]')).toContainText('In 10 · Out 40')
  await model.selectOption('gpt-5.1@openai-main')
  await page.getByRole('textbox', { name: 'messages[0].content[0].text', exact: true }).fill('Hello')
  await page.getByRole('button', { name: 'Run request', exact: true }).click()
  const note = page.getByTestId('policy-refusal')
  await expect(note).toHaveAttribute('data-code', 'credit_insufficient')
  await expect(note).toContainText('AI Credit balance is -2.50')
  await expect(note).toContainText('not the Provider account balance')
})

test('Add Provider: open it, see how much is priced and make it free', async ({ page }, testInfo) => {
  const errors = trackErrors(page)
  await openAiCenter(page)
  await openPage(page, 'Providers')
  await page.getByRole('button', { name: 'Add Provider' }).click()
  await page.getByRole('button', { name: /Google Gemini/ }).click()
  await page.getByRole('button', { name: 'Next', exact: true }).click()
  await page.getByPlaceholder(/^sk-|API/).first().fill('mock-key')
  await page.getByRole('button', { name: 'Next', exact: true }).click()
  await expect(page.getByRole('button', { name: 'Next', exact: true })).toBeEnabled()
  await page.getByRole('button', { name: 'Next', exact: true }).click()
  await expect(page.getByTestId('wizard-coverage')).toBeVisible()
  await page.getByRole('button', { name: 'Next', exact: true }).click()
  const access = page.getByTestId('provider-access')
  await expect(access).toContainText('Gets it automatically (All Providers): Administrators, Users')
  await expect(access.locator('[data-testid="provider-access-subject"][data-subject="type:limited"]')).toBeVisible()
  await page.screenshot({ path: testInfo.outputPath('wizard-access.png'), fullPage: true })
  await page.getByRole('button', { name: 'Create Provider', exact: true }).click()
  const created = page.getByTestId('provider-created')
  await expect(created.getByTestId('provider-price-coverage')).toContainText('0 of its 2 models have an AI Credit price, 2 do not')
  await created.getByTestId('provider-make-free').click()
  await expect(created.getByTestId('provider-make-free')).toHaveCount(0)
  await expect(created).toContainText('It is free for everyone who can use it.')
  await page.screenshot({ path: testInfo.outputPath('wizard-created.png'), fullPage: true })
  await created.getByTestId('provider-created-done').click()

  await openPage(page, 'Policies')
  await page.locator('[data-testid="policy-subject"][data-subject="type:user"]').click()
  const gemini = page.locator('[data-testid="policy-provider"][data-provider="google-gemini-main"]')
  await expect(gemini.getByTestId('policy-provider-coefficient').getByRole('radio', { name: 'Free', exact: true })).toHaveAttribute('aria-checked', 'true')
  expect(errors).toEqual([])
})

test('dark theme policies and credit pages', async ({ page }, testInfo) => {
  await openAiCenter(page, { dark: true })
  await openPage(page, 'Policies')
  await expect(page.getByTestId('policy-detail')).toBeVisible()
  await page.screenshot({ path: testInfo.outputPath('policies-dark.png'), fullPage: true })
  await openPage(page, 'Price list')
  await expect(page.getByTestId('prices-models')).toBeVisible()
  await page.screenshot({ path: testInfo.outputPath('prices-dark.png'), fullPage: true })
  await openPage(page, 'AI Credit')
  await expect(page.getByTestId('credit-account').first()).toBeVisible()
  await page.screenshot({ path: testInfo.outputPath('credit-dark.png'), fullPage: true })
})

test.describe('touch viewport', () => {
  test.use({ hasTouch: true })

  test('mobile Chinese policies, price list and AI Credit fit the viewport', async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 375, height: 812 })
    await page.addInitScript(() => localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
    await page.goto('/?aiccScenario=populated')
    await page.getByTestId('desktop-app-ai-center').tap()
    await page.getByRole('button', { name: '策略', exact: true }).tap()
    const policies = page.getByTestId('policies-page')
    await expect(policies.getByRole('heading', { name: '策略' })).toBeVisible()
    await policies.locator('[data-testid="policy-subject"][data-subject="type:user"]').tap()
    await expect(page.getByTestId('policy-detail')).toBeVisible()
    expect(await policies.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
    await page.screenshot({ path: testInfo.outputPath('policies-mobile.png'), fullPage: true })

    await page.getByRole('button', { name: '价目表', exact: true }).tap()
    const prices = page.getByTestId('prices-page')
    await expect(prices.getByTestId('price-model').first()).toBeVisible()
    expect(await prices.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
    await page.screenshot({ path: testInfo.outputPath('prices-mobile.png'), fullPage: true })

    await page.getByRole('button', { name: 'AI 点数', exact: true }).tap()
    const credit = page.getByTestId('credit-page')
    await expect(credit.getByTestId('credit-account').first()).toBeVisible()
    expect(await credit.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
    await credit.getByTestId('credit-recharge-open').first().tap()
    await expect(page.getByTestId('credit-recharge')).toBeVisible()
    await page.screenshot({ path: testInfo.outputPath('credit-mobile.png'), fullPage: true })
  })
})
