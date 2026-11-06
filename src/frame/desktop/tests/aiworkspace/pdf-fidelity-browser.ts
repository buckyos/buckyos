import { openPdfDocument, renderOperators, renderPage, loadPdfjs } from '../../src/app/aiworkspace/ui/media/pdfDoc'
import { extractPage, worldOf, type PdfjsDocument, type PdfjsPage } from '@pdf/mod'
import { interpret } from '@pdf/interpret'
import { groupText, readStruct } from '@pdf/text'

const decode = (base64: string) => Uint8Array.from(atob(base64), c => c.charCodeAt(0))
const styleKey = (style: unknown) => JSON.stringify(style)

export async function inventory(base64: string, pages?: number[]) {
  const { doc, close } = await openPdfDocument(decode(base64))
  try {
    const results = []
    for (const n of pages ?? Array.from({ length: doc.numPages }, (_, i) => i + 1)) {
      const proxy = await doc.getPage(n)
      const sourceOperators = await proxy.getOperatorList()
      const operators = await renderOperators(proxy)
      const { page } = await extractPage(doc as unknown as PdfjsDocument, proxy as unknown as PdfjsPage, new Map(), null, operators ?? { fnArray: [], argsArray: [] })
      const { world } = worldOf(page)
      const marks = interpret(page, world)
      const text = groupText(page, world, marks.runs, readStruct(page.struct), { unit: 'paragraph' })
      const paragraphs = text.paragraphs.map(p => ({ text: p.text, rect: p.rect, rotation: p.rotation, style: p.style, mixed: p.mixed, layout: p.layout,
        styleOverrides: p.lines.flatMap(line => line.pieces).filter(piece => piece.str.trim() && styleKey(piece.style) !== styleKey(p.style)).map(piece => ({ text: piece.str, style: piece.style })) }))
      const renderLengths = new Set<number>()
      await renderPage(proxy, 32, { transparent: true, filter: (_index, list) => { renderLengths.add(list.fnArray.length); return false } })
      const images = []
      for (const mark of marks.images) {
        const store = mark.ref.startsWith('g_') ? proxy.commonObjs : proxy.objs
        const data = await new Promise<unknown>(resolve => { const timer = setTimeout(() => resolve(null), 1000); try { store.get(mark.ref, (value: unknown) => { clearTimeout(timer); resolve(value) }) } catch { clearTimeout(timer); resolve(null) } })
        const v = data as { width?: number; height?: number; kind?: number; bitmap?: ImageBitmap; data?: Uint8Array } | null
        const xs = mark.corners.map(p => p.x), ys = mark.corners.map(p => p.y)
        images.push({ ref: mark.ref, width: v?.width, height: v?.height, kind: v?.kind, bitmap: v?.bitmap ? { width: v.bitmap.width, height: v.bitmap.height } : null, dataLength: v?.data?.length, dataType: v?.data?.constructor.name, keys: v ? Object.keys(v) : [], complex: mark.complex,
          rect: { x: Math.min(...xs), y: Math.min(...ys), w: Math.max(...xs) - Math.min(...xs), h: Math.max(...ys) - Math.min(...ys) } })
      }
      results.push({ page: n, width: page.width, height: page.height, worldWidth: Math.round(page.width * worldOf(page).scale), worldHeight: Math.round(page.height * worldOf(page).scale), rawOperations: sourceOperators.fnArray.length, extractedOperations: page.fnArray.length, renderOperations: [...renderLengths], fonts: page.fonts, text: page.text, paragraphs, images, textStats: { total: text.total, unreadable: text.unreadable, invisible: text.invisible, vertical: text.vertical }, badSourceItems: page.text.filter(item => /[\uE000-\uF8FF\uFFFD]/u.test(item.str)), badOperators: page.fnArray.flatMap((op, index) => text.converted.has(index) && /[\uE000-\uF8FF\uFFFD]/u.test(JSON.stringify(page.argsArray[index])) ? [{ index, op, args: page.argsArray[index] }] : []) })
      proxy.cleanup()
    }
    return { pageCount: doc.numPages, pages: results }
  } finally { close() }
}

export async function reference(base64: string, pageNumber: number, edge: number, wasmUrl?: string | null) {
  const handle = wasmUrl !== undefined ? await (async () => { const pdfjs = await loadPdfjs(); const task = pdfjs.getDocument({ data: decode(base64), cMapUrl: new URL('/aiworkspace/pdfjs/cmaps/', location.href).href, cMapPacked: true, standardFontDataUrl: new URL('/aiworkspace/pdfjs/standard_fonts/', location.href).href, fontExtraProperties: true, ...(wasmUrl ? { wasmUrl: new URL(wasmUrl, location.href).href } : {}) }); return { doc: await task.promise, close: () => { void task.destroy() } } })() : await openPdfDocument(decode(base64))
  const { doc, close } = handle
  try { return (await renderPage(await doc.getPage(pageNumber), edge)).toDataURL('image/png') } finally { close() }
}

export async function compare(referenceBase64: string, importedBase64: string, regions: { name: string; x: number; y: number; w: number; h: number }[], colors: string[] = []) {
  const load = async (base64: string) => createImageBitmap(new Blob([decode(base64)], { type: 'image/png' }))
  const [source, actual] = await Promise.all([load(referenceBase64), load(importedBase64)])
  const width = actual.width, height = actual.height
  const pixels = (image: ImageBitmap) => { const c = document.createElement('canvas'); c.width = width; c.height = height; const ctx = c.getContext('2d')!; ctx.drawImage(image, 0, 0, width, height); return ctx.getImageData(0, 0, width, height).data }
  const a = pixels(source), b = pixels(actual)
  const colorCoverage = colors.filter(color => /^#[0-9a-f]{6}$/i.test(color)).map(color => {
    const rgb = [1, 3, 5].map(start => Number.parseInt(color.slice(start, start + 2), 16))
    let sourcePixels = 0, actualPixels = 0
    for (let i = 0; i < a.length; i += 4) {
      if (rgb.every((value, channel) => Math.abs(a[i + channel] - value) <= 32)) sourcePixels++
      if (rgb.every((value, channel) => Math.abs(b[i + channel] - value) <= 32)) actualPixels++
    }
    return { color, sourceCoverage: sourcePixels / (a.length / 4), actualCoverage: actualPixels / (b.length / 4) }
  })
  const measure = (r: typeof regions[number]) => {
    let difference = 0, sourceDark = 0, actualDark = 0, count = 0
    const sourceSum = [0, 0, 0], actualSum = [0, 0, 0], sourceSq = [0, 0, 0], actualSq = [0, 0, 0]
    const x0 = Math.max(0, Math.round(r.x * width)), y0 = Math.max(0, Math.round(r.y * height)), x1 = Math.min(width, Math.round((r.x + r.w) * width)), y1 = Math.min(height, Math.round((r.y + r.h) * height))
    for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) { const i = (y * width + x) * 4; for (let c = 0; c < 3; c++) { difference += Math.abs(a[i + c] - b[i + c]); sourceSum[c] += a[i + c]; actualSum[c] += b[i + c]; sourceSq[c] += a[i + c] ** 2; actualSq[c] += b[i + c] ** 2 } if ((a[i] + a[i + 1] + a[i + 2]) / 3 < 120) sourceDark++; if ((b[i] + b[i + 1] + b[i + 2]) / 3 < 120) actualDark++; count++ }
    const sourceMean = sourceSum.map(n => n / Math.max(count, 1)), actualMean = actualSum.map(n => n / Math.max(count, 1))
    const sourceStd = sourceSq.map((n, i) => Math.sqrt(Math.max(0, n / Math.max(count, 1) - sourceMean[i] ** 2))), actualStd = actualSq.map((n, i) => Math.sqrt(Math.max(0, n / Math.max(count, 1) - actualMean[i] ** 2)))
    return { name: r.name, sourceMean, actualMean, sourceStd, actualStd, meanColorError: sourceMean.reduce((n, v, i) => n + Math.abs(v - actualMean[i]), 0) / (3 * 255), meanAbsoluteError: count ? difference / (count * 3 * 255) : 0, sourceDarkCoverage: count ? sourceDark / count : 0, actualDarkCoverage: count ? actualDark / count : 0 }
  }
  const heat = document.createElement('canvas'); heat.width = width; heat.height = height; const ctx = heat.getContext('2d')!; const diff = ctx.createImageData(width, height)
  for (let i = 0; i < a.length; i += 4) { const v = Math.max(Math.abs(a[i] - b[i]), Math.abs(a[i + 1] - b[i + 1]), Math.abs(a[i + 2] - b[i + 2])); diff.data[i] = 255; diff.data[i + 1] = diff.data[i + 2] = 255 - Math.min(255, v * 4); diff.data[i + 3] = 255 }
  ctx.putImageData(diff, 0, 0); source.close(); actual.close()
  return { width, height, colorCoverage, regions: [measure({ name: 'page', x: 0, y: 0, w: 1, h: 1 }), ...regions.map(measure)], heatmap: heat.toDataURL('image/png') }
}

export async function reviewTriggers(base64: string) {
  const { doc, close } = await openPdfDocument(decode(base64))
  try {
    const page = await doc.getPage(2)
    const text = (await page.getTextContent()).items.flatMap(item => 'str' in item ? [item.str] : [])
    const imagePage = await doc.getPage(3)
    const operators = await renderOperators(imagePage)
    const { OPS } = await loadPdfjs()
    return {
      text,
      repeat: operators?.fnArray.includes(OPS.paintImageXObjectRepeat) ?? false,
      inlineGroup: operators?.fnArray.includes(OPS.paintInlineImageXObjectGroup) ?? false,
    }
  } finally { close() }
}

export async function renderWithoutAnimationFrames(base64: string) {
  const controller = new AbortController()
  const { doc, close } = await openPdfDocument(decode(base64), controller.signal)
  const raf = window.requestAnimationFrame
  const timer = setTimeout(() => controller.abort(new Error('PDF import waited for animation frames')), 3000)
  window.requestAnimationFrame = () => 0
  try {
    const page = await doc.getPage(1)
    const operators = await renderOperators(page, controller.signal)
    if (!operators?.fnArray.length) throw new Error('No display operators captured')
    const canvas = await renderPage(page, 720, { importWork: true, signal: controller.signal, filter: (_i, list) => list === operators })
    const pixel = [...canvas.getContext('2d')!.getImageData(60, 190, 1, 1).data]
    return { operators: operators.fnArray.length, pixel }
  } finally {
    window.requestAnimationFrame = raf
    clearTimeout(timer)
    close()
  }
}


export async function abortActivePdfRender(base64: string) {
  const { doc, close } = await openPdfDocument(decode(base64))
  const controller = new AbortController()
  const reason = new Error('Cancelled synthetic import')
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    const page = await doc.getPage(1)
    try {
      await renderPage(page, 2560, {
        importWork: true, signal: controller.signal, filter: () => {
          timer ??= setTimeout(() => controller.abort(reason), 0)
          return true
        },
      })
      return false
    } catch (error) {
      return error === reason
    }
  } finally {
    clearTimeout(timer)
    close()
  }
}

export async function syntheticColorControl(base64: string) {
  const { doc, close } = await openPdfDocument(decode(base64))
  try {
    const proxy = await doc.getPage(1)
    const ops = await renderOperators(proxy)
    const { page } = await extractPage(doc as unknown as PdfjsDocument, proxy as unknown as PdfjsPage, new Map(), null, ops ?? undefined)
    const fills = interpret(page, worldOf(page).world).paths.map(path => path.fill)
    const canvas = await renderPage(proxy, 200, { importWork: true })
    const context = canvas.getContext('2d')!
    return { fills, pixels: [50, 150].map(x => [...context.getImageData(x, 50, 1, 1).data]) }
  } finally { close() }
}
