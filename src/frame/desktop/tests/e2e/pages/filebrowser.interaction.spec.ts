/**
 * Files interaction pass (notepads/filebrowser-interaction-optimization-todo.md):
 * "My files" start, the collapsed lazy folder tree, icon sizes and thumbnails,
 * and the Desktop Lite computer tree behind BuckyOS sudo (mock host realm,
 * `?fbHost=1`; the mock sudo password check fails only for "wrong").
 * Every scenario runs at desktop and at phone width.
 */

import { expect, test, type Locator, type Page } from '@playwright/test'
import { putZoneFile } from '../fixtures/selectDialog'

const LAYOUTS = [
  { name: 'desktop', viewport: { width: 1440, height: 900 }, mobile: false },
  { name: 'phone', viewport: { width: 390, height: 844 }, mobile: true },
] as const

/** The Files root: the desktop window, or the full-screen app on a phone. */
async function openFiles(page: Page, mobile: boolean, query = ''): Promise<Locator> {
  await page.goto(`/?scenario=normal${query}`)
  await page.getByTestId('desktop-app-files').click()
  const win = mobile ? page.getByTestId('filebrowser') : page.getByTestId('window-files')
  await expect(mobile ? win : win.getByTestId('filebrowser')).toBeVisible()
  return win
}

/** The places panel: the desktop sidebar, or the phone drawer (opened on demand). */
async function places(win: Locator, mobile: boolean): Promise<Locator> {
  if (mobile) await win.getByRole('button', { name: 'Places' }).click()
  return win.getByTestId('files-places')
}

const item = (win: Locator, name: string) => win.locator('[data-item-key]').filter({ hasText: name }).first()
/** Opens a folder from the main area, in list or icon view. */
async function enter(win: Locator, mobile: boolean, name: string) {
  if (mobile) await item(win, name).click()
  else await item(win, name).dblclick()
}

async function viewMenu(page: Page, win: Locator, mobile: boolean, item: string | RegExp) {
  if (mobile) {
    await win.getByRole('button', { name: 'More actions' }).first().click()
    await page.getByRole('button', { name: item }).click()
  } else {
    await win.getByRole('button', { name: 'View mode' }).click()
    await page.getByRole('menuitem', { name: item }).click()
  }
}

for (const layout of LAYOUTS) {
  test.describe(`Files interaction (${layout.name})`, () => {
    test.use({ viewport: layout.viewport, hasTouch: layout.mobile, isMobile: layout.mobile })

    test('opens on "My files" with a single tab and highlights it in the sidebar', async ({ page }) => {
      const win = await openFiles(page, layout.mobile)
      if (layout.mobile) {
        await expect(page.getByText('My files', { exact: true }).first()).toBeVisible()
      } else {
        await expect(win.locator('main[data-pane="left"]')).toHaveAttribute('data-location', 'dfs:///home')
        await expect(win.getByRole('tab')).toHaveCount(1)
        await expect(win.getByRole('tab', { name: /My files/ })).toBeVisible()
      }
      const panel = await places(win, layout.mobile)
      await expect(panel.locator('[aria-current="page"]')).toHaveText('My files')
      await expect(panel.getByRole('button', { name: 'Recent', exact: true }).first()).toBeVisible()
    })

    test('the folder tree starts collapsed, loads on expand to any depth and remembers what was opened', async ({ page }) => {
      let win = await openFiles(page, layout.mobile)
      let panel = await places(win, layout.mobile)
      await expect(panel.getByRole('button', { name: 'Expand Home' })).toBeVisible()
      await expect(panel.getByRole('button', { name: 'Pictures', exact: true })).toHaveCount(0)
      await panel.getByRole('button', { name: 'Expand Home' }).click()
      await panel.getByRole('button', { name: 'Expand Pictures' }).click()
      await panel.getByRole('button', { name: 'Expand Trips' }).click()
      await expect(panel.getByRole('button', { name: 'Kyoto', exact: true })).toBeVisible()
      // Expanding an empty folder drops its arrow.
      await panel.getByRole('button', { name: 'Expand Kyoto' }).click()
      await expect(panel.getByRole('button', { name: /Kyoto$/ })).toHaveCount(1)
      win = await openFiles(page, layout.mobile)
      panel = await places(win, layout.mobile)
      await expect(panel.getByRole('button', { name: 'Kyoto', exact: true })).toBeVisible()
    })

    test('icon view sizes persist and images, videos and PDFs show thumbnails or their type', async ({ page }) => {
      let win = await openFiles(page, layout.mobile)
      await putZoneFile(page, '/home/Pictures/Trips/Kyoto/clip.mp4', Buffer.from('mock-video'), 'video/mp4')
      await win.page().evaluate(() => { (window as { __mockPreviewWork?: Record<string, number> }).__mockPreviewWork = {} })
      for (const folder of ['Pictures', 'Trips', 'Kyoto']) await enter(win, layout.mobile, folder)
      await viewMenu(page, win, layout.mobile, layout.mobile ? 'View as icons' : 'Icon view')
      const grid = win.getByTestId('icon-grid')
      await expect(grid).toHaveAttribute('data-icon-size', 'medium')
      await expect(grid).toHaveAttribute('data-columns', layout.mobile ? '3' : /\d/)
      await expect(grid.locator('[data-item-key]').filter({ hasText: 'kyoto-temple-0412.jpg' }).locator('[data-thumb="image"]')).toBeVisible()
      const clip = grid.locator('[data-item-key]').filter({ hasText: 'clip.mp4' })
      await expect(clip.locator('[data-thumb="image"]')).toBeVisible({ timeout: 15000 })
      await expect(clip.getByTestId('thumb-play')).toBeVisible()

      await viewMenu(page, win, layout.mobile, layout.mobile ? 'Large icons' : 'Large icons')
      await expect(grid).toHaveAttribute('data-icon-size', 'large')
      if (layout.mobile) await expect(grid).toHaveAttribute('data-columns', '2')
      await viewMenu(page, win, layout.mobile, 'Small icons')
      await expect(grid).toHaveAttribute('data-icon-size', 'small')
      if (layout.mobile) await expect(grid).toHaveAttribute('data-columns', '4')
      await page.reload()
      win = await openFiles(page, layout.mobile)
      await enter(win, layout.mobile, 'Documents')
      await expect(win.getByTestId('icon-grid')).toHaveAttribute('data-icon-size', 'small')

      // A PDF the Pipeline cannot thumbnail keeps its type icon and is not asked again.
      const pdf = win.getByTestId('icon-grid').locator('[data-item-key]').filter({ hasText: 'Friend Contract Draft.pdf' })
      await expect(pdf.locator('[data-file-visual="pdf"]')).toBeVisible()
      await expect.poll(() => page.evaluate(() => (window as { __mockPreviewWork?: Record<string, number> }).__mockPreviewWork?.['Friend Contract Draft.pdf'] ?? 0)).toBe(1)
      if (layout.mobile) await win.getByRole('button', { name: 'home', exact: true }).click()
      else await win.getByRole('button', { name: 'up', exact: true }).click()
      await enter(win, layout.mobile, 'Documents')
      await expect(pdf.locator('[data-file-visual="pdf"]')).toBeVisible()
      await page.waitForTimeout(800)
      expect(await page.evaluate(() => (window as { __mockPreviewWork?: Record<string, number> }).__mockPreviewWork?.['Friend Contract Draft.pdf'] ?? 0)).toBe(1)
    })

    test('without a host realm there is no Devices section', async ({ page }) => {
      const win = await openFiles(page, layout.mobile)
      const panel = await places(win, layout.mobile)
      await expect(panel.getByRole('button', { name: /Devices/ })).toHaveCount(0)
      await expect(panel.getByTestId('sidebar-unlock-host')).toHaveCount(0)
      await expect(win.getByText('Enable advanced mode')).toHaveCount(0)
    })

    test('BuckyOS sudo unlocks the computer tree; wrong password, no-access folders, copy to My files and back', async ({ page, context }) => {
      await context.grantPermissions(['clipboard-read', 'clipboard-write'])
      const win = await openFiles(page, layout.mobile, '&fbHost=1')
      let panel = await places(win, layout.mobile)
      if (layout.mobile) await panel.getByRole('button', { name: 'Devices' }).click()
      await panel.getByTestId('sidebar-unlock-host').click()
      const password = page.getByRole('textbox', { name: 'Password' })
      await expect(page.getByText('expires in about 30 minutes')).toBeVisible()
      await password.fill('wrong')
      await page.keyboard.press('Enter')
      await expect(page.getByText(/Incorrect password/)).toBeVisible()
      await password.fill('secret')
      await page.keyboard.press('Enter')
      await expect(password).toHaveCount(0)
      if (!layout.mobile) {
        await expect(win.locator('main[data-pane="left"]')).toHaveAttribute('data-location', /^device:\/\/w\w+$/)
        await expect(win.getByRole('tab', { name: /Studio-Mac/ })).toBeVisible()
      }
      // On a phone the drawer closes on the computer's root; open it on Devices again.
      panel = await places(win, layout.mobile)
      if (layout.mobile) await panel.getByRole('button', { name: 'Devices' }).click()
      await expect(panel.getByRole('button', { name: 'Studio-Mac', exact: true })).toBeVisible()
      await expect(panel.getByRole('button', { name: 'System', exact: true })).toBeDisabled()
      await panel.getByRole('button', { name: 'Expand Users' }).click()
      await panel.getByRole('button', { name: 'Expand alice' }).click()
      await panel.getByRole('button', { name: 'Documents', exact: true }).last().click()
      // The computer's files show the standard file URI, never device://.
      if (!layout.mobile) {
        await expect(win.getByText('file:///Users/alice/Documents', { exact: true })).toBeVisible()
        await win.getByRole('cell', { name: 'Report Q3.pdf', exact: true }).click({ button: 'right' })
        await page.getByRole('menuitem', { name: 'Copy path' }).click()
        await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('file:///Users/alice/Documents/Report%20Q3.pdf')
        await win.getByRole('cell', { name: 'Report Q3.pdf', exact: true }).click({ button: 'right' })
        await expect(page.getByRole('menuitem', { name: 'Share…' })).toHaveCount(0)
        await expect(page.getByRole('menuitem', { name: /Add to collection/i })).toHaveCount(0)
        await page.getByRole('menuitem', { name: 'Copy files' }).click()
        await win.locator('aside').getByRole('button', { name: 'My files', exact: true }).click()
        await expect(win.getByRole('cell', { name: 'Documents', exact: true })).toBeVisible()
        await win.locator('[data-testid="filebrowser"]').focus()
        await page.keyboard.press('Control+v')
        await expect(win.getByRole('cell', { name: 'Report Q3.pdf', exact: true })).toBeVisible()
        // Back to the computer: a Zone file copies into /tmp.
        await win.getByRole('cell', { name: 'Report Q3.pdf', exact: true }).click()
        await page.keyboard.press('Control+c')
        await win.locator('aside').getByRole('button', { name: 'tmp', exact: true }).click()
        await expect(win.getByRole('cell', { name: 'a b#1.txt', exact: true })).toBeVisible()
        await win.locator('[data-testid="filebrowser"]').focus()
        await page.keyboard.press('Control+v')
        await expect(win.getByRole('cell', { name: 'Report Q3.pdf', exact: true })).toBeVisible()
      } else {
        await expect(win.getByText('Report Q3.pdf')).toBeVisible()
        await expect(page.getByText('file:///Users/alice/Documents').first()).toBeVisible()
      }
    })

    test('an expired unlock locks the tab and the dialog unlocks it again; a pasted file URI opens the folder', async ({ page, context }) => {
      await context.grantPermissions(['clipboard-read', 'clipboard-write'])
      const win = await openFiles(page, layout.mobile, '&fbHost=1&fbHostTtl=4')
      const panel = await places(win, layout.mobile)
      if (layout.mobile) await panel.getByRole('button', { name: 'Devices' }).click()
      await panel.getByTestId('sidebar-unlock-host').click()
      await page.getByRole('textbox', { name: 'Password' }).fill('secret')
      await page.keyboard.press('Enter')
      await expect(page.getByRole('textbox', { name: 'Password' })).toHaveCount(0)
      await expect(win.getByTestId('filebrowser-host-locked')).toBeVisible({ timeout: 15000 })
      await win.getByTestId('filebrowser-host-locked').getByRole('button', { name: 'Unlock' }).click()
      await page.getByRole('textbox', { name: 'Password' }).fill('secret')
      await page.keyboard.press('Enter')
      await expect(win.getByTestId('filebrowser-host-locked')).toHaveCount(0)
      await expect(win.getByText('Applications', { exact: true }).first()).toBeVisible()
      if (!layout.mobile) {
        await win.getByTestId('filebrowser').focus()
        await page.keyboard.press('Control+l')
        const input = win.getByRole('textbox', { name: 'path' })
        await input.fill('file:///Users/alice/Pictures')
        await input.press('Enter')
        await expect(win.locator('main[data-pane="left"]')).toHaveAttribute('data-location', /^device:\/\/w\w+\/Users\/alice\/Pictures$/)
        await expect(win.getByRole('cell', { name: 'Beach 1.jpg', exact: true })).toBeVisible()
      }
    })
  })
}
