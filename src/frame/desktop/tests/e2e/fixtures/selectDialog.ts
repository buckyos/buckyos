import type { Locator, Page } from '@playwright/test'

/** The system file picker (`sysdlg/select`) currently shown. */
export const selectDialog = (page: Page): Locator => page.getByTestId('select-dialog')

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
