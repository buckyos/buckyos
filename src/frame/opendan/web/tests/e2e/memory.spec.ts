import { expect, test, type Page } from '@playwright/test'

const open = async (page: Page, query = '', hash = '') => {
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.goto(`/?data=mock&lang=en${query}${hash}`)
  await expect(page.getByTestId('data-source')).toHaveText('data: mock')
  return errors
}

test('home shows the memory counts and what was added most recently', async ({ page }) => {
  const errors = await open(page)
  const card = page.getByTestId('memory-card')
  await expect(card.getByTestId('memory-cognitions')).toContainText('23')
  await expect(card.getByTestId('memory-perceptions')).toContainText('8')
  await expect(card.getByTestId('memory-perceptions')).toContainText('4 to consolidate')
  await expect(card.getByTestId('memory-consolidation').getByRole('link')).toHaveAttribute('href', '#/goal/goal-memory-maintenance')
  await expect(card.getByTestId('memory-consolidation')).toContainText('Last consolidated 1 h ago')
  // Cognitions first, then perceptions.
  await expect(card.getByTestId('memory-recent-cognition')).toHaveCount(5)
  await expect(card.getByTestId('memory-recent-perception')).toHaveCount(3)
  await expect(card.locator('li').first()).toContainText('weekly reports as a single PDF')
  await expect(card.locator('li').first()).toContainText('did:bns:devtest')
  await expect(card.locator('li').nth(5)).toContainText('report before 9 am')
  // The page's left column: profile, overview, memory, goals, skills, workspaces, runtimes.
  const order = await page.locator('[data-testid="work-overview"], [data-testid="memory-card"], [data-testid="goals"], [data-testid="skills-card"], [data-testid="workspaces"], [data-testid="known-runtimes"]')
    .evaluateAll((nodes) => nodes.map((n) => n.getAttribute('data-testid')))
  expect(order).toEqual(['work-overview', 'memory-card', 'goals', 'skills-card', 'workspaces', 'known-runtimes'])
  await card.getByTestId('memory-all').click()
  await expect(page.getByTestId('memory-page')).toBeVisible()
  expect(errors).toEqual([])
})

test('an empty memory and a missing consolidation goal are said plainly', async ({ page }) => {
  await open(page, '&memory=empty')
  const card = page.getByTestId('memory-card')
  await expect(card.getByTestId('memory-cognitions')).toContainText('0')
  await expect(card.getByTestId('memory-perceptions')).toContainText('0 to consolidate')
  await expect(card.getByTestId('memory-consolidation')).toHaveText('This agent has no memory consolidation goal.')
  await expect(card).toContainText('Nothing remembered yet.')
  await page.goto('/?data=mock&lang=en&memory=empty#/memory')
  await expect(page.getByTestId('memory-list-cognition')).toContainText('Nothing remembered yet.')
})

test('the memory page pages through both lists and deletes a cognition', async ({ page }) => {
  const errors = await open(page, '', '#/memory')
  const cognitions = page.getByTestId('memory-list-cognition')
  const rows = cognitions.locator('[data-testid^="memory-entry-"]')
  await expect(rows.first()).toContainText('weekly reports as a single PDF')
  // The second page arrives when the end of the first comes into view; nothing is repeated or lost.
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight))
  await expect(rows).toHaveCount(23)
  await expect(cognitions.getByTestId('memory-more')).toHaveCount(0)
  const references = await rows.evaluateAll((nodes) => nodes.map((n) => n.getAttribute('data-testid')))
  expect(new Set(references).size).toBe(23)
  expect(references[0]).toBe('memory-entry-cog:023')
  expect(references[22]).toBe('memory-entry-cog:001')

  const second = cognitions.getByTestId('memory-entry-cog:022')
  await expect(second).toContainText('revised')
  await second.getByRole('button', { expanded: false }).click()
  await expect(second.getByTestId('memory-detail')).toContainText('"confidence": 0.8')
  await expect(second.getByTestId('memory-detail')).toContainText('cog:022')

  await second.getByTestId('memory-delete').click()
  const dialog = page.getByTestId('memory-forget')
  await expect(dialog).toContainText('The agent no longer uses it. Its earlier revisions are kept')
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await expect(rows).toHaveCount(23)
  await second.getByTestId('memory-delete').click()
  await dialog.getByRole('button', { name: 'Delete' }).click()
  await expect(dialog).toHaveCount(0)
  await expect(rows).toHaveCount(22)
  await expect(cognitions.getByTestId('memory-entry-cog:022')).toHaveCount(0)

  // A perception already consolidated is not deleted here: the cognition that took it in is.
  const perceptions = page.getByTestId('memory-list-perception')
  await expect(perceptions.locator('[data-testid^="memory-entry-"]')).toHaveCount(8)
  const absorbed = perceptions.getByTestId('memory-entry-per:ui-7f3a2c91:37')
  await expect(absorbed).toContainText('Consolidated into a cognition')
  await expect(absorbed.getByTestId('memory-absorbed')).toContainText('cog:023')
  await expect(absorbed.getByTestId('memory-delete')).toHaveCount(0)
  const pending = perceptions.getByTestId('memory-entry-per:ui-7f3a2c91:40')
  await expect(pending).toContainText('To consolidate')
  await pending.getByTestId('memory-delete').click()
  await expect(dialog).toContainText('discarded before consolidation')
  await dialog.getByRole('button', { name: 'Delete' }).click()
  await expect(perceptions.locator('[data-testid^="memory-entry-"]')).toHaveCount(7)

  await page.getByTestId('back-home').click()
  await expect(page.getByTestId('memory-card').getByTestId('memory-cognitions')).toContainText('22')
  await expect(page.getByTestId('memory-card').getByTestId('memory-perceptions')).toContainText('7')
  expect(errors).toEqual([])
})

test('while the agent consolidates its memory a delete is refused and the entry stays', async ({ page }) => {
  await open(page, '&memory=busy', '#/memory')
  const cognitions = page.getByTestId('memory-list-cognition')
  const first = cognitions.getByTestId('memory-entry-cog:023')
  await first.getByTestId('memory-delete').click()
  const dialog = page.getByTestId('memory-forget')
  await dialog.getByRole('button', { name: 'Delete' }).click()
  await expect(dialog.getByRole('alert')).toContainText('consolidating its memory right now. Try again in a moment.')
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await expect(first).toBeVisible()
})

test.describe('on a phone', () => {
  test.use({ viewport: { width: 375, height: 740 }, hasTouch: true, isMobile: true })

  test('the two lists are tabs, cognitions first', async ({ page }) => {
    await open(page, '', '#/memory')
    await expect(page.getByRole('tab', { name: 'Cognitions' })).toHaveAttribute('aria-selected', 'true')
    await expect(page.getByTestId('memory-list-cognition')).toBeVisible()
    await expect(page.getByTestId('memory-list-perception')).toBeHidden()
    await page.getByRole('tab', { name: 'Perceptions' }).click()
    await expect(page.getByTestId('memory-list-perception')).toBeVisible()
    await expect(page.getByTestId('memory-list-cognition')).toBeHidden()
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
    expect(overflow).toBeLessThanOrEqual(0)
  })
})
