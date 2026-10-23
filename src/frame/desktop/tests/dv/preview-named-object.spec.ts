import { expect, test, type Locator, type Page, type TestInfo } from '@playwright/test'

const password = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || ''
const objectId = process.env.BUCKYOS_TEST_PREVIEW_OBJECT_ID || ''
const taskId = process.env.BUCKYOS_TEST_PREVIEW_TASK_ID || ''

test.use({ timezoneId: 'America/Los_Angeles' })
test.skip(!password || !objectId, 'Set the administrator password and a stored video Object ID.')

async function login(page: Page) {
  await page.goto('/')
  await page.getByLabel('Username', { exact: true }).fill(process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest')
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  await expect(page.getByTestId('desktop-app-preview')).toBeVisible()
}

function trackErrors(page: Page, info: TestInfo) {
  const errors: string[] = []
  page.on('pageerror', (error) => errors.push(error.message))
  page.on('console', (message) => {
    if (message.type() !== 'error') return
    const text = message.text()
    if (text === 'An SSL certificate error occurred when fetching the script.') {
      info.annotations.push({ type: 'known-environment-error', description: 'DV certificate blocks service worker registration in Chromium.' })
    } else if (message.location().url.endsWith('/sso_refresh') && text.includes('401 (Unauthorized)')) {
      info.annotations.push({ type: 'known-auth-probe', description: 'The anonymous session refresh returns 401 before sign-in.' })
    } else errors.push(text)
  })
  return errors
}

async function verifyTaskWidth(page: Page) {
  for (const main of await page.getByRole('main').all()) {
    expect(await main.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(1)
  }
}

async function verifyVideo(preview: Locator, page: Page, info: TestInfo) {
  await expect(preview).toHaveAttribute('data-status', 'ready')
  await expect(preview).toHaveAttribute('data-renderer', 'video')
  const video = preview.getByTestId('content-preview-video')
  await expect(video).toBeVisible()
  await expect.poll(() => video.evaluate((element: HTMLVideoElement) => element.readyState)).toBeGreaterThanOrEqual(2)
  await preview.focus()
  await page.keyboard.press('Space')
  await expect.poll(() => video.evaluate((element: HTMLVideoElement) => element.currentTime)).toBeGreaterThan(0.1)
  await page.keyboard.press('Space')
  const metadata = await video.evaluate((element: HTMLVideoElement) => ({
    width: element.videoWidth, height: element.videoHeight, duration: element.duration,
    currentTime: element.currentTime, error: element.error?.message ?? null,
  }))
  expect(metadata.width).toBeGreaterThan(0)
  expect(metadata.height).toBeGreaterThan(0)
  expect(metadata.duration).toBeGreaterThan(0)
  expect(metadata.error).toBeNull()
  await info.attach('video-metadata', { body: JSON.stringify(metadata), contentType: 'application/json' })
  await page.screenshot({ path: info.outputPath('video.png'), fullPage: true })
}

test('manual Object ID opens and plays the stored video', async ({ page }, info) => {
  const errors = trackErrors(page, info)
  await login(page)
  await page.getByTestId('desktop-app-preview').click()
  await page.getByTestId('preview-landing-input').fill(objectId)
  const metadataResponse = page.waitForResponse((response) => response.url().endsWith('/ndm/v1/store/get_object'))
  const contentResponse = page.waitForResponse((response) => response.url().endsWith('/ndm/proxy/v1/read/object/open'))
  await page.getByTestId('preview-landing-open').click()
  expect((await metadataResponse).status()).toBe(200)
  expect((await contentResponse).status()).toBe(200)
  await verifyVideo(page.getByTestId('content-preview'), page, info)
  expect(errors).toEqual([])
})

for (const width of [1440, 375]) {
  test(`completed task previews its video at ${width}px`, async ({ page }, info) => {
    test.skip(!taskId, 'Set the completed AICC task ID containing the video.')
    const errors = trackErrors(page, info)
    await login(page)
    await page.setViewportSize({ width, height: 900 })
    await page.goto(`/taskcenter?taskid=${encodeURIComponent(taskId)}`)
    const results = page.getByTestId('task-results')
    await expect(results).toContainText('video/mp4')
    await verifyTaskWidth(page)
    const contentResponse = page.waitForResponse((response) =>
      response.url().endsWith('/ndm/proxy/v1/read/object/open')
      && response.request().postDataJSON()?.obj_id === objectId)
    await results.getByTestId('task-result-preview').first().click()
    expect((await contentResponse).status()).toBe(200)
    await expect(page.getByTestId('task-result-content-preview')).toBeFocused()
    await verifyVideo(page.getByTestId('task-result-content-preview'), page, info)
    await verifyTaskWidth(page)
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width)
    await page.getByTestId('content-preview-exit').click()
    await expect(page.getByTestId('task-result-content-preview')).toHaveCount(0)
    expect(errors).toEqual([])
  })
}
