import { expect, test, type Page } from '@playwright/test'

const BUILTIN = process.env.BUILTIN_GOAL_ID ?? ''
const DV_GOAL = process.env.GOAL_ID ?? ''
const OUT = process.env.EVIDENCE_DIR ?? 'test-results-zone'

const signIn = async (page: Page) => {
  await page.locator('input[type="text"]').fill('devtest')
  await page.locator('input[type="password"]').fill('bucky2025')
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
}

for (const width of [390, 1280]) {
  test(`goal gates on the real zone at ${width}px`, async ({ page }) => {
    test.setTimeout(120_000)
    await page.setViewportSize({ width, height: 844 })
    const errors: string[] = []
    page.on('pageerror', (e) => errors.push(e.message))
    await page.goto('/?lang=en')
    await expect(page).toHaveURL(/\/login\?/)
    await signIn(page)
    await expect(page.getByTestId('data-source')).toHaveText('data: krpc')
    await expect(page.getByTestId('goals')).toBeVisible()
    await page.screenshot({ path: `${OUT}/${width}-00-goals.png`, fullPage: true })

    await page.goto(`/?lang=en#/goal/${BUILTIN}`)
    await expect(page.getByTestId('gate-script')).toContainText('package')
    await expect(page.getByTestId('gate-timetable')).toBeVisible()
    await expect(page.getByTestId('gate-last-check')).toBeVisible()
    await expect(page.getByTestId('gate-guarantee')).toBeVisible()
    await page.getByTestId('gate-script-toggle').click()
    await expect(page.getByTestId('gate-script-text')).toContainText('memory.status')
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)
    expect(overflow).toBeLessThanOrEqual(0)
    await page.screenshot({ path: `${OUT}/${width}-01-builtin-gates.png`, fullPage: true })

    if (DV_GOAL) {
      await page.goto(`/?lang=en#/goal/${DV_GOAL}`)
      await expect(page.getByTestId('gate-counts')).toBeVisible()
      await expect(page.getByTestId('cost-period')).toBeVisible()
      await page.screenshot({ path: `${OUT}/${width}-02-dv-goal.png`, fullPage: true })
    }

    await page.goto('/?lang=en#/skills')
    await expect(page.getByTestId('skills-crystallize-switch')).toBeVisible()
    await expect(page.getByTestId('skills-crystallize-state')).toBeVisible()
    await page.screenshot({ path: `${OUT}/${width}-03-skills.png`, fullPage: true })
    expect(errors).toEqual([])
  })
}
