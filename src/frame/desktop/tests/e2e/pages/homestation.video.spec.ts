import { resolve } from 'node:path'
import { expect, test, type Locator, type Page } from '@playwright/test'
import type { HomeStationFileSourceValue } from '../../../src/app/homestation/media'
import { fid } from '../../../src/app/homestation/mock/data'

const videoFixture = resolve(import.meta.dirname, '../../../../../../test/jarvis_media_dv/assets/video_fresh.mp4')
const videoIds = [fid('bob-assistant-tutorial.mp4'), fid('alice-setup-demo.mp4')]

async function openVideoFeed(page: Page, status = 200) {
  const contentRequests: string[] = []
  await page.route(url => videoIds.some(id => decodeURIComponent(url.pathname).endsWith(`/objects/${id}/content`)), async route => {
    contentRequests.push(decodeURIComponent(new URL(route.request().url()).pathname))
    if (status === 200) await route.fulfill({ path: videoFixture, contentType: 'video/mp4' })
    else await route.fulfill({ status, body: `HTTP ${status}` })
  })
  await page.goto('/homestation')
  await expect(page.getByTestId('hs-card').first()).toBeVisible()
  await page.getByTestId('hs-filter-videos').click()
  await page.getByTestId('hs-topic-topic-tech').click()
  await expect(page.getByTestId('hs-card')).toHaveCount(2)
  await page.evaluate(async ids => {
    const modulePath = '/src/components/preview/hostSources.ts'
    const { previewSourceResolverFor, registerPreviewSourceResolver } = await import(modulePath) as typeof import('../../../src/components/preview/hostSources')
    const resolver = previewSourceResolverFor('homestation-file')!
    registerPreviewSourceResolver('homestation-file', {
      resolvePreviewSource(source, options) {
        const value = (source as { value: HomeStationFileSourceValue }).value
        return resolver.resolvePreviewSource(ids.includes(value.objId) ? {
          ...source,
          value: { ...value, name: value.file!.name, url: `/home/fixture/objects/${encodeURIComponent(value.objId)}/content` },
        } : source, options)
      },
    })
  }, videoIds)
  return contentRequests
}

async function openVideoDetail(page: Page) {
  await page.getByTestId('hs-card').filter({ hasText: 'Building a personal AI assistant' }).getByTestId('hs-play').click()
  return page.getByTestId('hs-video-player')
}

async function playAndPause(preview: Locator) {
  await expect(preview).toHaveAttribute('data-status', 'ready')
  const video = preview.getByTestId('content-preview-video')
  await expect(video).toBeVisible()
  await expect(video).toHaveAttribute('controls', '')
  await expect.poll(() => video.evaluate(element => {
    const media = element as HTMLVideoElement
    return media.videoWidth > 0 && media.videoHeight > 0 && media.clientWidth > 0 && media.clientHeight > 0
  })).toBe(true)
  await preview.press('Space')
  await expect.poll(() => video.evaluate(element => !(element as HTMLVideoElement).paused)).toBe(true)
  await expect.poll(() => video.evaluate(element => (element as HTMLVideoElement).currentTime)).toBeGreaterThan(0.2)
  const audioDecoded = await video.evaluate(element => (element as HTMLVideoElement & { webkitAudioDecodedByteCount?: number }).webkitAudioDecodedByteCount)
  if (audioDecoded !== undefined) expect(audioDecoded).toBeGreaterThan(0)
  await preview.press('Space')
  await expect.poll(() => video.evaluate(element => (element as HTMLVideoElement).paused)).toBe(true)
  const pausedAt = await video.evaluate(element => (element as HTMLVideoElement).currentTime)
  await video.press('Space')
  await expect.poll(() => video.evaluate(element => (element as HTMLVideoElement).currentTime)).toBeGreaterThan(pausedAt + 0.1)
  await video.press('Space')
  await expect.poll(() => video.evaluate(element => (element as HTMLVideoElement).paused)).toBe(true)
  return video
}

test.describe('HomeStation video with fixture feed metadata and real MP4 bytes', () => {
  test.use({ viewport: { width: 1440, height: 900 } })

  test('detail loads the wrapped video and plays and pauses actual media', async ({ page }) => {
    const contentRequests = await openVideoFeed(page)
    const player = await openVideoDetail(page)
    await playAndPause(player.getByTestId('content-preview'))
    expect(contentRequests.some(path => path.endsWith(`/objects/${videoIds[0]}/content`))).toBe(true)
    await expect(player).not.toContainText('Prototype playback')
  })

  test('immersive playback keeps Space local and unmounts media on navigation and Escape', async ({ page }) => {
    await openVideoFeed(page)
    await page.getByTestId('hs-mode-immersive').click()
    const counter = page.getByTestId('hs-immersive-counter')
    const preview = page.getByTestId('hs-immersive-video').getByTestId('content-preview')
    await expect(counter).toHaveText('1 / 2')
    const first = await playAndPause(preview)
    await expect(counter).toHaveText('1 / 2')
    await first.press('Space')
    const oldVideo = await first.elementHandle()
    await first.press('ArrowDown')
    await expect(counter).toHaveText('1 / 2')
    await preview.press('ArrowDown')
    await expect(counter).toHaveText('2 / 2')
    await expect.poll(() => oldVideo!.evaluate(element => !element.isConnected && (element as HTMLVideoElement).paused)).toBe(true)
    const second = await playAndPause(preview)
    await second.press('Space')
    const closingVideo = await second.elementHandle()
    await second.press('Escape')
    await expect(page.getByTestId('hs-immersive')).toHaveCount(0)
    await expect.poll(() => closingVideo!.evaluate(element => !element.isConnected && (element as HTMLVideoElement).paused)).toBe(true)
  })

  for (const [status, kind] of [[403, 'permission-denied'], [404, 'not-found'], [503, 'error']] as const) {
    test(`detail exposes HTTP ${status} as ${kind}`, async ({ page }) => {
      await openVideoFeed(page, status)
      const player = await openVideoDetail(page)
      await expect(player.getByTestId('content-preview-error')).toHaveAttribute('data-error-kind', kind)
      await expect(player.getByTestId('content-preview-video')).toHaveCount(0)
    })
  }

  for (const [code, kind] of [[4, 'unsupported'], [3, 'corrupted']] as const) {
    test(`immersive exposes browser media error ${code} as ${kind} without a conversion pipeline`, async ({ page }) => {
      await openVideoFeed(page)
      await page.evaluate(async () => {
        const modulePath = '/src/components/preview/provider.ts'
        const { registerPreviewProvider, ensurePreviewProvider } = await import(modulePath) as typeof import('../../../src/components/preview/provider')
        registerPreviewProvider({ ...await ensurePreviewProvider(), ensurePreviewWork: async () => ({ kind: 'unsupported', reason: 'no-pipeline' }) })
      })
      await page.getByTestId('hs-mode-immersive').click()
      const preview = page.getByTestId('hs-immersive-video').getByTestId('content-preview')
      await expect(preview).toHaveAttribute('data-status', 'ready')
      await preview.getByTestId('content-preview-video').evaluate((element, errorCode) => {
        Object.defineProperty(element, 'error', { configurable: true, value: { code: errorCode } })
        element.dispatchEvent(new Event('error'))
      }, code)
      const error = preview.getByTestId('content-preview-error')
      await expect(error).toHaveAttribute('data-error-kind', kind)
      if (code === 4) await expect(error).toContainText('This browser does not support this audio or video encoding')
    })
  }
})
