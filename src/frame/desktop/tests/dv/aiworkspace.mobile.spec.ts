import { createSample, expect, screenshot, test } from './aiworkspace.fixtures'

test.use({ viewport: { width: 375, height: 812 }, isMobile: true, hasTouch: true })

test('a phone viewport opens a real workspace read-only without horizontal overflow', async ({ owner }, info) => {
  const ws = await createSample(owner)
  const page = owner.page
  const before = (await owner.client.info(ws)).head_seq
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
  await expect(page.getByTestId('aiws-main-toolbar')).toHaveCount(0)
  await expect(page.locator('[contenteditable="true"]')).toHaveCount(0)
  await expect(page.getByRole('heading', { name: '本周项目进展', exact: true })).toBeVisible()
  const toolbar = page.getByTestId('aiws-view-toolbar')
  await expect(toolbar).toBeVisible()
  const box = await toolbar.boundingBox()
  if (!box) throw new Error('mobile toolbar has no visible bounds')
  expect(box.x).toBeGreaterThanOrEqual(0)
  expect(box.x + box.width).toBeLessThanOrEqual(375)
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375)
  await screenshot(page, info, 'mobile-workspace')
  await page.reload()
  await expect(page.getByRole('heading', { name: '本周项目进展', exact: true })).toBeVisible()
  expect((await owner.client.info(ws)).head_seq).toBe(before)
})
