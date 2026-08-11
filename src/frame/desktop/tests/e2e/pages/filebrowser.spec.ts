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
