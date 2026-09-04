/* ── Minimal XLSX reader: ZIP (DecompressionStream) + SpreadsheetML. Reads cached values only. ── */

interface ZipEntry {
  name: string
  method: number
  compressedSize: number
  offset: number
}

function readZipEntries(buf: ArrayBuffer): ZipEntry[] {
  const view = new DataView(buf)
  const bytes = new Uint8Array(buf)
  // locate end-of-central-directory
  let eocd = -1
  for (let i = buf.byteLength - 22; i >= Math.max(0, buf.byteLength - 70_000); i--) {
    if (view.getUint32(i, true) === 0x06054b50) {
      eocd = i
      break
    }
  }
  if (eocd < 0) throw new Error('不是有效的 XLSX（ZIP）文件')
  const count = view.getUint16(eocd + 10, true)
  let p = view.getUint32(eocd + 16, true)
  const entries: ZipEntry[] = []
  const dec = new TextDecoder()
  for (let i = 0; i < count; i++) {
    if (view.getUint32(p, true) !== 0x02014b50) break
    const method = view.getUint16(p + 10, true)
    const compressedSize = view.getUint32(p + 20, true)
    const nameLen = view.getUint16(p + 28, true)
    const extraLen = view.getUint16(p + 30, true)
    const commentLen = view.getUint16(p + 32, true)
    const offset = view.getUint32(p + 42, true)
    const name = dec.decode(bytes.subarray(p + 46, p + 46 + nameLen))
    entries.push({ name, method, compressedSize, offset })
    p += 46 + nameLen + extraLen + commentLen
  }
  return entries
}

async function readZipFile(buf: ArrayBuffer, entry: ZipEntry): Promise<string> {
  const view = new DataView(buf)
  const p = entry.offset
  if (view.getUint32(p, true) !== 0x04034b50) throw new Error('ZIP 本地文件头损坏')
  const nameLen = view.getUint16(p + 26, true)
  const extraLen = view.getUint16(p + 28, true)
  const start = p + 30 + nameLen + extraLen
  const data = new Uint8Array(buf, start, entry.compressedSize)
  if (entry.method === 0) return new TextDecoder().decode(data)
  if (entry.method !== 8) throw new Error(`不支持的压缩方式 ${entry.method}`)
  if (typeof DecompressionStream === 'undefined') throw new Error('当前浏览器不支持解压 XLSX')
  const stream = new Blob([data]).stream().pipeThrough(new DecompressionStream('deflate-raw'))
  return await new Response(stream).text()
}

export interface XlsxWorkbook {
  sheets: Array<{ name: string; path: string }>
}

function xml(text: string): Document {
  return new DOMParser().parseFromString(text, 'application/xml')
}

function attr(el: Element, name: string): string | null {
  return el.getAttribute(name) ?? el.getAttributeNS('http://schemas.openxmlformats.org/officeDocument/2006/relationships', name)
}

export async function listXlsxSheets(buf: ArrayBuffer): Promise<XlsxWorkbook> {
  const entries = readZipEntries(buf)
  const find = (n: string) => entries.find((e) => e.name === n)
  const wbEntry = find('xl/workbook.xml')
  if (!wbEntry) throw new Error('XLSX 缺少 workbook.xml')
  const wb = xml(await readZipFile(buf, wbEntry))
  const relsEntry = find('xl/_rels/workbook.xml.rels')
  const rels = new Map<string, string>()
  if (relsEntry) {
    const relDoc = xml(await readZipFile(buf, relsEntry))
    for (const r of Array.from(relDoc.getElementsByTagName('Relationship'))) {
      const id = r.getAttribute('Id')
      let target = r.getAttribute('Target') ?? ''
      if (target.startsWith('/')) target = target.slice(1)
      else if (!target.startsWith('xl/')) target = `xl/${target}`
      if (id) rels.set(id, target)
    }
  }
  const sheets: XlsxWorkbook['sheets'] = []
  const sheetEls = Array.from(wb.getElementsByTagName('sheet'))
  sheetEls.forEach((s, i) => {
    const name = s.getAttribute('name') ?? `Sheet${i + 1}`
    const rid = attr(s, 'id') ?? s.getAttribute('r:id')
    const path = (rid && rels.get(rid)) || `xl/worksheets/sheet${i + 1}.xml`
    if (entries.some((e) => e.name === path)) sheets.push({ name, path })
  })
  if (sheets.length === 0) throw new Error('XLSX 中没有可读取的工作表')
  return { sheets }
}

const DATE_FMT_IDS = new Set([14, 15, 16, 17, 18, 19, 20, 21, 22, 27, 28, 29, 30, 31, 36, 45, 46, 47, 50, 57, 58])

function serialToDate(serial: number): string {
  const ms = Math.round((serial - 25569) * 86400 * 1000)
  const d = new Date(ms)
  const y = d.getUTCFullYear()
  const m = String(d.getUTCMonth() + 1).padStart(2, '0')
  const day = String(d.getUTCDate()).padStart(2, '0')
  return `${y}-${m}-${day}`
}

function colIndex(ref: string): number {
  let n = 0
  for (const ch of ref) {
    if (ch < 'A' || ch > 'Z') break
    n = n * 26 + (ch.charCodeAt(0) - 64)
  }
  return n - 1
}

export interface XlsxSheetResult {
  matrix: Array<Array<string | number | boolean | null>>
  formulaCellsWithoutCache: number
}

export async function readXlsxSheet(buf: ArrayBuffer, path: string, maxRows = Infinity): Promise<XlsxSheetResult> {
  const entries = readZipEntries(buf)
  const find = (n: string) => entries.find((e) => e.name === n)
  const shared: string[] = []
  const ssEntry = find('xl/sharedStrings.xml')
  if (ssEntry) {
    const ss = xml(await readZipFile(buf, ssEntry))
    for (const si of Array.from(ss.getElementsByTagName('si'))) {
      shared.push(Array.from(si.getElementsByTagName('t')).map((t) => t.textContent ?? '').join(''))
    }
  }
  const dateStyles = new Set<number>()
  const stylesEntry = find('xl/styles.xml')
  if (stylesEntry) {
    const st = xml(await readZipFile(buf, stylesEntry))
    const customDate = new Set<number>()
    for (const nf of Array.from(st.getElementsByTagName('numFmt'))) {
      const code = (nf.getAttribute('formatCode') ?? '').toLowerCase()
      if (/[ymd]/.test(code.replace(/\[.*?\]/g, '')) && !/[#0]/.test(code)) {
        customDate.add(Number(nf.getAttribute('numFmtId')))
      }
    }
    const xfs = st.getElementsByTagName('cellXfs')[0]
    if (xfs) {
      Array.from(xfs.getElementsByTagName('xf')).forEach((xf, i) => {
        const id = Number(xf.getAttribute('numFmtId') ?? -1)
        if (DATE_FMT_IDS.has(id) || customDate.has(id)) dateStyles.add(i)
      })
    }
  }
  const sheetEntry = find(path)
  if (!sheetEntry) throw new Error(`工作表不存在: ${path}`)
  const doc = xml(await readZipFile(buf, sheetEntry))
  const matrix: XlsxSheetResult['matrix'] = []
  let missingCache = 0
  const rowEls = doc.getElementsByTagName('row')
  for (let ri = 0; ri < rowEls.length; ri++) {
    const rowEl = rowEls[ri]
    const rIdx = Number(rowEl.getAttribute('r') ?? ri + 1) - 1
    if (rIdx >= maxRows) break
    const row: Array<string | number | boolean | null> = matrix[rIdx] ?? []
    for (const c of Array.from(rowEl.getElementsByTagName('c'))) {
      const ref = c.getAttribute('r') ?? ''
      const ci = colIndex(ref)
      const t = c.getAttribute('t')
      const s = Number(c.getAttribute('s') ?? -1)
      const vEl = c.getElementsByTagName('v')[0]
      const fEl = c.getElementsByTagName('f')[0]
      let value: string | number | boolean | null = null
      if (t === 'inlineStr') {
        value = Array.from(c.getElementsByTagName('t')).map((x) => x.textContent ?? '').join('')
      } else if (vEl) {
        const raw = vEl.textContent ?? ''
        if (t === 's') value = shared[Number(raw)] ?? ''
        else if (t === 'b') value = raw === '1'
        else if (t === 'str' || t === 'e') value = raw
        else {
          const n = Number(raw)
          value = Number.isFinite(n) ? (dateStyles.has(s) ? serialToDate(n) : n) : raw
        }
      } else if (fEl) {
        missingCache += 1
        value = null
      }
      if (ci >= 0) row[ci] = value
    }
    matrix[rIdx] = row
  }
  for (let i = 0; i < matrix.length; i++) if (!matrix[i]) matrix[i] = []
  const width = Math.max(0, ...matrix.map((r) => r.length))
  const normalized = matrix.map((r) => Array.from({ length: width }, (_, i) => r[i] ?? null))
  while (normalized.length && normalized[normalized.length - 1].every((v) => v === null || v === '')) normalized.pop()
  return { matrix: normalized, formulaCellsWithoutCache: missingCache }
}
