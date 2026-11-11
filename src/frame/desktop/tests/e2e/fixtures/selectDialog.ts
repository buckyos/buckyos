import { readFileSync } from 'node:fs'
import type { Locator, Page } from '@playwright/test'

type FilePayload = { name: string; mimeType: string; buffer: Buffer }
/** A page, or the frame of the system page that path C puts over a standalone App (`selectEmbedFrame`). */
type Scope = Pick<Page, 'getByTestId'>

/** The system file picker (`sysdlg/select`) currently shown. */
export const selectDialog = (scope: Scope): Locator => scope.getByTestId('select-dialog')

/** Cancels the picker shown in a window dialog with the × of its title bar. */
export const closeSelect = (page: Page) =>
  page.getByTestId('window-dialog').filter({ has: selectDialog(page) }).getByRole('button', { name: 'Close dialog' }).click()

/** The picker framed over a page without a Desktop window (path C, `/embed/sysdlg/select`). */
export const selectEmbedFrame = (page: Page) => page.frameLocator('[data-buckyos-sysdlg="select"] iframe')

export const selectRow = (dialog: Locator, name: string): Locator => dialog.locator(`[data-testid="select-row"][data-name="${name}"]`)

/** Walks the picker from a root location down to a DFS path such as `/home/Downloads`. */
export async function selectGoto(dialog: Locator, path: string, tap = false) {
  const act = (target: Locator) => tap ? target.tap() : target.click()
  const [root, ...rest] = path.split('/').filter(Boolean)
  const label = root === 'home' ? 'My files' : root.charAt(0).toUpperCase() + root.slice(1)
  const locations = dialog.getByTestId('select-locations')
  if (await locations.evaluate((el) => el.tagName === 'SELECT')) await locations.selectOption({ label })
  else await act(locations.getByRole('button', { name: label, exact: true }))
  for (const segment of rest) await act(selectRow(dialog, segment))
}

/**
 * Chooses Zone files in the open picker and confirms: all paths must be in one folder, e.g.
 * `['/home/Documents/a.pdf']`.
 */
export async function pickZone(dialog: Locator, paths: string | string[], tap = false) {
  const list = Array.isArray(paths) ? paths : [paths]
  const folder = list[0].slice(0, list[0].lastIndexOf('/'))
  await selectGoto(dialog, folder, tap)
  for (const path of list) {
    const row = selectRow(dialog, path.slice(path.lastIndexOf('/') + 1))
    await (tap ? row.tap() : row.click())
  }
  await dialog.getByTestId('select-confirm').click()
}

/** Chooses a Zone folder in the folder picker and confirms. */
export async function pickZoneFolder(dialog: Locator, path: string, tap = false) {
  await selectGoto(dialog, path, tap)
  await dialog.getByTestId('select-confirm').click()
}

/** Chooses files from this device through the picker's "Choose from this device" button (the OS picker opens on the page). */
export async function pickLocal(page: Page, files: string | string[] | FilePayload | FilePayload[], dialog: Locator = selectDialog(page)) {
  const chooser = page.waitForEvent('filechooser')
  await dialog.getByTestId('select-local').click()
  await (await chooser).setFiles(files)
}

/**
 * Mock runtime: puts a file into the mock Zone of this document (missing folders are created), from a fixture on disk
 * or from bytes. Path C's system page is another document: pass its frame to put the file there.
 */
export async function putZoneFile(target: Page | ReturnType<Page['mainFrame']>, path: string, content: string | Buffer, mime?: string) {
  const bytes = typeof content === 'string' ? readFileSync(content) : content
  type Hook = { __mockZoneFiles?: { put(path: string, base64: string, mime?: string): void } }
  await target.waitForFunction(() => !!(window as Hook).__mockZoneFiles)
  await target.evaluate(([p, b, m]) => (window as Hook).__mockZoneFiles!.put(p, b, m), [path, bytes.toString('base64'), mime] as const)
}
