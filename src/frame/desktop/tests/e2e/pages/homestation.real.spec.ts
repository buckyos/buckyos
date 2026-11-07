import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test, type Locator, type Page } from '@playwright/test'

const DEV = 'hsDevToken=tok-me'

function trackErrors(page: Page) {
  const errors: string[] = []
  page.on('console', message => {
    if (message.type() === 'error') errors.push(message.text())
  })
  page.on('pageerror', error => errors.push(error.message))
  return errors
}

async function openHomeStation(page: Page) {
  await page.goto(`/homestation?${DEV}`)
  await expect(page.getByTestId('hs-card').first()).toBeVisible()
}

const card = (page: Page, text: string) => page.getByTestId('hs-card').filter({ hasText: text })
const post = (page: Page, text: string) => page.locator('[data-testid="hs-card"]:not([data-content-type="comment"])').filter({ hasText: text })

async function scrollToCard(page: Page, target: Locator) {
  const scroll = page.getByTestId('hs-feed-scroll')
  await expect.poll(async () => {
    if ((await target.count()) > 0) return true
    await scroll.evaluate(element => element.scrollTo(0, element.scrollHeight))
    return false
  }, { timeout: 20_000, intervals: [300] }).toBe(true)
  await target.first().scrollIntoViewIfNeeded()
  return target.first()
}

// The Vite proxy only knows the HomeStation service; in a zone the gateway routes `/ndm` to the nfs-server.
async function routeUploadsToNfsServer(page: Page) {
  const nfs = readFileSync(join(process.env.HS_E2E_DATA_DIR ?? '', 'me', 'nfs.url'), 'utf8').trim()
  await page.route('**/ndm/v1/**', async route => {
    const url = new URL(route.request().url())
    const response = await route.fetch({ url: `${nfs}${url.pathname}${url.search}` })
    const headers = { ...response.headers() }
    if (headers.location) headers.location = headers.location.replace(nfs, url.origin)
    await route.fulfill({ response, headers })
  })
}

test.describe('HomeStation on the homestation service (devnet)', () => {
  test.skip(!process.env.HS_REAL_E2E, 'run through playwright.homestation.config.ts')
  test.use({ viewport: { width: 1440, height: 900 } })

  test('the reading list shows Alice’s seeded posts with their media and article body', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await expect(page.getByTestId('hs-card-publisher').filter({ hasText: 'Alice Chen' }).first()).toBeVisible()
    await expect(await scrollToCard(page, post(page, 'Started our balcony garden today'))).toHaveAttribute('data-content-type', 'text')
    const gallery = await scrollToCard(page, post(page, 'Before and after of the garden weekend'))
    await expect(gallery).toHaveAttribute('data-content-type', 'image')
    const images = gallery.getByTestId('hs-media-grid').locator('img')
    await expect(images).toHaveCount(2)
    // A Preview Pipeline thumbnail when the node has ffmpeg, else the original content URL.
    await expect(images.first()).toHaveAttribute('src', /^\/(home\/me\/objects\/cyfile[^/]+\/content\?access=|bucket\/v1\/zone\/nfs-server\/_\/preview-cache\/)/)
    await expect.poll(() => images.evaluateAll(list => list.every(img => (img as HTMLImageElement).complete && (img as HTMLImageElement).naturalWidth > 0))).toBe(true)
    const article = await scrollToCard(page, post(page, 'A week of balcony gardening'))
    await expect(article).toHaveAttribute('data-content-type', 'article')
    await article.getByText('A week of balcony gardening').first().click()
    await expect(page.getByTestId('hs-detail')).toBeVisible()
    await expect(page.getByTestId('hs-article-body')).toHaveAttribute('data-state', 'ready')
    await expect(page.getByTestId('hs-article-body')).toContainText('Six hours of sun is the minimum for tomatoes.')
    await page.getByRole('button', { name: 'Back' }).click()
    await gallery.getByTestId('hs-media-grid').click()
    await expect(page.getByTestId('hs-image-preview')).toBeVisible()
    await expect(page.getByTestId('hs-image-preview').locator('img').first()).toBeVisible()
    expect(errors).toEqual([])
  })

  test('AI-tagged items are hidden by default and shown on request', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await expect(page.getByTestId('hs-hidden-summary')).toContainText(/\d+ hidden by your filters/)
    await scrollToCard(page, post(page, 'A week of balcony gardening'))
    await expect(card(page, 'AIGC experiment: a poem about soil')).toHaveCount(0)
    await page.getByTestId('hs-toggle-filtered').click()
    const filtered = await scrollToCard(page, post(page, 'AIGC experiment: a poem about soil'))
    await expect(filtered.getByTestId('hs-filtered-badge')).toBeVisible()
    await page.getByTestId('hs-toggle-filtered').click()
    await expect(card(page, 'AIGC experiment: a poem about soil')).toHaveCount(0)
    expect(errors).toEqual([])
  })

  test('a like is stored by the service and survives a reload', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const target = await scrollToCard(page, post(page, 'Before and after of the garden weekend'))
    await expect(target.getByTestId('hs-like')).toHaveAttribute('aria-pressed', 'false')
    await target.getByTestId('hs-like').click()
    await expect(target.getByTestId('hs-like')).toHaveAttribute('aria-pressed', 'true')
    await expect(target.getByTestId('hs-like-count')).toHaveText('1')
    await page.reload()
    await expect(page.getByTestId('hs-card').first()).toBeVisible()
    const reloaded = await scrollToCard(page, post(page, 'Before and after of the garden weekend'))
    await expect(reloaded.getByTestId('hs-like')).toHaveAttribute('aria-pressed', 'true')
    await page.getByTestId('hs-nav-published').click()
    await page.getByTestId('hs-published-kind-reactions').click()
    await expect(card(page, 'Before and after of the garden weekend')).toHaveCount(1)
    expect(errors).toEqual([])
  })

  test('a comment on Alice’s post appears in the detail view', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const target = await scrollToCard(page, post(page, 'Started our balcony garden today'))
    await target.getByTestId('hs-comment').click()
    await expect(page.getByTestId('hs-detail')).toBeVisible()
    await expect(page.getByTestId('hs-comment-row').filter({ hasText: 'Try leafy greens first, they forgive mistakes.' })).toBeVisible()
    const text = `Basil wants the sunniest box (${Date.now().toString(36)})`
    await page.getByTestId('hs-comment-form').getByRole('textbox').fill(text)
    await page.getByTestId('hs-comment-submit').click()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Comment published' })).toBeVisible()
    await expect(page.getByTestId('hs-comment-row').filter({ hasText: text })).toBeVisible()
    await page.getByRole('button', { name: 'Back' }).click()
    await page.getByTestId('hs-nav-published').click()
    await page.getByTestId('hs-published-kind-comments').click()
    await expect(card(page, text)).toHaveCount(1)
    expect(errors).toEqual([])
  })

  test('a published text post is listed in My publications with its delivery state', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const text = `Hello from the HomeStation e2e suite (${Date.now().toString(36)})`
    await page.getByTestId('hs-quick-text').fill(text)
    await page.getByTestId('hs-quick-submit').click()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Published to your feed' })).toBeVisible()
    await expect(card(page, text)).toHaveCount(0)
    await page.getByTestId('hs-nav-published').click()
    const mine = card(page, text)
    await expect(mine).toHaveCount(1)
    const status = mine.getByTestId('hs-publish-status')
    await expect(status).toHaveAttribute('data-stage', 'published')
    await expect(status).toHaveAttribute('data-delivery', /^(delivering|delivered|partially_failed)$/)
    await expect(status).toHaveAttribute('data-delivery', 'delivered', { timeout: 45_000 })
    await expect(mine.getByTestId('hs-published-audience')).toContainText('Public')
    expect(errors).toEqual([])
  })

  test('an uploaded photo carries what its bytes say in the object itself', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    // A 6000x4000 JPEG header with EXIF and GPS: `jpeg_exif_little_endian` of the SDK's tests/media_meta_vectors.json.
    const photo = Buffer.from('/9j/4AAQSkZJRgABAgAAAQABAAD/4QAraHR0cDovL25zLmFkb2JlLmNvbS94YXAvMS4wLwA8eDp4bXBtZXRhLz7/4QHURXhpZgAASUkqAAgAAAAEAA8BAgAFAAAAPgAAABABAgAJAAAAQwAAAGmHBAABAAAATAAAACWIBAABAAAAHgEAAAAAAABTb255AElMQ0UtN00zAAoAmoIFAAEAAADKAAAAnYIFAAEAAADSAAAAJ4gDAAEAAACQAQAAA5ACABQAAADaAAAABJACABQAAADuAAAAEZACAAcAAAACAQAAkZICAAQAAAAxMjUACpIFAAEAAAAJAQAABaQDAAEAAABLAAAANKQCAA0AAAARAQAAAAAAAAoAAACwBAAAHAAAAAoAAAAyMDI0OjAyOjI5IDEzOjA1OjA5ADIwMjA6MDE6MDEgMDA6MDA6MDAAKzA4OjAwAIgTAABkAAAARkUgNTBtbSBGMS44AAgAAQACAAIAAABOAAAAAgAFAAMAAACEAQAAAwACAAIAAABFAAAABAAFAAMAAACcAQAABQABAAEAAAAAAAAABgAFAAEAAAC0AQAAEQAFAAEAAAC8AQAAHwAFAAEAAADEAQAAAAAAACIAAAABAAAAOwAAAAEAAABEEAAAZAAAAIcAAAABAAAALQAAAAEAAABsCQAAZAAAANIEAABkAAAAWiMAAGQAAABBAAAACgAAAP/AAAsID6AXcAEBEQD/2gAC/9k=', 'base64')
    await routeUploadsToNfsServer(page)
    const registered = page.waitForResponse(response => response.url().endsWith('/kapi/homestation') && (response.request().postData() ?? '').includes('"object.register"'))
    await page.getByTestId('hs-quick-file').setInputFiles({ name: 'shot.bin', mimeType: 'application/octet-stream', buffer: photo })
    const response = await registered
    expect(response.request().postData()).not.toContain('metadata')
    const { result } = await response.json() as { result: { objId: string; object: Record<string, unknown> } }
    expect(result.object).toMatchObject({
      name: 'shot.bin',
      mime: 'image/jpeg',
      width: 6000,
      height: 4000,
      captured_at: '2024-02-29T13:05:09.125+08:00',
      geo: { lat: 34.9949, lon: 135.7567 },
      camera: { make: 'Sony', model: 'ILCE-7M3' },
      exposure: { exposure_time: '1/120', f_number: 2.8 },
    })
    await expect(page.getByTestId('hs-quick-attachments').locator('li')).toHaveAttribute('data-status', 'uploaded')
    const text = `A photo from the HomeStation e2e suite (${Date.now().toString(36)})`
    await page.getByTestId('hs-quick-text').fill(text)
    await page.getByTestId('hs-quick-submit').click()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Published to your feed' })).toBeVisible()
    await page.getByTestId('hs-nav-published').click()
    await expect(card(page, text).getByTestId('hs-publish-status')).toHaveAttribute('data-stage', 'published')
    const content = await page.request.get(`/home/me/objects/${encodeURIComponent(result.objId)}/content?access=tok-me`)
    expect(content.headers()['content-type']).toBe('image/jpeg')
    // A lookup miss answers 404 by design, and this JPEG has no image data for the card to decode.
    expect(errors.filter(error => !error.includes('Failed to load resource'))).toEqual([])
  })

  test('the followed-candidates view loads', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await page.getByTestId('hs-nav-candidates').click()
    const view = page.getByTestId('hs-candidates')
    await expect(view).toBeVisible()
    await expect(view.getByTestId('hs-loading')).toHaveCount(0)
    await expect(view.getByTestId('hs-error')).toHaveCount(0)
    await expect(view).toContainText('Kept for 14 days')
    await expect(view.getByTestId('hs-candidate-footer').or(view.getByTestId('hs-empty')).first()).toBeVisible()
    expect(errors).toEqual([])
  })

  test('owner previews read as a real follower and friend', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await page.getByTestId('hs-nav-profile').click()
    await expect(page.getByTestId('hs-preview-follower')).toHaveText('Follower (Sarah Kim)')
    await expect(page.getByTestId('hs-preview-friend')).toHaveText(/^Friend \((Alice Chen|Bob Zhang|Kai Wen)\)$/)
    const list = page.getByTestId('hs-profile-list')
    await page.getByTestId('hs-preview-friend').click()
    await expect(page.locator('[data-testid="hs-profile"][data-reader="friend"]')).toBeVisible()
    await expect(list).toContainText('Friends only: dinner photos coming soon')
    await page.getByTestId('hs-preview-follower').click()
    await expect(page.locator('[data-testid="hs-profile"][data-reader="follower"]')).toBeVisible()
    await expect(list).toContainText('Testing my HomeStation')
    await expect(list).not.toContainText('Friends only: dinner photos coming soon')
    await expect(page.getByTestId('hs-reader-approximated')).toHaveCount(0)
    expect(errors).toEqual([])
  })

  test('mute choices come from the HomeStation’s people', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await expect(post(page, 'Color palettes for winter interfaces')).toBeVisible()
    await page.getByTestId('hs-nav-prefs').click()
    const choice = page.getByTestId('hs-mute-choice')
    await expect(choice.locator('option[value="person|did:test:sarah"]')).toHaveText('Sarah Kim')
    await expect(choice.locator('option[value="person|did:test:alice"]')).toHaveText('Alice Chen')
    await expect(choice.locator('option[value^="person|did:bns:"]')).toHaveCount(0)
    await choice.selectOption('person|did:test:sarah')
    await page.getByTestId('hs-mute-add').click()
    await expect(page.getByTestId('hs-mute-list')).toContainText('Sarah Kim')
    await page.getByTestId('hs-nav-feed').click()
    await expect(page.getByTestId('hs-hidden-summary')).toContainText(/\d+ from people you chose not to see/)
    await expect(post(page, 'Color palettes for winter interfaces')).toHaveCount(0)
    await page.getByTestId('hs-nav-prefs').click()
    await page.getByTestId('hs-mute-list').getByTestId('hs-unmute').click()
    await expect(page.getByTestId('hs-mute-list')).not.toContainText('Sarah Kim')
    expect(errors).toEqual([])
  })

  test('change polling pauses while the page is hidden', async ({ page }) => {
    const errors = trackErrors(page)
    const polls: number[] = []
    page.on('request', request => {
      if (request.url().endsWith('/kapi/homestation') && request.postData()?.includes('"ui.versions"')) polls.push(Date.now())
    })
    await openHomeStation(page)
    await expect.poll(() => polls.length, { timeout: 10_000 }).toBeGreaterThan(0)
    const setHidden = (hidden: boolean) => page.evaluate(value => {
      Object.defineProperty(document, 'hidden', { configurable: true, get: () => value })
      document.dispatchEvent(new Event('visibilitychange'))
    }, hidden)
    await setHidden(true)
    await page.waitForTimeout(500)
    const paused = polls.length
    await page.waitForTimeout(7_000)
    expect(polls.length).toBe(paused)
    await setHidden(false)
    await expect.poll(() => polls.length, { timeout: 2_000 }).toBeGreaterThan(paused)
    expect(errors).toEqual([])
  })

  test('a user’s portal shows public posts to visitors and more to a signed-in friend of the same zone', async ({ page }) => {
    const errors = trackErrors(page)
    await page.goto('/homestation/me?hsDevToken=anonymous')
    await expect(page.getByTestId('hs-portal')).toHaveAttribute('data-feed', 'me')
    await expect(page.getByTestId('hs-portal-sign-in')).toBeVisible()
    await expect(page.getByTestId('hs-profile')).toContainText('Lin')
    const list = page.getByTestId('hs-profile-list')
    await expect(list).toContainText('Testing my HomeStation')
    await expect(list).not.toContainText('Friends only: dinner photos coming soon')
    // Kai lives in the same zone and is Lin's friend: the same page, read as Kai.
    await page.goto('/homestation/me?hsDevToken=tok-kai')
    await expect(page.getByTestId('hs-portal-my-page')).toHaveAttribute('href', '/homestation/kai')
    await expect(list).toContainText('Friends only: dinner photos coming soon')
    // A post has its own link: /homestation/<user>/<key>.
    await card(page, 'Testing my HomeStation').first().getByText('Testing my HomeStation').click()
    await expect(page).toHaveURL(/\/homestation\/me\/p-[0-9a-f]+$/)
    await expect(page.getByTestId('hs-detail')).toContainText('Testing my HomeStation')
    await page.reload()
    await expect(page.getByTestId('hs-detail')).toContainText('Testing my HomeStation')
    expect(errors).toEqual([])
  })

  test('the zone feed lists what users chose to show there, including a new post', async ({ page }) => {
    const errors = trackErrors(page)
    await page.goto('/homestation/~zone?hsDevToken=anonymous')
    const zone = page.getByTestId('hs-zone-list')
    await expect(zone).toContainText('Testing my HomeStation')
    await expect(zone).toContainText('our zone now has a shared front page')
    await expect(zone).not.toContainText('Only for my own home feed')
    await zone.getByTestId('hs-zone-author').filter({ hasText: 'kai' }).first().click()
    await expect(page.getByTestId('hs-portal')).toHaveAttribute('data-feed', 'kai')
    await expect(page.getByTestId('hs-profile-list')).toContainText('Only for my own home feed')

    await page.goto(`/homestation?${DEV}`)
    const text = `On the zone page too (${Date.now().toString(36)})`
    await page.getByTestId('hs-quick-text').fill(text)
    await page.getByTestId('hs-quick-zone-feed').locator('input').check()
    await page.getByTestId('hs-quick-submit').click()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Published to your feed' })).toBeVisible()
    await page.getByTestId('hs-nav-published').click()
    await expect(card(page, text).getByTestId('hs-published-zone')).toBeVisible()
    await page.goto('/homestation/~zone?hsDevToken=anonymous')
    await expect(page.getByTestId('hs-zone-list')).toContainText(text)
    expect(errors).toEqual([])
  })

  test('`/` on the www short host opens the zone’s default feed; its links point at the zone host', async ({ page, baseURL }) => {
    const errors = trackErrors(page)
    const port = new URL(baseURL!).port
    await page.goto(`http://www.localhost:${port}/?hsDevToken=anonymous`)
    await expect(page.getByTestId('hs-portal')).toHaveAttribute('data-feed', 'me')
    await expect(page.getByTestId('hs-profile-list')).toContainText('Testing my HomeStation')
    await expect(page.getByTestId('hs-portal-zone')).toHaveAttribute('href', `http://localhost:${port}/homestation/~zone`)
    expect(errors).toEqual([])
  })
})
