import { expect, test } from '@playwright/test'

// Minimizing a desktop window must hide it, not unmount it: an embedded web
// app (iframe) would otherwise reload and lose its state on every restore.
test('desktop minimize keeps the window content mounted', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('button', { name: 'Settings' }).first().click()
  const win = page.getByTestId('window-settings')
  await expect(win).toBeVisible()
  await expect(page.getByPlaceholder('Search settings')).toBeVisible()
  await win.evaluate((el) => { (el as HTMLElement).dataset.marker = 'mounted-before-minimize' })

  await win.getByRole('button', { name: 'Minimize' }).click()
  await expect(win).toBeHidden()
  await expect(page.getByPlaceholder('Search settings')).toBeHidden()
  await expect(win).toHaveCount(1)

  await page.getByRole('button', { name: 'Settings' }).first().click()
  await expect(win).toBeVisible()
  await expect(page.getByPlaceholder('Search settings')).toBeVisible()
  await expect(win).toHaveAttribute('data-marker', 'mounted-before-minimize')
})

// Raising a window must not move window DOM nodes: React reorders keyed
// siblings with insertBefore, and a moved <iframe> reloads its page.
test('desktop focus changes keep window DOM nodes in place', async ({ page }) => {
  await page.goto('/')
  await page.getByRole('button', { name: 'Settings' }).first().click()
  const settings = page.getByTestId('window-settings')
  await expect(settings).toBeVisible()
  await settings.getByRole('button', { name: 'Minimize' }).click()
  await page.getByRole('button', { name: 'Files' }).first().click()
  const files = page.getByTestId('window-files')
  await expect(files).toBeVisible()
  await page.getByRole('button', { name: 'Settings' }).first().click()
  await expect(settings).toBeVisible()
  await files.evaluate((el) => { (el as HTMLElement).dataset.marker = 'same-node' })
  const order = () => page.evaluate(() => [...document.querySelectorAll('[data-testid^="window-"]')].map((el) => el.getAttribute('data-testid')).filter((id) => id === 'window-settings' || id === 'window-files'))
  const before = await order()

  // Windows overlap, so raise them through events on their title bars
  // instead of pointer coordinates.
  const raise = async (id: string) => {
    const bar = page.getByTestId(`window-drag-${id}`)
    await bar.dispatchEvent('pointerdown', { bubbles: true, button: 0, pointerId: 1, pointerType: 'mouse' })
    await bar.dispatchEvent('pointerup', { bubbles: true, button: 0, pointerId: 1, pointerType: 'mouse' })
    await bar.dispatchEvent('click', { bubbles: true })
  }
  await raise('files')
  await raise('settings')
  await raise('files')

  expect(await order()).toEqual(before)
  await expect(files).toHaveAttribute('data-marker', 'same-node')
  const z = await page.evaluate(() => [document.querySelector('[data-testid="window-files"]'), document.querySelector('[data-testid="window-settings"]')].map((el) => Number((el as HTMLElement).style.zIndex)))
  expect(z[0]).toBeGreaterThan(z[1])
})
