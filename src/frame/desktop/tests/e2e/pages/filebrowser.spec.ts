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

    // Click a Topic in the sidebar — main content should switch to topic aggregation banner.
    await page
      .locator('aside')
      .getByRole('button', { name: /Kyoto trip · April 42 6 days/ })
      .click()
    await expect(page.getByText('Topic view')).toBeVisible()
    await expect(page.getByText('Aggregated · not copied')).toBeVisible()

    // Run a search — AI-enhanced matches should appear for "trip".
    await page.getByPlaceholder(/Search across files/).fill('trip')
    await expect(page.getByText('Search results')).toBeVisible()
    await expect(page.getByText(/AI-enhanced matches/)).toBeVisible()

    expect(consoleErrors).toEqual([])
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
