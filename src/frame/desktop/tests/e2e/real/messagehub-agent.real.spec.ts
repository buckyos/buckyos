import { expect, test } from '@playwright/test'

const baseURL = process.env.MESSAGEHUB_REAL_BASE_URL || 'https://test.buckyos.io'
const zoneHost = new URL(baseURL).hostname
const zoneIp = process.env.MESSAGEHUB_REAL_ZONE_IP
const agentDid = process.env.BUCKYOS_TEST_AGENT_DID || `did:web:jarvis.${zoneHost}`

test.use({
  baseURL,
  ignoreHTTPSErrors: true,
  launchOptions: zoneIp ? {
    args: [`--host-resolver-rules=MAP ${zoneHost} ${zoneIp}, MAP *.${zoneHost} ${zoneIp}`, '--no-proxy-server'],
  } : {},
})

test('New Session includes the installed Jarvis agent', async ({ page }) => {
  test.skip(process.env.MESSAGEHUB_REAL_E2E !== '1', 'Requires a running development zone.')
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto('/messagehub')
  await page.getByLabel('Username', { exact: true }).fill(process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest')
  await page.getByLabel('Password', { exact: true }).fill(process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025')
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  await page.getByRole('button', { name: 'New Session', exact: true }).first().click()
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('combobox', { name: 'Entity', exact: true }).selectOption(agentDid)
  await expect(dialog.getByRole('combobox', { name: 'Connection', exact: true })).toHaveValue('native')
  await expect(dialog.getByRole('button', { name: 'Create', exact: true })).toBeEnabled()
  await page.screenshot({ path: 'test-results-real/new-session-jarvis.png' })
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click()
  expect(errors).toEqual([])
})
