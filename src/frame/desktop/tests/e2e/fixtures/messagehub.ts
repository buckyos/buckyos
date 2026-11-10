import { expect, type Locator, type Page } from '@playwright/test'

/** The page, or the desktop window MessageHub runs in. */
type Scope = Page | Locator

/**
 * "New Session" lives in the session list. Opens the list when it is closed and
 * clicks the entry; the returned function puts the list back as it was (a
 * created session closes the phone's drawer by itself).
 */
export async function openNewSession(page: Scope) {
  const sidebar = page.getByTestId('session-sidebar')
  const wasOpen = await sidebar.isVisible()
  if (!wasOpen) await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  await sidebar.getByRole('button', { name: 'New Session', exact: true }).click()
  return async () => {
    if (wasOpen || !await sidebar.isVisible()) return
    const close = sidebar.getByRole('button', { name: 'Close', exact: true })
    if (await close.count()) await close.click()
    else await page.getByRole('button', { name: 'Sessions', exact: true }).click()
    await expect(sidebar).toHaveCount(0)
  }
}

/** The session list's "New Session" entry itself (the list is opened for it and stays open). */
export async function newSessionButton(page: Scope) {
  const sidebar = page.getByTestId('session-sidebar')
  if (!await sidebar.isVisible()) await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  return sidebar.getByRole('button', { name: 'New Session', exact: true })
}

/** Creates a session of the open entity over its default (native) connection. */
export async function createNamedSession(page: Scope, name: string, options: { description?: string } = {}) {
  const restore = await openNewSession(page)
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('Session name', { exact: true }).fill(name)
  if (options.description) await dialog.getByLabel('Description (optional)', { exact: true }).fill(options.description)
  await expect(dialog.getByRole('combobox', { name: 'Connection', exact: true })).toHaveValue('native')
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await restore()
}

/** Session details open from the conversation's ⋮ menu ("Edit info"). */
export async function openSessionDetails(page: Scope) {
  await page.getByTestId('session-menu').click()
  await page.getByTestId('session-menu-edit-info').click()
  const panel = page.getByTestId('session-details')
  await expect(panel).toBeVisible()
  return panel
}

/** The list's "New chat": finds an entity by a fuzzy query and opens it. */
export async function openChatWith(page: Scope, query: string, name: string | RegExp) {
  await page.getByTestId('hub-create-menu').click()
  await page.getByTestId('new-chat').click()
  const dialog = page.getByRole('dialog', { name: 'New chat' })
  await dialog.getByTestId('new-chat-search').fill(query)
  await expect(dialog.getByRole('option', { selected: true })).toContainText(name)
  await dialog.getByTestId('new-chat-enter').click()
  await expect(dialog).toHaveCount(0)
}

export async function closeSessionList(page: Scope) {
  const sidebar = page.getByTestId('session-sidebar')
  if (!await sidebar.isVisible()) return
  const close = sidebar.getByRole('button', { name: 'Close', exact: true })
  if (await close.count()) await close.click()
  else await page.getByRole('button', { name: 'Sessions', exact: true }).click()
  await expect(sidebar).toHaveCount(0)
}
