import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { expect, test, type Locator, type Page } from '@playwright/test'

/**
 * iPhone media on a real zone (notepads/preview-iphone-media-compatibility-todo.md §6):
 * HEIC photos and HEVC / H.264 MOV videos are picked, uploaded through HomeStation, published,
 * shown in the feed as Pipeline thumbnails and in the detail page / immersive mode through the
 * Preview Pipeline (Chromium on Linux decodes neither HEIC nor HEVC), and the original bytes
 * download unchanged. Needs ffmpeg ≥ 7.1 on the node (nfs-server `hello.repr`).
 *
 * Samples: PREVIEW_DV_SAMPLES (a folder with the file names below), default the real
 * libheif / x265 encoded fixtures of nfs_server's Pipeline tests.
 */

const adminUser = process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest'
const adminPassword = process.env.BUCKYOS_TEST_ADMIN_PASSWORD || 'bucky2025'
const friendUser = process.env.BUCKYOS_TEST_USER || 'bob'
const friendPassword = process.env.BUCKYOS_TEST_USER_PASSWORD || 'buckyos2026'
const samples = process.env.PREVIEW_DV_SAMPLES || resolve(import.meta.dirname, '../../../nfs_server/tests/fixtures/preview')
const sample = (name: string) => readFileSync(resolve(samples, name))
const sha256 = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex')
const CACHE_URL = /^(https:\/\/[^/]+)?\/bucket\/v1\/zone\/nfs-server\/_\/preview-cache\/[0-9a-f]{64}\?token=bkt_/

async function loginThroughUi(page: Page, user = adminUser, password = adminPassword): Promise<void> {
  await page.goto('/')
  await expect(page.getByLabel('Username')).toBeVisible()
  await page.getByLabel('Username').fill(user)
  await page.getByLabel('Password').fill(password)
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  await expect(page.getByRole('button', { name: 'BuckyOS' })).toBeVisible()
}

/** Chromium leaves `File.type` empty for HEIC: the upload must decide by the bytes. */
async function publish(page: Page, text: string, files: { name: string; file: string; mimeType: string }[]) {
  await page.getByTestId('hs-quick-text').fill(text)
  await page.getByTestId('hs-quick-file').setInputFiles(files.map(f => ({ name: f.name, mimeType: f.mimeType, buffer: sample(f.file) })))
  const attachments = page.getByTestId('hs-quick-attachments').locator('li')
  await expect(attachments).toHaveCount(files.length)
  await expect(page.getByTestId('hs-quick-attachments').locator('li[data-status="uploaded"]')).toHaveCount(files.length, { timeout: 60_000 })
  await page.getByTestId('hs-quick-submit').click()
  await expect(page.getByTestId('hs-toast').filter({ hasText: 'Published to your feed' }).last()).toBeVisible({ timeout: 30_000 })
  // The composer resets after the publish settles; the next post starts from an empty form.
  await expect(page.getByTestId('hs-quick-attachments')).toHaveCount(0, { timeout: 30_000 })
  await expect(page.getByTestId('hs-quick-text')).toHaveValue('')
}

async function imageSize(img: Locator) {
  await expect.poll(() => img.evaluate(el => (el as HTMLImageElement).complete && (el as HTMLImageElement).naturalWidth > 0), { timeout: 60_000 }).toBe(true)
  return img.evaluate(el => ({ width: (el as HTMLImageElement).naturalWidth, height: (el as HTMLImageElement).naturalHeight, src: el.getAttribute('src') ?? '' }))
}

async function playsConverted(preview: Locator, portrait: boolean) {
  await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 90_000 })
  await expect(preview).toHaveAttribute('data-via', 'pipeline')
  const video = preview.getByTestId('content-preview-video')
  expect(await video.getAttribute('src')).toMatch(CACHE_URL)
  const size = await video.evaluate(el => ({ width: (el as HTMLVideoElement).videoWidth, height: (el as HTMLVideoElement).videoHeight }))
  expect(size.width).toBeGreaterThan(0)
  expect(size.height > size.width).toBe(portrait)
  await video.evaluate(el => { const v = el as HTMLVideoElement; v.muted = true; return v.play() })
  await expect.poll(() => video.evaluate(el => (el as HTMLVideoElement).currentTime), { timeout: 20_000 }).toBeGreaterThan(0.3)
  // Seeking needs Range through the gateway.
  await video.evaluate(el => new Promise<void>((done, fail) => {
    const v = el as HTMLVideoElement
    v.pause()
    v.addEventListener('seeked', () => done(), { once: true })
    v.addEventListener('error', () => fail(new Error(`media error ${v.error?.code}`)), { once: true })
    v.currentTime = Math.max(0.1, v.duration * 0.6)
  }))
  return video
}

test.describe('iPhone media through HomeStation and the Preview Pipeline (real zone)', () => {
  test.use({ viewport: { width: 1440, height: 900 } })

  test('without ffmpeg on the node: no repr, and HEIC reads as a format this browser lacks, not as damage', async ({ page }) => {
    test.skip(process.env.PREVIEW_DV_NO_FFMPEG !== '1', 'hide ffmpeg on the node first (e.g. mv /usr/bin/ffmpeg /usr/bin/ffmpeg.off), then set PREVIEW_DV_NO_FFMPEG=1')
    await loginThroughUi(page)
    await page.goto('/homestation')
    await expect(page.getByTestId('homestation')).toBeVisible({ timeout: 30_000 })
    const photos = `HEIC without converter ${Date.now().toString(36)}`
    await publish(page, photos, [{ name: 'IMG_0101.HEIC', file: 'portrait_grid.heic', mimeType: '' }])
    const hello = (await (await page.request.post('/nfs/v1/hello', { data: { args: {} } })).json()).result
    expect(hello.features).not.toContain('repr')
    expect(hello.repr).toMatchObject({ available: false })
    expect(hello.repr.detail).toContain('ffmpeg')
    await page.getByTestId('hs-nav-published').click()
    const card = page.getByTestId('hs-card').filter({ hasText: photos })
    await card.getByTestId('hs-media-grid').click()
    const gallery = page.getByTestId('hs-image-preview')
    await expect(gallery).toHaveAttribute('data-status', 'error', { timeout: 60_000 })
    const error = gallery.getByTestId('content-preview-error')
    await expect(error).toHaveAttribute('data-error-kind', 'unsupported')
    await expect(error).toContainText(/This browser does not support this image format|此浏览器不支持该图片格式/)
    await expect(error).toContainText(/Download original|下载原文件/)
  })

  test('HEIC and MOV: upload → publish → feed thumbnails → detail → immersive → original download', async ({ page, browser }) => {
    test.skip(process.env.PREVIEW_DV_NO_FFMPEG === '1', 'needs ffmpeg on the node')
    test.setTimeout(600_000)
    await loginThroughUi(page)

    const hello = await page.request.post('/nfs/v1/hello', { data: { args: {} } })
    const capability = (await hello.json()).result
    expect(capability.features, JSON.stringify(capability.repr)).toContain('repr')
    expect(capability.repr).toMatchObject({ available: true, heif: true, h264_encoder: true })
    test.info().annotations.push({ type: 'converter', description: `ffmpeg ${capability.repr.ffmpeg.version} at ${capability.repr.ffmpeg.path}` })

    await page.goto('/homestation')
    await expect(page.getByTestId('homestation')).toBeVisible({ timeout: 30_000 })
    const run = Date.now().toString(36)
    const photos = `iPhone HEIC DV ${run}`
    await publish(page, photos, [
      { name: 'IMG_0001.HEIC', file: 'portrait_grid.heic', mimeType: '' },
      { name: 'IMG_0002.HEIC', file: 'landscape_grid.heic', mimeType: '' },
    ])
    const videos = [
      { text: `iPhone HEVC DV ${run}`, name: 'IMG_0003.MOV', file: 'hevc_aac.mov', portrait: false },
      { text: `iPhone portrait HEVC DV ${run}`, name: 'IMG_0004.MOV', file: 'portrait_hevc.mov', portrait: true },
      { text: `iPhone HDR HLG DV ${run}`, name: 'IMG_0005.MOV', file: 'hdr_hlg_hevc.mov', portrait: false },
      { text: `H.264 MOV DV ${run}`, name: 'IMG_0006.MOV', file: 'h264_aac.mov', portrait: false },
    ]
    for (const v of videos) await publish(page, v.text, [{ name: v.name, file: v.file, mimeType: 'video/quicktime' }])

    // Feed: Pipeline thumbnails (JPEG ≤ 640) from the preview-cache bucket, portrait upright.
    // This run's posts lead the list (video cards show no text; each is told apart by its file name).
    await page.getByTestId('hs-nav-published').click()
    const cards = page.getByTestId('hs-card')
    const photoCard = cards.filter({ hasText: photos })
    await expect(photoCard).toHaveCount(1, { timeout: 30_000 })
    const thumbs = photoCard.getByTestId('hs-media-grid').locator('img')
    await expect(thumbs).toHaveCount(2, { timeout: 90_000 })
    await expect(thumbs.first()).toHaveAttribute('data-thumb', 'pipeline', { timeout: 90_000 })
    await expect(thumbs.nth(1)).toHaveAttribute('data-thumb', 'pipeline', { timeout: 90_000 })
    const portraitThumb = await imageSize(thumbs.first())
    expect(portraitThumb.src).toMatch(CACHE_URL)
    expect(Math.max(portraitThumb.width, portraitThumb.height)).toBeLessThanOrEqual(640)
    expect(portraitThumb.height).toBeGreaterThan(portraitThumb.width)
    const landscapeThumb = await imageSize(thumbs.nth(1))
    expect(landscapeThumb.width).toBeGreaterThan(landscapeThumb.height)

    // Detail: the converted JPEG at full preview size; the original downloads byte for byte.
    await photoCard.getByTestId('hs-media-grid').click()
    const gallery = page.getByTestId('hs-image-preview')
    await expect(gallery).toHaveAttribute('data-status', 'ready', { timeout: 90_000 })
    await expect(gallery).toHaveAttribute('data-via', 'pipeline')
    const full = await imageSize(gallery.getByTestId('content-preview-image'))
    expect(full.src).toMatch(CACHE_URL)
    expect(full.height).toBeGreaterThan(full.width)
    expect(Math.max(full.width, full.height)).toBeGreaterThan(640)
    const cached = await page.request.get(full.src, { headers: { Range: 'bytes=0-1023' } })
    expect(cached.status(), 'Range through the gateway').toBe(206)
    expect(cached.headers()['content-type']).toBe('image/jpeg')
    await gallery.hover()
    await gallery.getByTestId('content-preview-more').click()
    const [download] = await Promise.all([page.waitForEvent('download'), page.getByRole('menuitem', { name: /Download original|下载原文件/ }).click()])
    const saved = readFileSync(await download.path())
    expect(sha256(saved), 'the original HEIC, not the preview').toBe(sha256(sample('portrait_grid.heic')))
    await page.getByRole('button', { name: 'Back' }).click()

    // Videos: HEVC transcoded, portrait upright, HDR noted, H.264 MOV remuxed or played as is.
    const played = new Set<string>()
    for (let i = 0; i <= videos.length; i += 1) {
      const card = cards.nth(i)
      await expect(card).toBeVisible({ timeout: 30_000 })
      if (!(await card.getByTestId('hs-play').count())) continue
      await card.getByTestId('hs-play').click()
      const player = page.getByTestId('hs-video-player').getByTestId('content-preview')
      await expect(player).toHaveAttribute('aria-label', /^IMG_000[3-6]\.MOV$/, { timeout: 30_000 })
      const label = await player.getAttribute('aria-label')
      const v = videos.find(entry => entry.name === label)!
      played.add(v.name)
      if (v.file === 'h264_aac.mov') {
        await expect(player).toHaveAttribute('data-status', 'ready', { timeout: 90_000 })
        await expect(player).toHaveAttribute('data-via', 'direct')
        const video = player.getByTestId('content-preview-video')
        await video.evaluate(el => { const m = el as HTMLVideoElement; m.muted = true; return m.play() })
        await expect.poll(() => video.evaluate(el => (el as HTMLVideoElement).currentTime), { timeout: 20_000 }).toBeGreaterThan(0.3)
      } else {
        await playsConverted(player, v.portrait)
        if (v.file === 'hdr_hlg_hevc.mov') {
          await player.hover()
          await player.getByTestId('content-preview-info').click()
          await expect(player.getByTestId('content-preview-info-panel')).toContainText('HDR 已映射为 SDR')
        }
      }
      await page.getByRole('button', { name: 'Back' }).click()
    }
    expect([...played].sort()).toEqual(videos.map(v => v.name).sort())

    // Immersive mode is the reading feed's: a friend (bob) reads devtest's public posts there,
    // through his own HomeStation's copies (`following`) and the same Pipeline.
    const friend = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1440, height: 900 } })
    const bob = await friend.newPage()
    await loginThroughUi(bob, friendUser, friendPassword)
    await bob.goto('/homestation')
    await expect(bob.getByTestId('homestation')).toBeVisible({ timeout: 30_000 })
    await expect.poll(async () => {
      await bob.getByTestId('hs-nav-feed').click()
      return bob.getByTestId('hs-card').filter({ hasText: photos }).count()
    }, { timeout: 180_000, intervals: [5_000] }).toBeGreaterThan(0)
    await bob.getByTestId('hs-mode-immersive').click()
    const immersive = bob.getByTestId('hs-immersive')
    await expect(immersive).toBeVisible()
    const seen = new Set<string>()
    const total = Number((await immersive.getByTestId('hs-immersive-counter').textContent())?.split('/')[1]?.trim() || '1')
    for (let step = 0; step < Math.min(total, 30) && seen.size < 2; step += 1) {
      const image = immersive.getByTestId('hs-immersive-image').getByTestId('content-preview')
      const video = immersive.getByTestId('hs-immersive-video').getByTestId('content-preview')
      for (const [kind, preview] of [['image', image], ['video', video]] as const) {
        if (await preview.count()) {
          await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 90_000 })
          if ((await preview.getAttribute('aria-label'))?.match(/^IMG_000\d\.(HEIC|MOV)$/)) {
            if (kind === 'image' || (await preview.getAttribute('aria-label')) !== 'IMG_0006.MOV') await expect(preview).toHaveAttribute('data-via', 'pipeline')
            seen.add(kind)
          }
        }
      }
      await bob.keyboard.press('ArrowDown')
      await bob.waitForTimeout(500)
    }
    expect([...seen].sort()).toEqual(['image', 'video'])
    await bob.keyboard.press('Escape')
    await expect(immersive).toHaveCount(0)
    await friend.close()
  })
})
