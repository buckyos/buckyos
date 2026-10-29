import { expect, test, type Page } from '@playwright/test'
import { MOCK_DESKTOP_FOLDER_KEY, openFromAllApps, seedDesktopFolder } from '../fixtures/desktop'

/** The mock Desktop folder (api/desktop_items_mock.ts): file name → entry. */
async function folderEntries(page: Page): Promise<Record<string, { kind: string; text?: string }>> {
  return page.evaluate((key) => JSON.parse(window.localStorage.getItem(key) ?? '{"entries":{}}').entries, MOCK_DESKTOP_FOLDER_KEY)
}

async function itemIds(page: Page) {
  return page
    .locator('[data-testid^="desktop-item-"]')
    .evaluateAll((elements) => elements.map((el) => el.getAttribute('data-testid')!.slice('desktop-item-'.length)).sort())
}

async function center(page: Page, testId: string) {
  const box = await page.getByTestId(testId).boundingBox()
  if (!box) throw new Error(`${testId} is not shown`)
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 }
}

test.describe('Desktop folder', () => {
  test.use({ viewport: { width: 1440, height: 900 } })

  test('a new account starts with the getting-started note, the default shortcuts and taskbar pins', async ({ page }) => {
    const consoleErrors: string[] = []
    page.on('console', (message) => { if (message.type() === 'error') consoleErrors.push(message.text()) })
    await page.goto('/?scenario=normal')

    // The note explains the first steps.
    const note = page.getByTestId('notepad-preview-Getting Started.widget')
    await expect(note).toContainText('Open AI Center and check that a model provider is available.')
    await expect(note).toContainText('Click Jarvis to create your own Agent.')
    await expect(note).toContainText('Chat with your Agent in MessageHub.')
    await expect(page.locator('[data-widget-type="clock"]')).toBeVisible()
    await expect(page.getByTestId('system-info-fields')).toContainText('0.7.0')
    const noteBox = await page.getByTestId('desktop-item-Getting Started.widget').boundingBox()
    const infoBox = await page.getByTestId('system-info-widget').boundingBox()
    expect(Math.abs(infoBox!.x - noteBox!.x)).toBeLessThan(2)
    expect(infoBox!.y).toBeGreaterThanOrEqual(noteBox!.y + noteBox!.height)

    // Three rows of shortcuts to the right of the widgets.
    const rows = [
      ['agent-guide', 'messagehub', 'my-network'],
      ['aiworkspace', 'ai-center', 'homestation'],
      ['files', 'app-service', 'settings', 'users-agents', 'task-center'],
    ]
    const clock = await center(page, 'desktop-item-Clock.widget')
    let previousRowY = -Infinity
    for (const row of rows) {
      const points = await Promise.all(row.map((id) => center(page, `desktop-app-${id}`)))
      for (const point of points) expect(Math.abs(point.y - points[0].y)).toBeLessThan(2)
      for (let i = 1; i < points.length; i++) expect(points[i].x).toBeGreaterThan(points[i - 1].x)
      expect(points[0].x).toBeGreaterThan(clock.x)
      expect(points[0].y).toBeGreaterThan(previousRowY)
      previousRowY = points[0].y
    }

    // Apps without a shortcut are only under All apps; the content tools are on the second screen.
    await expect(page.getByTestId('desktop-app-diagnostics')).toHaveCount(0)
    await expect(page.getByTestId('desktop-app-systest')).toHaveCount(0)
    await expect(page.locator('.swiper-slide-active').getByTestId('desktop-app-preview')).toHaveCount(0)
    await page.locator('.swiper-pagination-bullet').nth(1).click()
    await expect(page.locator('.swiper-slide-active').getByTestId('desktop-app-preview')).toBeVisible()
    await expect(page.locator('.swiper-slide-active').getByTestId('desktop-app-workflow')).toBeVisible()

    // The desktop is the folder: every item is a file there.
    const entries = await folderEntries(page)
    expect(Object.keys(entries).sort()).toEqual([
      'AI Center.lnk', 'AI Workspace.lnk', 'App Service.lnk', 'Clock.widget', 'Files.lnk', 'Getting Started.widget',
      'HomeStation.lnk', 'Jarvis.lnk', 'MessageHub.lnk', 'My Network.lnk', 'Preview.lnk', 'Settings.lnk',
      'System Info.widget', 'Task Center.lnk', 'Users & Agents.lnk', 'Workflow.lnk',
    ])
    expect(JSON.parse(entries['MessageHub.lnk'].text!)).toEqual({ app: 'messagehub' })

    // The taskbar pins MessageHub, AI Workspace, HomeStation and AI Center.
    await page.getByTestId('desktop-taskbar-avatar').click()
    const pinned = page.getByTestId('desktop-taskbar').getByRole('navigation').locator('[data-app-id]')
    await expect.poll(() => pinned.evaluateAll((buttons) => buttons.map((b) => b.getAttribute('data-app-id'))))
      .toEqual(['messagehub', 'aiworkspace', 'homestation', 'ai-center'])

    expect(consoleErrors).toEqual([])
  })

  test('the note and the default shortcuts are named in the account language', async ({ page }) => {
    await page.addInitScript(() => {
      if (!window.sessionStorage.getItem('locale-seeded')) {
        window.localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN')
        window.sessionStorage.setItem('locale-seeded', '1')
      }
    })
    await page.goto('/?scenario=normal')
    await expect(page.getByTestId('notepad-preview-使用说明.widget')).toContainText('点击 Jarvis，创建你自己的 Agent')
    const names = Object.keys(await folderEntries(page))
    expect(names).toEqual(expect.arrayContaining(['时钟.widget', '使用说明.widget', '设置.lnk', '文件.lnk', 'MessageHub.lnk']))
  })

  test('shortcuts are added from All apps and removed from the desktop; the folder keeps the change', async ({ page }) => {
    await page.goto('/?scenario=normal')

    await page.getByTestId('desktop-app-settings').click({ button: 'right' })
    await page.getByTestId('desktop-menu-remove').click()
    await expect(page.getByTestId('desktop-app-settings')).toHaveCount(0)
    await expect.poll(async () => Object.keys(await folderEntries(page))).not.toContain('Settings.lnk')

    await page.getByTestId('desktop-taskbar-avatar').click()
    await page.getByTestId('taskbar-all-apps').click()
    await page.getByTestId('all-apps-item-diagnostics').click({ button: 'right' })
    await expect(page.getByTestId('taskbar-menu-desktop')).toHaveText('Add to desktop')
    await page.getByTestId('taskbar-menu-desktop').click()
    await expect(page.getByTestId('desktop-app-diagnostics')).toBeVisible()
    await expect.poll(async () => (await folderEntries(page))['Diagnostics.lnk']?.text).toContain('"app": "diagnostics"')

    // The same app offers removal once it is there.
    await page.getByTestId('all-apps-item-diagnostics').click({ button: 'right' })
    await expect(page.getByTestId('taskbar-menu-desktop')).toHaveText('Remove from desktop')
    await page.keyboard.press('Escape')

    await page.reload()
    await expect(page.getByTestId('desktop-app-diagnostics')).toBeVisible()
    await expect(page.getByTestId('desktop-app-settings')).toHaveCount(0)
  })

  test('the note is saved into its widget file', async ({ page }) => {
    await page.goto('/?scenario=normal')
    await page.getByTestId('notepad-preview-Getting Started.widget').click()
    await page.getByTestId('notepad-editor-Getting Started.widget').fill('Line one\nLine two')
    await page.getByTestId('notepad-save-Getting Started.widget').click()
    await expect.poll(async () => JSON.parse((await folderEntries(page))['Getting Started.widget'].text!))
      .toEqual({ widget: 'notepad', text: 'Line one\nLine two' })
    await page.reload()
    await expect(page.getByTestId('notepad-preview-Getting Started.widget')).toHaveText('Line one\nLine two')
  })

  test('documents and folders in the Desktop folder show on the desktop and open in Files', async ({ page }) => {
    await seedDesktopFolder(page, { Files: 'files' }, {
      'Trip plan.md': { kind: 'file', size: 120, mtime: 1 },
      Projects: { kind: 'dir', size: 0, mtime: 1 },
      'Broken.lnk': { kind: 'file', size: 7, mtime: 1, text: 'garbage' },
    })
    await page.goto('/?scenario=normal')

    await expect(page.getByTestId('desktop-file-Trip plan.md')).toBeVisible()
    await expect(page.getByTestId('desktop-file-Projects')).toBeVisible()
    // A shortcut file that cannot be read is shown as the plain file it is.
    await expect(page.getByTestId('desktop-file-Broken.lnk')).toBeVisible()
    expect(await itemIds(page)).toEqual(['Broken.lnk', 'Files.lnk', 'Projects', 'Trip plan.md'])

    // A document cannot be removed from the desktop here; Files is where it is changed.
    await page.getByTestId('desktop-file-Trip plan.md').click({ button: 'right' })
    await expect(page.getByTestId('desktop-menu-remove')).toHaveAttribute('aria-disabled', 'true')
    await page.getByTestId('desktop-menu-show-in-files').click()
    const files = page.getByTestId('window-files')
    await expect(files).toBeVisible()
    await expect(files.locator('[data-pane="left"]')).toHaveAttribute('data-location', 'dfs:///home/Desktop')
    await expect(files.getByText('Files.lnk', { exact: true })).toBeVisible()
    await expect(files.getByText('Trip plan.md', { exact: true })).toBeVisible()

    // A folder opens in Files.
    await page.getByTestId('window-drag-files').getByRole('button', { name: 'Close' }).click()
    await page.getByTestId('desktop-file-Projects').click()
    await expect(page.getByTestId('window-files').locator('[data-pane="left"]')).toHaveAttribute('data-location', 'dfs:///home/Desktop/Projects')
  })

  test('a phone shows the same items, in its own arrangement', async ({ page }) => {
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-task-center').click({ button: 'right' })
    await page.getByTestId('desktop-menu-remove').click()
    await expect(page.getByTestId('desktop-app-task-center')).toHaveCount(0)
    const desktopItems = await itemIds(page)

    await page.setViewportSize({ width: 390, height: 844 })
    await expect(page.getByTestId('mobile-taskbar')).toBeAttached()
    await expect.poll(() => itemIds(page)).toEqual(desktopItems)
    // The note spans the phone's width under the clock.
    await expect.poll(async () => (await page.getByTestId('desktop-item-Getting Started.widget').boundingBox())?.width ?? 0).toBeGreaterThan(300)
  })

  test('restoring defaults brings back the default items the folder lost', async ({ page }) => {
    await seedDesktopFolder(page, {})
    await page.goto('/?scenario=normal')
    await expect(page.getByText('Layout is empty')).toBeVisible()
    await page.getByRole('button', { name: 'Restore defaults' }).click()
    await expect(page.getByTestId('desktop-app-messagehub')).toBeVisible()
    await expect(page.getByTestId('notepad-preview-Getting Started.widget')).toContainText('Getting started')
    expect(Object.keys(await folderEntries(page))).toContain('Settings.lnk')
    // In the default arrangement, measured on the full-height grid (not the empty-state card).
    const files = await center(page, 'desktop-app-files')
    const taskCenter = await center(page, 'desktop-app-task-center')
    const note = await center(page, 'desktop-item-Getting Started.widget')
    expect(Math.abs(files.y - taskCenter.y)).toBeLessThan(2)
    expect(files.x).toBeGreaterThan(note.x)
  })

  test('an app with no shortcut still opens from All apps', async ({ page }) => {
    await page.goto('/?scenario=normal')
    await openFromAllApps(page, 'diagnostics')
    await expect(page.getByTestId('window-diagnostics')).toBeVisible()
  })
})
