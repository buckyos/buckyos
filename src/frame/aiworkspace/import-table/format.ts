/* Spreadsheet number formats (表格数据导入 TODO §5.3): what an xlsx `numFmt`, an Excel HTML `mso-number-format` or a
 * Google Sheets pattern says a number is — a date, a date with time, a time of day, money (with its code or symbol),
 * a percentage — and Excel's date serial numbers in both of its date systems. */

import type { CellHint } from './grid.ts'
import { LCID_CURRENCY } from './currency.ts'

export interface FormatInfo { hint?: CellHint | 'time'; currency?: string }

/** Built-in format ids (ECMA-376 §18.8.30 and the CJK ids Excel writes). */
function builtin(id: number): FormatInfo | null {
  if (id === 9 || id === 10) return { hint: 'percent' }
  if ((id >= 5 && id <= 8) || id === 42 || id === 44) return { hint: 'currency' }
  if ((id >= 14 && id <= 17) || (id >= 27 && id <= 31) || (id >= 34 && id <= 36) || (id >= 50 && id <= 58)) return { hint: 'date' }
  if (id === 22) return { hint: 'datetime' }
  if ((id >= 18 && id <= 21) || id === 32 || id === 33 || (id >= 45 && id <= 47)) return { hint: 'time' }
  return null
}

/** Classify a format code (and a built-in id, used when the code is absent or general). */
export function classifyFormat(code: string | null | undefined, id?: number): FormatInfo {
  const known = id !== undefined ? builtin(id) : null
  if (!code || /^general$/i.test(code.trim())) return known ?? {}
  // the first section (positive numbers) decides
  const section = splitSections(code)[0]
  const named = /^(short|medium|long) date$/i.test(section) ? 'date' : /^general date$/i.test(section) ? 'datetime' : /^(short|medium|long) time$/i.test(section) ? 'time'
    : /^percent$/i.test(section) ? 'percent' : /^currency$/i.test(section) ? 'currency' : null
  if (named) return { hint: named as FormatInfo['hint'] }
  let currency: string | undefined
  // [$XXX-lcid] / [$¥-804] / [$€-2]: the code, else the locale, else the symbol
  for (const m of section.matchAll(/\[\$([^\]-]*)(?:-([0-9A-Fa-f]+))?\]/g)) {
    const mark = m[1].trim()
    if (/^[A-Z]{3}$/.test(mark)) { currency = mark; break }
    const byLcid = m[2] ? LCID_CURRENCY[m[2].toLowerCase().replace(/^0+/, '')] : undefined
    if (byLcid && mark) { currency = byLcid; break }
    if (mark) { currency = mark; break }
  }
  // what is left once quoted text, escapes, brackets and fill / padding are gone
  const quoted = [...section.matchAll(/"([^"]*)"/g)].map((m) => m[1]).join('')
  const bare = section.replace(/"[^"]*"/g, '').replace(/\[[^\]]*\]/g, '').replace(/\\./g, '').replace(/[_*]./g, '')
  const escaped = [...section.matchAll(/\\(.)/g)].map((m) => m[1]).join('')
  if (!currency) {
    const literal = quoted + escaped + bare
    const symbol = /(HK\$|NT\$|US\$|R\$|A\$|C\$|S\$|[$€£¥￥₩₹₫₽฿₺]|[A-Z]{3}(?=\s|$))/.exec(literal)
    if (symbol && /[$€£¥￥₩₹₫₽฿₺]|^[A-Z]{3}$/.test(symbol[1]) && /[#0]/.test(bare)) currency = symbol[1]
  }
  if (currency) return { hint: 'currency', currency }
  if (/[yYdD]/.test(bare)) return { hint: /[hHsS]/.test(bare) ? 'datetime' : 'date' }
  if (/[hHsS]/.test(bare)) return { hint: 'time' }
  // m alone (no d, y, h, s): a month — "mmm", "mmmm yy" (yy is caught above)
  if (/[mM]/.test(bare) && !/[#0?]/.test(bare)) return { hint: 'date' }
  if (bare.includes('%')) return { hint: 'percent' }
  if (/^@$/.test(bare.trim())) return { hint: 'text' }
  return known ?? {}
}

function splitSections(code: string): string[] {
  const out: string[] = []
  let cur = ''
  let quoted = false
  for (let i = 0; i < code.length; i++) {
    const c = code[i]
    if (c === '"') quoted = !quoted
    if (c === '\\' && !quoted) { cur += c + (code[i + 1] ?? ''); i += 1; continue }
    if (c === ';' && !quoted) { out.push(cur); cur = ''; continue }
    cur += c
  }
  out.push(cur)
  return out
}

const pad = (n: number, w = 2) => String(n).padStart(w, '0')

/** An Excel date serial number as ISO local text: `YYYY-MM-DD`, or `YYYY-MM-DDTHH:MM:SS[.fff]` with a fraction.
 * The 1900 system counts the 1900-02-29 that never was (serial 60, read as 1900-02-28); 1904 starts at 0. */
export function serialToIso(serial: number, date1904: boolean, withTime: boolean): string | null {
  if (!Number.isFinite(serial) || serial < 0 || serial > 2958465) return null
  let days = Math.floor(serial)
  let ms = Math.round((serial - days) * 86_400_000)
  if (ms >= 86_400_000) { days += 1; ms -= 86_400_000 }
  // days since 1970-01-01
  let epoch: number
  if (date1904) epoch = days - 24107
  else if (days === 0) return null
  else epoch = days >= 61 ? days - 25569 : days === 60 ? -25509 : days - 25568
  const d = new Date(epoch * 86_400_000)
  const date = `${pad(d.getUTCFullYear(), 4)}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}`
  if (!withTime) return date
  const h = Math.floor(ms / 3_600_000), mi = Math.floor(ms / 60_000) % 60, s = Math.floor(ms / 1000) % 60, f = ms % 1000
  return `${date}T${pad(h)}:${pad(mi)}:${pad(s)}${f ? `.${pad(f, 3)}` : ''}`
}
