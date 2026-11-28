import { expect, test, type Page } from '@playwright/test'

// The usecase table, routing groups and the Add Provider coverage step against the mock usecase world
// (src/app/ai-center/mock/usecases.ts): in the populated scenario the Zone default locks llm.plan to
// gpt-5.1@openai-main and gives llm.code the "next model" policy; no mock model meets llm.vision.

async function openAiCenter(page: Page, options: { user?: string; populated?: boolean } = {}) {
  await page.addInitScript(({ user }) => {
    localStorage.setItem('buckyos.prototype.locale.v1', 'en')
    if (user) {
      localStorage.setItem('buckyos.mock.control-panel.v1', JSON.stringify({
        account: { user_id: user, user_name: user, user_type: 'user', did: `did:bns:${user}` },
      }))
    }
  }, { user: options.user })
  await page.goto(options.populated === false ? '/' : '/?aiccScenario=populated')
  await page.getByTestId('desktop-app-ai-center').click()
}

async function openPage(page: Page, name: string) {
  const nav = page.getByRole('navigation')
  const target = await nav.count() > 0 ? nav : page
  await target.getByRole('button', { name, exact: true }).click()
}

function trackErrors(page: Page) {
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()) })
  return errors
}

const row = (page: Page, usecase: string) => page.locator(`[data-testid="usecase-row"][data-usecase="${usecase}"]`)
const option = (page: Page, kind: 'model' | 'policy', value: string) => page.getByTestId('usecase-edit').locator(`[data-testid="usecase-${kind}-option"][data-value="${value}"]`)

async function closePanel(page: Page) {
  await page.getByTestId('usecase-edit').getByRole('button', { name: 'Close', exact: true }).last().click()
  await expect(page.getByTestId('usecase-edit')).toHaveCount(0)
}

test('usecase table: sources, locks and coverage; edit a usecase, Zone default with a lock, fallback LLM', async ({ page }, testInfo) => {
  const errors = trackErrors(page)
  await openAiCenter(page)
  await openPage(page, 'Usecases')

  await expect(page.getByTestId('usecase-row')).toHaveCount(29)
  await expect(row(page, 'llm.plan').getByTestId('usecase-source')).toHaveText('Zone default')
  await expect(row(page, 'llm.plan').getByTestId('usecase-lock')).toBeVisible()
  await expect(row(page, 'llm.code').getByTestId('usecase-policy')).toContainText('Next model')
  await expect(row(page, 'llm.code').getByTestId('usecase-policy')).toContainText('would switch to gpt-5.1@openai-main')
  await expect(row(page, 'llm.vision')).toHaveAttribute('data-coverage', 'uncovered')
  await expect(row(page, 'llm.vision').getByTestId('usecase-reason')).toContainText('none meets what this usecase needs')
  await expect(row(page, 'video.generate').getByTestId('usecase-reason')).toContainText('No connected Provider offers a model of this type')
  await page.getByTestId('usecase-filter').getByRole('radio', { name: /^Needs attention/ }).click()
  await expect(row(page, 'llm.chat')).toHaveCount(0)
  await expect(row(page, 'llm.vision')).toBeVisible()
  await page.getByTestId('usecase-filter').getByRole('radio', { name: /^All/ }).click()
  await page.screenshot({ path: testInfo.outputPath('usecases.png'), fullPage: true })

  // My own layer: another model and a policy; the explanation says how prices are judged.
  await row(page, 'llm.chat').click()
  const panel = page.getByTestId('usecase-edit')
  await expect(panel.getByTestId('usecase-save')).toBeDisabled()
  await option(page, 'model', 'gpt-5.1-mini@openai-main').click()
  await option(page, 'policy', 'not_pricier').click()
  await expect(option(page, 'policy', 'not_pricier')).toContainText('models without a known price are never chosen as “not pricier”')
  await expect(option(page, 'policy', 'next_model')).toContainText('models without a known price are never chosen as “not pricier”')
  await page.screenshot({ path: testInfo.outputPath('usecase-edit.png'), fullPage: true })
  await panel.getByTestId('usecase-save').click()
  await expect(panel.getByTestId('usecase-edit-saved')).toBeVisible()
  await expect(panel.getByTestId('usecase-edit-prediction')).toContainText('no model with a known price that is not higher')
  await closePanel(page)
  await expect(row(page, 'llm.chat').getByTestId('usecase-model')).toHaveText('gpt-5.1-mini@openai-main')
  await expect(row(page, 'llm.chat').getByTestId('usecase-source')).toHaveText('Me')
  await expect(row(page, 'llm.chat').getByTestId('usecase-policy')).toHaveAttribute('data-policy', 'not_pricier')

  // A cell the administrator locked cannot be changed in my layer.
  await row(page, 'llm.plan').click()
  await expect(panel.getByTestId('usecase-edit-locked').first()).toBeVisible()
  await expect(option(page, 'model', 'gpt-5.1-mini@openai-main').locator('input')).toBeDisabled()
  await closePanel(page)

  // Zone default: my layer is not part of it; lock the policy of llm.chat for everyone.
  await page.getByTestId('usecase-view-select').selectOption('zone')
  await expect(page.getByTestId('usecase-view')).toHaveAttribute('data-view', 'zone')
  await expect(row(page, 'llm.chat').getByTestId('usecase-source')).toHaveText('Default')
  await row(page, 'llm.chat').click()
  await option(page, 'policy', 'same_model').click()
  await panel.getByTestId('usecase-lock-policy').check()
  await panel.getByTestId('usecase-save').click()
  await expect(panel.getByTestId('usecase-edit-saved')).toBeVisible()
  await closePanel(page)
  await expect(row(page, 'llm.chat').getByTestId('usecase-policy')).toHaveAttribute('data-policy', 'same_model')
  await page.getByTestId('usecase-view-select').selectOption('self')
  const chatPolicy = row(page, 'llm.chat').getByTestId('usecase-policy')
  await expect(chatPolicy).toHaveAttribute('data-policy', 'same_model')
  await expect(chatPolicy.getByTestId('usecase-lock')).toBeVisible()
  await expect(chatPolicy).toContainText('no other model to switch to')

  // Follow the lower layer again: the default model comes back.
  await row(page, 'llm.chat').click()
  await option(page, 'model', 'follow').click()
  await panel.getByTestId('usecase-save').click()
  await expect(panel.getByTestId('usecase-edit-saved')).toBeVisible()
  await closePanel(page)
  await expect(row(page, 'llm.chat').getByTestId('usecase-source')).toHaveText('Default')
  await expect(row(page, 'llm.chat').getByTestId('usecase-model')).toHaveText('claude-sonnet-4.5@anthropic-work')

  // The fallback LLM serves the LLM usecase without a model once an administrator turns it on.
  const fallback = page.getByTestId('fallback-llm')
  await expect(fallback).toHaveAttribute('data-enabled', 'false')
  await fallback.getByTestId('fallback-llm-switch').click()
  await expect(fallback).toHaveAttribute('data-enabled', 'true')
  await expect(row(page, 'llm.vision')).toHaveAttribute('data-coverage', 'fallback_llm')
  await expect(row(page, 'llm.vision').getByTestId('usecase-source')).toHaveText('Fallback LLM')
  await expect(row(page, 'llm.vision').getByTestId('usecase-model')).toHaveText('sn-router-balanced@sn-router')
  await expect(row(page, 'video.generate')).toHaveAttribute('data-coverage', 'uncovered')

  await openPage(page, 'Home')
  const health = page.getByTestId('usecase-health')
  await expect(health.getByTestId('home-uncovered')).toContainText('Usecases without a model: 21')
  await expect(health.getByTestId('home-uncovered')).toContainText('Served by the fallback LLM: 1')
  await expect(health.getByTestId('home-switches')).toContainText('Recent switches by an unavailable policy: 1')
  await page.screenshot({ path: testInfo.outputPath('home.png'), fullPage: true })
  await health.getByTestId('home-switches').click()
  await expect(page.getByTestId('usage-fallback-only')).toBeChecked()
  await expect(page.getByTestId('usage-fallback-from').first()).toContainText('Switched from claude-sonnet-4.5@anthropic-work')
  expect(errors).toEqual([])
})

test('an ordinary user changes only their own layer', async ({ page }) => {
  const errors = trackErrors(page)
  await openAiCenter(page, { user: 'bob' })
  await openPage(page, 'Usecases')
  await expect(page.getByTestId('usecase-view-select').locator('option[value="zone"]')).toHaveCount(0)
  await expect(page.getByTestId('fallback-llm-switch')).toHaveCount(0)
  await expect(page.getByTestId('fallback-llm')).toContainText('Only an administrator can turn it on or off.')
  await row(page, 'llm.summarize').click()
  await option(page, 'model', 'gpt-5.1@openai-main').click()
  await page.getByTestId('usecase-save').click()
  await expect(page.getByTestId('usecase-edit-saved')).toBeVisible()
  await closePanel(page)
  await expect(row(page, 'llm.summarize').getByTestId('usecase-source')).toHaveText('Me')
  await expect(row(page, 'llm.summarize').getByTestId('usecase-model')).toHaveText('gpt-5.1@openai-main')
  expect(errors).toEqual([])
})

test('routing groups: create, edit its usecases, use it for an App, see the App view, delete', async ({ page }, testInfo) => {
  const errors = trackErrors(page)
  await openAiCenter(page)
  await openPage(page, 'Routing groups')
  await page.getByRole('button', { name: 'New group' }).click()
  await page.getByTestId('routing-group-name').fill('Coding agent')
  await page.getByTestId('routing-group-name-save').click()
  const detail = page.getByTestId('routing-group-detail')
  await expect(detail).toContainText('Coding agent')
  const groupId = await detail.getAttribute('data-group')
  await expect(detail.getByTestId('usecase-view')).toHaveAttribute('data-view', `group:${groupId}`)

  await row(page, 'llm.code').click()
  await option(page, 'model', 'gpt-5.1@openai-main').click()
  await page.getByTestId('usecase-save').click()
  await expect(page.getByTestId('usecase-edit-saved')).toBeVisible()
  await closePanel(page)
  await expect(row(page, 'llm.code').getByTestId('usecase-source')).toHaveText('Group Coding agent')

  await detail.getByTestId('routing-group-bind').click()
  await page.getByTestId('routing-group-bind-app').selectOption('aiworkspace')
  await page.getByTestId('routing-group-bind-confirm').click()
  await expect(detail.locator('[data-testid="routing-group-app"][data-app="aiworkspace"]')).toBeVisible()
  await page.screenshot({ path: testInfo.outputPath('group.png'), fullPage: true })
  await detail.getByRole('button', { name: 'All groups' }).click()
  await expect(page.getByTestId('routing-group')).toContainText('Usecases set: 1 · Apps: 1')

  await openPage(page, 'Usecases')
  await page.getByTestId('usecase-view-select').selectOption('app:aiworkspace')
  await expect(page.getByTestId('usecase-view')).toHaveAttribute('data-can-edit', 'false')
  await expect(page.getByTestId('usecase-view-hint')).toContainText('Coding agent')
  await expect(row(page, 'llm.code').getByTestId('usecase-source')).toHaveText('Group Coding agent')
  await row(page, 'llm.code').click()
  await expect(page.getByTestId('usecase-edit-readonly')).toBeVisible()
  await expect(page.getByTestId('usecase-save')).toHaveCount(0)
  await page.keyboard.press('Escape')

  await openPage(page, 'Routing groups')
  await page.getByTestId('routing-group').click()
  await page.getByTestId('routing-group-delete').click()
  await page.getByRole('button', { name: 'Delete', exact: true }).last().click()
  await expect(page.getByText('No routing groups yet')).toBeVisible()
  expect(errors).toEqual([])
})

test('Add Provider shows the usecase coverage and keeps an unchecked usecase on its old model', async ({ page }, testInfo) => {
  const errors = trackErrors(page)
  await openAiCenter(page, { populated: false })
  const addProvider = async (name: RegExp) => {
    await openPage(page, 'Providers')
    await page.getByRole('button', { name: 'Add Provider' }).first().click()
    await page.getByRole('button', { name }).click()
    await page.getByRole('button', { name: 'Next', exact: true }).click()
    await page.getByPlaceholder(/^sk-|API/).first().fill('mock-key')
    await page.getByRole('button', { name: 'Next', exact: true }).click()
    await expect(page.getByRole('button', { name: 'Next', exact: true })).toBeEnabled()
    await page.getByRole('button', { name: 'Next', exact: true }).click()
    return page.getByTestId('wizard-coverage')
  }

  let coverage = await addProvider(/^OpenAI/)
  await expect(coverage.getByTestId('coverage-newly').locator('[data-usecase="llm.chat"]')).toContainText('gpt-5.1@openai-main')
  await expect(coverage.getByTestId('coverage-changed')).toHaveCount(0)
  await expect(coverage.getByTestId('coverage-uncovered').locator('[data-usecase="video.generate"]')).toBeVisible()
  await page.getByRole('button', { name: 'Next', exact: true }).click()
  await page.getByRole('button', { name: 'Create Provider', exact: true }).click()
  await page.getByTestId('provider-created-done').click()

  coverage = await addProvider(/Anthropic Claude/)
  const plan = coverage.getByTestId('coverage-changed').locator('[data-usecase="llm.plan"]')
  await expect(plan).toContainText('gpt-5.1@openai-main')
  await expect(plan).toContainText('claude-opus-4.7@claude-main')
  await plan.getByTestId('coverage-use-new').uncheck()
  await expect(plan).toContainText('Keeps gpt-5.1@openai-main (saved in the Zone default)')
  await page.screenshot({ path: testInfo.outputPath('wizard-coverage.png'), fullPage: true })
  await page.getByRole('button', { name: 'Next', exact: true }).click()
  await expect(page.getByText(/Model changes: \d+ \(kept: 1\)/)).toBeVisible()
  await page.getByRole('button', { name: 'Create Provider', exact: true }).click()
  await expect(page.getByTestId('provider-kept-models')).toContainText('1')
  await page.getByTestId('provider-created-done').click()

  await openPage(page, 'Usecases')
  await expect(row(page, 'llm.plan').getByTestId('usecase-model')).toHaveText('gpt-5.1@openai-main')
  await expect(row(page, 'llm.plan').getByTestId('usecase-source')).toHaveText('Zone default')
  await expect(row(page, 'llm.code').getByTestId('usecase-model')).toHaveText('claude-sonnet-4.5@claude-main')
  expect(errors).toEqual([])
})

test.describe('touch viewport', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true })

  test('the usecase table is one card per usecase and the editor fits the phone', async ({ page }, testInfo) => {
    const errors = trackErrors(page)
    await openAiCenter(page)
    await page.getByRole('button', { name: 'Usecases', exact: true }).click()
    await expect(row(page, 'llm.chat')).toBeVisible()
    await expect(page.locator('table[data-testid="usecase-table"], [data-testid="usecase-table"] table')).toHaveCount(0)
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
    await page.screenshot({ path: testInfo.outputPath('usecases-phone.png'), fullPage: true })
    await row(page, 'llm.translate').click()
    await option(page, 'model', 'gpt-5.1-mini@openai-main').click()
    await page.getByTestId('usecase-save').click()
    await expect(page.getByTestId('usecase-edit-saved')).toBeVisible()
    await page.screenshot({ path: testInfo.outputPath('usecase-edit-phone.png') })
    await closePanel(page)
    await expect(row(page, 'llm.translate').getByTestId('usecase-source')).toHaveText('Me')
    expect(errors).toEqual([])
  })
})
