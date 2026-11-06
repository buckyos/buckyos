import { createHash } from 'node:crypto'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { basename, join } from 'node:path'
import { test, expect, openWorkspacePage, mainMenu } from './fixtures'
import type { Page } from '@playwright/test'
import { reviewPdf } from '../../../aiworkspace/fixtures/pdf/src/review'
import type { compare, inventory } from './pdf-fidelity-browser'

type Inventory = Awaited<ReturnType<typeof inventory>>
const input = process.env.AIWS_PDF_FILE
const bytes = input && existsSync(input) ? readFileSync(input) : null
const selected = (process.env.AIWS_PDF_PAGES ?? '1').split(',').map(Number)
const profile = process.env.AIWS_PDF_PROFILE ? JSON.parse(readFileSync(process.env.AIWS_PDF_PROFILE, 'utf8')) : null
const decoderPage = Number(process.env.AIWS_PDF_DECODER_PAGE ?? '2')
let cached: Inventory | undefined

async function framePage(page: Page, groupId: string, multiPage: boolean) {
  const block = page.getByTestId(`aiws-canvas-block-${groupId}`)
  await expect(block).toBeVisible()
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-all').click()
  if (multiPage) {
    const canvas = (await page.getByTestId('aiws-canvas').boundingBox())!
    const box = (await block.boundingBox())!
    const x = canvas.x + canvas.width / 2, y = canvas.y + canvas.height / 2
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
    await page.mouse.down({ button: 'middle' })
    await page.mouse.move(x, y)
    await page.mouse.up({ button: 'middle' })
    await expect.poll(async () => {
      const b = (await block.boundingBox())!
      return Math.abs(b.y + b.height / 2 - y)
    }).toBeLessThan(4)
    const scale = Math.min((canvas.width - 120) / box.width, (canvas.height - 140) / box.height)
    await page.keyboard.down('Control')
    await page.mouse.wheel(0, -Math.log(scale) / 0.0015)
    await page.keyboard.up('Control')
    await expect.poll(async () => Math.abs((await block.boundingBox())!.height - box.height * scale)).toBeLessThan(4)
    await page.mouse.move(5, 5)
  }
  return block
}

async function decodedPageImages(page: Page, ids: string[]) {
  const images = await Promise.all(ids.map(async id => {
    const img = page.getByTestId(`aiws-canvas-block-${id}`).getByTestId('aiws-asset-image')
    await expect(img).toBeVisible()
    return img.evaluate(async (node: HTMLImageElement) => {
      await node.decode()
      const bytes = await (await fetch(node.src)).arrayBuffer()
      const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))
      return { width: node.naturalWidth, height: node.naturalHeight, sha256: [...digest].map(b => b.toString(16).padStart(2, '0')).join('') }
    })
  }))
  return images
}

const cases = selected.map(pageNumber => ({ pageNumber, end: pageNumber }))
if (profile && selected.includes(1)) cases.push({ pageNumber: 1, end: 4 })
for (const { pageNumber, end } of cases) {
  const multiPage = end !== pageNumber
  test(`PDF fidelity ${multiPage ? 'first page of a multi-page import' : `page ${pageNumber}`}`, async ({ page, api }, info) => {
    test.skip(!bytes, 'Set AIWS_PDF_FILE to compare a local PDF')
    const source = bytes!
    if (profile) expect(createHash('sha256').update(source).digest('hex')).toBe(profile.sha256)
    const mode = info.project.name === 'raster' ? 'raster' : 'editable'
    const errors: string[] = []
    const consoleMessages: { phase: string; type: string; text: string }[] = []
    let phase = 'inventory'
    page.on('pageerror', error => errors.push(error.message))
    page.on('console', message => { if (['warning', 'error'].includes(message.type())) consoleMessages.push({ phase, type: message.type(), text: message.text() }) })
    const ws = await api.freeCanvas('tok-alice', `pdf-fidelity-${mode}-${pageNumber}-${Date.now()}`)
    await openWorkspacePage(page, 'tok-alice', ws.workspace_id, 'free')
    if (!cached) {
      cached = await page.evaluate(async ({ base64, pages }) => {
        const modulePath = '/tests/aiworkspace/pdf-fidelity-browser.ts'
        const module = await import(modulePath)
        return module.inventory(base64, pages)
      }, { base64: source.toString('base64'), pages: process.env.AIWS_PDF_ANALYZE_ALL === '1' ? undefined : selected })
    }
    const original = cached!.pages.find(p => p.page === pageNumber)
    expect(original, `page ${pageNumber} must exist in ${cached!.pageCount} pages`).toBeTruthy()
    await info.attach('source-inventory', { contentType: 'application/json', body: Buffer.from(JSON.stringify(cached)) })
    phase = 'import'
    const [chooser] = await Promise.all([page.waitForEvent('filechooser', { timeout: 15_000 }), mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-pdf')])
    await chooser.setFiles({ name: basename(input!), mimeType: 'application/pdf', buffer: source })
    const dialog = page.getByTestId('aiws-pdf-dialog')
    await expect(dialog.getByTestId('aiws-pdf-pages')).toBeVisible()
    await dialog.getByTestId('aiws-pdf-from').fill(String(pageNumber))
    await dialog.getByTestId('aiws-pdf-to').fill(String(end))
    await dialog.getByTestId(`aiws-pdf-mode-${mode}`).check()
    await dialog.getByTestId('aiws-pdf-columns').fill('1')
    const started = performance.now()
    await dialog.getByTestId('aiws-pdf-start').click()
    await expect(dialog).toHaveCount(0, { timeout: 60_000 })
    const report = page.getByTestId('aiws-import-report')
    const notes: Record<string, string> = {}
    if (await report.isVisible()) {
      for (const note of await report.locator('[data-kind]').all()) notes[(await note.getAttribute('data-kind'))!] = await note.innerText()
      await report.getByTestId('aiws-import-report-close').click()
    }
    const outline = await api.outline('tok-alice', ws.workspace_id)
    const surface = outline.find(e => e.kind === 'surface' && e.entity_id !== 'free' && !e.deleted)
    expect(surface).toBeTruthy()
    const groups = outline.filter(e => e.parent_id === surface.entity_id && e.kind === 'group' && !e.deleted)
    expect(groups).toHaveLength(end - pageNumber + 1)
    const group = groups[0]
    const objects = await Promise.all(outline.filter(e => e.parent_id === group.entity_id && !e.deleted).map(async entity => ({ entity, payload: (await api.read('tok-alice', ws.workspace_id, entity.entity_id)).content.payload })))
    const imageIds = objects.filter(o => ['raster', 'image'].includes(o.payload.config?.pdf_source?.kind)).map(o => o.entity.entity_id)
    await page.keyboard.press('Escape')
    const block = await framePage(page, group.entity_id, multiPage)
    await page.evaluate(() => document.fonts.ready)
    const decoded = await decodedPageImages(page, imageIds)
    const actualPath = info.outputPath('imported.png')
    const actual = await block.screenshot({ path: actualPath })
    phase = 'reference'
    const referenceDir = process.env.AIWS_PDF_REFERENCE_DIR
    const browserReference = referenceDir ? null : await page.evaluate(async ({ base64, pageNumber, edge }) => {
      const modulePath = '/tests/aiworkspace/pdf-fidelity-browser.ts'
      return (await import(modulePath)).reference(base64, pageNumber, edge)
    }, { base64: source.toString('base64'), pageNumber, edge: Math.round((await block.boundingBox())!.width) })
    const referenceBytes = referenceDir ? readFileSync(join(referenceDir, `page-${pageNumber}.png`)) : Buffer.from(browserReference.split(',')[1], 'base64')
    const referencePath = info.outputPath('reference.png'); writeFileSync(referencePath, referenceBytes)
    const comparison = await page.evaluate(async ({ reference, imported, regions, colors }) => {
      const modulePath = '/tests/aiworkspace/pdf-fidelity-browser.ts'
      return (await import(modulePath)).compare(reference, imported, regions, colors)
    }, { reference: referenceBytes.toString('base64'), imported: actual.toString('base64'), colors: [...new Set(original!.paragraphs.flatMap(p => [p.style.color, ...p.styleOverrides.map(piece => piece.style.color)]))].filter(color => color !== '#000000'), regions: [
      ...(profile?.pages.find((entry: { page: number }) => entry.page === pageNumber)?.regions ?? []),
      { name: 'paper-bottom-left', x: 0.01, y: 0.92, w: 0.10, h: 0.06 },
      ...original!.images.map((image, index) => ({ name: `image-${index}`, x: image.rect.x / original!.worldWidth, y: image.rect.y / original!.worldHeight, w: image.rect.w / original!.worldWidth, h: image.rect.h / original!.worldHeight })),
    ] })
    const heatPath = info.outputPath('difference.png'); writeFileSync(heatPath, Buffer.from(comparison.heatmap.split(',')[1], 'base64')); delete comparison.heatmap
    const texts = objects.flatMap(o => o.payload.config?.pdf_source?.text_layout?.spans ?? []).map(span => span.text).join('')
    const count = (text: string) => { const result = new Map<string, number>(); for (const c of text.normalize('NFKC').replace(/\s/gu, '')) result.set(c, (result.get(c) ?? 0) + 1); return result }
    const before = count(original!.text.map(t => t.str).join('')), after = count(texts)
    const sourceCharacters = [...before.values()].reduce((a, b) => a + b, 0)
    const retainedCharacters = [...before].reduce((sum, [c, n]) => sum + Math.min(n, after.get(c) ?? 0), 0)
    const missingCharacters = mode === 'raster' ? [] : [...before].flatMap(([c, n]) => n > (after.get(c) ?? 0) ? [{ character: c, codepoint: `U+${c.codePointAt(0)!.toString(16).toUpperCase()}`, count: n - (after.get(c) ?? 0) }] : [])
    const decodingFailures = consoleMessages.filter(message => message.phase !== 'inventory' && /Unable to decode image|failed to initialize|wasmUrl.*provided|Failed to resolve module specifier.*fallback/i.test(message.text))
    const result = { referenceEngine: referenceDir ? 'independent PNG reference' : 'PDF.js (cannot detect shared renderer failures)', mode, page: pageNumber, inputSha256: createHash('sha256').update(source).digest('hex'), importSeconds: (performance.now() - started) / 1000, decoded, notes, objects: objects.length, rasters: objects.filter(o => o.payload.config?.pdf_source?.kind === 'raster').length, sourceCharacters, retainedCharacters, editableCharacterCoverage: mode === 'raster' ? null : sourceCharacters ? retainedCharacters / sourceCharacters : 1, sourceMixedStyleParagraphs: original!.paragraphs.filter(p => p.mixed).length, sourceStyleOverrides: original!.paragraphs.reduce((n, p) => n + p.styleOverrides.length, 0), sourceOperations: original!.rawOperations, importOperations: original!.extractedOperations, renderOperations: original!.renderOperations, comparison, errors, consoleMessages, decodingFailures, missingCharacters, unreadableText: original!.textStats, filteredPrivateUseOperators: mode === 'editable' && texts ? original!.badOperators : [] }
    writeFileSync(info.outputPath('metrics.json'), JSON.stringify(result, null, 2))
    for (const [name, path] of [['source', referencePath], ['imported', actualPath], ['difference', heatPath]]) await info.attach(name, { path, contentType: 'image/png' })
    await info.attach('metrics', { path: info.outputPath('metrics.json'), contentType: 'application/json' })
    expect.soft(errors).toEqual([])
    expect.soft(notes.pdf_page_failed, 'conversion must not silently fall back to a whole-page image').toBeUndefined()
    if (profile && mode === 'editable' && pageNumber === 1) {
      expect.soft(texts).toContain('ROW 01 - SAMPLE TEXT')
      expect.soft(result.editableCharacterCoverage).toBeGreaterThanOrEqual(0.995)
    }
    expect.soft(notes.pdf_image_unreadable === undefined || result.rasters > 0, 'source images must decode or survive in a raster').toBe(true)
    expect.soft(decodingFailures, 'PDF image decoders must load successfully').toEqual([])
    for (const color of comparison.colorCoverage) {
      if (color.sourceCoverage > 0.00005) expect.soft(color.actualCoverage / color.sourceCoverage, `${color.color} text colour must survive`).toBeGreaterThan(0.5)
    }
    const paper = comparison.regions.find((region: { name: string }) => region.name === 'paper-bottom-left')!
    expect.soft(paper.meanColorError, 'paper should retain its source colour').toBeLessThan(0.02)
    const sourceTexture = paper.sourceStd.reduce((a: number, b: number) => a + b, 0)
    const actualTexture = paper.actualStd.reduce((a: number, b: number) => a + b, 0)
    if (sourceTexture > 6) expect.soft(actualTexture / sourceTexture, 'paper texture must not disappear').toBeGreaterThan(0.4)
    if (mode === 'editable' && texts) expect.soft(result.filteredPrivateUseOperators, 'unreadable source glyphs must not be filtered out with editable text').toEqual([])
    if (mode === 'editable' && result.editableCharacterCoverage! < 0.995) expect.soft(result.rasters, 'symbols absent from editable text need a source raster fallback').toBeGreaterThan(0)
    for (const image of comparison.regions.filter((region: { name: string }) => region.name.startsWith('image-') || region.name.startsWith('glyph-'))) {
      if (image.sourceDarkCoverage > 0.001) expect.soft(image.actualDarkCoverage / image.sourceDarkCoverage, `${image.name} must retain its visible ink`).toBeGreaterThan(0.5)
    }
    await page.reload()
    await framePage(page, group.entity_id, multiPage)
    await page.evaluate(() => document.fonts.ready)
    await decodedPageImages(page, imageIds)
    const reloadedPath = info.outputPath('reloaded.png')
    const reloaded = await block.screenshot({ path: reloadedPath })
    const persisted: Awaited<ReturnType<typeof compare>> = await page.evaluate(async ({ reference, imported, colors }) => {
      const modulePath = '/tests/aiworkspace/pdf-fidelity-browser.ts'
      return (await import(modulePath)).compare(reference, imported, [{ name: 'paper-bottom-left', x: 0.01, y: 0.92, w: 0.10, h: 0.06 }], colors)
    }, { reference: referenceBytes.toString('base64'), imported: reloaded.toString('base64'), colors: comparison.colorCoverage.map((color: { color: string }) => color.color) })
    const persistedPaper = persisted.regions.find(region => region.name === 'paper-bottom-left')!
    expect.soft(persistedPaper.meanColorError, 'paper colour must survive reload').toBeLessThan(0.02)
    if (sourceTexture > 6) expect.soft(persistedPaper.actualStd.reduce((a, b) => a + b, 0) / sourceTexture, 'paper texture must survive reload').toBeGreaterThan(0.4)
    for (const color of persisted.colorCoverage) {
      if (color.sourceCoverage > 0.00005) expect.soft(color.actualCoverage / color.sourceCoverage, `${color.color} must survive reload`).toBeGreaterThan(0.5)
    }
    await info.attach('reloaded', { path: reloadedPath, contentType: 'image/png' })
    expect((await api.outline('tok-alice', ws.workspace_id)).filter(e => e.parent_id === group.entity_id && !e.deleted)).toHaveLength(objects.length)
  })
}


test('PDF decoder asset diagnostic control', async ({ page }, info) => {
  test.skip(!bytes || process.env.AIWS_PDF_DECODER_CONTROL !== '1' || !selected.includes(decoderPage) || info.project.name !== 'editable', 'Enable decoder control for the selected image page in the editable project')
  test.skip(!process.env.AIWS_PDF_REFERENCE_DIR, 'Independent PNG reference required for decoder control')
  await page.goto('/')
  const messages: { phase: string; text: string }[] = []
  let phase = 'without-wasm'
  page.on('console', message => { if (['warning', 'error'].includes(message.type())) messages.push({ phase, text: message.text() }) })
  const render = (wasmUrl?: string | null) => page.evaluate(async ({ base64, wasmUrl, pageNumber }) => {
    const modulePath = '/tests/aiworkspace/pdf-fidelity-browser.ts'
    return (await import(modulePath)).reference(base64, pageNumber, 1200, wasmUrl)
  }, { base64: bytes!.toString('base64'), wasmUrl, pageNumber: decoderPage })
  const without = await render(null)
  phase = 'with-wasm'
  const responses: { path: string; status: number }[] = []
  page.on('response', response => { if (response.url().endsWith('.wasm')) responses.push({ path: new URL(response.url()).pathname, status: response.status() }) })
  const withWasm = await render()
  phase = 'inventory'
  const source = cached?.pages.find(p => p.page === decoderPage) ?? (await page.evaluate(async ({ base64, pageNumber }) => { const path = '/tests/aiworkspace/pdf-fidelity-browser.ts'; return (await import(path)).inventory(base64, [pageNumber]) }, { base64: bytes!.toString('base64'), pageNumber: decoderPage })).pages[0]
  const image = source.images[0]
  const rect = { name: 'decoded-image', x: image.rect.x / source.worldWidth, y: image.rect.y / source.worldHeight, w: image.rect.w / source.worldWidth, h: image.rect.h / source.worldHeight }
  const refDir = process.env.AIWS_PDF_REFERENCE_DIR
  test.skip(!refDir, 'Independent PNG reference required for decoder control')
  const ref = readFileSync(join(refDir!, `page-${decoderPage}.png`)).toString('base64')
  const compare = (actual: string) => page.evaluate(async ({ ref, actual, rect }) => { const path = '/tests/aiworkspace/pdf-fidelity-browser.ts'; const result = await (await import(path)).compare(ref, actual.split(',')[1], [rect]); delete result.heatmap; return result }, { ref, actual, rect })
  const before = await compare(without), after = await compare(withWasm)
  for (const [name, data] of [['without-wasm', without], ['with-wasm', withWasm]]) {
    const path = info.outputPath(`${name}.png`); writeFileSync(path, Buffer.from(data.split(',')[1], 'base64')); await info.attach(name, { path, contentType: 'image/png' })
  }
  const result = { before, after, messages, responses, productionCodeChanged: false }
  writeFileSync(info.outputPath('decoder-control.json'), JSON.stringify(result, null, 2));await info.attach('decoder-control', { path: info.outputPath('decoder-control.json'), contentType: 'application/json' })
  expect(messages.some(m => m.phase === 'without-wasm' && /Unable to decode image/.test(m.text))).toBe(true)
  expect(messages.filter(m => m.phase === 'with-wasm' && /Unable to decode image|failed to initialize|wasmUrl.*provided/.test(m.text))).toEqual([])
  expect(responses.some(r => r.path.endsWith('/aiworkspace/pdfjs/wasm/jbig2.wasm') && r.status === 200)).toBe(true)
  const fallback = await page.request.get('/aiworkspace/pdfjs/wasm/jbig2_nowasm_fallback.js')
  expect(fallback.status()).toBe(200)
  expect(fallback.headers()['content-type']).toContain('javascript')
  expect(after.regions[1].meanAbsoluteError).toBeLessThan(before.regions[1].meanAbsoluteError)
})


test('PDF review glyphs, paint order and optimized images', async ({ page, api }, info) => {
  test.skip(info.project.name !== 'editable', 'These checks exercise editable import')
  const source = Buffer.from(reviewPdf())
  const name = 'synthetic-pdf-review.pdf'
  writeFileSync(info.outputPath(name), source)
  const ws = await api.freeCanvas('tok-alice', 'pdf-review')
  await openWorkspacePage(page, 'tok-alice', ws.workspace_id, 'free')
  const triggers = await page.evaluate(async base64 => {
    const modulePath = '/tests/aiworkspace/pdf-fidelity-browser.ts'
    return (await import(modulePath)).reviewTriggers(base64)
  }, source.toString('base64'))
  expect(triggers.text).toContain('ABCDEFGH\uE123IJ')
  expect(triggers).toMatchObject({ repeat: true, inlineGroup: true })
  const [chooser] = await Promise.all([page.waitForEvent('filechooser'), mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-pdf')])
  await chooser.setFiles({ name, mimeType: 'application/pdf', buffer: source })
  const dialog = page.getByTestId('aiws-pdf-dialog')
  await expect(dialog.getByTestId('aiws-pdf-pages')).toBeVisible()
  await dialog.getByTestId('aiws-pdf-columns').fill('1')
  await dialog.getByTestId('aiws-pdf-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 60_000 })
  const report = page.getByTestId('aiws-import-report')
  await expect.soft(report.locator('[data-kind="pdf_paint_order"]')).toBeVisible()
  await expect.soft(report.locator('[data-kind="pdf_glyph_raster"]')).toBeVisible()
  await expect.soft(report.locator('[data-kind="pdf_invisible_text"]')).toBeVisible()
  if (await report.isVisible()) await report.getByTestId('aiws-import-report-close').click()
  const outline = await api.outline('tok-alice', ws.workspace_id)
  const surface = outline.find(e => e.kind === 'surface' && e.entity_id !== 'free' && !e.deleted)!
  const groups = outline.filter(e => e.parent_id === surface.entity_id && e.kind === 'group' && !e.deleted)
  expect(groups).toHaveLength(3)
  const result = []
  for (const [index, group] of groups.entries()) {
    const children = outline.filter(e => e.parent_id === group.entity_id && !e.deleted)
    const objects = await Promise.all(children.map(async entity => ({
      id: entity.entity_id,
      source: (await api.read('tok-alice', ws.workspace_id, entity.entity_id)).content.payload.config?.pdf_source,
    })))
    const pageNumber = index + 1
    expect(objects.every(o => o.source?.page === pageNumber)).toBe(true)
    const rasters = objects.filter(o => o.source?.kind === 'raster')
    const images = objects.filter(o => o.source?.kind === 'image')
    const texts = objects.flatMap(o => o.source?.text_layout?.spans ?? []).map(span => span.text).join(' ')
    if (pageNumber === 1) {
      expect.soft(objects.map(o => o.source?.kind).sort()).toEqual(['page', 'raster'])
    } else if (pageNumber === 2) {
      expect.soft(texts).toBe('STAYS EDITABLE')
      expect.soft(rasters).toHaveLength(1)
    } else {
      expect.soft(images).toHaveLength(14)
      expect.soft(rasters).toHaveLength(0)
    }
    const block = await framePage(page, group.entity_id, true)
    const decoded = await decodedPageImages(page, [...rasters, ...images].map(o => o.id))
    await page.evaluate(() => document.fonts.ready)
    await block.screenshot({ path: info.outputPath(`review-page-${pageNumber}.png`) })
    result.push({ page: pageNumber, texts, rasters: rasters.length, images: images.length, decoded })
  }
  await info.attach('review-results', { contentType: 'application/json', body: Buffer.from(JSON.stringify({ triggers, pages: result }, null, 2)) })
})

test('PDF import rendering continues without animation frames', async ({ page }, info) => {
  test.skip(info.project.name !== 'editable', 'The two import modes share the same display renderer')
  await page.goto('/')
  const result = await page.evaluate(async base64 => {
    const modulePath = '/tests/aiworkspace/pdf-fidelity-browser.ts'
    const { renderWithoutAnimationFrames, abortActivePdfRender } = await import(modulePath)
    return { ...(await renderWithoutAnimationFrames(base64)), aborted: await abortActivePdfRender(base64) }
  }, Buffer.from(reviewPdf()).toString('base64'))
  expect(result.operators).toBeGreaterThan(0)
  expect(result.pixel[2]).toBeGreaterThan(result.pixel[0])
  expect(result.aborted).toBe(true)
})
