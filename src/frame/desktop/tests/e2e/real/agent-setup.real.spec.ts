import { expect, test } from '@playwright/test'

/**
 * Agent setup against a real zone, as an ordinary user that has no Agent yet:
 * the desktop Jarvis guide opens the wizard, the Agent is created through the
 * real control panel, and the guide turns into the Agent's entry.
 *
 *   AGENT_SETUP_REAL_E2E=1 AGENT_SETUP_USER=<user> AGENT_SETUP_PASSWORD=<pw> \
 *   pnpm exec playwright test --config=playwright.real.config.ts agent-setup.real
 */
const baseURL = process.env.MESSAGEHUB_REAL_BASE_URL || 'https://test.buckyos.io'
const user = process.env.AGENT_SETUP_USER || 'dave'
const password = process.env.AGENT_SETUP_PASSWORD || 'dave2025'
const nickname = process.env.AGENT_SETUP_NICKNAME || '小戴'
const shots = process.env.AGENT_SETUP_SHOTS || 'test-results-real'

test.use({ baseURL, ignoreHTTPSErrors: true, viewport: { width: 1440, height: 900 } })

test('the desktop Jarvis guide creates the user\'s own Agent', async ({ page }) => {
  test.skip(process.env.AGENT_SETUP_REAL_E2E !== '1', 'Requires a running development zone.')
  test.setTimeout(600_000)
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))

  await page.goto('/')
  await page.getByLabel('Username', { exact: true }).fill(user)
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()

  const guide = page.getByTestId('desktop-app-agent-guide')
  await expect(guide).toHaveAttribute('title', 'Jarvis', { timeout: 60_000 })
  await page.screenshot({ path: `${shots}/agent-guide-before.png` })
  await guide.click()
  const wizard = page.getByTestId('window-agent-setup')
  await expect(wizard).toBeVisible()

  // `jarvis` is taken in this zone: the wizard switches to the suggested name.
  const name = wizard.getByTestId('agent-setup-name')
  await expect(name).toHaveValue(`${user}-jarvis`, { timeout: 30_000 })
  await expect(wizard.getByTestId('agent-setup-did')).toHaveText(new RegExp(`${user}-jarvis\\.`))
  await wizard.getByTestId('agent-setup-nickname').fill(nickname)
  await page.screenshot({ path: `${shots}/agent-setup-step1.png` })
  await wizard.getByTestId('agent-setup-next').click()
  await expect(wizard.getByRole('switch', { name: 'Allow other users to use this Agent' })).toBeDisabled()
  await wizard.getByTestId('agent-setup-next').click()

  await expect(wizard.getByRole('combobox', { name: 'Agent Loader' })).toHaveText('OpenDAN')
  await expect(wizard.getByRole('combobox', { name: 'Agent template' })).toHaveText('Jarvis · Built in')
  await page.screenshot({ path: `${shots}/agent-setup-step2.png` })
  await wizard.getByTestId('agent-setup-next').click()

  await expect(wizard.getByTestId('agent-setup-identity-missing')).toBeVisible()
  await wizard.getByTestId('agent-setup-skip-channel').click()
  await expect(wizard.getByTestId('agent-setup-confirm-name')).toHaveText(nickname)
  await wizard.getByTestId('agent-setup-create').click()

  await expect(wizard.getByTestId('agent-setup-status')).toBeVisible()
  await page.screenshot({ path: `${shots}/agent-setup-creating.png` })
  await expect(wizard.getByTestId('agent-setup-status-success')).toBeVisible({ timeout: 300_000 })
  await page.screenshot({ path: `${shots}/agent-setup-success.png` })
  await wizard.getByTestId('agent-setup-close').click()
  await expect(wizard).toHaveCount(0)

  await expect(guide).toHaveAttribute('title', nickname, { timeout: 30_000 })
  await page.screenshot({ path: `${shots}/agent-guide-after.png` })
  expect(errors).toEqual([])
})
