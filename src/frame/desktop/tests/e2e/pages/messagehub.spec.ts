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

for (const viewport of [{ width: 1440, height: 900 }, { width: 375, height: 812 }]) {
  test(`codeassistant wheel scrolling preserves message positions at ${viewport.width}px`, async ({
    page,
  }) => {
    const errors: string[] = []
    page.on('pageerror', (error) => errors.push(error.message))
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text())
    })
    await page.addInitScript(() => {
      window.addEventListener('error', (event) => console.error(event.message))
    })

    await page.setViewportSize(viewport)
    await page.route('https://upload.wikimedia.org/**', (route) => route.fulfill({
      contentType: 'image/svg+xml',
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"><rect width="800" height="600" fill="lightblue"/></svg>',
    }))
    await page.goto('/messagehub')

    const history = page.locator('.shell-scrollbar').filter({ has: page.locator('[data-index]') })
    await expect.poll(async () => Number(
      await history.locator('[data-index]').last().getAttribute('data-index'),
    )).toBeGreaterThan(2000)
    await expect.poll(() => history.evaluate((element) => (
      element.scrollHeight - element.clientHeight - element.scrollTop
    ))).toBeLessThanOrEqual(1)
    await expect.poll(() => history.locator('img').evaluateAll((images) => (
      images.every((image) => image.complete)
    ))).toBe(true)
    await history.hover()

    for (const delta of [-220, 220]) {
      for (let step = 0; step < 40; step += 1) {
        const anchor = await history.evaluate((element, delta) => {
          const viewport = element.getBoundingClientRect()
          const row = Array.from(element.querySelectorAll<HTMLElement>('[data-index]'))
            .find((item) => item.getBoundingClientRect().bottom > viewport.top + viewport.height / 2)!

          return {
            index: row.dataset.index,
            top: row.getBoundingClientRect().top,
            delta: delta < 0
              ? Math.max(delta, -element.scrollTop)
              : Math.min(delta, element.scrollHeight - element.clientHeight - element.scrollTop),
          }
        }, delta)

        await page.mouse.wheel(0, delta)
        await page.waitForTimeout(100)

        const top = await history.locator(`[data-index="${anchor.index}"]`)
          .evaluate((element) => element.getBoundingClientRect().top)
        expect(Math.abs(top - anchor.top + anchor.delta), `wheel ${delta}, step ${step}`)
          .toBeLessThanOrEqual(1)
      }
    }

    await history.hover()
    await page.mouse.wheel(0, -440)
    const bottomButton = page.getByRole('button', { name: 'Scroll to bottom' })
    await expect(bottomButton).toBeVisible()
    await bottomButton.click()
    await expect.poll(() => history.evaluate((element) => (
      element.scrollHeight - element.clientHeight - element.scrollTop
    ))).toBeLessThanOrEqual(1)

    await history.hover()
    await page.mouse.wheel(0, -220)
    await page.waitForTimeout(600)
    await expect(bottomButton).toBeVisible()
    expect(await history.evaluate((element) => (
      element.scrollHeight - element.clientHeight - element.scrollTop
    ))).toBeGreaterThan(200)

    await page.screenshot({ path: `test-results/messagehub-scroll-${viewport.width}.png` })
    expect(errors).toEqual([])
  })
}
