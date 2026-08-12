import { expect, test } from '@playwright/test'

async function getComposerMetrics(page: Parameters<typeof test>[0]['page']) {
  return page.evaluate(() => {
    const textarea = document.querySelector('textarea')
    const composer = document.querySelector('[data-testid="message-composer"]')
    if (
      !(textarea instanceof HTMLTextAreaElement)
      || !(composer instanceof HTMLElement)
    ) {
      return null
    }

    return {
      textareaHeight: Math.round(textarea.getBoundingClientRect().height),
      composerHeight: Math.round(composer.getBoundingClientRect().height),
    }
  })
}

test('messagehub composer auto-resizes for multiline text and attachments', async ({
  page,
}) => {
  await page.goto('/messagehub')

  const textarea = page.locator('textarea')
  await expect(textarea).toBeVisible()

  const initial = await getComposerMetrics(page)
  expect(initial).not.toBeNull()

  await textarea.click()
  await page.keyboard.type('line 1')
  await page.keyboard.press('Shift+Enter')
  await page.keyboard.type('line 2')
  await page.keyboard.press('Shift+Enter')
  await page.keyboard.type('line 3')

  await expect.poll(async () => (await getComposerMetrics(page))?.composerHeight ?? 0)
    .toBeGreaterThan(initial?.composerHeight ?? 0)
  const multiline = await getComposerMetrics(page)
  expect(multiline?.textareaHeight).toBeGreaterThan(initial?.textareaHeight ?? 0)

  await page.locator('input[type="file"]').nth(0).setInputFiles([
    'package.json',
  ])

  await expect.poll(async () => (await getComposerMetrics(page))?.composerHeight ?? 0)
    .toBeGreaterThan(multiline?.composerHeight ?? 0)

  await page.getByRole('button', { name: /Clear|清空/ }).click()

  await expect.poll(async () => (await getComposerMetrics(page))?.composerHeight ?? 0)
    .toBe(multiline?.composerHeight)
})
