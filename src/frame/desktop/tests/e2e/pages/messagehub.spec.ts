import { expect, test } from '@playwright/test'

async function getComposerMetrics(page: Parameters<typeof test>[0]['page']) {
  return page.evaluate(() => {
    const textarea = document.querySelector('textarea')
    if (!(textarea instanceof HTMLTextAreaElement)) {
      return null
    }

    let current: HTMLElement | null = textarea
    const chain: HTMLElement[] = []
    while (current && chain.length < 6) {
      chain.push(current)
      current = current.parentElement
    }

    const pane = chain[5]
    const content = pane?.querySelector<HTMLElement>('[data-composer-content]')

    return {
      textareaHeight: Math.round(textarea.getBoundingClientRect().height),
      paneHeight: pane ? Math.round(pane.getBoundingClientRect().height) : null,
      contentHeight: content ? Math.round(content.getBoundingClientRect().height) : null,
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
  expect(initial?.paneHeight).toBe(72)

  await textarea.click()
  await page.keyboard.type('line 1')
  await page.keyboard.press('Shift+Enter')
  await page.keyboard.type('line 2')
  await page.keyboard.press('Shift+Enter')
  await page.keyboard.type('line 3')
  await page.waitForTimeout(100)

  const multiline = await getComposerMetrics(page)
  expect(multiline?.paneHeight).toBeGreaterThan(initial?.paneHeight ?? 0)
  expect(multiline?.paneHeight).toBe(multiline?.contentHeight)

  await page.locator('input[type="file"]').nth(0).setInputFiles([
    '/Users/liuzhicong/project/buckyos_webdesktop/package.json',
  ])
  await page.waitForTimeout(100)

  const withAttachment = await getComposerMetrics(page)
  expect(withAttachment?.paneHeight).toBeGreaterThan(multiline?.paneHeight ?? 0)

  await page.getByRole('button', { name: /Clear|清空/ }).click()
  await page.waitForTimeout(100)

  const afterClear = await getComposerMetrics(page)
  expect(afterClear?.paneHeight).toBe(multiline?.paneHeight)
  expect(afterClear?.paneHeight).toBe(afterClear?.contentHeight)
})
