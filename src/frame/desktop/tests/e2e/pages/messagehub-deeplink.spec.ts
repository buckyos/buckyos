import { expect, test, type Page } from '@playwright/test'

const CODER = 'did:buckyos:agent:codeassistant'

async function activeSession(page: Page) {
  const toggle = page.getByRole('button', { name: 'Sessions', exact: true })
  await expect(toggle).toBeVisible()
  if (!(await page.getByTestId('session-sidebar').isVisible())) await toggle.click()
  return page.locator('[data-testid="session-row"]:has(button[aria-current="true"])')
}

test('a sessionId in the address opens that session instead of the default one', async ({ page }) => {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(CODER)}&sessionId=session-coder-2`)
  await expect(await activeSession(page)).toHaveAttribute('data-session-id', 'session-coder-2')
  // The requested session stays selected after the default session resolves.
  await page.waitForTimeout(500)
  await expect(await activeSession(page)).toHaveAttribute('data-session-id', 'session-coder-2')
})

test('a sessionId without an entity opens the session with its own entity', async ({ page }) => {
  await page.goto('/messagehub?sessionId=session-alice-1')
  await expect(await activeSession(page)).toHaveAttribute('data-session-id', 'session-alice-1')
})

test('an unknown sessionId falls back to the default session', async ({ page }) => {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(CODER)}&sessionId=nope`)
  await expect(await activeSession(page)).toHaveAttribute('data-session-id', 'session-coder-1')
})
