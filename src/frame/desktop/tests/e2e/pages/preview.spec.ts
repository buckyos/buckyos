import { expect, test, type Page } from '@playwright/test'

/**
 * Preview App / Component acceptance (PRD §21) on the mock runtime.
 *
 * File Browser hands a Source + Container Context to the Preview App; the
 * component renders content-first (Auto UI), navigates the session with the
 * keyboard, exits on Esc, and manual "new window" always creates a second,
 * independent window.
 */

async function openFilesAndGoToDocuments(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-files').click()
  const win = page.getByTestId('window-files')
  await expect(win).toBeVisible()
  await page.locator('aside').getByRole('button', { name: /Documents/ }).first().click()
  await expect(page.getByText('Kyoto Trip Plan.md')).toBeVisible()
  return win
}

test.describe('Preview App', () => {
  test('double-click opens the Preview App with a folder session; keyboard navigates; Esc closes', async ({ page }) => {
    const errors: string[] = []
    page.on('pageerror', (err) => errors.push(err.message))

    await openFilesAndGoToDocuments(page)
    await page.getByRole('cell', { name: /^Kyoto Trip Plan\.md/ }).first().dblclick()

    const previewWindow = page.getByTestId('window-preview')
    await expect(previewWindow).toBeVisible()
    const preview = previewWindow.getByTestId('content-preview')
    await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 15_000 })
    await expect(preview).toHaveAttribute('data-renderer', 'text')
    await expect(previewWindow.getByTestId('content-preview-text')).toContainText('Kyoto Trip Plan')

    // Container context: the four files of /home/Documents form the session.
    await expect(preview).toHaveAttribute('data-item-count', '4')
    // Auto UI mode: toolbar hidden until the pointer moves.
    await expect(preview).toHaveAttribute('data-ui-visible', 'false')
    const box = await preview.boundingBox()
    if (!box) throw new Error('preview has no box')
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
    await page.mouse.move(box.x + box.width / 2 + 20, box.y + box.height / 2 + 10)
    await expect(preview).toHaveAttribute('data-ui-visible', 'true')
    await expect(previewWindow.getByTestId('content-preview-counter')).toContainText('of 4')

    // Next item via keyboard: the alias JPEG follows its link and renders as an image.
    await preview.focus()
    await page.keyboard.press('ArrowRight')
    await expect(preview).toHaveAttribute('data-renderer', 'image', { timeout: 15_000 })
    await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 15_000 })
    await expect(previewWindow.getByTestId('content-preview-image')).toBeVisible()
    await expect(previewWindow.locator('[data-testid="window-drag-preview"]')).toContainText('kyoto-temple-0412 (alias).jpg')

    // Image interaction: zoom in changes the reported level; fit restores.
    const level = previewWindow.getByTestId('content-preview-zoom-level')
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
    const before = await level.textContent()
    await page.keyboard.press('+')
    await expect(level).not.toHaveText(before ?? '')

    // Bounded container: Previous from the first item stays put.
    await page.keyboard.press('Home')
    await expect(preview).toHaveAttribute('data-item-index', '0')
    await page.keyboard.press('ArrowLeft')
    await expect(preview).toHaveAttribute('data-item-index', '0')

    // Esc → requestExit → the Preview App closes its window.
    await page.keyboard.press('Escape')
    await expect(previewWindow).toBeHidden()
    expect(errors).toEqual([])
  })

  test('Smart Window reuses the related window; "new window" always creates an independent one', async ({ page }) => {
    await openFilesAndGoToDocuments(page)
    const win = page.getByTestId('window-files')

    await page.getByRole('cell', { name: /^Kyoto Trip Plan\.md/ }).first().dblclick()
    await expect(page.getByTestId('window-preview')).toHaveCount(1)
    await expect(page.getByTestId('content-preview')).toHaveAttribute('data-status', 'ready', { timeout: 15_000 })

    // A sibling from the same folder reuses the automatic window (§13.4).
    // (Raise the Files window first: the preview window overlaps its list.)
    await page.getByTestId('window-drag-files').click()
    await win.getByRole('cell', { name: /^2026 Q1 Review\.docx/ }).first().dblclick()
    await expect(page.getByTestId('window-preview')).toHaveCount(1)
    const preview = page.getByTestId('content-preview')
    // Office document → simulated Pipeline → HTML result with a fidelity note.
    await expect(preview).toHaveAttribute('data-renderer', 'html', { timeout: 20_000 })
    await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 20_000 })

    // Manual new window via the context menu → second, independent window.
    await page.getByTestId('window-drag-files').click()
    await win.getByRole('cell', { name: /^Kyoto Trip Plan\.md/ }).first().click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Open in new Preview window' }).click()
    await expect(page.getByTestId('window-preview')).toHaveCount(2)
    const second = page.getByTestId('window-preview').nth(1)
    await expect(second.getByTestId('content-preview')).toHaveAttribute('data-renderer', 'text', { timeout: 15_000 })
    // The first window keeps showing its own item.
    await expect(page.getByTestId('window-preview').nth(0).getByTestId('content-preview')).toHaveAttribute('data-renderer', 'html')
  })

  test('landing page opens the sample gallery; unsupported, permission and retry states', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-preview').click()
    const previewWindow = page.getByTestId('window-preview')
    await expect(previewWindow).toBeVisible()
    await expect(previewWindow.getByTestId('preview-landing')).toBeVisible()

    await previewWindow.getByTestId('preview-sample-keynote.pptx').click()
    const preview = previewWindow.getByTestId('content-preview')
    await expect(preview).toHaveAttribute('data-status', 'error', { timeout: 15_000 })
    await expect(previewWindow.getByTestId('content-preview-error')).toHaveAttribute('data-error-kind', 'unsupported')

    // Session comes from the sample container: navigate to the locked file.
    await preview.focus()
    await page.keyboard.press('End')
    await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 15_000 })
    await page.keyboard.press('ArrowLeft')
    await expect(previewWindow.getByTestId('content-preview-error')).toHaveAttribute('data-error-kind', 'permission-denied', { timeout: 15_000 })

    // Spreadsheet: first Pipeline attempt fails (retryable), Retry succeeds.
    await page.keyboard.press('Home')
    await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 15_000 })
    for (let i = 0; i < 9; i += 1) await page.keyboard.press('ArrowRight')
    await expect(previewWindow.locator('[data-testid="window-drag-preview"]')).toContainText('budget.xlsx')
    await expect(previewWindow.getByTestId('content-preview-retry')).toBeVisible({ timeout: 20_000 })
    await previewWindow.getByTestId('content-preview-retry').click()
    await expect(preview).toHaveAttribute('data-renderer', 'html', { timeout: 20_000 })
    await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 20_000 })
    await expect(previewWindow.getByTestId('content-preview-fidelity')).toBeVisible()

    // Info panel via shortcut.
    await preview.focus()
    await page.keyboard.press('i')
    await expect(previewWindow.getByTestId('content-preview-info-panel')).toContainText('budget.xlsx')
  })
})
