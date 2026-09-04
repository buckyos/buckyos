/* ── CSV / TSV parsing (RFC 4180-ish, delimiter + encoding detection) ── */

export interface ParsedSheet {
  matrix: string[][]
  encoding: string
  encodingWarning?: string
  delimiter: string
}

export function detectDelimiter(sample: string): string {
  const candidates = [',', '\t', ';', '|']
  const lines = sample.split(/\r?\n/).filter((l) => l.trim()).slice(0, 10)
  let best = ','
  let bestScore = -1
  for (const d of candidates) {
    const counts = lines.map((l) => l.split(d).length - 1)
    if (counts.length === 0) continue
    const min = Math.min(...counts)
    const max = Math.max(...counts)
    const score = min > 0 && min === max ? min * 10 : min
    if (score > bestScore) {
      bestScore = score
      best = d
    }
  }
  return best
}

export function parseDelimited(text: string, delimiter?: string): string[][] {
  const d = delimiter ?? detectDelimiter(text.slice(0, 20_000))
  const rows: string[][] = []
  let row: string[] = []
  let field = ''
  let inQuotes = false
  const src = text.charCodeAt(0) === 0xfeff ? text.slice(1) : text
  for (let i = 0; i < src.length; i++) {
    const ch = src[i]
    if (inQuotes) {
      if (ch === '"') {
        if (src[i + 1] === '"') {
          field += '"'
          i++
        } else inQuotes = false
      } else field += ch
      continue
    }
    if (ch === '"') {
      inQuotes = true
    } else if (ch === d) {
      row.push(field)
      field = ''
    } else if (ch === '\n' || ch === '\r') {
      if (ch === '\r' && src[i + 1] === '\n') i++
      row.push(field)
      rows.push(row)
      row = []
      field = ''
    } else field += ch
  }
  if (field !== '' || row.length) {
    row.push(field)
    rows.push(row)
  }
  // drop fully empty trailing rows
  while (rows.length && rows[rows.length - 1].every((v) => v.trim() === '')) rows.pop()
  return rows
}

/** Decode bytes: UTF-8 first; fall back to GBK if the UTF-8 result has replacement chars. */
export function decodeText(buffer: ArrayBuffer): { text: string; encoding: string; warning?: string } {
  const utf8 = new TextDecoder('utf-8', { fatal: false }).decode(buffer)
  const bad = (utf8.match(/�/g) ?? []).length
  if (bad === 0) return { text: utf8, encoding: 'utf-8' }
  try {
    const gbk = new TextDecoder('gbk').decode(buffer)
    if ((gbk.match(/�/g) ?? []).length < bad) {
      return { text: gbk, encoding: 'gbk', warning: '文件不是 UTF-8 编码，已按 GBK 解码，请检查中文是否正确' }
    }
  } catch {
    /* gbk unsupported */
  }
  return { text: utf8, encoding: 'utf-8', warning: `文件包含 ${bad} 处无法解码的字符` }
}

export function parseCsvBuffer(buffer: ArrayBuffer): ParsedSheet {
  const { text, encoding, warning } = decodeText(buffer)
  const delimiter = detectDelimiter(text.slice(0, 20_000))
  return { matrix: parseDelimited(text, delimiter), encoding, encodingWarning: warning, delimiter }
}

export function toTsv(matrix: string[][]): string {
  return matrix.map((r) => r.map((v) => v.replace(/\t/g, ' ').replace(/\r?\n/g, ' ')).join('\t')).join('\n')
}

export function toCsv(matrix: string[][]): string {
  const esc = (v: string) => (/[",\n\r]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v)
  return matrix.map((r) => r.map(esc).join(',')).join('\r\n')
}
