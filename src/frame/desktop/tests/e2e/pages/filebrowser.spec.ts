import { expect, test } from '@playwright/test'

test.describe('File browser app panel', () => {
  test('desktop: sidebar, preview panel, search + topic aggregation', async ({ page }) => {
    const consoleErrors: string[] = []
    page.on('console', (message) => {
      if (message.type() === 'error') consoleErrors.push(message.text())
    })

    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal')

    await page.getByTestId('desktop-app-files').click()
    await expect(page.getByTestId('window-files')).toBeVisible()

    // Top bar tabs are present.
    await expect(
      page.locator('[data-testid="window-files"]').getByText('Home', { exact: true }).first(),
    ).toBeVisible()

    // Sidebar header "AI Topics".
    await expect(page.getByText('AI Topics').first()).toBeVisible()
    // Home folder entries should be in the main list.
    await expect(
      page.getByRole('cell', { name: /^Documents(\s|$)/ }).first(),
    ).toBeVisible()
    await expect(
      page.getByRole('cell', { name: /^Pictures(\s|$)/ }).first(),
    ).toBeVisible()

    // Navigate to Documents from the sidebar (DFS tree).
    await page
      .locator('aside')
      .getByRole('button', { name: /Documents/ })
      .first()
      .click()
    await expect(page.getByText('Kyoto Trip Plan.md')).toBeVisible()

    // Select Kyoto Trip Plan → preview panel renders AI summary.
    await page.getByText('Kyoto Trip Plan.md').click()
    await expect(
      page.getByText('Day-by-day itinerary', { exact: false }),
    ).toBeVisible()
    // Status bar surfaces the selected file path.
    await expect(
      page.getByText('/home/Documents/Kyoto Trip Plan.md').first(),
    ).toBeVisible()

    // Click a Topic in the sidebar — main content shows the generic view banner.
    await page
      .locator('aside')
      .getByRole('button', { name: /Kyoto trip · April/ })
      .first()
      .click()
    await expect(page.getByText('View: Kyoto trip · April')).toBeVisible()
    await expect(page.getByText('Aggregated · not copied')).toBeVisible()

    // Run a search — AI-enhanced matches should appear for "trip".
    // (The desktop search input is collapsed behind the toolbar Search button.)
    await page
      .locator('[data-testid="window-files"]')
      .getByRole('button', { name: 'Search', exact: true })
      .click()
    await page.getByPlaceholder(/Search across files/).fill('trip')
    await expect(page.getByText('Search results')).toBeVisible()
    await expect(page.getByText(/AI-enhanced matches/)).toBeVisible()

    expect(consoleErrors).toEqual([])
  })

  test('desktop: stress folder virtualizes 10k entries and sorts via the reader', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-files').click()
    const win = page.getByTestId('window-files')

    // Open /home/stress-10k from the Home listing.
    await win.getByRole('cell', { name: /^stress-10k/ }).dblclick()
    await expect(win.getByText(/10,?000 items/)).toBeVisible()

    // Virtualization: DOM row count stays bounded regardless of 10k entries.
    const rows = win.locator('[role="row"]')
    expect(await rows.count()).toBeLessThan(100)

    // Scroll deep into the list — skeletons resolve into rows, count stays bounded.
    const scroller = win.locator('div.overflow-y-auto').filter({ has: page.locator('[role="table"]') })
    await scroller.evaluate((el) => {
      el.scrollTop = el.scrollHeight / 2
    })
    await expect(win.getByText(/item-[45]\d{3}\./).first()).toBeVisible()
    expect(await rows.count()).toBeLessThan(100)

    // Sort switch goes through the reader (loading state) and re-renders.
    await win.getByRole('button', { name: 'Sort by' }).click()
    await page.getByRole('menuitem', { name: 'Size' }).click()
    await expect(win.getByText(/10,?000 items/)).toBeVisible()
    expect(await rows.count()).toBeLessThan(100)
  })

  test('desktop: collection add → reorder → remove round-trip on the mock store', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-files').click()
    const win = page.getByTestId('window-files')
    const sidebar = win.locator('aside').first()

    // Add a Documents file to the seed collection from the context menu.
    await sidebar.getByRole('button', { name: /Documents/ }).first().click()
    await win.getByRole('cell', { name: /2026 Q1 Review/ }).click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Add to Collection' }).click()
    await page.getByRole('menuitem', { name: 'Reading List' }).click()

    // The collection lists the new reference with the collection banner.
    await sidebar.getByRole('button', { name: /Reading List/ }).click()
    await expect(win.getByText('Collection: Reading List')).toBeVisible()
    const reviewRow = win.locator('[role="row"]').filter({ hasText: '2026 Q1 Review' })
    await expect(reviewRow).toBeVisible()

    // Manual order is adjustable (order drives the default preview flow).
    const rowTexts = () =>
      win.locator('[role="row"]').allTextContents().then((texts) => texts.slice(1))
    const before = await rowTexts()
    const fromIndex = before.findIndex((text) => text.includes('2026 Q1 Review'))
    await reviewRow.click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Move up' }).click()
    await expect
      .poll(async () => {
        const after = await rowTexts()
        return after.findIndex((text) => text.includes('2026 Q1 Review'))
      })
      .toBe(fromIndex - 1)

    // Removing drops only the reference…
    await reviewRow.click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Remove from collection' }).click()
    await expect(reviewRow).toHaveCount(0)

    // …the original file is untouched in its folder.
    await sidebar.getByRole('button', { name: /Documents/ }).first().click()
    await expect(win.getByRole('cell', { name: /2026 Q1 Review/ })).toBeVisible()
  })

  test('desktop: unknown-total view loads sequentially via the sentinel row', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-files').click()
    const win = page.getByTestId('window-files')
    const sidebar = win.locator('aside').first()

    // The diagnostics views live behind advanced mode.
    await sidebar.getByRole('button', { name: 'Enable advanced mode' }).click()
    await sidebar.getByRole('button', { name: 'Unknown total' }).click()

    // First cursor page (40 items) leaves the skeleton; status shows "40+".
    await expect(win.getByText('View: Demo · unknown total')).toBeVisible()
    await expect(win.getByText(/40\+ items/)).toBeVisible()

    // Scrolling to the sentinel demand-loads the next cursor pages.
    const scroller = win
      .locator('div.overflow-y-auto')
      .filter({ has: page.locator('[role="table"]') })
    await scroller.evaluate((el) => {
      el.scrollTop = el.scrollHeight
    })
    await expect
      .poll(async () => {
        await scroller.evaluate((el) => {
          el.scrollTop = el.scrollHeight
        })
        const text = await win
          .getByText(/\d+\+ items/)
          .first()
          .textContent()
        return Number.parseInt(text ?? '0', 10)
      })
      .toBeGreaterThan(40)
  })

  test('desktop: async search — partial banner, error retry, cursor load-more', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-files').click()
    const win = page.getByTestId('window-files')

    await win.getByRole('button', { name: 'Search', exact: true }).click()
    const input = page.getByPlaceholder(/Search across files/)

    // Deterministic partial scenario: degraded sources surface a banner but
    // results stay usable.
    await input.fill('partial:trip')
    await expect(page.getByTestId('search-partial-banner')).toBeVisible()
    await expect(
      page.getByText(/Traditional matches|AI-enhanced matches/).first(),
    ).toBeVisible()

    // Deterministic error scenario: search-scoped error with Retry.
    await input.fill('error:anything')
    await expect(
      page.getByText('Search backend unavailable', { exact: false }),
    ).toBeVisible()
    await expect(win.getByRole('button', { name: 'Retry' })).toBeVisible()

    // Broad query pages at 8 hits — the cursor continuation appends more.
    await input.fill('e')
    await expect(page.getByTestId('search-load-more')).toBeVisible()
    const countBefore = await win.locator('[class*="rounded-[16px]"]').count()
    await page.getByTestId('search-load-more').click()
    await expect
      .poll(() => win.locator('[class*="rounded-[16px]"]').count())
      .toBeGreaterThan(countBefore)
  })

  test('desktop: collection and folder forms validate through the Zod schemas', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-files').click()
    const win = page.getByTestId('window-files')
    const sidebar = win.locator('aside').first()

    // New collection: empty title is rejected by collectionTitleSchema.
    await sidebar.getByRole('button', { name: 'New collection…' }).click()
    const dialog = page.getByTestId('name-prompt-dialog')
    await expect(dialog).toBeVisible()
    await dialog.getByRole('button', { name: 'Create' }).click()
    await expect(dialog.getByText('A collection title is required')).toBeVisible()
    await dialog.getByRole('textbox').fill('My Papers')
    await dialog.getByRole('button', { name: 'Create' }).click()
    await expect(win.getByText('Collection: My Papers')).toBeVisible()

    // New folder in a writable location: reserved names are rejected, valid
    // names create a real (mock) folder that shows up after invalidation.
    await sidebar.getByRole('button', { name: /Documents/ }).first().click()
    await win
      .locator('div.overflow-y-auto')
      .filter({ has: page.locator('[role="table"]') })
      .click({ button: 'right', position: { x: 420, y: 420 } })
    await page.getByRole('menuitem', { name: 'New folder' }).click()
    await dialog.getByRole('textbox').fill('..')
    await dialog.getByRole('button', { name: 'Create' }).click()
    await expect(dialog.getByText('This name is reserved')).toBeVisible()
    await dialog.getByRole('textbox').fill('Fixtures')
    await dialog.getByRole('button', { name: 'Create' }).click()
    await expect(win.getByRole('cell', { name: /^Fixtures/ })).toBeVisible()

    // Rename through the same schema-driven form.
    await win.getByRole('cell', { name: /^Fixtures/ }).click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Rename…' }).click()
    await expect(dialog.getByRole('textbox')).toHaveValue('Fixtures')
    await dialog.getByRole('textbox').fill('Fixtures v2')
    await dialog.getByRole('button', { name: 'Rename' }).click()
    await expect(win.getByRole('cell', { name: /^Fixtures v2/ })).toBeVisible()
  })

  test('desktop: upload runs the probe→upload→commit lifecycle with retryable failure', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-files').click()
    const win = page.getByTestId('window-files')
    const sidebar = win.locator('aside').first()
    await sidebar.getByRole('button', { name: /Documents/ }).first().click()
    await expect(win.getByText('Kyoto Trip Plan.md')).toBeVisible()

    // Success path: the committed entry appears in the destination listing.
    const chooser1 = page.waitForEvent('filechooser')
    await win.getByRole('button', { name: 'Upload', exact: true }).click()
    await (
      await chooser1
    ).setFiles({
      name: 'notes-upload.txt',
      mimeType: 'text/plain',
      buffer: Buffer.from('hello from e2e'),
    })
    await expect(page.getByTestId('transfers-panel')).toBeVisible()
    await expect(page.getByTestId('transfer-success')).toBeVisible({ timeout: 15000 })
    await expect(win.getByRole('cell', { name: /notes-upload\.txt/ })).toBeVisible()

    // Deterministic failure ("fail" in the name) keeps retry context; retrying
    // completes the transfer.
    const chooser2 = page.waitForEvent('filechooser')
    await win.getByRole('button', { name: 'Upload', exact: true }).click()
    await (
      await chooser2
    ).setFiles({
      name: 'fail-clip.txt',
      mimeType: 'text/plain',
      buffer: Buffer.from('destined to fail once'),
    })
    const failedRow = page.getByTestId('transfer-error')
    await expect(failedRow).toBeVisible({ timeout: 15000 })
    await failedRow.getByRole('button', { name: 'Retry' }).click()
    await expect(page.getByTestId('transfer-success')).toHaveCount(2, { timeout: 15000 })
    await expect(win.getByRole('cell', { name: /fail-clip\.txt/ })).toBeVisible()
  })

  test('desktop: a failing sidebar source stays isolated and retries', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal&fbFail=topics')
    await page.getByTestId('desktop-app-files').click()
    const win = page.getByTestId('window-files')
    const sidebar = win.locator('aside').first()

    // Topics failed — inline error in that section only; DFS still works.
    const sectionError = sidebar.getByTestId('sidebar-section-error')
    await expect(sectionError).toBeVisible()
    await expect(sidebar.getByRole('button', { name: /Documents/ }).first()).toBeVisible()

    // Retry recovers the source.
    await sectionError.getByRole('button', { name: 'Retry' }).click()
    await expect(sidebar.getByRole('button', { name: /Kyoto trip · April/ })).toBeVisible()
  })

  test('desktop: public folder surfaces Public URL column', async ({ page }) => {
    const consoleErrors: string[] = []
    page.on('console', (message) => {
      if (message.type() === 'error') consoleErrors.push(message.text())
    })

    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-files').click()

    // Navigate to /public via the sidebar.
    await page
      .locator('aside')
      .getByRole('button', { name: /^Public$/ })
      .click()
    await expect(
      page.getByRole('cell', { name: /^resume\.pdf$/ }),
    ).toBeVisible()
    // Public URL header is visible.
    await expect(
      page.getByRole('columnheader', { name: /Public URL/ }),
    ).toBeVisible()
    // Public URL value is rendered.
    await expect(
      page.getByText('https://alice.personal.buckyos.dev/public/resume.pdf'),
    ).toBeVisible()

    expect(consoleErrors).toEqual([])
  })
})
