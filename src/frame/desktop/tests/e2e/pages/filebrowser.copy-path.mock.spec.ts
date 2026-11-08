import { test, expect } from '@playwright/test'

test('Copy path puts the cyfs:/// URL on the clipboard', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.goto('/?scenario=normal&fbData=mock')
  await page.getByTestId('desktop-app-files').click()
  const win = page.getByTestId('window-files')
  await win.locator('aside').first().getByRole('button', { name: 'Documents', exact: true }).click()
  const clipboard = () => page.evaluate(() => navigator.clipboard.readText())

  await win.getByRole('cell', { name: 'Kyoto Trip Plan.md', exact: true }).click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Copy path', exact: true }).click()
  await expect.poll(clipboard).toMatch(/^cyfs:\/\/\/home\/.*\/Kyoto Trip Plan\.md$/)

  await win.getByRole('button', { name: 'Copy path', exact: true }).first().click()
  await expect.poll(clipboard).toMatch(/^cyfs:\/\/\/home\/.*Documents$/)
})
