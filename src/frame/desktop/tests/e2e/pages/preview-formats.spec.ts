import { expect, test, type Locator, type Page } from '@playwright/test'
import { pickLocal } from '../fixtures/selectDialog'

/**
 * Preview formats (preview-more-formats TODO, Phase A) on the mock runtime: spreadsheets in the sheet renderer
 * (read from the original bytes, or from the xlsx a Pipeline converts to) and PDFs drawn by pdf.js on touch devices.
 */

async function openSample(page: Page, name: string): Promise<Locator> {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-preview').click()
  const win = page.getByTestId('window-preview')
  await win.getByTestId(`preview-sample-${name}`).click()
  return win
}

async function openLocal(page: Page, file: { name: string; mimeType: string; buffer: Buffer }): Promise<Locator> {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-preview').click()
  const win = page.getByTestId('window-preview')
  await win.getByTestId('preview-landing-choose').click()
  await pickLocal(page, file)
  return win
}

/** Opens the Preview App through its host entry point (the mobile shell has no landing-page shortcut to a sample). */
async function openPreview(page: Page, source: unknown) {
  await page.goto('/?scenario=normal')
  await page.waitForLoadState('networkidle')
  await page.evaluate(async (ref) => {
    const launch = '/src/app/preview/launch.ts'
    const { openPreview: open } = await import(/* @vite-ignore */ launch)
    open({ source: ref })
  }, source)
  return page.getByTestId('content-preview')
}

function pdfObjects(objects: string[], trailer = ''): Buffer {
  let out = '%PDF-1.4\n'
  const offsets: number[] = []
  objects.forEach((body, i) => {
    offsets.push(out.length)
    out += `${i + 1} 0 obj\n${body}\nendobj\n`
  })
  const xref = out.length
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`
  for (const offset of offsets) out += `${String(offset).padStart(10, '0')} 00000 n \n`
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R ${trailer}>>\nstartxref\n${xref}\n%%EOF\n`
  return Buffer.from(out, 'latin1')
}

/** A PDF whose text uses a predefined CJK encoding (UniGB-UCS2-H) with a font that is not embedded: pdf.js has to
 * fetch the CMap. Each page carries padding so the file spans many Range chunks. */
function cjkPdf(pageCount: number, padding: number): Buffer {
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    `<< /Type /Pages /Kids [${Array.from({ length: pageCount }, (_, i) => `${6 + i} 0 R`).join(' ')}] /Count ${pageCount} >>`,
    '<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /UniGB-UCS2-H /DescendantFonts [4 0 R] >>',
    '<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 2 >> /FontDescriptor 5 0 R /DW 1000 >>',
    '<< /Type /FontDescriptor /FontName /STSong-Light /Flags 6 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 880 /StemV 80 >>',
  ]
  // The page objects come first and their (large) contents after, as a writer that keeps the page tree together does.
  for (let page = 0; page < pageCount; page += 1) {
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents ${6 + pageCount + page} 0 R /Resources << /Font << /F1 3 0 R >> >> >>`)
  }
  for (let page = 0; page < pageCount; page += 1) {
    // 中文预览 in UCS-2, then a filled rectangle so the page is never blank whatever font the Runtime falls back to.
    const content = `BT /F1 36 Tf 72 700 Td <4E2D6587988489C8> Tj ET\n0.2 0.4 0.8 rg 72 600 200 40 re f\n% ${'x'.repeat(padding)}`
    objects.push(`<< /Length ${content.length} >>\nstream\n${content}\nendstream`)
  }
  return pdfObjects(objects)
}

/** A PDF with a Standard security handler whose user password is not empty. */
function lockedPdf(): Buffer {
  const hex = (seed: number) => Array.from({ length: 32 }, (_, i) => ((seed * 31 + i * 17) % 256).toString(16).padStart(2, '0')).join('')
  const content = 'BT /F1 24 Tf 72 720 Td (secret) Tj ET'
  return pdfObjects(
    [
      '<< /Type /Catalog /Pages 2 0 R >>',
      '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
      '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
      `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
      '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
      `<< /Filter /Standard /V 1 /R 2 /O <${hex(3)}> /U <${hex(7)}> /P -44 >>`,
    ],
    '/Encrypt 6 0 R /ID [<00112233445566778899aabbccddeeff> <00112233445566778899aabbccddeeff>] ',
  )
}

/** Serves `body` at `path` the way the content server does: single-range requests answered with 206. */
async function serveWithRanges(page: Page, path: string, body: Buffer, contentType: string) {
  const served = { ranges: 0, rangeBytes: 0, full: 0 }
  await page.route(`**${path}`, async (route) => {
    const range = /^bytes=(\d+)-(\d*)$/.exec(route.request().headers().range ?? '')
    const headers = { 'Content-Type': contentType, 'Accept-Ranges': 'bytes', 'Cache-Control': 'no-store' }
    if (!range) {
      served.full += 1
      await route.fulfill({ status: 200, headers: { ...headers, 'Content-Length': String(body.length) }, body })
      return
    }
    const start = Number(range[1])
    const end = Math.min(range[2] ? Number(range[2]) : body.length - 1, body.length - 1)
    served.ranges += 1
    served.rangeBytes += end - start + 1
    await route.fulfill({
      status: 206,
      headers: { ...headers, 'Content-Length': String(end - start + 1), 'Content-Range': `bytes ${start}-${end}/${body.length}` },
      body: body.subarray(start, end + 1),
    })
  })
  return served
}

/** A source the test resolves itself: a URL read reference, as the content server gives for a path. */
async function registerUrlSource(page: Page, name: string, url: string, mediaType: string, size: number) {
  await page.evaluate(async ({ name, url, mediaType, size }) => {
    const module = '/src/components/preview/hostSources.ts'
    const { registerPreviewSourceResolver } = await import(/* @vite-ignore */ module)
    registerPreviewSourceResolver('e2e-url', {
      resolvePreviewSource: async (source: unknown) => ({ originalSource: source, displayName: name, size, objectType: 'file', mediaTypeHints: [mediaType], readRef: { kind: 'url', url } }),
    })
  }, { name, url, mediaType, size })
}

async function paintedPixels(page: Locator): Promise<number> {
  return page.locator('canvas').evaluate((canvas: HTMLCanvasElement) => {
    const data = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data
    let painted = 0
    for (let i = 0; i < data.length; i += 4) if (data[i] < 240 || data[i + 1] < 240 || data[i + 2] < 240) painted += 1
    return painted
  })
}

test.describe('sheet renderer', () => {
  test('an xlsx is read from its bytes: formats, merged cells, sheet tabs, zoom', async ({ page }) => {
    const win = await openSample(page, 'budget.xlsx')
    const preview = win.getByTestId('content-preview')
    await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 15_000 })
    await expect(preview).toHaveAttribute('data-renderer', 'sheet')
    await expect(preview).toHaveAttribute('data-via', 'direct')
    const sheet = win.getByTestId('content-preview-sheet')
    // The hidden sheet has no tab.
    await expect(sheet.getByTestId('content-preview-sheet-tab')).toHaveText(['Budget', 'Notes'])
    await expect(sheet.getByTestId('content-preview-sheet-merge')).toHaveText('Budget 2026')
    const firstItem = sheet.locator('[data-testid="content-preview-sheet-row"][data-row-index="2"]')
    // A date serial and a ratio are shown as the workbook formats them.
    await expect(firstItem).toContainText('2024-01-01')
    await expect(firstItem).toContainText('2.5%')
    // The header row stays in view while the rows scroll.
    await sheet.getByTestId('content-preview-sheet-scroll').evaluate((el) => el.scrollTo({ top: 400 }))
    await expect(sheet.getByText('A', { exact: true })).toBeInViewport()

    await sheet.getByTestId('content-preview-sheet-tab').nth(1).click()
    await expect(sheet).toContainText('人民币（元）')

    await preview.hover()
    await win.getByTestId('content-preview-zoom-in').click()
    await expect(sheet).toHaveAttribute('data-scale', '1.25')
    await expect(win.getByTestId('content-preview-zoom-level')).toHaveText('125%')
  })

  test('two fingers zoom a sheet on a touch screen', async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width: 412, height: 839 }, hasTouch: true, isMobile: true })
    const page = await context.newPage()
    const preview = await openPreview(page, { kind: 'cyfs-path', path: 'cyfs:///samples/budget.xlsx' })
    await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 20_000 })
    const sheet = page.getByTestId('content-preview-sheet')
    await sheet.getByTestId('content-preview-sheet-scroll').evaluate((el) => {
      const rect = el.getBoundingClientRect()
      const touch = (id: number, x: number, y: number) => new Touch({ identifier: id, target: el, clientX: rect.left + x, clientY: rect.top + y })
      const fire = (type: string, touches: Touch[]) => el.dispatchEvent(new TouchEvent(type, { touches, changedTouches: touches, bubbles: true, cancelable: true }))
      fire('touchstart', [touch(1, 150, 300), touch(2, 250, 300)])
      fire('touchmove', [touch(1, 125, 300), touch(2, 275, 300)])
      fire('touchend', [])
    })
    await expect(sheet).toHaveAttribute('data-scale', '1.5')
    await expect(sheet.getByTestId('content-preview-sheet-merge')).toHaveText('Budget 2026')
    await context.close()
  })

  test('a csv is a table that can be read as plain text', async ({ page }) => {
    const win = await openSample(page, 'contacts.csv')
    const sheet = win.getByTestId('content-preview-sheet')
    await expect(sheet).toBeVisible({ timeout: 15_000 })
    await expect(sheet.getByText('Kyoto, Japan', { exact: true }).first()).toBeVisible()
    await sheet.getByTestId('content-preview-sheet-text-toggle').click()
    await expect(win.getByTestId('content-preview-text')).toContainText('"Kyoto, Japan"')
    await sheet.getByTestId('content-preview-sheet-text-toggle').click()
    await expect(sheet.getByTestId('content-preview-sheet-row').first()).toBeVisible()
  })

  test('rows beyond the limit are left out with a notice; only the rows in view are in the page', async ({ page }) => {
    const rows = Array.from({ length: 6000 }, (_, i) => `row ${i + 1},${i * 3},${(i % 97) / 100}`)
    const win = await openLocal(page, { name: 'big.csv', mimeType: 'text/csv', buffer: Buffer.from(['name,count,ratio', ...rows].join('\n')) })
    const sheet = win.getByTestId('content-preview-sheet')
    await expect(sheet).toHaveAttribute('data-rows', '5000', { timeout: 15_000 })
    await expect(win.getByTestId('content-preview-sheet-truncated')).toContainText('5000')
    expect(await sheet.getByTestId('content-preview-sheet-row').count()).toBeLessThan(120)
    await sheet.getByTestId('content-preview-sheet-scroll').evaluate((el) => el.scrollTo({ top: el.scrollHeight }))
    await expect(sheet.locator('[data-testid="content-preview-sheet-row"][data-row-index="4999"]')).toContainText('row 4999')
  })

  test('an encrypted workbook says so and offers the download', async ({ page }) => {
    const win = await openLocal(page, { name: 'secret.xlsx', mimeType: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', buffer: Buffer.concat([Buffer.from([0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]), Buffer.alloc(2048)]) })
    await expect(win.getByTestId('content-preview-sheet-encrypted')).toBeVisible({ timeout: 15_000 })
    await expect(win.getByTestId('content-preview')).toHaveAttribute('data-degraded', 'true')
  })

  test('an xls goes through the Pipeline and its xlsx is shown by the same renderer', async ({ page }) => {
    const win = await openSample(page, 'ledger.xls')
    const preview = win.getByTestId('content-preview')
    await expect(win.getByTestId('content-preview-retry')).toBeVisible({ timeout: 20_000 })
    await win.getByTestId('content-preview-retry').click()
    await expect(preview).toHaveAttribute('data-renderer', 'sheet', { timeout: 20_000 })
    await expect(preview).toHaveAttribute('data-via', 'pipeline')
    await expect(win.getByTestId('content-preview-sheet-tab')).toHaveText(['Budget', 'Notes'])
  })

  test('an epub is a Pipeline page in the sandboxed HTML renderer', async ({ page }) => {
    const win = await openSample(page, 'handbook.epub')
    const preview = win.getByTestId('content-preview')
    await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 20_000 })
    await expect(preview).toHaveAttribute('data-renderer', 'html')
    await expect(preview).toHaveAttribute('data-via', 'pipeline')
  })
})

for (const device of [
  { name: 'Pixel 7', viewport: { width: 412, height: 839 }, userAgent: 'Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36', deviceScaleFactor: 2.625 },
  { name: 'iPhone 14', viewport: { width: 390, height: 664 }, userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1', deviceScaleFactor: 3 },
]) {
  test.describe(`pdf.js on a phone (${device.name})`, () => {
    test.use({ viewport: device.viewport, userAgent: device.userAgent, deviceScaleFactor: device.deviceScaleFactor, hasTouch: true, isMobile: true })

    test('a long PDF scrolls page by page: only the pages around the view are drawn', async ({ page }) => {
      const preview = await openPreview(page, { kind: 'cyfs-path', path: 'cyfs:///samples/annual-report.pdf' })
      await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 20_000 })
      await expect(preview).toHaveAttribute('data-renderer', 'pdf')
      const viewer = page.getByTestId('content-preview-pdfjs')
      await expect(viewer).toHaveAttribute('data-page-count', '12')
      const pages = viewer.getByTestId('content-preview-pdf-canvas')
      await expect(pages.first()).toHaveAttribute('data-rendered', 'true')
      expect(await pages.count()).toBeLessThanOrEqual(6)
      // A canvas never exceeds what iOS allocates.
      const pixels = await pages.first().locator('canvas').evaluate((canvas: HTMLCanvasElement) => canvas.width * canvas.height)
      expect(pixels).toBeLessThanOrEqual(16_777_216)
      expect(await paintedPixels(pages.first())).toBeGreaterThan(100)

      // Jump to a page: it is drawn, the first pages are released.
      const pageInput = page.getByTestId('content-preview-pdf-page')
      await pageInput.fill('9')
      await pageInput.press('Enter')
      await expect(viewer).toHaveAttribute('data-page', '9')
      await expect(viewer.locator('[data-page-index="8"]')).toHaveAttribute('data-rendered', 'true')
      await expect(viewer.locator('[data-page-index="0"]')).toHaveCount(0)
      expect(await pages.count()).toBeLessThanOrEqual(6)

      // Two fingers moving apart zoom in; the pages are drawn again at the new size.
      const before = await viewer.locator('[data-page-index="8"]').evaluate((el) => el.getBoundingClientRect().width)
      await viewer.evaluate((el) => {
        const rect = el.getBoundingClientRect()
        const touch = (id: number, x: number, y: number) => new Touch({ identifier: id, target: el, clientX: rect.left + x, clientY: rect.top + y })
        const fire = (type: string, touches: Touch[]) => el.dispatchEvent(new TouchEvent(type, { touches, changedTouches: touches, bubbles: true, cancelable: true }))
        fire('touchstart', [touch(1, 150, 300), touch(2, 250, 300)])
        fire('touchmove', [touch(1, 100, 300), touch(2, 300, 300)])
        fire('touchend', [])
      })
      await expect(viewer).toHaveAttribute('data-zoom', '2')
      const after = await viewer.locator('[data-page-index="8"]').evaluate((el) => el.getBoundingClientRect().width)
      expect(after / before).toBeCloseTo(2, 1)
      await expect(viewer.locator('[data-page-index="8"]')).toHaveAttribute('data-rendered', 'true')
    })

    test('a PDF behind a URL is read by Range, and CJK text gets its CMap', async ({ page }) => {
      const body = cjkPdf(40, 60_000)
      const served = await serveWithRanges(page, '/e2e-content/chinese.pdf', body, 'application/pdf')
      const cmaps: number[] = []
      page.on('response', (response) => {
        if (response.url().includes('/aiworkspace/pdfjs/cmaps/UniGB-UCS2-H.bcmap')) cmaps.push(response.status())
      })
      await page.goto('/?scenario=normal')
      await page.waitForLoadState('networkidle')
      await registerUrlSource(page, 'chinese.pdf', '/e2e-content/chinese.pdf', 'application/pdf', body.length)
      await page.evaluate(async () => {
        const launch = '/src/app/preview/launch.ts'
        const { openPreview: open } = await import(/* @vite-ignore */ launch)
        open({ source: { kind: 'e2e-url', value: { name: 'chinese.pdf' } } })
      })
      const preview = page.getByTestId('content-preview')
      await expect(preview).toHaveAttribute('data-status', 'ready', { timeout: 20_000 })
      const viewer = page.getByTestId('content-preview-pdfjs')
      await expect(viewer).toHaveAttribute('data-page-count', '40')
      const first = viewer.locator('[data-page-index="0"]')
      await expect(first).toHaveAttribute('data-rendered', 'true')
      expect(await paintedPixels(first)).toBeGreaterThan(100)
      expect(cmaps).toContain(200)
      // The first pages needed a fraction of the file.
      expect(served.ranges).toBeGreaterThan(1)
      expect(served.rangeBytes).toBeLessThan(body.length * 0.6)
    })

    test('a password-protected PDF says so and offers the download', async ({ page }) => {
      const body = lockedPdf()
      await serveWithRanges(page, '/e2e-content/locked.pdf', body, 'application/pdf')
      await page.goto('/?scenario=normal')
      await page.waitForLoadState('networkidle')
      await registerUrlSource(page, 'locked.pdf', '/e2e-content/locked.pdf', 'application/pdf', body.length)
      await page.evaluate(async () => {
        const launch = '/src/app/preview/launch.ts'
        const { openPreview: open } = await import(/* @vite-ignore */ launch)
        open({ source: { kind: 'e2e-url', value: { name: 'locked.pdf' } } })
      })
      await expect(page.getByTestId('content-preview-pdf-locked')).toBeVisible({ timeout: 20_000 })
      await expect(page.getByTestId('content-preview')).toHaveAttribute('data-degraded', 'true')
    })
  })
}
