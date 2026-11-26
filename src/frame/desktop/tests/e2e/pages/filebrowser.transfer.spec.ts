import { expect, test, type Locator, type Page } from '@playwright/test'

// Files on the system clipboard and in drags (doc/desktop/BuckyOS 剪贴板管理与拖拽支持规范.md §2.2, §3, §6): Copy and
// Cut write Zone references with readable names and links plus a desktop record's handle; Paste reads only what the
// current paste carries.

const CONTENT_REFS = 'application/vnd.buckyos.content-refs+json'

/** Two Files windows side by side: the first on Documents, the second on `second`. */
async function twoWindows(page: Page, second = '/home/Pictures', query = '') {
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.goto(`/?scenario=normal&fbData=mock${query}`)
  await page.getByTestId('desktop-app-files').click()
  const a = page.getByTestId('window-files').first()
  await a.getByRole('cell', { name: 'Documents', exact: true }).dblclick()
  await expect(a.getByRole('cell', { name: 'Kyoto Trip Plan.md', exact: true })).toBeVisible()
  await openWindow(page, second)
  await layout(page)
  const b = page.getByTestId('window-files').nth(1)
  await expect(b.locator('[data-pane="left"]')).toHaveAttribute('data-location', `dfs://${second}`)
  await expect(b.locator('[data-pane="left"]')).toHaveAttribute('data-list-status', 'ready')
  return { a, b }
}
async function openWindow(page: Page, path: string) {
  await page.evaluate(async (path) => {
    const { desktopUIStore } = await import('/src/models/' + 'DesktopUIDataModel.ts')
    const { filesOpenLaunch } = await import('/src/app/filebrowser/' + 'launch.ts')
    desktopUIStore.openAppWindow('files', { multi: true, mode: 'new', launch: filesOpenLaunch(path) })
  }, path)
}
/** Lays the Files windows out in columns so each one's rows are visible and can be dragged. */
async function layout(page: Page) {
  await page.evaluate(async () => {
    const { desktopUIStore } = await import('/src/models/' + 'DesktopUIDataModel.ts')
    const windows = desktopUIStore.getSnapshot().runtime.windows.filter((w: { appId: string }) => w.appId === 'files')
    const width = Math.floor(1430 / windows.length)
    windows.forEach((w: { id: string }, i: number) => desktopUIStore.updateWindowGeometry(w.id, { x: 5 + i * width, y: 40, width: width - 5, height: 900 }))
  })
}
const row = (win: Locator, name: string) => win.getByRole('row').filter({ has: win.page().getByRole('cell', { name, exact: true }) })
const cell = (win: Locator, name: string) => win.getByRole('cell', { name, exact: true })
async function focusFiles(win: Locator) {
  await win.getByTestId('filebrowser').focus()
}
async function closeResults(win: Locator) {
  await win.getByTestId('batch-results').getByRole('button', { name: 'Close', exact: true }).click()
}
const mockPath = (page: Page, path: string) => page.evaluate(async (path) => !!(await import('/src/app/filebrowser/mock/' + 'data.ts')).mockEntryByPath(path), path)

test.beforeEach(async ({ context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
})

test('a copy in one Files window pastes a copy in another; the system clipboard holds names and marked links', async ({ page }) => {
  const { a, b } = await twoWindows(page)
  await cell(a, 'Kyoto Trip Plan.md').click()
  await page.keyboard.press('Control+c')
  await expect(a.getByText('Copied 1 item(s)')).toBeVisible()

  const clipboard = await page.evaluate(async () => {
    const out: Record<string, string> = {}
    for (const item of await navigator.clipboard.read()) for (const type of item.types) out[type] = await (await item.getType(type)).text()
    return out
  })
  expect(clipboard['text/plain']).toBe('Kyoto Trip Plan.md')
  expect(clipboard['text/plain']).not.toMatch(/[{}]|offerId|cyfs:/)
  expect(clipboard['text/html']).toMatch(/<a href="cyfs:\/\/\/home\/Documents\/Kyoto%20Trip%20Plan\.md" data-buckyos-ref="[^"]+">Kyoto Trip Plan\.md<\/a>/)

  await focusFiles(b)
  await page.keyboard.press('Control+v')
  await expect(b.getByTestId('batch-results')).toContainText('1 succeeded')
  await expect(cell(b, 'Kyoto Trip Plan.md')).toBeVisible()
  await expect(cell(a, 'Kyoto Trip Plan.md')).toBeVisible()
  expect(await mockPath(page, '/home/Documents/Kyoto Trip Plan.md')).toBe(true)
  expect(await mockPath(page, '/home/Pictures/Kyoto Trip Plan.md')).toBe(true)
  await closeResults(b)

  // A copy can be pasted again; the toolbar Paste reads the same clipboard.
  await b.locator('[data-pane="left"]').getByRole('button', { name: 'Paste', exact: true }).click()
  await page.getByTestId('conflict-dialog').getByRole('button', { name: 'Keep both', exact: true }).click()
  await expect(cell(b, 'Kyoto Trip Plan (2).md')).toBeVisible()
})

test('cut then paste in another window moves and clears the marking; Esc cancels a cut and a stale cut asks before copying', async ({ page }) => {
  const { a, b } = await twoWindows(page)
  await cell(a, 'Kyoto Trip Plan.md').click()
  await page.keyboard.press('Control+x')
  await expect(row(a, 'Kyoto Trip Plan.md')).toHaveClass(/opacity-50/)
  await focusFiles(b)
  await page.keyboard.press('Control+v')
  await expect(b.getByTestId('batch-results')).toContainText('1 succeeded')
  await expect(cell(b, 'Kyoto Trip Plan.md')).toBeVisible()
  await expect(cell(a, 'Kyoto Trip Plan.md')).toHaveCount(0)
  expect(await mockPath(page, '/home/Documents/Kyoto Trip Plan.md')).toBe(false)
  await closeResults(b)

  // Esc withdraws the pending cut: no marking, and a paste does not move.
  await cell(a, 'Friend Contract Draft.pdf').click()
  await page.keyboard.press('Control+x')
  await expect(row(a, 'Friend Contract Draft.pdf')).toHaveClass(/opacity-50/)
  await page.keyboard.press('Escape')
  await expect(row(a, 'Friend Contract Draft.pdf')).not.toHaveClass(/opacity-50/)
  await focusFiles(b)
  await page.keyboard.press('Control+v')
  const choice = b.getByTestId('files-transfer-choice')
  await expect(choice).toContainText('The cut is no longer available. Copy 1 item(s) here instead?')
  await choice.getByRole('button', { name: 'Cancel', exact: true }).click()
  await expect(choice).toHaveCount(0)
  await expect(cell(b, 'Friend Contract Draft.pdf')).toHaveCount(0)
  await page.keyboard.press('Control+v')
  await choice.getByRole('button', { name: 'Copy instead', exact: true }).click()
  await expect(b.getByTestId('batch-results')).toContainText('1 succeeded')
  await expect(cell(b, 'Friend Contract Draft.pdf')).toBeVisible()
  await expect(cell(a, 'Friend Contract Draft.pdf')).toBeVisible()
})

test('a second window pasting a cut that is being moved is told so', async ({ page }) => {
  const { a, b } = await twoWindows(page, '/home/Pictures', '&fbSlowOps=1')
  await openWindow(page, '/home/Downloads')
  await layout(page)
  const c = page.getByTestId('window-files').nth(2)
  await expect(c.locator('[data-pane="left"]')).toHaveAttribute('data-list-status', 'ready')
  await cell(a, 'Kyoto Trip Plan.md').click()
  await page.keyboard.press('Control+x')
  await focusFiles(b)
  await page.keyboard.press('Control+v')
  await expect(b.getByTestId('batch-results')).toBeVisible()
  await focusFiles(c)
  await page.keyboard.press('Control+v')
  await expect(c.getByText('These items are already being moved in another window.')).toBeVisible()
  await expect(b.getByTestId('batch-results')).toContainText('1 succeeded')
  expect(await mockPath(page, '/home/Pictures/Kyoto Trip Plan.md')).toBe(true)
  expect(await mockPath(page, '/home/Downloads/Kyoto Trip Plan.md')).toBe(false)
})

test('dragging between Files windows moves; Zone references dropped from another App are copied', async ({ page }) => {
  const { a, b } = await twoWindows(page)
  await row(a, 'Kyoto Trip Plan.md').dragTo(b.locator('[data-pane="left"]'), { targetPosition: { x: 200, y: 500 } })
  await expect(b.getByTestId('batch-results')).toContainText('1 succeeded')
  await expect(cell(b, 'Kyoto Trip Plan.md')).toBeVisible()
  await expect(cell(a, 'Kyoto Trip Plan.md')).toHaveCount(0)
  await closeResults(b)

  // Another App's drag: only the standard content, no Files handle.
  const transfer = await page.evaluateHandle((format) => {
    const data = new DataTransfer()
    data.setData(format, JSON.stringify({ v: 1, items: [{ origin: 'zone', kind: 'file', source: { kind: 'cyfs-path', path: 'cyfs:///home/Documents/Friend Contract Draft.pdf' }, hints: { name: 'Friend Contract Draft.pdf' } }] }))
    data.setData('text/plain', 'Friend Contract Draft.pdf')
    return data
  }, CONTENT_REFS)
  const pane = b.locator('[data-pane="left"]')
  await pane.dispatchEvent('dragover', { dataTransfer: transfer })
  await pane.dispatchEvent('drop', { dataTransfer: transfer })
  await expect(b.getByTestId('batch-results')).toContainText('1 succeeded')
  await expect(cell(b, 'Friend Contract Draft.pdf')).toBeVisible()
  await expect(cell(a, 'Friend Contract Draft.pdf')).toBeVisible()
  expect(await mockPath(page, '/home/Documents/Friend Contract Draft.pdf')).toBe(true)
})

test('after copying text elsewhere, Paste in Files only explains: no earlier copy comes back', async ({ page }) => {
  const { a, b } = await twoWindows(page)
  await cell(a, 'Kyoto Trip Plan.md').click()
  await page.keyboard.press('Control+c')
  await expect(a.getByText('Copied 1 item(s)')).toBeVisible()
  await page.evaluate(() => navigator.clipboard.writeText('Friday release'))

  await b.locator('[data-pane="left"]').getByRole('button', { name: 'Paste', exact: true }).click()
  await expect(b.getByText('This is text. Files can only paste files and folders here.')).toBeVisible()
  await focusFiles(b)
  await page.keyboard.press('Control+v')
  await expect(b.getByText('This is text. Files can only paste files and folders here.')).toBeVisible()
  await expect(b.getByTestId('batch-results')).toHaveCount(0)
  await expect(cell(b, 'Kyoto Trip Plan.md')).toHaveCount(0)
})

test('BuckyOS addresses are pasted only after the user chooses', async ({ page }) => {
  const { b } = await twoWindows(page)
  await page.evaluate(() => navigator.clipboard.writeText('cyfs:///home/Documents/Kyoto Trip Plan.md'))
  await focusFiles(b)
  await page.keyboard.press('Control+v')
  const choice = b.getByTestId('files-transfer-choice')
  await expect(choice).toContainText('Copy the 1 item(s) these BuckyOS addresses name here?')
  await expect(cell(b, 'Kyoto Trip Plan.md')).toHaveCount(0)
  await choice.getByRole('button', { name: 'Copy here', exact: true }).click()
  await expect(b.getByTestId('batch-results')).toContainText('1 succeeded')
  await expect(cell(b, 'Kyoto Trip Plan.md')).toBeVisible()
})

test('a copy pasted into a collection adds references; links in text/html stand in for a stripped reference list', async ({ page }) => {
  const { a, b } = await twoWindows(page)
  await cell(a, 'Kyoto Trip Plan.md').click()
  await page.keyboard.press('Control+c')
  await b.locator('aside').first().getByRole('button', { name: /Reading List/ }).click()
  await expect(b.getByText('6 items · originals stay in place')).toBeVisible()
  await focusFiles(b)
  await page.keyboard.press('Control+v')
  await expect(b.getByTestId('batch-results')).toContainText('1 succeeded')
  await expect(b.getByText('7 items · originals stay in place')).toBeVisible()
  await closeResults(b)

  // Only the marked links of text/html, as a cross-origin drop keeps them.
  const transfer = await page.evaluateHandle(() => {
    const data = new DataTransfer()
    data.setData('text/html', '<meta charset="utf-8"><a href="cyfs:///home/Documents/Friend%20Contract%20Draft.pdf" data-buckyos-ref="{&quot;v&quot;:1,&quot;kind&quot;:&quot;file&quot;}">Friend Contract Draft.pdf</a>')
    data.setData('text/plain', 'Friend Contract Draft.pdf')
    return data
  })
  const pane = b.locator('[data-pane="left"]')
  await pane.dispatchEvent('dragover', { dataTransfer: transfer })
  await pane.dispatchEvent('drop', { dataTransfer: transfer })
  await expect(b.getByTestId('batch-results')).toContainText('1 succeeded')
  await expect(b.getByText('8 items · originals stay in place')).toBeVisible()
  await expect(cell(a, 'Friend Contract Draft.pdf')).toBeVisible()
})
