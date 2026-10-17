import { expect, type Page } from '@playwright/test'

/**
 * Opens an app the way a user reaches one that has no desktop shortcut: the taskbar's All apps panel.
 * Apps are only on the desktop when the Desktop folder holds a shortcut to them (models/desktopFolder.ts).
 */
export async function openFromAllApps(page: Page, appId: string) {
  const rail = page.getByTestId('mobile-taskbar')
  if (await rail.count()) {
    // Phone: the status bar's avatar opens the rail.
    if ((await rail.getAttribute('data-open')) !== 'true') await page.getByRole('button', { name: 'BuckyOS' }).first().click()
    await rail.getByTestId('taskbar-all-apps').click()
  } else {
    const taskbar = page.getByTestId('desktop-taskbar')
    if ((await taskbar.getAttribute('aria-hidden')) === 'true') {
      await page.getByTestId('desktop-taskbar-avatar').click()
    }
    await page.getByTestId('taskbar-all-apps').click()
  }
  const item = page.getByTestId(`all-apps-item-${appId}`)
  await expect(item).toBeVisible()
  await item.click()
}

export const MOCK_DESKTOP_FOLDER_KEY = 'buckyos.mock.desktop-folder.v1'

/**
 * Starts the mock account with exactly these shortcuts in its Desktop folder (`{label}.lnk` → app id), set
 * before the first page load. Other desktop items come from `extra` (name → entry as api/desktop_items_mock.ts
 * stores it).
 */
export async function seedDesktopFolder(
  page: Page,
  shortcuts: Record<string, string>,
  extra: Record<string, { kind: 'file' | 'dir'; size: number; mtime: number; text?: string }> = {},
) {
  const entries = {
    ...Object.fromEntries(
      Object.entries(shortcuts).map(([label, app]) => {
        const text = JSON.stringify({ app })
        return [`${label}.lnk`, { kind: 'file', size: text.length, mtime: 1, text }]
      }),
    ),
    ...extra,
  }
  await page.addInitScript(([key, value]) => {
    if (!window.sessionStorage.getItem('desktop-folder-seeded')) {
      window.localStorage.setItem(key, value)
      window.sessionStorage.setItem('desktop-folder-seeded', '1')
    }
  }, [MOCK_DESKTOP_FOLDER_KEY, JSON.stringify({ entries })] as const)
}

/** The taskbar's pinned apps before the first page load (instead of the defaults). */
export async function pinTaskbarApps(page: Page, appIds: string[]) {
  await page.addInitScript((ids) => {
    if (window.localStorage.getItem('buckyos.desktop.taskbar-apps.v2') === null) {
      window.localStorage.setItem('buckyos.desktop.taskbar-apps.v2', JSON.stringify(ids))
    }
  }, appIds)
}
