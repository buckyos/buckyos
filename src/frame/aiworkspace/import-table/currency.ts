/* Currency marks in imported text (表格数据导入 TODO §6.3): ISO 4217 codes, names, unambiguous symbols, and the two
 * symbols several currencies share (`$`, `¥`), resolved per column (Q12). The core only takes codes; everything
 * here is the importer's. */

/** The codes taken as currency marks in text (any three capital letters would catch too much). */
export const ISO_CODES = ['USD', 'EUR', 'JPY', 'GBP', 'CNY', 'HKD', 'TWD', 'KRW', 'SGD', 'AUD', 'CAD', 'CHF', 'INR', 'RUB', 'BRL', 'MXN', 'THB', 'VND', 'MYR', 'IDR', 'PHP', 'NZD', 'SEK', 'NOK', 'DKK', 'PLN', 'TRY', 'ZAR', 'AED', 'SAR', 'CZK', 'HUF', 'ILS']
const CODES = new Set(ISO_CODES)

/** Names users write next to amounts (i18n-exempt: input vocabulary, not interface text). */
const ALIASES: [string, string][] = [
  ['人民币', 'CNY'], ['RMB', 'CNY'], ['￥元', 'CNY'], ['元', 'CNY'], ['美元', 'USD'], ['美金', 'USD'], ['日元', 'JPY'], ['円', 'JPY'], // i18n-exempt: input vocabulary
  ['欧元', 'EUR'], ['英镑', 'GBP'], ['港币', 'HKD'], ['港元', 'HKD'], ['新台币', 'TWD'], ['韩元', 'KRW'],
]

/** Symbols with one meaning, longest first (so `US$` wins over `$`). */
const SYMBOLS: [string, string][] = [
  ['HK$', 'HKD'], ['NT$', 'TWD'], ['US$', 'USD'], ['CN¥', 'CNY'], ['JP¥', 'JPY'], ['R$', 'BRL'], ['A$', 'AUD'], ['C$', 'CAD'], ['S$', 'SGD'],
  ['€', 'EUR'], ['£', 'GBP'], ['₩', 'KRW'], ['₹', 'INR'], ['₫', 'VND'], ['₽', 'RUB'], ['฿', 'THB'], ['₺', 'TRY'],
]

export type Ambiguous = '$' | '¥'

/** The currencies whose own symbol is a bare `$` (a `$` in such a column means the column's currency). */
const DOLLAR_CURRENCIES = new Set(['USD', 'AUD', 'CAD', 'NZD', 'SGD', 'HKD', 'TWD', 'MXN'])

/** What one mark means: a code, an ambiguous symbol, or nothing known. */
export function resolveMark(mark: string): { code: string } | { ambiguous: Ambiguous } | null {
  const m = mark.trim()
  if (!m) return null
  const upper = m.toUpperCase()
  if (/^[A-Z]{3}$/.test(upper) && CODES.has(upper) && /^[A-Za-z]{3}$/.test(m)) return { code: upper }
  for (const [name, code] of ALIASES) if (m === name) return { code }
  for (const [symbol, code] of SYMBOLS) if (m === symbol) return { code }
  if (m === '$') return { ambiguous: '$' }
  if (m === '¥' || m === '￥') return { ambiguous: '¥' } // i18n-exempt: input vocabulary
  return null
}

/** The interface language's currency when nothing else says (zh → CNY, any other → USD). */
export const languageCurrency = (locale: string) => (locale.toLowerCase().startsWith('zh') ? 'CNY' : 'USD')

/** Q12: `$` is the column's currency when that one is written `$`, else USD; `¥` is the column's currency when it
 * is CNY or JPY, else CNY in a Chinese interface and JPY in any other. */
export function resolveAmbiguous(symbol: Ambiguous, columnDefault: string | null, locale: string): string {
  if (symbol === '$') return columnDefault && DOLLAR_CURRENCIES.has(columnDefault) ? columnDefault : 'USD'
  if (columnDefault === 'CNY' || columnDefault === 'JPY') return columnDefault
  return locale.toLowerCase().startsWith('zh') ? 'CNY' : 'JPY'
}

/** Marks a money text may start with, longest first. */
const PREFIXES = [...SYMBOLS.map(([s]) => s), ...ALIASES.map(([a]) => a).filter((a) => a === 'RMB' || a === '人民币'), '$', '¥', '￥'].sort((a, b) => b.length - a.length) // i18n-exempt: input vocabulary
const SUFFIXES = [...ALIASES.map(([a]) => a), ...SYMBOLS.map(([s]) => s), '$', '¥', '￥'].sort((a, b) => b.length - a.length)

/** Split a text into (prefix mark, amount text, suffix mark). Codes may stand on either side, separated or not. */
export function splitMoney(text: string): { prefix: string; amount: string; suffix: string } {
  let t = text.trim().replace(/ /g, ' ')
  let sign = ''
  if (t.startsWith('-') || t.startsWith('+') || t.startsWith('−')) { sign = t[0] === '+' ? '' : '-'; t = t.slice(1).trimStart() }
  let prefix = ''
  const code = /^([A-Za-z]{3})(?=[\s\d.+-])/.exec(t)
  if (code && CODES.has(code[1].toUpperCase())) { prefix = code[1]; t = t.slice(3).trimStart() }
  else for (const p of PREFIXES) if (t.startsWith(p)) { prefix = p; t = t.slice(p.length).trimStart(); break }
  if (prefix && (t.startsWith('-') || t.startsWith('−'))) { sign = sign ? '' : '-'; t = t.slice(1).trimStart() }
  let suffix = ''
  const tail = /[\d.\s]([A-Za-z]{3})$/.exec(t)
  if (tail && CODES.has(tail[1].toUpperCase())) { suffix = tail[1]; t = t.slice(0, -3).trimEnd() }
  else for (const s of SUFFIXES) if (t.endsWith(s)) { suffix = s; t = t.slice(0, -s.length).trimEnd(); break }
  return { prefix, amount: sign + t, suffix }
}

/** Excel locale ids of currency formats (`[$¥-804]`) → code. */
export const LCID_CURRENCY: Record<string, string> = {
  '804': 'CNY', '404': 'TWD', 'c04': 'HKD', '1004': 'SGD', '411': 'JPY', '412': 'KRW', '409': 'USD', '809': 'GBP', '1009': 'CAD', 'c09': 'AUD', '1409': 'NZD',
  '407': 'EUR', '40c': 'EUR', '410': 'EUR', '40a': 'EUR', 'c0a': 'EUR', '413': 'EUR', '816': 'EUR', '40b': 'EUR', 'c07': 'EUR', '813': 'EUR', '80c': 'EUR', '807': 'CHF', '100c': 'CHF',
  '419': 'RUB', '416': 'BRL', '80a': 'MXN', '41e': 'THB', '42a': 'VND', '439': 'INR', '41d': 'SEK', '414': 'NOK', '406': 'DKK', '415': 'PLN', '41f': 'TRY', '436': 'ZAR', '40d': 'ILS', '405': 'CZK', '40e': 'HUF',
}
